# branflakes

> [!WARNING]
> **This app is entirely vibecoded.** Every line of code, test and document here was written by an AI coding assistant (Claude Code), directed through prompts. I did not write or line-by-line review the code myself. It works for my own use, but it has not had a human code review or security audit, so use it at your own risk.

A Windows 11 desktop music app that puts **local files**, **Spotify** and **SoundCloud** in one library, with **Last.fm scrobbling for local files, Spotify and SoundCloud**. It uses each service's official API, except for an optional, off-by-default SoundCloud *web mode* that uses SoundCloud's unofficial website API (see below).

- Tauri 2 · Rust backend · React + TypeScript + Vite · Tailwind CSS + shadcn/ui
- SQLite with versioned migrations · Windows Credential Manager for secrets
- Windows media keys / System Media Transport Controls · taskbar thumbnail playback buttons
- NSIS and MSI installers

It does **not** embed, automate, or talk over IPC to any other music application. Spotify playback goes through Spotify's documented Web API (Spotify Connect), and audio is never downloaded or extracted.

> [!CAUTION]
> **SoundCloud web mode is unofficial.** When you turn it on, the app reads the public `client_id` from soundcloud.com's own JavaScript and calls the website's internal `api-v2.soundcloud.com` endpoints. That API is undocumented and not meant for third-party apps. Using it likely goes against SoundCloud's Terms of Use, and it can break at any time. It is off by default, and the official API is always used instead when an official SoundCloud app is connected. Details: [docs/PROVIDERS.md §3](docs/PROVIDERS.md#web-mode-unofficial-opt-in).

> Read **[docs/PROVIDERS.md](docs/PROVIDERS.md)** before connecting accounts. Spotify and SoundCloud both impose access restrictions (Premium, Artist Pro, 5-user Development Mode, …) that decide what works.

## Features

