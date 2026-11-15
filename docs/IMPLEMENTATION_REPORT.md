# Implementation report

Date: 14 November 2026 · Version 0.1.0

This report separates what was **built and verified**, what was **built but not verified against live services**, and what is **not implemented**. Nothing here claims more than was tested.

## 1. Starting point

The repository was empty, and the "accompanying product specification" referred to in the brief was not present. The brief itself was used as the spec. The project was scaffolded with `create-tauri-app` (Tauri 2, React + TS + Vite), then Tailwind v4 and shadcn/ui were added.

## 2. Verification performed

| Check | Result |
|---|---|
| `cargo test` (Rust) | **57 passed, 0 failed** |
| `vitest run` (frontend) | **8 passed, 0 failed** |
| `tsc --noEmit` | clean |
| `npm run tauri build` | release exe + **NSIS** (`branflakes_0.1.0_x64-setup.exe`, ~3.6 MB) + **MSI** (`…_x64_en-US.msi`, ~5 MB) produced |
| Release exe launch | process stays up; creates `%APPDATA%\com.unifiedmusic.desktop\library.sqlite3` at schema v1 with all tables and views |
| UI render (screenshots of the release build) | Library (empty state, filters, disabled transport) and Settings → Accounts render correctly |

What the Rust tests cover:

- **DB:**
  - migrations are idempotent and foreign keys are enforced
  - stable internal IDs across re-imports
  - search upserts never remove a track from the library
  - filter, sort and LIKE escaping; SQL injection in the sort key is rejected
  - overrides apply, survive provider refresh and clear correctly
  - cross-source artist and album grouping
  - folder removal keeps referenced tracks as unavailable
  - mixed-source playlist CRUD and reordering
  - queue persistence, current-index maths and repeat rules
  - settings round-trip
- **Metadata (real files in a temp dir):**
  - write and read back on MP3, WAV and FLAC
  - TXXX and USLT frames survive an edit
  - **FLAC pictures survive a text edit** (regression for a lofty data-loss behaviour found during development)
  - backup → write → verify → DB update, then restore gives byte-identical original files
  - invalid edits and edits to non-local tracks are rejected
  - scans handle WMA (reported as unsupported), corrupt files (reported as errors), unchanged files (skipped) and deleted files (marked unavailable)
- **HTTP layer:**
  - 429 with Retry-After is retried; `QUOTA_EXCEEDED` is not
  - 5xx backoff stops at the retry limit
  - 401 maps to Auth; `403 PREMIUM_REQUIRED` maps to PremiumRequired
- **OAuth:** the PKCE challenge matches the RFC 7636 test vector; the loopback listener ignores a wrong `state` and accepts the right one.
- **Spotify adapter (mock server):**
  - PKCE authorize URL with no secret; tokens are stored in the secret store
  - refresh keeps the old refresh token when none is returned; one retry after a 401
  - search sends `limit=10`; playlist items accept both the `item` and `track` shapes
  - Premium errors are explicit; 204 from `/me/player` is handled
  - device and `disallows` parsing (this test caught a real snake_case/camelCase bug, now fixed)
- **SoundCloud adapter (mock server):**
  - `playable` / `preview` / `blocked` mapping
  - AAC-HLS stream preference and fallbacks
  - the secret and PKCE verifier are sent on token exchange
  - single-use refresh tokens are replaced
  - `next_href` pagination
  - stream redirect resolved without the token appearing in the playable URL
  - URN validation blocks path injection
- **Last.fm and scrobbler:**
  - signature algorithm, including UTF-8 and indexed keys
  - session parsing and error 14
  - indexed batch body
  - 30 s / half / 4 min eligibility rules
  - **Spotify and SoundCloud plays are never queued**
  - the scrobble snapshot is persisted
  - ordered flush, kept for retry on error 11, marked rejected on error 6

What the frontend tests cover: capability → control resolution. A control is never enabled unless the source declares it. They also cover Spotify device and volume rules, unavailable tracks, and queue edges.

## 3. Completed features

- **Architecture:** Tauri 2, a Rust backend, React/TS/Vite, Tailwind + shadcn/ui, SQLite with versioned migrations, Windows Credential Manager, and a typed command layer. UI components contain no provider logic.
- **Adapters:** `LocalMusicAdapter`, `SpotifyAdapter`, `SoundCloudAdapter`, `LastFmAdapter`. Each declares its own `PlaybackCapabilities`; they are not forced into an identical interface.
- **Unified library:**
  - one track model with stable UUIDs and provider IDs, plus source badges everywhere
  - search, sort, source and availability filters
  - artist and album browsing across sources
  - mixed-source playlists and queue
  - persisted queue and player state
  - clear reasons for unavailable tracks; the queue auto-skips unplayable ones, with a loop guard
- **Local files:**
  - folder scanning, metadata and embedded artwork extraction (content-addressed cache)
  - safe in-file tag editing (backup, native-tag write, read-back verification, automatic restore)
  - backup list and restore
  - unrelated tags preserved
  - unsupported formats reported rather than failing
  - app-level overrides and custom artwork for any source
