# Provider configuration, capabilities and restrictions

Research date: **14 November 2026**, against the providers' official developer documentation (sources at the end). Provider policies change often. If something below no longer matches what you see, trust the provider's current docs.

You supply your own developer credentials for each service. The app ships with no shared keys.

---

## 1. Capability matrix

Next/previous always move through the **unified app queue**, not the provider's own queue.

| Capability | Local files | Spotify (connected, Premium) | Spotify (no Premium / not connected) | SoundCloud (connected) | Last.fm |
|---|---|---|---|---|---|
| Play / pause | ✓ in-app | ✓ on a Spotify Connect device | ✗ | ✓ in-app (`playable` tracks; `preview` → preview only) | — |
| Next / previous | ✓ (app queue) | ✓ (app queue) | queue skip only | ✓ (app queue) | — |
| Seek | ✓ | ✓ unless Spotify reports `disallows.seeking` | ✗ | ✓ | — |
| Volume | ✓ | ✓ only if device `supports_volume` | ✗ | ✓ | — |
| Queue | ✓ app queue | ✓ app queue (plus `POST /me/player/queue` in the backend) | ✗ | ✓ app queue (no SoundCloud queue API) | — |
| Device selection | ✗ (Windows default output) | ✓ Connect device picker | ✗ | ✗ | — |
| Playback position | ✓ exact | ✓ polled every 1.5 s, interpolated | ✗ | ✓ exact | — |
| Duration | ✓ | ✓ | ✗ | ✓ | — |
| Library import | folder scan | saved tracks, own/collaborative playlists | — | likes, own playlists | — |
| Search | local DB | `GET /search`, **max 10 per page** | — | `GET /tracks?q=` (or web mode) | — |
| Metadata editing | ✓ in-file tags + app overrides | app overrides only | app overrides only | app overrides only | — |
| Scrobbling | ✓ Now Playing + scrobble | ✓ Now Playing + scrobble (toggle: *Scrobble Spotify tracks*) | ✗ (nothing plays) | ✓ Now Playing + scrobble | — |
| Gapless / crossfade | ✗ | ✗ | ✗ | ✗ | — |

The live version of this table is in **Settings → Capabilities**. It is generated from the same adapter declarations the player uses.

---

## 2. Spotify

### What the app uses

- **Auth:** Authorization Code with PKCE (`accounts.spotify.com/authorize`, `/api/token`). No client secret is stored. Tokens are kept in Windows Credential Manager and refreshed automatically. If a refresh response omits `refresh_token`, the previous one is kept.
- **Scopes:** `user-read-playback-state user-modify-playback-state user-read-currently-playing user-library-read playlist-read-private playlist-read-collaborative streaming user-read-email user-read-private`. The last three are what the Web Playback SDK checks; without them it fails with "Invalid token scopes". Older sign-ins without them are flagged in Settings with a **Reconnect Spotify** button (refreshing a token never adds scopes).
- **Library:** `GET /me/tracks`, `GET /me/playlists`, `GET /playlists/{id}/items` (renamed from `/tracks`; reads `items[].item` and falls back to `.track`), `GET /search?type=track&limit=10`, `GET /me`.
- **Playback:** `GET /me/player`, `GET /me/player/devices`, `PUT /me/player/play` (`uris`, `position_ms`), `PUT /me/player/pause`, `PUT /me/player/seek`, `PUT /me/player/volume`, `PUT /me/player` (transfer), `POST /me/player/queue`.

### Setup

1. Go to <https://developer.spotify.com/dashboard> and create an app. Select **Web API**, and **Web Playback SDK** if you want to try the in-app device.
2. Add this redirect URI exactly: `http://127.0.0.1:43821/callback/spotify`. It's also shown in Settings and changes if you change the OAuth port. Spotify rejects `localhost`, so loopback redirects must use the IP literal.
3. Copy the **Client ID** into Settings → Accounts → Spotify, save, then press **Connect Spotify**.
4. In Development Mode, add each user's Spotify account under *User Management* in the dashboard.

