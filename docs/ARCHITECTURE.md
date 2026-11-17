# Architecture

## Overview

```
┌──────────────────────────── WebView2 (React + TS) ────────────────────────────┐
│ views/        Library, Artists, Albums, Playlist, Queue, Search, Settings     │
│ components/   TrackTable (virtualised), PlayerBar, DevicePicker, badges       │
│ playback/                                                                     │
│   controls.ts     pure capability → control resolution (unit tested)          │
│   store.ts        playback controller: single active engine, queue, scrobble  │
│                   time accounting, SMTC sync, persistence                     │
│   engines/        AudioElementEngine (Local + SoundCloud HLS)                 │
│                   SpotifyConnectEngine (Web API remote control)               │
│                   spotifySdk (optional in-app Connect device)                 │
│ lib/api.ts       typed wrappers over Tauri commands (no provider logic in UI) │
└───────────────────────────────▲───────────────────────────────────────────────┘
                                │ Tauri IPC (commands + events)
┌───────────────────────────────┴──────────── Rust ─────────────────────────────┐
│ commands.rs     thin command layer (validation, delegation)                   │
│ providers/      local.rs      LocalMusicAdapter (lofty)                       │
│                 spotify.rs    SpotifyAdapter (PKCE, Web API, Connect)         │
│                 soundcloud.rs SoundCloudAdapter (OAuth 2.1, streams)          │
│                 lastfm.rs     LastFmAdapter (desktop auth, signing)           │
│                 mod.rs        per-source capability declaration               │
│ scrobbler.rs    eligibility rules, local-only policy, ordered batch flushing  │
│ db/             SQLite: migrations, library, playlists, queue, scrobbles      │
│ http.rs         retries, 429 / Retry-After, quota detection, error mapping    │
│ oauth.rs        PKCE, CSRF state, loopback redirect listener                  │
│ secrets.rs      Windows Credential Manager (keyring) + test store             │
│ media.rs        Windows SMTC / media keys (souvlaki) on a dedicated thread    │
│ taskbar.rs      taskbar thumbnail toolbar (ITaskbarList3) on the UI thread    │
│ sync/           Spotify playlist sync: merge.rs (pure 3-way merge, push plan) │
│                 mod.rs (enable, sync pass, add/fork rule, scheduler state)    │
└───────────────────────────────────────────────────────────────────────────────┘
```

## Key decisions

**Capability-based playback.** Each adapter declares a `PlaybackCapabilities` struct with fields for play, pause, next, previous, seek, volume, queue, device selection, playback position and duration. `providers::playback_capabilities` is the single source of truth on the backend. The frontend merges it with runtime facts: whether the Spotify device supports remote volume, Spotify `disallows`, whether the duration is known, and track availability. That merge happens in `resolveControls`. The player bar renders only that result, and a disabled control shows a tooltip with the reason.

**Single owner.** The controller keeps exactly one `activeEngine`. Before a track from a different engine loads, the old engine is `release()`d: the audio element stops, or Spotify is paused. Cross-source transitions are sequential. There is no mixing, crossfade or gapless claim.

**Unified queue in the backend.** Queue order, the current index and next/previous rules (repeat off/all/one) live in Rust (`db::queue`) and are unit-tested. The frontend asks `queue_advance` for the next index, so restart restores exactly what was persisted.

**Stable IDs.** `tracks.id` is a UUID assigned once. `(source, provider_id)` is unique. Provider refreshes update metadata in place, so playlists, queue items and overrides survive re-imports. Search results are stored with `in_library = 0`, which gives them IDs for queueing and playlists without polluting the library. An upsert can only promote a track into the library, never demote it.

**Overrides.** `track_overrides` and the `tracks_effective` view apply app-level metadata and artwork on top of provider data for any source. Provider refreshes never overwrite them.

**Safe tag writing** (`providers/local.rs`):
1. Validate the edit and check the format is writable.
2. Copy the file to `backups/` and record it in `metadata_backups`.
3. Read the file as its concrete type (MPEG/FLAC/MP4/Ogg/WAV/AIFF/APE/WavPack/MPC) and take the native tag (ID3v2, Vorbis Comments, ilst or APE). Split it into generic and remainder parts with lofty's `SplitTag`, change only the edited fields, merge back, and save that tag only. Frames the generic model can't represent, such as TXXX, USLT, PRIV and custom atoms, are kept.
4. FLAC needs special care: lofty's FLAC writer replaces all PICTURE blocks with the pictures in the saved tag, so existing pictures are carried over explicitly. A regression test covers this.
5. Re-read the file and compare every edited field. On any mismatch or read failure, restore the backup and return `VerificationFailed`.
6. Update the library row from the file on disk.

