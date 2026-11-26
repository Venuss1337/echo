CREATE TABLE radio_stations (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    stream_url TEXT NOT NULL,
    artwork_url TEXT,
    country TEXT NOT NULL,
    language TEXT NOT NULL,
    tags TEXT NOT NULL,
    codec TEXT NOT NULL,
    bitrate INTEGER NOT NULL,
    reported_online INTEGER,
    manual INTEGER NOT NULL CHECK (manual IN (0, 1)),
    favorite INTEGER NOT NULL DEFAULT 0 CHECK (favorite IN (0, 1)),
    heard_at INTEGER,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE INDEX radio_recent ON radio_stations (heard_at DESC) WHERE heard_at IS NOT NULL;
CREATE INDEX radio_ephemeral ON radio_stations (updated_at) WHERE manual = 0 AND favorite = 0 AND heard_at IS NULL;

CREATE TABLE radio_preferences (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    sort TEXT NOT NULL CHECK (sort IN ('popular', 'name', 'recent'))
) STRICT;

CREATE TABLE radio_directory_cache (
    query TEXT PRIMARY KEY NOT NULL,
    response TEXT NOT NULL,
    fetched_at INTEGER NOT NULL
) STRICT;

CREATE TABLE radio_artwork_cache (
    id TEXT PRIMARY KEY NOT NULL REFERENCES radio_stations (id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    raster BLOB NOT NULL,
    palette TEXT NOT NULL,
    fetched_at INTEGER NOT NULL
) STRICT;