### Restrictions that apply

- **Premium.**
  - All `/me/player/*` control endpoints require Spotify Premium. On `403 PREMIUM_REQUIRED` the app disables Spotify playback controls and says why.
  - Since February 2026 the *app owner* must also have Premium for a Development Mode app to work at all.
- **Development Mode:**
  - **5 users** per new app.
  - Up to 25 Client IDs per developer account (raised in July 2026), with the quota shared across them.
  - A `429` with `"reason": "QUOTA_EXCEEDED"` is surfaced immediately and not retried.
- **Removed in Development Mode (Feb 2026):**
  - Batch `GET /tracks|albums|artists`, `GET /artists/{id}/top-tracks`, `browse/new-releases`, `browse/categories`, and `GET /users/{id}[/playlists]`.
  - The `popularity`, `available_markets`, `label` and `followers` fields.
  - `product`, `email` and `country` on `/me`. Because `product` is gone, the app can't detect Premium up front and only learns it from `PREMIUM_REQUIRED` errors.
  - The app doesn't use any of these.
- **Search:** `limit` is at most 10 (default 5). Use *Load 10 more* to page.
- **Playlists:** `/playlists/{id}/items` only returns items for playlists you **own or collaborate on**. Others are skipped during import and listed in the import summary.
- **Where audio plays:**
  - Spotify audio plays on a **Spotify Connect device**, such as a phone, smart speaker, TV, `open.spotify.com` in a browser, or the official client.
  - The app sends documented Web API commands to that device. It does not automate or depend on any particular Spotify application, and it never downloads, extracts or decodes Spotify audio.
  - If no device is available, the play button stays disabled with *"Choose a Spotify Connect device"*.
- **In-app device (experimental):**
  - Spotify's Web Playback SDK needs Widevine DRM (EME), and Spotify only supports it in desktop browsers, not embedded webviews.
  - The app probes `requestMediaKeySystemAccess("com.widevine.alpha")` in WebView2 and loads the SDK only if that succeeds. Otherwise it reports the reason.
  - **Expect this to be unavailable** on standard WebView2 runtimes.
  - The SDK API requires giving the page an access token. This is the only place a token enters the webview.
- **Scrobbling:** Spotify plays that go through this app are scrobbled to Last.fm using the first credited artist. Turn this off under Settings → Last.fm → *Scrobble Spotify tracks* if your Spotify account is already linked to Last.fm in Spotify's own settings, or every play will be scrobbled twice.
- **Not supported:** downloading or caching audio, gapless or crossfade.
- **Playback position** comes from polling `GET /me/player` and is approximate. End of track is inferred from progress, so with Spotify *Autoplay* on, a track may briefly hand over to Spotify's autoplay before the app moves to the next queue item.

---

## 3. SoundCloud

### What the app uses

- **Auth:**
  - OAuth 2.1 Authorization Code + PKCE (S256) at `https://secure.soundcloud.com/authorize` and `/oauth/token`.
  - SoundCloud treats every client as confidential, so the **client secret** is sent on token requests. It's stored in Credential Manager.
  - Access tokens last about 1 hour. **Refresh tokens are single-use**, and each new one is saved immediately.
  - Requests use the header `Authorization: OAuth <token>`.
- **Library:** `GET /me`, `GET /me/likes/tracks`, `GET /me/playlists`, `GET /playlists/{urn}/tracks`, `GET /tracks?q=` (all with `linked_partitioning=true`, following `next_href`).
- **Playback:**
  - `GET /tracks/{urn}/streams` returns stream URLs. The app prefers `hls_aac_160_url`, the current AAC HLS format. The deprecated MP3/Opus fields and `preview_mp3_128_url` are only fallbacks.
  - The API stream URL is resolved in Rust, with the token, to its signed CDN URL. The webview then plays that URL through hls.js. The OAuth token never reaches the page.
