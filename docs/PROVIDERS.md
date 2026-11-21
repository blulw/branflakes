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
- **Scopes:** `user-read-playback-state user-modify-playback-state user-read-currently-playing user-library-read playlist-read-private playlist-read-collaborative streaming user-read-email user-read-private playlist-modify-public playlist-modify-private ugc-image-upload`. The last three were added for playlist sync; sign-ins from before that are flagged **Reconnect Spotify**. The last three are what the Web Playback SDK checks; without them it fails with "Invalid token scopes". Older sign-ins without them are flagged in Settings with a **Reconnect Spotify** button (refreshing a token never adds scopes).
- **Followed artists:** `GET /search?type=artist&limit=10`, `GET /artists/{id}`, `GET /artists/{id}/albums` (`include_groups=album,single`, `limit=10`, paged), `GET /albums/{id}` (its tracks, following `tracks.next`). Only releases not read before are fetched, so a daily sync costs a few requests. Tracks are kept only when they credit the artist, and a song already found on an earlier release (same title and length) is not added again.
- **Library:** `GET /me/tracks`, `GET /me/playlists`, `GET /playlists/{id}/items` (renamed from `/tracks`; reads `items[].item` and falls back to `.track`), `GET /search?type=track&limit=10`, `GET /me`.
- **Playlist lifecycle:** `POST /me/playlists` (create, sent with `public: false`), `DELETE /me/library?uris=spotify:playlist:{id}` (remove from library), `GET /me/library/contains?uris=spotify:playlist:{id}` (still in your library?). The pre-2026 follow/unfollow and `/users/{id}/playlists` endpoints were removed by Spotify and are not used.
- **Playlist sync:** `GET /playlists/{id}` (`fields=id,name,snapshot_id,collaborative,owner(id),images`), `GET /playlists/{id}/items`, `POST /playlists/{id}/items` (`uris`, `position`), `DELETE /playlists/{id}/items` (`items[{uri}]`, `snapshot_id`), `PUT /playlists/{id}/items` (reorder `range_start`/`insert_before`/`range_length`, or replace `uris`), `PUT /playlists/{id}` (`name`), `PUT /playlists/{id}/images` (base64 JPEG). Cover images are downloaded from the URL in the playlist's `images`. The deprecated/removed `/tracks` variants are not used.
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
- **Playlist sync** (see below).
- **Scrobbling:** Spotify plays that go through this app are scrobbled to Last.fm using the first credited artist. Turn this off under Settings → Last.fm → *Scrobble Spotify tracks* if your Spotify account is already linked to Last.fm in Spotify's own settings, or every play will be scrobbled twice.
- **Not supported:** downloading or caching audio, gapless or crossfade.
- **Playback position** comes from polling `GET /me/player` and is approximate. End of track is inferred from progress, so with Spotify *Autoplay* on, a track may briefly hand over to Spotify's autoplay before the app moves to the next queue item.

### Playlist sync

**Which playlists are synced.**
- **Imported** Spotify playlists become Spotify Synced automatically, both during *Import* and, for earlier imports, in the background.
  - Re-importing never duplicates a playlist.
  - Old unsynced copies of the same Spotify playlist are removed when its synced copy is set up, but only if they hold nothing but Spotify tracks. Copies with your own local or SoundCloud additions, and Branflakes-only copies, are kept.
  - Playlists that can't be synced (someone else's, missing permissions, or local/SoundCloud tracks in the Branflakes copy) are imported as ordinary playlists, and the import summary says why.
- **New playlists** are local-only. Turning on *Also create it on Spotify* when creating one creates a **private** playlist in your Spotify account and syncs it.
  - Local-only playlists, including Branflakes-only copies, can be linked later with **Sync with Spotify…**, after a confirmation.
  - If creation fails, the Branflakes playlist stays. A retry never makes a second Spotify playlist: an earlier attempt whose answer was lost is adopted.

**Deleting.** Spotify has **no API to permanently delete a playlist**. "Delete" in Spotify's own apps removes the playlist from your library, and so does Branflakes (`DELETE /me/library`).
- Deleting a synced playlist opens a confirmation explaining exactly that.
  - For **your own** playlist, it is deleted from your Spotify account (and can be restored from your Spotify account page).
  - For **someone else's** playlist, Branflakes only unfollows it; the owner's playlist is not changed.
  - You can also choose *Keep on Spotify, delete here only*.
- The Branflakes copy is removed only after Spotify confirms the playlist is gone from your library. A failure changes nothing.
- An interrupted deletion is finished or undone after a restart.
- Deleting a local-only playlist or a Branflakes-only copy never touches Spotify.

