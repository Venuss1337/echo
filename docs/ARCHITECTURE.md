# Architecture

ECHO is one Tauri 2 desktop application. A Rust host process owns every
capability that matters: playback, files, the database, networking and the
operating system. A React renderer in WebView2 presents what the host
confirms and sends it typed intents. There is no embedded HTTP server, no
remote code, and no audio in the page.

```
React renderer (WebView2)        views, design system, query cache, mirrored player state
  │  typed Tauri commands and channels (generated TypeScript from Rust)
Rust host
  ├── playback worker ─── libmpv (reviewed callback-only build) ─── WASAPI
  ├── library service ─── SQLite (WAL, FTS5) · scanner · artwork cache
  ├── radio ──────────── Radio Browser directory · pinned HTTPS connector
  ├── Windows ────────── window chrome · mini player · tray · SMTC · single instance
  └── startup, diagnostics, persistence
```

## Why this shape

- **Tauri and WebView2** give a small native shell with a capability system,
  so the page can be granted exactly the commands it needs and nothing else.
  ([ADR 0001](ADR/0001-local-first-tauri.md))
- **libmpv** provides mature decoding, buffering and output. One engine,
  behind a minimal FFI, avoids maintaining a media stack and a fallback.
  ([ADR 0002](ADR/0002-libmpv-and-gpl.md))
- **Rust owns state** so behaviour can be tested from the command line
  without a window, and so a compromised or buggy page cannot reach files,
  sockets or the decoder. ([ADR 0003](ADR/0003-headless-first-testing.md))

## Source layout

| Path | Contents |
| --- | --- |
| `src-tauri/src/lib.rs` | Composition root: services, commands, capabilities, lifecycle. |
| `src-tauri/src/contract.rs` | Every IPC wire type; generates `src/generated/contracts.ts`. |
| `src-tauri/src/playback*` | State machine, queue, the playback worker, the libmpv FFI, local sources. See [Playback](PLAYBACK.md). |
| `src-tauri/src/library*` | SQLite bootstrap and migrations, index, queries, scanner, watcher, artwork, palettes, playlists, favorites, history. See [Library](LIBRARY.md). |
| `src-tauri/src/radio*` | Directory client, station store, the pinned HTTPS connector, ICY handling, station artwork. See [Radio](RADIO.md). |
| `src-tauri/src/window.rs`, `tray.rs`, `media_session.rs`, `geometry.rs`, `diagnostics.rs` | Windows integration. See [Windows](WINDOWS.md). |
| `src-tauri/src/native.rs` | Verified loading of the packaged decoder DLLs. |
| `src-tauri/migrations` | Forward-only SQL migrations. |
| `src-tauri/capabilities` | The single main-window capability listing every allowed command. |
| `src/` | The React renderer. See [Frontend](FRONTEND.md) and [Design](DESIGN.md). |
| `native/windows-x64` | Decoder build recipe, staging, installer, release and real-window test scripts. See [Native decoder](NATIVE.md). |
| `scripts/` | Fixture generation, contract and notice tooling, desktop test drivers. |
| `tests/` | Browser tests, fixtures and visual baselines. Rust tests live beside their modules. |

## Ownership rules

- **One writer per domain.** The playback worker is the only code that
  changes playback state; it serializes commands from the page, the tray and
  the system media controls through one bounded mailbox. The library index
  has a single writer; reads use separate read-only connections, so a scan
  never blocks a page.
- **Confirmed state flows out.** Commands are acknowledged as accepted.
  Observable state (`PlayerView`, `LibrarySummary`, `WindowView`) arrives as
  complete snapshots with monotonic revisions over channels; the renderer
  ignores anything not newer than what it shows.
- **The page names, Rust resolves.** The renderer sends IDs, enums and bounded
  numbers. Rust turns an ID into a file or station, authorizes it at the
  moment of use, and never returns a path to open.
- **Background work is bounded.** Scans, radio fetches and artwork decoding
  run off the UI thread with bounded channels, batch sizes, timeouts and
  byte limits, and are cancellable.
- **Platform code stays at the edge.** Windows APIs live in the host's
  platform modules; the renderer contains no Windows-specific behaviour
  beyond presenting window state.

## Data and persistence

One SQLite database per user (WAL, foreign keys, migrations applied
atomically with the schema version) holds the library index, playlists,
favorites, history, radio stations and directory cache, and the playback
checkpoint. Artwork thumbnails and large covers are a regenerable,
content-addressed cache beside it. Nothing is stored remotely.

## Errors

Rust errors are typed per domain and reach the renderer as small, closed
sets of machine-readable codes (for example `outputUnavailable`,
`fileUnavailable`, `decodeFailed`). They never carry paths, URLs, response
bodies or native error text. Malformed media and network data are ordinary
errors, not panics.

## Lifecycle

Startup opens the database and verifies the decoder on a worker while the
window appears; nothing plays or connects until the user asks. Shutdown
stops playback, writes the final listening event and checkpoint, tears down
the decoder once and ends within a ten-second bound. See
[Windows integration](WINDOWS.md#startup-and-shutdown).

## Technology

| Layer | Choice |
| --- | --- |
| Shell | Tauri 2, WebView2 (Evergreen runtime) |
| Host | Rust (toolchain pinned in `rust-toolchain.toml`), Tokio for network I/O |
| Audio | libmpv 0.42.0 with FFmpeg 9.1, built from source |
| Storage | SQLite via rusqlite (bundled), FTS5 |
| Metadata and images | Lofty; `image` with JPEG and PNG only |
| Networking | reqwest with Rustls and bundled Mozilla roots, HTTP/1 |
| Renderer | React 19, TypeScript (strict), Vite, React Router, TanStack Query and Virtual, Tailwind CSS 4 with authored tokens |
| Contracts | ts-rs generated from Serde types |
| Tests | Cargo tests, Vitest, Testing Library, Playwright (Chromium), PowerShell real-window harnesses |
| Packaging | Tauri NSIS installer, per-user |

Exact versions are the lockfiles: `src-tauri/Cargo.lock` and `bun.lock`.
