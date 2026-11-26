# ADR 0007 — Durable local library: index, scanner and IPC

**Status:** accepted for M3, 2026-11-24. **Owner:** Rust library service.

## Context

M2 plays files the user picks one at a time. M3 must turn ECHO into a library
application: user-chosen music folders, an index that survives restarts, search,
artwork, and playback from the index. The index must never become a way to lose
user data, and the renderer must stay unable to name a file path.

## Decisions

### 1. Rows describe files; scans never delete tracks

- Track, artist, album and artwork ids are `AUTOINCREMENT`, so an id is never
  reused for a different thing. The renderer sees ids as plain numbers (`u32`).
- A scan can only *flag* a track `state = 1` (missing). It does so only when
  the walker visited every directory it could list, and never for tracks below
  a directory that could not be listed (permission errors, deep trees).
- A complete walk of a root that has indexed tracks but finds none of them is
  treated as an unreachable or remounted drive: nothing is flagged and the
  root is marked offline.
- An unreachable root is marked `online = 0`. Its tracks keep their rows and are
  reported as unavailable; they return on their own when the folder does.
- Cancelling, crashing or filling the disk mid-scan leaves committed batches in
  place (each batch is one transaction) and flags nothing. A `running` scan row
  left by a crash is rewritten to `interrupted` at startup.
- Only the user removing a root deletes its index rows, in one transaction, along
  with artists, albums and cached thumbnails nothing references. Music files are
  never touched. M4 must re-evaluate this when playlists and history reference
  tracks: they should tombstone instead of cascading.

### 2. Identity and relationships

- Artists are keyed by case-folded name. "The Cure" and "Cure" are different.
- Albums are keyed by album-artist plus title when an album-artist tag exists,
  and by containing folder plus title when it does not. Title alone is never
  an identity. Two different primary artists in one untagged folder make the
  album "Various Artists".
- A renamed or moved file keeps its id: a new path whose size and timestamp
  match a row whose old file is gone takes over that row.
- Paths are stored relative to the root with the original case, and a lowercase
  `path_key` (NTFS is case-insensitive). Names that are not valid Unicode are
  skipped and counted, because they cannot be stored faithfully.

### 3. Scanner pipeline and bounds

`walker → bounded channel → ≤4 metadata workers → bounded channel → one writer`.
Memory is bounded by the channel capacities (256), the writer batch (200 rows)
and a process-wide artwork decode gate that allows one decode at a time, not by
library size. Unchanged files (same size, timestamp and extractor version) are
stamped as seen without opening them. Progress is coalesced to ≤ 5 updates a
second; the total is reported only when discovery has finished, so the UI never
shows a percentage it cannot support.

### 4. Reparse points, junctions and local-file authority

The walker neither follows nor reads junctions, symlinks, cloud placeholders
or offline files, which also removes loops. Every file is opened through the
M2 `open_selected` boundary, which refuses reparse points on any ancestor, so a
junction swapped in mid-scan cannot redirect a read. Roots are validated with
the same rules (`open_directory`): local drive only, no UNC or device names.
Playback of library tracks resolves ids to paths in Rust and reauthorizes the
file when it is opened; the decoder network isolation of ADR 0006 is unchanged.

### 5. Metadata and artwork

Lofty parses in best-attempt mode under an 8 MiB per-item allocation limit. All
strings are cleaned (controls and bidirectional overrides removed, whitespace
collapsed) and capped at 512 characters. A file whose container is not
recognized is counted as failed and not indexed.

Covers are decoded only after the header reports ≤ 8192 px per side and ≤ 16
megapixels, under a 160 MiB allocation limit, and only JPEG and PNG. The result
is a 320 px JPEG cached under `artwork/<hh>/<sha256-of-source>.jpg`, written
atomically. Missing, malformed and oversized pictures leave the track without
art and show a deterministic placeholder chosen by hashing the album text.
Thumbnails reach the renderer as raw bytes over IPC and become `blob:` URLs, so
the CSP gains only `img-src blob:`.

### 6. Search

Three service-managed FTS5 tables (tracks, albums, artists) use
`unicode61 remove_diacritics 2` with 2- and 3-character prefix indexes. User
text is reduced to quoted alphanumeric prefix terms, so no FTS operator can be
injected. Indexes can be rebuilt from the relational tables at any time and are
checked at startup. Known limit: ligatures such as Œ are letters of their own
and do not match "oe".

### 7. Watching

`notify` events are hints only. A hint schedules an incremental rescan after a
quiet period; the rescan compares the real tree to the index. The service also
re-checks root reachability every 5 s, rescans every online root at startup and
every 15 minutes, and rescans a root that comes back online.

### 8. IPC surface

Twelve main-window-only commands with scoped permissions: summary, subscribe,
add/remove root, scan, cancel, tracks/albums/artists pages (≤ 200 rows), search
(≤ 40 per kind), artwork bytes and play. Queries run on a pool of read-only
SQLite connections, so a scan never blocks a page. Folder paths come from the
native picker; the renderer sends only ids and bounded query values.

## Dependencies

| Crate | Version | License | Use |
|---|---|---|---|
| lofty | 0.25.5 | MIT OR Apache-2.0 | tags, properties, embedded pictures |
| image | 0.25.10 (`jpeg`, `png` only) | MIT OR Apache-2.0 | bounded decode, thumbnail, JPEG encode |
| notify | 9.0.0 | CC0-1.0 | change hints (added to the cargo-deny allow list) |
| sha2 | 0.11 | MIT OR Apache-2.0 | content hash (moved from Windows-only) |
| @tanstack/react-virtual | 3.14.28 | MIT | virtualized lists |

## Supply-chain notes

`cargo deny` also needed BSL-1.0 (permissive, GPL-compatible; used only by `xxhash-rust` via Lofty), added to the allow list. `cargo audit` reports one new unmaintained-crate warning, `paste` (RUSTSEC-2024-0436), a build-time macro pulled in by Lofty; it joins the three existing warnings and ships no runtime code.

## Consequences

- Positive: a failed, cancelled or confusing scan cannot remove or hide user data;
  large libraries stay responsive; search and pagination cost is independent of
  library size.
- Negative: the index duplicates some filesystem facts; renames across roots are
  new tracks; WAL files appear beside the database; one decode at a time limits
  artwork throughput on first import.
- Not done: online metadata, tag editing, network shares, and rename detection
  when the timestamp changes. Playlists, favorites and history are M4.
