-- 0005: Spotify playlist lifecycle (create/delete/external removal) and
-- "Find alternative" selections. Additive only; existing data is untouched.

-- External removal detection. A playlist is only treated as deleted or
-- removed after two consecutive, separate confirmations; temporary errors
-- never set these.
ALTER TABLE spotify_playlist_links ADD COLUMN missing_kind TEXT;   -- 'not_found' | 'not_in_library'
ALTER TABLE spotify_playlist_links ADD COLUMN missing_since TEXT;
-- Set while "delete from Spotify" runs, so an interrupted deletion is
-- finished (or undone) safely after a restart.
ALTER TABLE spotify_playlist_links ADD COLUMN pending_delete INTEGER NOT NULL DEFAULT 0;

-- A Spotify playlist creation whose result is not yet known (request sent,
-- no confirmed answer). Lets a retry adopt the playlist Spotify did create
-- instead of creating a duplicate.
CREATE TABLE spotify_pending_creates (
    playlist_id  TEXT PRIMARY KEY REFERENCES playlists (id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    started_at   TEXT NOT NULL
);

-- Alternatives the user chose for an unavailable Spotify track. The
-- alternative is a normal playlist item placed right after the original;
-- for Spotify Synced playlists it exists only in Branflakes and the sync
-- engine keeps it next to its original instead of treating it as a change.
CREATE TABLE playlist_alternatives (
    playlist_id           TEXT NOT NULL REFERENCES playlists (id) ON DELETE CASCADE,
    original_track_id     TEXT NOT NULL REFERENCES tracks (id) ON DELETE CASCADE,
    alternative_track_id  TEXT NOT NULL REFERENCES tracks (id) ON DELETE CASCADE,
    -- 1 once the user removed the alternative: it is never re-added
    -- automatically (only by an explicit new selection).
    removed               INTEGER NOT NULL DEFAULT 0,
    created_at            TEXT NOT NULL,
    updated_at            TEXT NOT NULL,
    PRIMARY KEY (playlist_id, original_track_id, alternative_track_id)
);
CREATE INDEX idx_playlist_alternatives_alt ON playlist_alternatives (playlist_id, alternative_track_id);
