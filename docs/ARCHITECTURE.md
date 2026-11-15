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

**Threads.** The SQLite connection is behind a `Mutex`. Long work (folder scans, tag writes) opens its own WAL connection on a blocking thread. SMTC objects live on a dedicated thread that receives updates over a channel. Button presses become `media-control` events, which the controller filters through the resolved controls.

## Database

Migrations are embedded (`src-tauri/migrations/*.sql`) and applied in order, tracked by `PRAGMA user_version`. Never edit a shipped migration; add `0002_….sql` and append it to `db::MIGRATIONS`.

| Table | Purpose |
|---|---|
| `tracks` | Unified track rows (source, provider_id, metadata, availability, attribution fields, file stamps). |
| `track_overrides` | App-level metadata and artwork overrides. |
| `tracks_effective` (view) | Tracks with overrides applied. |
| `library_folders` | User-selected scan roots. |
| `playlists`, `playlist_items` | Mixed-source playlists. `origin` links imported playlists. |
| `queue_items`, `playback_state` | Persisted queue and player state (a single row). |
| `scrobble_queue` | Persistent Last.fm cache (pending/failed, attempts, last error). |
| `metadata_backups` | Backup file per tag write. |
| `settings` | Non-secret key/value settings. |
