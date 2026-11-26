# Development

## Prerequisites

Windows 10 or 11, x64, with:

- [PowerShell 7](https://learn.microsoft.com/powershell/) for repository
  scripts.
- [Rust](https://rustup.rs/) via rustup. `rust-toolchain.toml` pins the
  toolchain (1.100.0 with rustfmt and Clippy); rustup installs it on first
  use.
- [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
  with the "Desktop development with C++" workload and a Windows SDK, as
  required by [Tauri](https://v2.tauri.app/start/prerequisites/).
- [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)
  (preinstalled on current Windows).
- [Bun](https://bun.sh/) 1.4.5 (`packageManager` in `package.json`) and
  Node.js 22 for tools that run under Node.
- Git.

## First run

```powershell
git clone https://github.com/Venuss1337/echo.git
cd echo
bun install --frozen-lockfile
bunx playwright install chromium
bun run verify
```

`bun run verify` builds the renderer and runs every static check, all Rust
tests and the browser suite. Expect a few minutes on the first run while
Cargo compiles dependencies.

## Running the app

```powershell
bun run dev          # renderer only, in a browser, with an empty fixture backend
bun run tauri dev    # the desktop app with hot reload
```

`bun run tauri dev` has no decoder, so the library, playlists, window and
tray all work but playback reports audio as unavailable. To develop with
audio, build and stage the decoder once ([Native decoder](NATIVE.md)) and
run a debug build against it:

```powershell
$env:ECHO_NATIVE_DLL_MANIFEST = 'C:/echo-native-stage/dlls.json'
bun run tauri build --debug --features native-resources -- --locked
```

## Day-to-day

- Change a wire type in `src-tauri/src/contract.rs`, then run
  `bun run gen:contracts`.
- Add a migration as a new numbered file in `src-tauri/migrations`.
- `bun run check` is the fast pre-commit loop; run `bun run verify` before
  opening a pull request.
- Narrow Rust runs: `cargo test --locked --manifest-path src-tauri/Cargo.toml radio::`.
- One browser spec: `bunx playwright test tests/browser/library.spec.ts`.

## Where things are

See [Architecture](ARCHITECTURE.md#source-layout) for the source layout and
[Testing](TESTING.md) for every suite. Contributor expectations are in
[CONTRIBUTING.md](../CONTRIBUTING.md).

## Data

The test harnesses run under disposable application identifiers and delete
their profiles afterwards; tests never read personal music. `bun run tauri
dev` and plain debug builds use the production identifier
`org.echo.player`, so they share the database in
`%LOCALAPPDATA%\org.echo.player` with an installed ECHO. Use a separate
Windows account, or back that folder up, if you develop on a machine where
you also listen with ECHO.
