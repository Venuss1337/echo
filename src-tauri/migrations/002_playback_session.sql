CREATE TABLE playback_session (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    payload TEXT NOT NULL CHECK (length(payload) <= 4194304)
) STRICT;
