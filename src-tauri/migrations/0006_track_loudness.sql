-- 0006: cached per-track loudness for volume normalization

CREATE TABLE track_loudness (
    track_id     TEXT PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
    source_key   TEXT NOT NULL,   -- file path/size/mtime or provider id; re-analyse when it changes
    lufs         REAL,            -- integrated loudness; NULL = could not be measured
    analyzed_at  TEXT NOT NULL
);
