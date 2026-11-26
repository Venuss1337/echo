-- M3 library index. Rows describe files; they never own or modify the files.
-- Identity rules: ids are AUTOINCREMENT so a deleted id is never reused by a
-- different track, artist or album. Natural keys (normalized text) decide when
-- two scans describe the same artist or album.

ALTER TABLE library_roots ADD COLUMN added_at INTEGER NOT NULL DEFAULT 0;
-- 0 when the last health check could not reach the folder (unplugged or
-- disconnected drive). Offline roots keep every track row untouched.
ALTER TABLE library_roots ADD COLUMN online INTEGER NOT NULL DEFAULT 1 CHECK (online IN (0, 1));
ALTER TABLE library_roots ADD COLUMN last_complete_scan_at INTEGER;

CREATE TABLE artists (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    key TEXT NOT NULL UNIQUE
) STRICT;

-- Content-addressed thumbnails: `hash` is the SHA-256 of the embedded picture
-- bytes, so identical covers on different tracks share one cached file.
CREATE TABLE artwork (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    hash TEXT NOT NULL UNIQUE CHECK (length(hash) = 64),
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    bytes INTEGER NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE albums (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    title_key TEXT NOT NULL,
    artist_id INTEGER REFERENCES artists(id),
    key TEXT NOT NULL UNIQUE,
    year INTEGER,
    artwork_id INTEGER REFERENCES artwork(id)
) STRICT;
CREATE INDEX albums_title ON albums(title_key, id);
CREATE INDEX albums_artist ON albums(artist_id, year, title_key);

CREATE TABLE tracks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    root_id INTEGER NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
    -- Path relative to the root, original case. path_key is the lowercase form
    -- because NTFS lookups are case-insensitive.
    rel_path TEXT NOT NULL,
    path_key TEXT NOT NULL,
    size INTEGER NOT NULL,
    mtime_ns INTEGER NOT NULL,
    -- 0 present, 1 missing from a complete scan of an online root.
    state INTEGER NOT NULL DEFAULT 0 CHECK (state IN (0, 1)),
    seen_scan INTEGER NOT NULL DEFAULT 0,
    title TEXT NOT NULL,
    title_key TEXT NOT NULL,
    artist_id INTEGER REFERENCES artists(id),
    artist_text TEXT NOT NULL DEFAULT '',
    artist_key TEXT NOT NULL DEFAULT '',
    album_id INTEGER REFERENCES albums(id),
    album_key TEXT NOT NULL DEFAULT '',
    track_no INTEGER,
    disc_no INTEGER,
    year INTEGER,
    genre TEXT NOT NULL DEFAULT '',
    duration_ms INTEGER NOT NULL DEFAULT 0,
    codec TEXT NOT NULL DEFAULT '',
    artwork_id INTEGER REFERENCES artwork(id),
    metadata_version INTEGER NOT NULL,
    UNIQUE (root_id, path_key)
) STRICT;
-- Rename detection compares a new file's size and timestamp to rows whose old
-- path no longer exists.
CREATE INDEX tracks_fingerprint ON tracks(size, mtime_ns);
CREATE INDEX tracks_title ON tracks(title_key, id);
CREATE INDEX tracks_artist ON tracks(artist_key, album_key, disc_no, track_no, id);
CREATE INDEX tracks_album_order ON tracks(album_id, disc_no, track_no, id);
CREATE INDEX tracks_album_sort ON tracks(album_key, album_id, disc_no, track_no, id);
CREATE INDEX tracks_artist_id ON tracks(artist_id);
CREATE INDEX tracks_seen ON tracks(root_id, seen_scan);

CREATE TABLE track_artists (
    track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    artist_id INTEGER NOT NULL REFERENCES artists(id),
    position INTEGER NOT NULL,
    PRIMARY KEY (track_id, artist_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX track_artists_artist ON track_artists(artist_id, track_id);

-- One row per scan attempt. 'running' rows left by a crash are repaired to
-- 'interrupted' at startup; neither status ever removes track rows.
CREATE TABLE scans (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    root_id INTEGER NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    status TEXT NOT NULL CHECK (status IN
        ('running', 'complete', 'partial', 'cancelled', 'failed', 'interrupted', 'offline')),
    discovered INTEGER NOT NULL DEFAULT 0,
    indexed INTEGER NOT NULL DEFAULT 0,
    unchanged INTEGER NOT NULL DEFAULT 0,
    skipped INTEGER NOT NULL DEFAULT 0,
    failed INTEGER NOT NULL DEFAULT 0,
    missing INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE INDEX scans_root ON scans(root_id, id);

-- Service-managed full-text indexes. rowid equals the entity id so updates and
-- deletes are direct. Rebuilding from the relational tables is always possible.
CREATE VIRTUAL TABLE search_tracks USING fts5(
    title, artist, album,
    tokenize = 'unicode61 remove_diacritics 2', prefix = '2 3'
);
CREATE VIRTUAL TABLE search_albums USING fts5(
    title, artist,
    tokenize = 'unicode61 remove_diacritics 2', prefix = '2 3'
);
CREATE VIRTUAL TABLE search_artists USING fts5(
    name,
    tokenize = 'unicode61 remove_diacritics 2', prefix = '2 3'
);
