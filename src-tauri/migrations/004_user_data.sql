-- M4 user data. Nothing here depends on a tracks row staying alive.
--
-- media_sources is the durable identity of a file the user cared about. It
-- points at the live index row while one exists and keeps a snapshot of its
-- metadata for when it does not. Removing a library root deletes tracks; the
-- foreign key then clears track_id and the source becomes a tombstone.
CREATE TABLE media_sources (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id INTEGER UNIQUE REFERENCES tracks(id) ON DELETE SET NULL,
    -- Survives track deletion so queue entries holding the old id still resolve.
    -- tracks.id is AUTOINCREMENT and therefore never reused.
    last_track_id INTEGER NOT NULL,
    abs_key TEXT NOT NULL,
    rel_key TEXT NOT NULL,
    size INTEGER NOT NULL,
    mtime_ns INTEGER NOT NULL,
    title TEXT NOT NULL,
    artist TEXT NOT NULL,
    album TEXT NOT NULL,
    duration_ms INTEGER NOT NULL,
    artwork_id INTEGER REFERENCES artwork(id)
) STRICT;
CREATE INDEX media_sources_last ON media_sources(last_track_id);
CREATE INDEX media_sources_lost ON media_sources(size, mtime_ns) WHERE track_id IS NULL;
CREATE INDEX media_sources_lost_path ON media_sources(abs_key) WHERE track_id IS NULL;
CREATE INDEX media_sources_artwork ON media_sources(artwork_id);

CREATE TABLE playlists (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    description TEXT NOT NULL DEFAULT '' CHECK (length(description) <= 2000),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

-- position is sparse so a move normally rewrites one row. Entry ids are the
-- stable identity; the same source may appear any number of times.
CREATE TABLE playlist_entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    source_id INTEGER NOT NULL REFERENCES media_sources(id),
    position INTEGER NOT NULL,
    added_at INTEGER NOT NULL,
    UNIQUE (playlist_id, position)
) STRICT;
CREATE INDEX playlist_entries_source ON playlist_entries(source_id);

CREATE TABLE favorite_tracks (
    source_id INTEGER PRIMARY KEY REFERENCES media_sources(id),
    created_at INTEGER NOT NULL
) STRICT;

-- Album rows are derived data and are collected when empty, so a favorite
-- keeps the album's natural key and a snapshot.
CREATE TABLE favorite_albums (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    album_key TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    artist TEXT NOT NULL,
    artwork_id INTEGER REFERENCES artwork(id),
    created_at INTEGER NOT NULL
) STRICT;

-- One row per playback generation that passed the listening threshold.
CREATE TABLE plays (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_id INTEGER NOT NULL REFERENCES media_sources(id),
    album_key TEXT,
    started_at INTEGER NOT NULL,
    listened_ms INTEGER NOT NULL CHECK (listened_ms >= 0)
) STRICT;
CREATE INDEX plays_recent ON plays(started_at DESC, id DESC);
CREATE INDEX plays_source ON plays(source_id);
CREATE INDEX plays_album ON plays(album_key, started_at);

-- Colors derived from the cached thumbnail as JSON, or NULL until requested.
ALTER TABLE artwork ADD COLUMN palette TEXT;
