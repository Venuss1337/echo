# Library, storage and user data

ECHO indexes music folders the user chooses into a local SQLite database and
keeps playlists, favorites and listening history beside it. The index may be
rebuilt at any time; the user's data and music files must never be lost to
it. Decisions are recorded in [ADR 0007](ADR/0007-library-index-and-scanner.md)
and [ADR 0008](ADR/0008-durable-user-data.md).

## Database

- One database per user in ECHO's application data directory, opened with
  WAL, foreign keys and a busy timeout. All SQL uses bound parameters.
- Migrations live in `src-tauri/migrations` (`001` … `005`). Each runs in one
  immediate transaction together with the `user_version` bump, so a failed
  migration leaves the previous schema intact. They are forward-only, and
  the test suite upgrades real databases from every earlier version.
- An incompatible, newer or corrupt database is reported as unavailable; it
  is never reset or deleted to make startup succeed.
- Reads use a pool of read-only connections. The index has a single writer;
  user data mutations use their own connection and immediate transactions.

## Model

| Area | Tables (migration) | Identity |
| --- | --- | --- |
| Roots and scans | `library_roots`, `scans` (001, 003) | Root ID; scans record every attempt and its outcome. |
| Index | `tracks`, `artists`, `albums`, `artwork`, three FTS5 tables (003) | `AUTOINCREMENT` IDs, never reused. Paths are stored relative to the root with a case-folded key. |
| Durable sources | `media_sources` (004) | What playlists, favorites and history point at; survives its track row. |
| User data | `playlists`, `playlist_entries`, `favorite_tracks`, `favorite_albums`, `plays` (004) | Entries have their own IDs; duplicates are separate rows. |
| Playback | `playback_session` (002) | The queue and mixer checkpoint. |
| Radio | stations, directory pages, station artwork, preferences (005) | Radio Browser UUID, or a random ID for manual stations. |

### Identity rules

- Artists are keyed by case-folded name. Albums are keyed by album artist and
  title when an album-artist tag exists, otherwise by containing folder and
  title; a title alone is never an identity.
- A file that moved within a root keeps its track ID when its size and
  timestamp match a row whose file is gone.
- Playlists, favorites and history reference **media sources**, not tracks.
  Removing a root snapshots each affected source's metadata, deletes the
  index rows and leaves the source as a tombstone. A tombstone reconnects
  only on strong evidence (same path with matching size and timestamp or a
  re-tag of the same recording; or the same relative path with identical
  size, timestamp and duration, and exactly one candidate). Ambiguous
  matches stay unavailable rather than guessing.

## Scanning

```
walker ─► bounded channel ─► ≤ 4 metadata workers ─► bounded channel ─► one writer
```

- Roots come from the native folder picker and must be local drive folders;
  duplicates and nested roots are rejected.
- The walker does not follow or read junctions, symbolic links, cloud
  placeholders or offline files, which also rules out loops. Every file is
  opened through the same checked, read-only open used for playback.
- Unchanged files (same size, timestamp and extractor version) are stamped as
  seen without being opened. A rescan of 20,000 unchanged files takes a
  fraction of a second.
- Memory is bounded by channel capacities, a 200-row writer batch and one
  artwork decode at a time, not by library size. Each batch is one
  transaction, so cancelling, crashing or a full disk keeps what committed.
- **A scan never deletes a track.** It can flag a track missing only after a
  complete walk that listed every directory. A root that is unreachable, or
  whose complete walk finds none of its indexed files, is marked offline and
  nothing is flagged. Only the user removing a root deletes its index rows.
- `notify` events are hints that schedule an incremental rescan. The service
  also checks root reachability every five seconds, rescans every online
  root at startup and every 15 minutes, and rescans a root that comes back.
- Progress is coalesced to at most about six updates a second and reports a
  count until discovery has finished.

## Metadata and artwork

- Lofty reads tags in best-attempt mode under an 8 MiB per-item allocation
  limit. Strings are cleaned of control and bidirectional override
  characters and capped at 512 characters.
- Embedded pictures (JPEG and PNG only) are decoded only when the header
  reports at most 8,192 px per side and 16 megapixels, under an allocation
  limit. The result is a 320 px JPEG thumbnail in a content-addressed cache.
  A 1,024 px variant is made on first use from the original picture, only if
  its hash still matches, within a 192 MiB on-disk cache.
- Each artwork gets a palette (`ink`, `glow`, `accent`) extracted in Rust and
  contrast-checked at the source, which the interface uses for its light.
- Missing or rejected art gets a deterministic placeholder.

## Search

Three FTS5 tables (tracks, albums, artists) use `unicode61` with diacritics
removed and 2- and 3-character prefix indexes. Input becomes quoted
alphanumeric prefix terms, so no FTS operator can be injected. The indexes
are checked at startup and can be rebuilt from the relational tables.

## Playlists, favorites and history

- Playlist positions are sparse integers, so a move rewrites one row. Every
  mutation is one transaction; an unknown ID rolls back the whole batch.
- A play is recorded once per playback generation after 30 seconds, or half
  of a shorter track, has actually been heard: both the position and the wall
  clock must advance. Files opened outside the library are not recorded.
  History is capped at 50,000 rows and can be cleared by scope.
- None of this data leaves the device.

## Known limits

Network shares are not supported. A rename that also changes the timestamp
becomes a new track. Ligatures such as Œ do not match their expansions in
search. Tag editing and online metadata are not implemented.

## Tests

`bun run test:db` runs every `library::` test against real temporary
directories and databases: migrations from each earlier version, identity
rules, FTS consistency and rebuild, pagination, the scanner (renames, missing
and restored files, corrupt and truncated files, oversized covers, Unicode
and long paths, a real junction loop and escape, a deny-ACL directory, a
directory swapped for a junction mid-scan, a full disk, cancellation),
tombstones, restoration, playlists, favorites and history.
`bun run test:library-scale` times 10,000- and 20,000-track imports and
queries; see [Performance](PERFORMANCE.md).
