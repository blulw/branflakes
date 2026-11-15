-- 0002: playlist cover art and "last played" for recently-played sorting
ALTER TABLE playlists ADD COLUMN artwork_path TEXT;
ALTER TABLE playlists ADD COLUMN last_played_at TEXT;
CREATE INDEX idx_playlists_last_played ON playlists (last_played_at);
