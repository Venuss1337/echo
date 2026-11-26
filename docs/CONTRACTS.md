# IPC contracts

The renderer talks to the host only through Tauri commands and channels.
This document describes the rules every command follows and lists the
surface. The types themselves are the source of truth: Serde structs and
enums in `src-tauri/src/contract.rs`, generated into
`src/generated/contracts.ts` by `bun run gen:contracts`. CI fails if the
generated file drifts (`bun run test:contracts`). Never edit it by hand.

## Rules

- **Allowlisted.** Every command is registered in `lib.rs` and granted
  individually in `src-tauri/capabilities/default.json` to the `main` window
  only. No Tauri plugin permission (`core:window`, file system, shell,
  dialog, HTTP) is granted to the page.
- **IDs in, views out.** Inputs are IDs, enums, bounded numbers and short
  bounded text. The page never sends a path, URL (except in the manual
  station editor), command name or decoder option, and never receives a path
  it could open. Library folder paths are returned as display text only.
- **Validated in Rust.** Unknown fields are rejected. Pages hold 1–200 rows;
  batches at most 1,000 IDs; names 120 and descriptions 500 characters after
  cleaning; search text at most 200 characters.
- **Accepted is not confirmed.** A command's success means the host accepted
  it. Observable state arrives as complete snapshots on channels.
- **Monotonic snapshots.** `PlayerView`, `LibrarySummary` and `WindowView`
  carry revisions. Revisions, generations and queue IDs are serialized as
  decimal strings so 64-bit values survive JavaScript. Subscribe first, then
  fetch the snapshot, and accept only strictly newer revisions; because each
  message is complete, a skipped revision loses nothing.
- **Closed error sets.** Each family returns a small enum of
  machine-readable faults (`PlayerFault`, `LibraryFault`, `RadioFault`,
  `WindowFault`, …). No stack traces, paths, response bodies or native
  messages cross the boundary.

## Surface

| Family | Commands | Notes |
| --- | --- | --- |
| Status | `app_get_status` | Library bootstrap state: `opening`, `empty` once SQLite is ready, or `unavailable`. |
| Player | `player_subscribe`, `player_snapshot`, `player_act`, `player_pick` | `player_act` takes a tagged `PlayerIntent` (play by entry ID, pause, resume, stop, next, previous, seek with its generation, volume, mute, repeat, shuffle, remove and move entries). `player_pick` opens the native audio file dialog in Rust. |
| Library | `library_summary`, `library_subscribe`, `library_add_root`, `library_remove_root`, `library_scan`, `library_cancel_scan` | Roots come from the native folder picker. Removing a root deletes index rows only, never files. One scan at a time. |
| Browse | `library_tracks`, `library_albums`, `library_artists`, `library_track`, `library_album`, `library_artist`, `library_home`, `library_search` | Paginated, stable sort orders. Search text becomes quoted prefix terms; no FTS syntax passes through. |
| Artwork | `library_artwork`, `library_palette` | Raw JPEG bytes (320 px or 1024 px) from the content-addressed cache, shown as `blob:` URLs; a contrast-checked palette per artwork. |
| Play from library | `library_play` | A `PlayTarget` (track, album, artist, playlist, favorites or a list) and a mode; at most 1,024 queued entries; unavailable tracks are skipped. |
| User data | `playlists_list`, `playlist_get`, `playlist_entries`, `playlist_act`, `favorite_tracks`, `favorite_albums`, `favorite_act`, `history_page`, `history_clear` | `playlist_act` is a tagged union (create, edit, delete, add tracks, add album, remove, move). Entries are named by entry ID, never by index. |
| Radio | `radio_search`, `radio_cancel`, `radio_collection`, `radio_station`, `radio_edit`, `radio_collection_act`, `radio_preferences`, `radio_artwork`, `radio_playback_available`, `radio_play` | Directory results expose stable station IDs, never stream or artwork URLs. `radio_edit` is the only command that accepts a URL; it validates syntax and literal host without contacting the station. |
| Window | `window_subscribe`, `window_act`, `window_caption` | `window_act` takes one of five fixed intents; see [Windows](WINDOWS.md). |

## Player view

`PlayerView` holds the revision, the source generation, the state, position
and duration in milliseconds (bounded to safe integers), whether the source
is seekable, volume, mute, repeat, shuffle, the queue (`tracks`, `current`,
`hasPrevious`, `hasNext`), the last fault and, during radio, a `live` block
with the station and its connection state. Titles are
sanitized display text. The page interpolates position only while the state
is `playing` and the window is visible, and snaps back on every snapshot.

## Library summary

`LibrarySummary` holds a revision, a user-data revision, the roots (display
path, online state, counts), totals and the live scan (`ScanView`). A scan
reports files found and processed; `totalKnown` stays false until discovery
finishes, so the interface shows a count, never an invented percentage. A
track row's `available` is false when its file was not found by the last
complete scan or its root is offline.

## Changing a contract

1. Change the Rust type and its validation; add Rust tests for the new
   bounds and failure cases.
2. Run `bun run gen:contracts` and update the renderer's boundary validation.
3. Add the command to the capability file if it is new, and explain in the
   pull request why the page needs it.
4. Update this document. A change that widens what the page can do also
   needs a note in [Security](SECURITY.md).