- **Availability:**
  - `access: playable` plays in full.
  - `preview` is marked *PREVIEW* and plays a preview only.
  - `blocked`, or `streamable: false`, is marked unavailable and skipped in the queue.
- **Attribution** (required for custom players): while a SoundCloud track plays, the player shows the uploader's name with a link to their profile and a "Listen on SoundCloud" link to the track page.

### Setup

1. **A SoundCloud Artist Pro subscription is required to register an API app** (current SoundCloud developer docs).
2. Register an app at <https://soundcloud.com/you/apps> or with SoundCloud's API-key CLI.
3. Add the redirect URI `http://127.0.0.1:43821/callback/soundcloud`.
   - SoundCloud's guide shows custom URL schemes for desktop apps and doesn't explicitly cover loopback HTTP redirects.
   - If SoundCloud rejects the loopback URI for your app, connecting won't work. A custom-scheme flow is not implemented (see the implementation report).
4. Paste the **client ID** and **client secret** into Settings → Accounts → SoundCloud, save, and press **Connect**.

### Restrictions

- Stream requests are limited to **15,000 per 24 hours per client ID**. A `429` is shown as a rate-limit error.
- Client-credentials tokens are limited to 50 per 12 h per app. This app doesn't use client credentials; it only uses user tokens.
- **Not verified live.** The streaming flow follows the documentation and is covered by mock tests, but it hasn't been run against real SoundCloud credentials. Two points depend on SoundCloud's servers:
  - whether the `/streams` URLs redirect to a CDN URL that WebView2 can fetch cross-origin;
  - which CDN hostnames are used (the CSP allows `*.sndcdn.com` and `*.soundcloud.cloud`).
  If the API returns a stream URL that can only be fetched with the token, the app reports *"stream requires authenticated access"* rather than leaking the token to the page.
