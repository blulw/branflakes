-- 0003: first credited artist, used for Last.fm scrobbles of streaming tracks.
-- Spotify stores all artists joined as "A, B" in `artist`; Last.fm expects the
-- primary artist only. NULL for local files and tracks imported before this.
ALTER TABLE tracks ADD COLUMN primary_artist TEXT;
