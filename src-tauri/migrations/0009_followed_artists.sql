-- 0009: followed Spotify artists and the releases found for them

CREATE TABLE followed_artists (
    id             TEXT PRIMARY KEY,           -- Spotify artist id
    name           TEXT NOT NULL,
    image_url      TEXT,
    genres         TEXT NOT NULL DEFAULT '',   -- comma separated
    auto_sync      INTEGER NOT NULL DEFAULT 1,
    added_at       TEXT NOT NULL,
    last_synced_at TEXT,                       -- last successful sync
    last_attempt_at TEXT,
    last_error     TEXT
);

-- Releases whose tracks were read completely (a release is only recorded
-- after all of its tracks were stored, so an interrupted sync redoes it).
CREATE TABLE followed_artist_releases (
    artist_id    TEXT NOT NULL REFERENCES followed_artists(id) ON DELETE CASCADE,
    album_id     TEXT NOT NULL,
    name         TEXT NOT NULL,
    album_type   TEXT NOT NULL,
    release_date TEXT NOT NULL,
    PRIMARY KEY (artist_id, album_id)
);

CREATE TABLE followed_artist_tracks (
    artist_id    TEXT NOT NULL REFERENCES followed_artists(id) ON DELETE CASCADE,
    track_id     TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    album_id     TEXT NOT NULL,
    album_type   TEXT NOT NULL,
    release_date TEXT NOT NULL,
    is_new       INTEGER NOT NULL DEFAULT 0,
    found_at     TEXT NOT NULL,
    PRIMARY KEY (artist_id, track_id)
);

CREATE INDEX idx_followed_artist_tracks_new ON followed_artist_tracks (artist_id, is_new);
