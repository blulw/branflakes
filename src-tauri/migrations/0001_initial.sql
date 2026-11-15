-- 0001: initial unified library schema

CREATE TABLE tracks (
    id                  TEXT PRIMARY KEY,                 -- stable internal UUID
    source              TEXT NOT NULL CHECK (source IN ('local', 'spotify', 'soundcloud')),
    provider_id         TEXT NOT NULL,                    -- normalised path / Spotify ID / SoundCloud URN
    provider_uri        TEXT,                             -- spotify:track:... / SoundCloud permalink
    title               TEXT NOT NULL,
    artist              TEXT NOT NULL DEFAULT '',
    album               TEXT NOT NULL DEFAULT '',
    album_artist        TEXT NOT NULL DEFAULT '',
    track_number        INTEGER,
    disc_number         INTEGER,
    year                INTEGER,
    genre               TEXT NOT NULL DEFAULT '',
    duration_ms         INTEGER,
    artwork_url         TEXT,
    file_path           TEXT,
    file_format         TEXT,
    file_size           INTEGER,
    file_mtime          INTEGER,
    availability        TEXT NOT NULL DEFAULT 'available'
                        CHECK (availability IN ('available', 'preview_only', 'unavailable')),
    unavailable_reason  TEXT,
    permalink_url       TEXT,
    uploader            TEXT,
    uploader_url        TEXT,
    in_library          INTEGER NOT NULL DEFAULT 1,       -- 0 = only referenced by search/queue/playlist
    added_at            TEXT NOT NULL,
    updated_at          TEXT NOT NULL,
    UNIQUE (source, provider_id)
);

CREATE INDEX idx_tracks_source ON tracks (source);
CREATE INDEX idx_tracks_artist ON tracks (artist COLLATE NOCASE);
CREATE INDEX idx_tracks_album  ON tracks (album COLLATE NOCASE);
CREATE INDEX idx_tracks_title  ON tracks (title COLLATE NOCASE);

-- App-level metadata overrides. Apply to every source; never written to provider.
CREATE TABLE track_overrides (
    track_id      TEXT PRIMARY KEY REFERENCES tracks (id) ON DELETE CASCADE,
    title         TEXT,
    artist        TEXT,
    album         TEXT,
    album_artist  TEXT,
    genre         TEXT,
    year          INTEGER,
    track_number  INTEGER,
    artwork_path  TEXT,
    updated_at    TEXT NOT NULL
);

CREATE VIEW tracks_effective AS
SELECT
    t.id, t.source, t.provider_id, t.provider_uri,
    COALESCE(o.title, t.title)                 AS title,
    COALESCE(o.artist, t.artist)               AS artist,
    COALESCE(o.album, t.album)                 AS album,
    COALESCE(o.album_artist, t.album_artist)   AS album_artist,
    COALESCE(o.track_number, t.track_number)   AS track_number,
    t.disc_number,
    COALESCE(o.year, t.year)                   AS year,
    COALESCE(o.genre, t.genre)                 AS genre,
    t.duration_ms,
    COALESCE(o.artwork_path, t.artwork_url)    AS artwork,
    t.file_path, t.file_format, t.availability, t.unavailable_reason,
    t.permalink_url, t.uploader, t.uploader_url, t.in_library,
    (o.track_id IS NOT NULL)                   AS has_overrides,
    t.added_at
FROM tracks t
LEFT JOIN track_overrides o ON o.track_id = t.id;

CREATE TABLE library_folders (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    path             TEXT NOT NULL UNIQUE,
    added_at         TEXT NOT NULL,
    last_scanned_at  TEXT
);

CREATE TABLE playlists (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    description  TEXT NOT NULL DEFAULT '',
    origin       TEXT,          -- e.g. 'spotify:playlist:ID' when imported
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

CREATE TABLE playlist_items (
    playlist_id  TEXT NOT NULL REFERENCES playlists (id) ON DELETE CASCADE,
    position     INTEGER NOT NULL,
    track_id     TEXT NOT NULL REFERENCES tracks (id) ON DELETE CASCADE,
    PRIMARY KEY (playlist_id, position)
);

CREATE TABLE queue_items (
    position  INTEGER PRIMARY KEY,
    track_id  TEXT NOT NULL REFERENCES tracks (id) ON DELETE CASCADE
);

CREATE TABLE playback_state (
    id                 INTEGER PRIMARY KEY CHECK (id = 1),
    current_index      INTEGER,
    position_ms        INTEGER NOT NULL DEFAULT 0,
    volume             REAL NOT NULL DEFAULT 0.8,
    shuffle            INTEGER NOT NULL DEFAULT 0,
    repeat_mode        TEXT NOT NULL DEFAULT 'off' CHECK (repeat_mode IN ('off', 'all', 'one')),
    spotify_device_id  TEXT,
    updated_at         TEXT NOT NULL
);
INSERT INTO playback_state (id, updated_at) VALUES (1, datetime('now'));

-- Persistent Last.fm scrobble cache (survives restarts, sent oldest first).
CREATE TABLE scrobble_queue (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id      TEXT,
    artist        TEXT NOT NULL,
    track         TEXT NOT NULL,
    album         TEXT,
    album_artist  TEXT,
    track_number  INTEGER,
    duration_s    INTEGER,
    timestamp     INTEGER NOT NULL,          -- UTC unix seconds when playback started
    status        TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'failed')),
    attempts      INTEGER NOT NULL DEFAULT 0,
    last_error    TEXT,
    created_at    TEXT NOT NULL
);

CREATE TABLE metadata_backups (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id     TEXT NOT NULL,
    file_path    TEXT NOT NULL,
    backup_path  TEXT NOT NULL,
    created_at   TEXT NOT NULL
);

CREATE TABLE settings (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);