**Secrets.** Tokens and secrets live in Windows Credential Manager behind the `SecretStore` trait. `Secret` has a redacting `Debug`. Last.fm calls send everything in a POST body, so session keys never appear in URLs. SoundCloud stream URLs are resolved in Rust, so the OAuth token never reaches the webview. The one deliberate exception is the optional Spotify Web Playback SDK, whose API requires the page to provide an access token. A restrictive CSP limits scripts to the app and `sdk.scdn.co`.

**Errors and rate limits.** `http::send` rebuilds and retries requests. It honours `Retry-After` on 429 (capped). It returns Spotify's `QUOTA_EXCEEDED` immediately, because retrying can't help. 5xx and transport errors get exponential backoff. 401 triggers one token refresh and a retry. 403 `PREMIUM_REQUIRED` maps to `AppError::PremiumRequired`, which turns the Spotify controls off. All errors reach the UI as `{ kind, message }`.

**Taskbar thumbnail toolbar** (`taskbar.rs`). Separate from SMTC. Uses `ITaskbarList3::ThumbBarAddButtons/UpdateButtons` and a window subclass (`SetWindowSubclass`) on the UI thread. A click (`WM_COMMAND` + `THBN_CLICKED`) is emitted as the same `media-control` event SMTC uses, so the frontend controller handles it exactly like a media key, filtered by the resolved controls. The frontend sends `canPrevious/canPlayPause/canNext` and the play state with every `media_update`, and pushes again whenever those flags change (a store subscription, no polling); the toolbar disables buttons accordingly. On Windows 11 the shell returns S_OK for every toolbar call, so duplicates are prevented by bookkeeping: buttons are added only on `TaskbarButtonCreated` (first show, Explorer restart), with a 3 s one-shot fallback for the case where that message arrived before the subclass was installed (observed in the real app). Icons are rendered at the small-icon size for the current DPI and re-rendered on theme/DPI change. On exit the subclass, timer, icons and COM object are released.

**Spotify playlist sync** (`sync/`). `spotify_playlist_links` stores, per synced playlist, the *base*: the item URIs, name, cover identity and snapshot both sides agreed on after the last sync. A pass:
1. Reads `GET /playlists/{id}` metadata (cheap). It skips the full item read when Spotify's `snapshot_id` and the local items are both unchanged.
2. Otherwise reads every item. A read whose pages don't add up to `total` is an error, never treated as removals.
3. Runs a three-way merge (`merge.rs`) on occurrence-tagged URIs: one-sided changes win, an item is kept if either side has it when both changed, and when both changed Spotify's order wins. Spotify-only items (local files, episodes, unavailable placeholders) are preserved and never pushed.
4. Re-checks the snapshot. If it moved, the pass reruns.
5. Plans the push with remove/move/add (LIS-minimal moves, ≤100 URIs per request; a full replace only when no unrepresentable item could be lost). It verifies the plan by simulation, sends it, then reads Spotify back and adopts what Spotify actually has.
6. Name and cover are three-way too.
7. Writes Branflakes inside a transaction that first checks that the playlist wasn't edited meanwhile; if it was, the pass reruns.

Because every pass is idempotent against the base, interrupted passes (crash/restart) are simply repeated, and a pass after a successful one is a no-op, so there are no sync loops. The scheduler (`commands::sync_scheduler`) is a Tokio task: a full pass every interval (default 5 min, `spotify.sync_interval_secs`), a debounced pass 3 s after each local edit, and backoff on `Retry-After`/quota (1 h)/offline (60 s). Only one sync runs at a time, and a second request for the same playlist is refused. Interrupted syncs are marked pending on startup.