- **Spotify:**
  - PKCE OAuth over a loopback redirect, with automatic refresh
  - import of saved tracks and owned/collaborative playlists (skipped playlists are reported)
  - search with Development Mode paging
  - Spotify Connect control: play, pause, seek, volume (debounced, and only where the device supports it), transfer, and device picker
  - Premium detection from API errors
  - an opt-in Web Playback SDK device behind an honest Widevine probe
- **SoundCloud:**
  - OAuth 2.1 + PKCE with the client secret, and single-use refresh handling
  - import of likes and playlists, and search
  - in-app AAC HLS streaming through hls.js, with the token kept in the backend
  - preview and blocked handling
  - attribution UI (uploader link plus "Listen on SoundCloud")
- **Last.fm:** desktop auth with polling, Now Playing, and scrobbling for local tracks only (enforced in the backend). A persistent ordered cache is retried after each play and every 5 minutes. Settings shows queue status and can discard rejected scrobbles.
- **Windows:**
  - SMTC and media keys through souvlaki on a dedicated thread. Chromium's own media-key handling is disabled to avoid double handling. SMTC events are filtered through the resolved controls.
  - desktop notifications on track change when the window is unfocused (toggleable)
  - NSIS (per-user, with uninstaller) and MSI installers, with a WebView2 bootstrapper
- **Engineering:**
  - explicit `AppError` kinds sent to the UI
  - retries and rate-limit handling
  - redacted `Secret` type; nothing logs tokens; Last.fm uses POST bodies
  - a restrictive CSP; the asset protocol scope is limited to library folders and the artwork cache, with the DB and backups explicitly denied
  - docs: README, ARCHITECTURE, PROVIDERS (setup, matrix, restrictions, sources)

## 4. Built but not verified against live services

No real Spotify, SoundCloud or Last.fm credentials were available. These paths follow the official docs and pass mock-server tests, but have **not** been exercised end to end:

1. **Spotify** OAuth, import, search and Connect playback with a real Premium account. The end-of-track detection heuristic (polling) may need tuning, especially with Spotify Autoplay on.
2. **Spotify Web Playback SDK in WebView2.** This is expected to fail the Widevine probe on standard WebView2. If it does, the app says so and falls back to other Connect devices.
3. **SoundCloud** OAuth with a loopback `http://127.0.0.1` redirect. SoundCloud's guide shows custom schemes for desktop apps. Also unverified: whether the `/streams` redirect target is fetchable cross-origin from WebView2, and which CDN hosts are used (the CSP allows `*.sndcdn.com` and `*.soundcloud.cloud`).
4. **Last.fm** live auth and scrobble submission.
5. **Interactive local playback in the release build.** Scanning, artwork and tag editing are covered by tests. An automated click-through of scan → play was started but **stopped early**, because the desktop was in active use and simulated input would have interfered with it. Audio output, SMTC flyout and media-key behaviour, and notifications have not been observed on screen.

## 4b. Later changes

- **Spotify in-app device:**
  - Sign-in now requests `user-read-email` and `user-read-private`, which the Web Playback SDK checks for.
  - Older sign-ins are detected and show a **Reconnect Spotify** prompt.
  - Token refreshes keep the granted scopes.
  - The in-app device stays selected while Spotify's device list catches up, and restarts automatically next launch.
- **SoundCloud web mode** (unofficial, opt-in, at the owner's request): see PROVIDERS.md §3. Mock-tested only.
- **Playlists:**
  - The sidebar sorts playlists by *Recently played* (the default), name, recently added or track count. The choice is remembered.
  - The playlist page always shows the name and cover. With no cover set it shows a plain grey square, and you can choose or remove a cover image.
  - The page has **Add songs** (library, or a Spotify/SoundCloud search), a track sort (custom order by default), duration and source summary, and an empty state.
  - Migration `0002` adds `playlists.artwork_path` and `last_played_at`. "Played" means playback was started from the playlist page.

## 5. Not implemented / known limitations

- **Gapless playback, crossfade, and mixing sources:** intentionally not supported.
- **Spotify audio inside the app:** not possible through approved means beyond the SDK path above. Spotify playback needs a Spotify Connect device.
- **SoundCloud custom-URL-scheme redirect** (deep link) as a fallback if loopback redirects are refused. Not implemented.
- **Output device selection** for local and SoundCloud playback (`setSinkId`). Not implemented; declared as unsupported.
- **Writing back to providers:** saving or liking on Spotify or SoundCloud, and editing provider playlists. Library and playlist changes are app-local.
- **Re-import removals:** tracks un-saved on Spotify or un-liked on SoundCloud are not removed from the app library on re-import. Imported playlists are refreshed in full.
- **Folder watching:** none; rescans are manual. A file modified while the app runs is picked up on the next rescan (size and mtime check).
- **Editing a playing file** may fail with "file in use"; the UI warns about this.
- **UI:** reordering uses move up/down, not drag-and-drop. There is no multi-select bulk action and no keyboard shortcut layer beyond media keys.
- **Distribution:** installers are **not code-signed**, and there is no auto-updater.
- **Frontend bundle** is a single ~600 kB chunk (Vite warning); this is harmless for a desktop app.
- **Spotify Development Mode:** limited to 5 users, the owner needs Premium, and quotas are shared per developer account. Wider distribution needs Spotify's extended quota approval, which as of 2025–2026 Spotify reserves for organisations.
