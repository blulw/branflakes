-- 0007: cached lyrics (Genius text, LRCLIB time-synced lines)

CREATE TABLE lyrics_cache (
    track_id    TEXT PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
    lookup_key  TEXT NOT NULL,   -- title/artist/length used for the lookup; refetch when it changes
    payload     TEXT,            -- JSON `Lyrics`; NULL = nothing found
    fetched_at  TEXT NOT NULL
);