**Playlist lifecycle** (`sync/`).
- `import_playlist` links imports via `enable` (Spotify's contents become the base). `auto_link_imported` runs on each full scheduler pass for earlier imports, and `remove_duplicates_of` drops stale unsynced copies that hold only Spotify tracks.
- `create_and_link` records a `spotify_pending_creates` row before `POST /me/playlists`. A definite refusal clears it; an unknown outcome keeps it, so a retry adopts the playlist Spotify did create (owned, same name, empty, unlinked) instead of duplicating it. The link starts with an empty base, so the next pass pushes the contents.
- `delete_synced` sets `pending_delete`, removes the playlist from the Spotify library only if it is still there, verifies that, then deletes locally. `resume_delete` handles restarts.
- Each pass checks existence and library membership. A definite "missing" is recorded (`missing_kind`, `missing_since`) and acted on only when a later check at least 60 s afterwards agrees; errors never count.
- **POST requests are never retried after a 5xx or a mid-request failure** (`RetryPolicy::non_idempotent`), because Spotify may already have applied them. A later pass reconciles idempotently instead.

**Alternatives** (`alternatives.rs`, `db/alternatives.rs`).
- Matching is pure and unit-tested (normalised title + artist, tiered score).
- A chosen alternative is a normal playlist item after its original, recorded in `playlist_alternatives`.
- For a synced playlist, `read_local` leaves active alternatives out of the merge, and `alternatives::reinsert` puts them back beside their originals after each write-back (and in forks). Removing one marks it `removed`, so it is never re-added.

**Playlist add rule** (`sync::add_tracks`). Every UI path (track menu, Add songs dialog, multi-track adds) calls `playlist_add_tracks`, which enforces the rule in the backend: if the playlist is Spotify Synced and any track isn't a Spotify track, the synced playlist is left untouched and `playlists::create_fork` makes a Branflakes-only copy (all items, the new tracks at the requested position, the same cover, `forked_from` set, no origin, no link).

**Lyrics** (`lyrics.rs`, `components/LyricsPanel.tsx`). `lyrics_get` looks the track up on Genius and LRCLIB in parallel. Genius's song search works without a key; the text is extracted from the song page's `data-lyrics-container` blocks (skipping `data-exclude-from-selection` parts such as the contributor header and ads). LRCLIB supplies LRC lines, which are trusted only for a recording within 5 s of the track's length. Both must pass `alternatives::score` (title and artist), so covers and similar titles are rejected. Results, including "nothing found" (retried after 3 days), are cached in `lyrics_cache`, keyed by title/artist/length so a metadata edit refetches. A lookup where a source failed is not cached. The panel highlights the line at `positionMs + 250 ms` and stops auto-scrolling for 4 s after the user scrolls.

**Threads.** The SQLite connection is behind a `Mutex`. Long work (folder scans, tag writes) opens its own WAL connection on a blocking thread. SMTC objects live on a dedicated thread that receives updates over a channel. Button presses become `media-control` events, which the controller filters through the resolved controls.

## Database

Migrations are embedded (`src-tauri/migrations/*.sql`) and applied in order, tracked by `PRAGMA user_version`. Never edit a shipped migration; add `0002_….sql` and append it to `db::MIGRATIONS`.

| Table | Purpose |
|---|---|
| `tracks` | Unified track rows (source, provider_id, metadata, availability, attribution fields, file stamps). |
| `track_overrides` | App-level metadata and artwork overrides. |
| `tracks_effective` (view) | Tracks with overrides applied. |
| `library_folders` | User-selected scan roots. |
| `playlists`, `playlist_items` | Mixed-source playlists. `origin` links imported playlists; `forked_from` marks Branflakes-only copies of synced playlists. |
| `queue_items`, `playback_state` | Persisted queue and player state (a single row). |
| `scrobble_queue` | Persistent Last.fm cache (pending/failed, attempts, last error). |
| `metadata_backups` | Backup file per tag write. |
| `settings` | Non-secret key/value settings. |
| `spotify_pending_creates` | Spotify playlist creations whose outcome is unknown (retry adopts instead of duplicating; migration 0005). |
| `playlist_alternatives` | Alternatives chosen for unavailable Spotify tracks, per playlist; `removed` once the user removes one (migration 0005). |
| `spotify_playlist_links` | Spotify Synced playlists: Spotify id, permissions, snapshot, 3-way-merge base, status, last sync/error (migration 0004). No credentials. |
