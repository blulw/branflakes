-- 0004: two-way Spotify playlist sync and Branflakes-only forks.
--
-- One row per Branflakes playlist that is linked to a Spotify playlist.
-- `base_*` columns hold the state both sides agreed on after the last
-- successful sync. The reconciliation engine compares the current Branflakes
-- and Spotify states against this base (a three-way merge), which is what
-- makes repeated or interrupted syncs safe and prevents sync loops.
-- No credentials are stored here (tokens live in Windows Credential Manager).
CREATE TABLE spotify_playlist_links (
    playlist_id          TEXT PRIMARY KEY REFERENCES playlists (id) ON DELETE CASCADE,
    spotify_playlist_id  TEXT NOT NULL UNIQUE,
    enabled              INTEGER NOT NULL DEFAULT 1,
    owner_id             TEXT,
    -- 0 for collaborative playlists owned by someone else: Spotify lets
    -- collaborators change items but only the owner can rename or set a cover.
    can_edit_details     INTEGER NOT NULL DEFAULT 1,
    snapshot_id          TEXT,
    base_name            TEXT NOT NULL,
    base_items           TEXT NOT NULL DEFAULT '[]',   -- JSON array of Spotify URIs, in order
    base_image_key       TEXT,                        -- cover identity (URL without query) or 'uploaded:<previous>'
    base_artwork_path    TEXT,                        -- Branflakes cover path at the last sync
    artwork_failed_path  TEXT,                        -- cover that Spotify rejected (not retried automatically)
    status               TEXT NOT NULL DEFAULT 'pending'
                         CHECK (status IN ('pending', 'syncing', 'ok', 'error')),
    last_synced_at       TEXT,
    last_attempt_at      TEXT,
    last_error           TEXT,
    last_notice          TEXT,
    created_at           TEXT NOT NULL,
    updated_at           TEXT NOT NULL
);

-- A Branflakes-only copy created when a non-Spotify track was added to a
-- synced playlist. Holds the source playlist's id (no FK: the copy outlives it).
ALTER TABLE playlists ADD COLUMN forked_from TEXT;
-- What the copy is based on and what could not be copied (shown on the copy).
ALTER TABLE playlists ADD COLUMN fork_note TEXT;