**Removed outside Branflakes.** Each sync checks that the playlist still exists (`GET /playlists/{id}`) and is still in your library (`GET /me/library/contains`).
- A `404` ("no longer exists"), or a playlist missing from your library (Spotify's delete, or an unfollow), removes the Branflakes copy only when a **second, separate check** at least a minute later says the same.
- The message distinguishes the two cases.
- Network errors, timeouts, rate limits, expired sign-ins, permission errors and 5xx answers never count as a removal. The playlist and its link are kept, and the error is shown.

Turn it on from a Spotify playlist's page (**Sync with Spotify**) or under Settings → Accounts → Spotify → *Choose playlists to sync…*. Synced playlists show a small Spotify mark in the sidebar, the playlist header and the *Add to playlist* menu. The playlist page has a **Sync Spotify Playlist** button next to the sort control, plus the last-synced time and any error.

What syncs, in both directions: added and removed tracks, track order, the playlist name, and the cover. It runs automatically every 120 minutes by default while the app is running (Settings → Accounts → Spotify), a few seconds after each edit in Branflakes, and on demand.

Spotify API limits that shape the behaviour:
- **Ownership.**
  - Only playlists you **own or collaborate on** can be synced. Spotify returns items only for those playlists (`403` otherwise), and only the owner may change details.
  - **Collaborative** playlists owned by someone else sync tracks only. Renaming and cover changes are blocked in Branflakes, with an explanation.
  - If you lose access later, the sync reports an error and changes nothing.
- **No cover removal.** The API can set a custom cover but not remove one. Removing the cover in Branflakes is local only, and Spotify keeps its cover.
- **Cover format.** Uploads must be base64 JPEG ≤ 256 KB. Branflakes re-encodes (≤ 640 px, lowering the quality as needed). If Spotify rejects the image, the Branflakes cover is kept, the failure is shown, and the same image is retried only on a manual sync.
- **Cover storage.** Spotify image URLs expire within a day, and Spotify asks apps not to store images unless necessary. The playlist cover is cached locally because Branflakes displays it as the playlist's artwork.
- **Detecting cover changes.** The API has no cover version, ETag or hash, and the docs don't say whether uploading a cover changes `snapshot_id`.
  - *Automatic sync:* compares the cover's identity, which is the URL path without host, size or query. Spotify image ids live in the path and have been observed to change with the image, but this isn't documented. Nothing is downloaded unless the identity changes.
  - *Manual sync* (Sync Spotify Playlist): also downloads the current cover and compares it by content hash. This catches a cover replaced behind an unchanged URL.
  - So such a change is picked up on the next manual sync, not automatically.
- **Removal is by URI.** `DELETE …/items` removes every occurrence of a URI. When only one of several duplicates should go, Branflakes removes the URI and re-adds the remaining copies at their positions.
- **Snapshots don't lock.** `snapshot_id` is not an optimistic lock; Spotify applies changes even if the playlist changed. Branflakes therefore re-reads the snapshot right before writing and starts over if it moved, then reads the result back.
- **Branflakes can't hold these items.** Spotify local files, podcast episodes and unavailable entries can't be represented, so Branflakes never edits them. They keep their place on Spotify, and a full replace is never used while they're present.
- **Rate limits and errors.**
  - Rate limits honour `Retry-After`. A `QUOTA_EXCEEDED` response pauses automatic sync for an hour.
  - Expired or revoked sign-ins show **Reconnect Spotify**. Offline periods retry every minute until the connection returns.

### Find alternative

An unavailable Spotify track is one Spotify itself reports as not playable (`is_playable: false` / `restrictions`) in a successful response. Temporary errors never mark a track unavailable.
- **Searching.** In a playlist, such a track has a **Find alternative** button.
  - It searches your **local library** first, then **SoundCloud**: the official API if connected, otherwise web mode if enabled. SoundCloud is searched even when there are local matches.
  - Results must match both the **title and the artist**. Case, punctuation, spacing, "feat." credits and "(Official Video)"-style noise are ignored. Exact matches rank first, and duplicate results are removed.
- **Adding.** Your pick is added **right after** the original. The original Spotify track is never removed or replaced.
- **In a Spotify Synced playlist**, the alternative exists only in Branflakes (Spotify can't hold local or SoundCloud tracks):
  - the sync engine keeps it next to its original and never sends it to Spotify;
  - Spotify-only entries that were never imported (shown by Spotify as empty items) have no title, so they can't be searched.
- **Remembered choices.** The choice is remembered across restarts. A removed alternative is never added back automatically, and choosing it again explicitly works.

Adding a **local or SoundCloud track** to a synced playlist never touches Spotify. Branflakes makes a new Branflakes-only playlist, “Name (Branflakes)”, with the playlist's tracks plus the new track. The copy shows a fork mark ("Branflakes-only copy of …"), is never synced, and can be edited independently.
- **Which version is copied.** Before copying, Branflakes reads Spotify's current playlist and merges it with any Branflakes edits that haven't synced yet, exactly as the next sync would. This is read-only: nothing is written to Spotify, the synced playlist or its sync state.
- **If Spotify can't be checked** (not connected, offline, sign-in problem, or no answer within 20 s), the copy uses the playlist as Branflakes last synced it. The toast (a warning) and the copy's page both say so.
- **Position.** The new track goes right after the track you inserted it after, even if Spotify added tracks before it.
- **Entries that are never copied**, because Branflakes can't hold them:
  - Spotify **local files** (`spotify:local:…`);
  - **podcast episodes**;
  - **unavailable entries**, for which Spotify returns no item.

  When any are left out, the toast is a warning and the copy's page shows a note such as “Not copied: 2 (1 podcast episode, 1 Spotify local file): …. Branflakes can't hold these, so this copy is not complete.”

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
