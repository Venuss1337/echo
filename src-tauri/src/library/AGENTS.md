# Library, SQLite and persistence

Read `docs/LIBRARY.md` and the ADRs it links before changing schema or scanning.

- Migrations in `src-tauri/migrations` are forward-only and applied
  atomically with the schema version. Add a new numbered file; never edit a
  released one. Test the upgrade from every earlier version.
- `index.rs` is the only writer of library rows. User data (playlists,
  favorites, history) must survive a missing file, an offline root, a
  cancelled scan and root removal; tombstone, never delete.
- Music files are read-only to ECHO. Every read goes through the checked
  open used by playback; junctions, symlinks, UNC paths and cloud
  placeholders are not followed.
- Tag text and embedded artwork are untrusted: keep the length, pixel and
  allocation limits in `text.rs`, `meta.rs` and `artwork.rs`.
- All SQL uses bound parameters. Paginated queries keep stable ordering and
  the 1–200 row limit.
- Tests: `bun run test:db`; timings with `bun run test:library-scale`.