| Area | What you get |
|---|---|
| Unified library | One track model with stable internal IDs plus provider IDs. Every row shows a Local / Spotify / SoundCloud badge. Search, sort, source and availability filters, plus artist and album browsing across sources. |
| Playlists and queue | Playlists and the queue can mix sources. The queue, current track, position, volume, repeat mode and Spotify device are saved in SQLite and restored on restart. |
| Spotify playlist sync | Imported Spotify playlists are **Spotify Synced** automatically (when Spotify allows it). New playlists are local-only unless you choose to create them on Spotify too. Synced playlists you own or collaborate on stay the same in both apps: tracks, order, name and cover stay the same in both apps (two-way, every 5 minutes and on demand). Adding a local or SoundCloud track to one makes a **Branflakes-only copy** instead. See [docs/PROVIDERS.md](docs/PROVIDERS.md#playlist-sync). |
| Local files | Scan the folders you choose. Read metadata and embedded artwork. Write tags into the actual files, with a backup first, read-back verification and automatic restore on mismatch. Unrelated tags and frames are preserved. App-only overrides and custom artwork work for any source. |
| Spotify | PKCE OAuth, import of saved tracks and owned playlists, search, and Spotify Connect playback control (Premium) with a device picker. There is also an experimental in-app Web Playback SDK device, used only when WebView2 exposes Widevine. |
| SoundCloud | OAuth 2.1 + PKCE, import of likes and playlists, search, and in-app AAC HLS streaming with the required attribution. Optional *web mode* (off by default) works without an official API app; see the note below. |
| Last.fm | Desktop auth, Now Playing and scrobbling for **local, Spotify and SoundCloud tracks** plus SoundCloud (Spotify can be turned off separately). The cache survives restarts and is retried in batches of 50. |
| Find alternative | Unavailable Spotify tracks in a playlist get a **Find alternative** button. It searches your local files first, then SoundCloud, for the same song (title and artist), and adds your pick right after the original, which stays. |
| Lyrics | The microphone button in the player bar opens a lyrics panel for the playing song. The words always come from **Genius**. Timing comes from **LRCLIB**, and for songs LRCLIB doesn't have, from **Musixmatch** and **NetEase Cloud Music** (Settings → General, on by default). The timing is matched word by word onto Genius's lines, so the current line is highlighted and followed, and clicking a line seeks there (when the source can seek). When no timing matches, Genius's text is shown unsynced. No account or API key is needed. |
| Themes | Settings → Themes has 50 themes (Candy Floss, Galaxy, Synthwave, Frosted Glass, …), each shown with a colour palette and a few words describing it. Some use translucent frosted surfaces or a glow. **Custom** lets you set every colour, surface opacity and blur, glow, a gradient, corner roundness, the font and your own background image (with fit, tint and blur). Saved on this computer. |
| Playback | Each provider declares its capabilities, and the UI disables any control the active source can't perform, with a tooltip saying why. One source owns playback at a time. Previous / Play-Pause / Next buttons appear in the Windows taskbar hover thumbnail. |

## Requirements

- Windows 11 x64. The installer bootstraps the Microsoft Edge WebView2 runtime if it is missing.
- To build from source:
  - Node.js 20+ and npm
  - Rust stable (MSVC toolchain): `rustup default stable-x86_64-pc-windows-msvc`
  - Visual Studio 2022 Build Tools with "Desktop development with C++"
  - Tauri downloads NSIS and WiX automatically during `tauri build`. This needs internet access the first time.

## Build and run

```powershell
npm install
npm run tauri dev        # run in development with hot reload
npm run tauri build      # release build + installers
```

Build outputs:

- `src-tauri/target/release/branflakes.exe`
- `src-tauri/target/release/bundle/nsis/branflakes_0.5.0_x64-setup.exe` — per-user installer with an uninstaller (Apps & Features)
- `src-tauri/target/release/bundle/msi/branflakes_0.5.0_x64_en-US.msi` — MSI package

The installers are **not code-signed**. Windows SmartScreen will warn until you sign them with your own certificate (see the Tauri docs on Windows code signing).

## Tests

```powershell
npm run test:rust   # Rust: DB, migrations, adapters (mock HTTP), metadata I/O, scrobbling
npm test            # Frontend: capability → control resolution (Vitest)
npm run typecheck   # TypeScript
```

Provider adapter tests run against a local mock HTTP server (`mockito`), so they need no credentials. Metadata tests create real MP3, WAV and FLAC files in a temp folder and write to them.

## First-run setup

1. **Settings → Local folders → Add folder.** The folder is scanned straight away.
2. **Settings → Accounts.** Enter your own API credentials for each service you want, then press *Connect*. Your browser opens for sign-in. Step-by-step instructions are in [docs/PROVIDERS.md](docs/PROVIDERS.md).
3. Press *Import* to pull your Spotify saved tracks and playlists, or your SoundCloud likes and playlists, into the library.

## Data and privacy

- **Library database:** `%APPDATA%\com.unifiedmusic.desktop\library.sqlite3`
- **Artwork cache:** `…\artwork\`. **Tag-edit backups:** `…\backups\` (never deleted automatically).
- **Secrets** (OAuth tokens, the SoundCloud client secret, the Last.fm shared secret and session key) are stored as generic credentials in **Windows Credential Manager** under `com.unifiedmusic.desktop`. They are never written to SQLite or logs.
- **Lyrics lookups** send only the song title, artist, album and length to Genius, LRCLIB and (when enabled) Musixmatch and NetEase. Results are cached per track in SQLite.
- The asset protocol (used to play local files and show cached artwork) only serves your library folders and the artwork cache.

## Documentation

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): technical architecture and module layout
- [docs/PROVIDERS.md](docs/PROVIDERS.md): API configuration, capability matrix, restrictions and unsupported features
- [docs/IMPLEMENTATION_REPORT.md](docs/IMPLEMENTATION_REPORT.md): what is complete, incomplete and verified
