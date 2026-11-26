# ADR 0008 — Durable user data, listening history and atmosphere

**Status:** accepted for M4, 2026-11-24.

## Media sources

Playlists, favorites and history never reference `tracks`. They reference
`media_sources`, the durable identity of a file the listener cared about.

- A source is created on first use and links to its index row (`track_id`).
- Removing a library root first refreshes the snapshots of its sources (title,
  artist, album, duration, size, timestamp, location, artwork), then deletes the
  index rows. The foreign key clears `track_id` and the source remains as a
  tombstone. Missing files and offline drives keep their link and simply report
  `available = false`.
- `last_track_id` lets a queue entry that still holds the old id resolve to its
  tombstone. Track ids are never reused.
- A source nothing refers to is pruned. Artwork used by a source or a favorite
  album is kept.

### Restoration

A tombstone reconnects only when a newly indexed file offers strong evidence:

1. the same absolute path, and either identical size and timestamp, or the same
   title with a duration within two seconds (re-tagged in place); or
2. the same path below its root, with identical size, timestamp and duration
   (moved folder or changed drive letter), and exactly one such tombstone.

A matching title is never enough, and ambiguous matches stay unavailable. Known
limit: replacing a file in place while its root stays in the library keeps the
index row, so the entry follows the path.

## Playlists, favorites, history

- Entries have their own ids; duplicates are separate rows. Positions are sparse
  integers, so a move rewrites one row and renumbers only when a gap closes.
  Every mutation is one immediate transaction; an unknown id rolls back a batch.
- Favorite albums store the album's natural key and a snapshot, because album
  rows are derived and collected. Folder-scoped album keys became absolute
  (metadata version 2), so equal folder names in two roots no longer merge. The
  first scan after upgrading therefore re-reads every file once.
- A play is recorded once per playback generation, after 30 seconds or half of a
  shorter track has actually been heard. Both position and wall clock must
  advance, so loading, pausing, buffering and seeking add nothing. Repeats are new
  generations. Files opened outside the library are not recorded. The table is
  capped at 50,000 rows, is local only, and is cleared by scope on request.

## Artwork and palette

The 1024 px variant is made on first use from the original embedded picture,
reopened through the checked-open boundary, and only if its SHA-256 still equals
the indexed hash. The regenerable cache is capped at 192 MiB on disk, and the
renderer keeps at most eight idle large covers. Rows only ever request 320 px.

The palette (`ink`, `glow`, `accent`) is extracted in Rust from the thumbnail
and cached per artwork. Contrast is enforced once, at the source: interface text
reads at 7:1 on `ink` and 4.5:1 on `glow`; `accent` reads at 4.5:1 on the
workspace and on `ink`. Monochrome covers keep the house charcoal. Photographs
are shown sharp under an ink wash and are never blurred.

## Presentation

React holds no queue, playlist or favorite authority. Favorites keep a small
overlay of confirmed changes so views agree without refetching pages. The compact
edition is a presentation of the same window and subscription. An always-on-top
native mini window is deferred to M6.