- **Scrobbling:** SoundCloud plays are scrobbled to Last.fm like local files, using the artist credit (or the uploader's name when there is none) and the track title. 30-second previews never reach the half-length rule, so they are not scrobbled.
- **Not supported:** downloads, uploads, comments, reposts, follows.

### Web mode (unofficial, opt-in)

Added at the project owner's request, as an explicit exception to "official APIs only". It is **off by default**. Turn it on under Settings → Accounts → SoundCloud → *Web mode*.

- **How it works:** the backend reads the public `client_id` that soundcloud.com embeds in its own JavaScript (the approach yt-dlp uses) and calls the website's internal `api-v2.soundcloud.com` endpoints:
  - `search/tracks`, `resolve`, `tracks?ids=` (to fill in playlist stubs) and `users/{id}/likes`.
  - Streams come from each track's `media.transcodings`. The app prefers full-length AAC HLS, then standard AES-HLS or progressive MP3.
  - On a 401 or 403 it fetches a fresh `client_id` once.
- **Guardrails:**
  - DRM transcodings (`ctr-*` / `cbc-*`, used for Go+) are never selected, so Go+ tracks play only the free 30-second preview.
  - Nothing is downloaded.
  - No sign-in, so only public content is available.
  - If an official SoundCloud app is connected, the official API is used instead.
- **Adding music:** search, or paste a track, playlist or profile link. A profile link imports that user's public likes as a playlist.
- **Risks:**
  - It is undocumented and breaks whenever SoundCloud changes its site.
  - SoundCloud may rate-limit or block it.
  - It may violate SoundCloud's Terms of Use.
  - It hasn't been tested against the live site; it's covered by mock tests only.

---

## 4. Last.fm

- **Scope:** Now Playing and scrobbling for **local, Spotify and SoundCloud tracks**. Spotify can be turned off separately (*Scrobble Spotify tracks*, on by default). The backend's `scrobbler::source_allowed` enforces this no matter what the UI sends. Unit tests cover this.
- **Artist name:** Spotify joins multiple artists as `A, B`; scrobbles send only the first credited artist (`tracks.primary_artist`, migration 0003). Spotify tracks imported before that migration fall back to the album artist when the joined name starts with it; re-importing fills in the primary artist. A user artist override always wins.
- **Auth** uses the desktop flow: `auth.getToken` → the browser opens `last.fm/api/auth/?api_key=…&token=…` → the app polls `auth.getSession` every 3 s for up to 5 minutes. The session key goes in Credential Manager.
- **Signing:** `api_sig` is the MD5 of the parameters sorted by name and concatenated as `namevalue` (excluding `format` and `callback`), followed by the shared secret. Every call is a form-encoded POST.
- **Rules** (from <https://www.last.fm/api/scrobbling>):
  - A track is scrobbled if it is longer than 30 s and has played for at least half its length or 4 minutes, whichever comes first.
  - Only real listening time counts, not seek jumps.
  - The timestamp is the playback start (UTC).
  - Now Playing is sent when a track starts and is not retried on failure.
- **Cache:**
  - Every scrobble goes into the `scrobble_queue` table first, which survives restarts.
  - Scrobbles are sent oldest first in batches of 50, after each play and every 5 minutes.
  - Errors **11** and **16** and network failures stay queued for retry.
  - Error **9** (invalid session) keeps the scrobbles and disconnects the account so you can reconnect.
  - Any other error marks the batch *rejected*, per Last.fm's "don't retry malformed requests". You can discard rejected scrobbles in Settings.

### Setup

1. Create an API account at <https://www.last.fm/api/account/create>. The callback URL can be left blank for the desktop flow.
2. Paste the **API key** and **shared secret** into Settings → Accounts → Last.fm, save, and press **Connect**. Approve in the browser; the app finishes on its own.

---

## 5. Local files

| Format | Read | Play in app | Write tags (native tag) |
|---|---|---|---|
| MP3 | ✓ | ✓ | ✓ ID3v2 |
| FLAC | ✓ | ✓ | ✓ Vorbis Comments (pictures preserved) |
| M4A / M4B (AAC) | ✓ | ✓ | ✓ MP4 ilst |
| M4A (ALAC) | ✓ | ✗ (Chromium has no ALAC decoder) | ✓ MP4 ilst |
| AAC (ADTS) | ✓ | ✓ | ✓ ID3v2 |
| Ogg Vorbis / Opus | ✓ | ✓ | ✓ Vorbis Comments |
| Speex | ✓ | ✗ | ✓ Vorbis Comments |
| WAV | ✓ | ✓ (PCM) | ✓ ID3v2 (RIFF INFO left untouched) |
| AIFF | ✓ | ✗ | ✓ ID3v2 |
| APE / WavPack / Musepack | ✓ | ✗ | ✓ APEv2 |
| WMA / ASF, DSF / DFF, MKA, TTA | ✗ (counted as unsupported in scan reports) | ✗ | ✗ |

- Files that can be read but not played are imported, marked *unavailable* with the reason, and skipped by the queue. Their metadata can still be edited.
- Editing a file that is currently playing can fail if Windows reports the file as in use. Pause it and retry.

---

## Sources

- Spotify — [February 2026 Dev Mode migration guide](https://developer.spotify.com/documentation/web-api/tutorials/february-2026-migration-guide), [Feb 2026 changelog](https://developer.spotify.com/documentation/web-api/references/changes/february-2026), [July 2026 changelog](https://developer.spotify.com/documentation/web-api/references/changes/july-2026), [Quota update blog (23 Jul 2026)](https://developer.spotify.com/blog/2026-07-23-web-api-quota-updates), [Web Playback SDK](https://developer.spotify.com/documentation/web-playback-sdk)
- SoundCloud — [API guide](https://developers.soundcloud.com/docs/api/guide), [Register an app](https://developers.soundcloud.com/docs/api/register-app), [LLM context / rate limits](https://developers.soundcloud.com/docs/llm-context), [AAC HLS streaming announcement](https://developers.soundcloud.com/blog/api-streaming-urls)
- Last.fm — [Scrobbling 2.0](https://www.last.fm/api/scrobbling), [Desktop auth](https://www.last.fm/api/desktopauth)
