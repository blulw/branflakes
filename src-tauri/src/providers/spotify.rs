//! SpotifyAdapter — official Web API only.
//!
//! * Auth: Authorization Code with PKCE (no client secret is stored).
//! * Library: saved tracks, the user's own/collaborative playlists, search.
//! * Playback: Spotify Connect remote control through the documented
//!   `/me/player/*` endpoints. These require Spotify **Premium** and an active
//!   Spotify Connect device. Audio is never downloaded or extracted.
//!
//! Written against the February 2026 Development Mode changes: search
//! `limit` max 10, `/playlists/{id}/items` (formerly `/tracks`), playlist item
//! field `item` (formerly `track`), no batch GET endpoints, `/me` no longer
//! returns `product` (so Premium is detected from `PREMIUM_REQUIRED` errors).

use crate::error::{AppError, AppResult};
use crate::http::{self, RetryPolicy};
use crate::model::{Availability, PlaybackCapabilities, ProviderStatus, TrackUpsert};
use crate::secrets::{Secret, SecretStore, SPOTIFY_TOKENS};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

pub const API_BASE: &str = "https://api.spotify.com/v1";
pub const ACCOUNTS_BASE: &str = "https://accounts.spotify.com";
const PROVIDER: &str = "spotify";

/// Scopes requested at sign-in.
pub const SCOPES: &[&str] = &[
    "user-read-playback-state",
    "user-modify-playback-state",
    "user-read-currently-playing",
    "user-library-read",
    "playlist-read-private",
    "playlist-read-collaborative",
    "streaming",
    "user-read-email",
    "user-read-private",
    // Two-way playlist sync (Change Playlist Details, /items writes) and
    // custom playlist covers.
    "playlist-modify-public",
    "playlist-modify-private",
    "ugc-image-upload",
];

/// Scopes needed to write to playlists (sync).
pub const PLAYLIST_WRITE_SCOPES: &[&str] = &["playlist-modify-public", "playlist-modify-private"];
/// Scope needed to upload a custom playlist cover.
pub const IMAGE_UPLOAD_SCOPE: &str = "ugc-image-upload";

/// Scopes the Web Playback SDK checks for. Spotify's official SDK example
/// requests exactly these; with fewer, the SDK reports
/// `authentication_error: Invalid token scopes`.
pub const SDK_SCOPES: &[&str] = &["streaming", "user-read-email", "user-read-private"];

/// Which of `required` are not present in a space-separated scope string.
pub fn missing_scopes(granted: &str, required: &[&'static str]) -> Vec<&'static str> {
    let have: std::collections::HashSet<&str> = granted.split_whitespace().collect();
    required.iter().copied().filter(|s| !have.contains(s)).collect()
}

/// Spotify's limit for the base64 cover upload payload.
pub const COVER_MAX_BASE64: usize = 256 * 1024;

/// Development Mode maximum for `GET /search` `limit`.
pub const SEARCH_LIMIT: u32 = 10;

#[derive(Clone, Serialize, Deserialize)]
pub struct SpotifyTokens {
    pub access_token: Secret,
    pub refresh_token: Option<Secret>,
    /// Unix seconds.
    pub expires_at: i64,
    pub scope: String,
}

impl std::fmt::Debug for SpotifyTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpotifyTokens").field("expires_at", &self.expires_at).finish_non_exhaustive()
    }
}

/// Deserialised from Spotify's snake_case JSON, serialised to the UI as camelCase.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct SpotifyDevice {
    pub id: Option<String>,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub is_active: bool,
    pub is_restricted: bool,
    pub supports_volume: bool,
    pub volume_percent: Option<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpotifyPlaybackState {
    pub device: Option<SpotifyDevice>,
    pub is_playing: bool,
    pub progress_ms: Option<u64>,
    pub item_uri: Option<String>,
    /// When Spotify relinks a track (the requested version isn't available in
    /// the user's market), `item_uri` is the substitute and this is the URI
    /// that was actually requested. Only present when `market` is sent.
    pub linked_from_uri: Option<String>,
    pub item_duration_ms: Option<u64>,
    /// Actions Spotify reports as disallowed for the current context.
    pub disallows: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpotifyPlaylistSummary {
    pub id: String,
    pub name: String,
    pub owner_id: Option<String>,
    pub total: Option<u32>,
    pub collaborative: bool,
}

/// `GET /playlists/{id}` metadata used by sync (no items).
#[derive(Debug, Clone, PartialEq)]
pub struct PlaylistMeta {
    pub id: String,
    pub name: String,
    pub snapshot_id: Option<String>,
    pub owner_id: Option<String>,
    pub collaborative: bool,
    /// Cover URLs, largest first. Spotify says these expire within a day.
    pub images: Vec<String>,
}

/// One entry of a playlist, in Spotify's order.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteItem {
    /// `spotify:track:…`, `spotify:episode:…`, `spotify:local:…`, or
    /// `spotify:unavailable:<offset>` when Spotify returned no item.
    pub uri: String,
    /// Mapped track for `spotify:track:` items.
    pub track: Option<TrackUpsert>,
    /// Display name, when Spotify returned one (used to report entries that
    /// Branflakes can't copy).
    pub name: Option<String>,
}

/// Request body for an authorised call.
enum Body<'a> {
    None,
    Json(&'a Value),
    /// Raw text with an explicit content type (cover upload).
    Raw(&'static str, &'a str),
}

pub struct SpotifyAdapter {
    http: reqwest::Client,
    api_base: String,
    accounts_base: String,
    policy: RetryPolicy,
    secrets: Arc<dyn SecretStore>,
    cache: tokio::sync::Mutex<Option<SpotifyTokens>>,
}

impl SpotifyAdapter {
    pub fn new(http: reqwest::Client, secrets: Arc<dyn SecretStore>) -> Self {
        Self::with_base(http, secrets, API_BASE, ACCOUNTS_BASE, RetryPolicy::default())
    }

    pub fn with_base(http: reqwest::Client, secrets: Arc<dyn SecretStore>, api: &str, accounts: &str, policy: RetryPolicy) -> Self {
        SpotifyAdapter {
            http,
            api_base: api.trim_end_matches('/').into(),
            accounts_base: accounts.trim_end_matches('/').into(),
            policy,
            secrets,
            cache: tokio::sync::Mutex::new(None),
        }
    }

    pub fn capabilities(connected: bool, premium_denied: bool) -> PlaybackCapabilities {
        if !connected || premium_denied {
            return PlaybackCapabilities::NONE;
        }
        PlaybackCapabilities {
            play: true,
            pause: true,
            next: true,     // app queue
            previous: true, // app queue
            seek: true,
            volume: true, // only when the active device reports supports_volume
            queue: true,
            device_selection: true,
            playback_position: true, // polled from GET /me/player, approximate
            duration: true,
        }
    }

    pub fn status(&self, configured: bool, account: Option<String>, premium_denied: bool) -> ProviderStatus {
        let connected = self.secrets.has(SPOTIFY_TOKENS);
        let missing: Vec<String> = if connected { self.stored_missing_scopes().iter().map(|s| s.to_string()).collect() } else { Vec::new() };
        let needs_reconnect = !missing.is_empty();
        let mut limitations = vec![
            "Playback control requires Spotify Premium (Spotify returns PREMIUM_REQUIRED otherwise).".into(),
            "Playback happens on a Spotify Connect device (phone, speaker, web player, etc.). This app sends documented Web API commands; it never downloads, extracts or decodes Spotify audio.".into(),
            "An in-app Spotify device (Web Playback SDK) is attempted only when the embedded WebView2 reports Widevine DRM support; Spotify does not officially support embedded webviews.".into(),
            "Development Mode apps are limited to 5 authorised users, the app owner must have Premium, and search returns at most 10 results per page.".into(),
            "Playback position is polled from Spotify and is approximate. Volume works only on devices that support remote volume.".into(),
            "Spotify plays are never scrobbled to Last.fm by this app.".into(),
        ];
        if premium_denied {
            limitations.insert(0, "Spotify reported that this account does not have Premium: playback controls are disabled.".into());
        }
        if needs_reconnect {
            limitations.insert(
                0,
                "This sign-in is missing permissions the app now needs (for the in-app device and Spotify playlist sync). Click Reconnect Spotify.".into(),
            );
        }
        ProviderStatus {
            provider: PROVIDER.into(),
            display_name: "Spotify".into(),
            configured,
            connected,
            needs_reconnect,
            missing_scopes: missing,
            account_name: account,
            capabilities: Self::capabilities(connected, premium_denied),
            limitations,
        }
    }

    /// Scopes from `SCOPES` that the stored token was not granted. Refreshing a
    /// token never adds scopes, so a non-empty result means the user must sign in again.
    pub fn stored_missing_scopes(&self) -> Vec<&'static str> {
        match self.secrets.get(SPOTIFY_TOKENS) {
            Ok(Some(raw)) => match serde_json::from_str::<SpotifyTokens>(&raw) {
                Ok(t) => missing_scopes(&t.scope, SCOPES),
                Err(_) => SCOPES.to_vec(),
            },
            _ => Vec::new(),
        }
    }

    /// Access token for the Web Playback SDK. Refuses early, with an actionable
    /// message, if the grant lacks the scopes the SDK checks.
    pub async fn sdk_access_token(&self, client_id: &str) -> AppResult<Secret> {
        let token = self.access_token(client_id).await?;
        let t = self.load_tokens().await?;
        let missing = missing_scopes(&t.scope, SDK_SCOPES);
        if !missing.is_empty() {
            return Err(AppError::Auth {
                provider: PROVIDER.into(),
                message: format!(
                    "your Spotify sign-in is missing permissions the in-app player needs ({}). Go to Settings > Accounts and click Reconnect Spotify",
                    missing.join(", ")
                ),
            });
        }
        Ok(token)
    }

    // ------------------------------------------------------------- auth

    pub fn authorize_url(&self, client_id: &str, redirect_uri: &str, code_challenge: &str, state: &str) -> String {
        let mut u = url::Url::parse(&format!("{}/authorize", self.accounts_base)).expect("valid accounts url");
        u.query_pairs_mut()
            .append_pair("client_id", client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("code_challenge_method", "S256")
            .append_pair("code_challenge", code_challenge)
            .append_pair("state", state)
            .append_pair("scope", &SCOPES.join(" "));
        u.into()
    }

    fn parse_token_response(v: &Value, previous_refresh: Option<Secret>, previous_scope: Option<String>) -> AppResult<SpotifyTokens> {
        let access = v.get("access_token").and_then(Value::as_str).ok_or_else(|| AppError::Auth {
            provider: PROVIDER.into(),
            message: "token response missing access_token".into(),
        })?;
        let expires_in = v.get("expires_in").and_then(Value::as_i64).unwrap_or(3600);
        Ok(SpotifyTokens {
            access_token: Secret::new(access),
            // Spotify may omit refresh_token on refresh: keep the previous one.
            refresh_token: v.get("refresh_token").and_then(Value::as_str).map(Secret::new).or(previous_refresh),
            expires_at: chrono::Utc::now().timestamp() + expires_in,
            // Keep the previously granted scopes if a refresh response omits them.
            scope: v
                .get("scope")
                .and_then(Value::as_str)
                .map(String::from)
                .or(previous_scope)
                .unwrap_or_default(),
        })
    }

    async fn token_request(&self, form: Vec<(&str, String)>) -> AppResult<Value> {
        let url = format!("{}/api/token", self.accounts_base);
        let resp = http::send(PROVIDER, self.policy, || self.http.post(&url).form(&form)).await.map_err(|e| match e {
            AppError::Http { status: 400, message, .. } => AppError::Auth { provider: PROVIDER.into(), message },
            other => other,
        })?;
        resp.json().await.map_err(|e| AppError::Network { provider: PROVIDER.into(), message: http::sanitize_reqwest_error(&e) })
    }

    pub async fn exchange_code(&self, client_id: &str, code: &str, verifier: &str, redirect_uri: &str) -> AppResult<()> {
        let v = self
            .token_request(vec![
                ("grant_type", "authorization_code".into()),
                ("code", code.into()),
                ("redirect_uri", redirect_uri.into()),
                ("client_id", client_id.into()),
                ("code_verifier", verifier.into()),
            ])
            .await?;
        let tokens = Self::parse_token_response(&v, None, None)?;
        self.store_tokens(tokens).await
    }

    async fn store_tokens(&self, tokens: SpotifyTokens) -> AppResult<()> {
        self.secrets.set(SPOTIFY_TOKENS, &serde_json::to_string(&tokens)?)?;
        *self.cache.lock().await = Some(tokens);
        Ok(())
    }

    pub async fn disconnect(&self) -> AppResult<()> {
        *self.cache.lock().await = None;
        self.secrets.delete(SPOTIFY_TOKENS)
    }

    async fn load_tokens(&self) -> AppResult<SpotifyTokens> {
        let mut cache = self.cache.lock().await;
        if let Some(t) = cache.as_ref() {
            return Ok(t.clone());
        }
        let raw = self.secrets.get(SPOTIFY_TOKENS)?.ok_or(AppError::NotConnected { provider: PROVIDER.into() })?;
        let t: SpotifyTokens = serde_json::from_str(&raw)?;
        *cache = Some(t.clone());
        Ok(t)
    }

    async fn refresh(&self, client_id: &str, current: &SpotifyTokens) -> AppResult<SpotifyTokens> {
        let refresh = current.refresh_token.clone().ok_or_else(|| AppError::Auth {
            provider: PROVIDER.into(),
            message: "no refresh token; please reconnect Spotify".into(),
        })?;
        let v = self
            .token_request(vec![
                ("grant_type", "refresh_token".into()),
                ("refresh_token", refresh.expose().to_string()),
                ("client_id", client_id.into()),
            ])
            .await?;
        let tokens = Self::parse_token_response(&v, Some(refresh), Some(current.scope.clone()))?;
        self.store_tokens(tokens.clone()).await?;
        Ok(tokens)
    }

    /// A valid access token, refreshed when within 60 s of expiry.
    pub async fn access_token(&self, client_id: &str) -> AppResult<Secret> {
        let t = self.load_tokens().await?;
        if t.expires_at - 60 > chrono::Utc::now().timestamp() {
            return Ok(t.access_token);
        }
        Ok(self.refresh(client_id, &t).await?.access_token)
    }

    // -------------------------------------------------------- requests

    /// Authorised request; on 401 refresh once and retry.
    async fn request(
        &self,
        client_id: &str,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> AppResult<Option<Value>> {
        let body = match body {
            Some(b) => Body::Json(b),
            None => Body::None,
        };
        self.request_with(client_id, method, path, query, body).await
    }

    async fn request_with(
        &self,
        client_id: &str,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Body<'_>,
    ) -> AppResult<Option<Value>> {
        let url = if path.starts_with("http") { path.to_string() } else { format!("{}{}", self.api_base, path) };
        for attempt in 0..2 {
            let token = self.access_token(client_id).await?;
            // POST creates or appends on Spotify: never blindly repeat it.
            let policy = if method == reqwest::Method::POST { self.policy.non_idempotent() } else { self.policy };
            let res = http::send(PROVIDER, policy, || {
                let mut rb = self.http.request(method.clone(), &url).bearer_auth(token.expose()).query(query);
                rb = match &body {
                    Body::Json(b) => rb.json(b),
                    Body::Raw(ct, text) => rb.header(reqwest::header::CONTENT_TYPE, *ct).body(text.to_string()),
                    // Spotify player PUT/POST endpoints expect a Content-Length.
                    Body::None if method != reqwest::Method::GET => rb.header(reqwest::header::CONTENT_LENGTH, 0),
                    Body::None => rb,
                };
                rb
            })
            .await;
            match res {
                Ok(resp) => {
                    if resp.status() == reqwest::StatusCode::NO_CONTENT {
                        return Ok(None);
                    }
                    let text = resp.text().await.unwrap_or_default();
                    if text.trim().is_empty() {
                        return Ok(None);
                    }
                    // Some player endpoints return non-JSON bodies (e.g. a snapshot id string).
                    return Ok(serde_json::from_str(&text).ok());
                }
                Err(AppError::Auth { .. }) if attempt == 0 => {
                    let t = self.load_tokens().await?;
                    self.refresh(client_id, &t).await?;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }

    async fn get(&self, client_id: &str, path: &str, query: &[(&str, String)]) -> AppResult<Value> {
        Ok(self.request(client_id, reqwest::Method::GET, path, query, None).await?.unwrap_or(Value::Null))
    }

    pub async fn me(&self, client_id: &str) -> AppResult<(String, Option<String>)> {
        let v = self.get(client_id, "/me", &[]).await?;
        let id = v.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        let name = v.get("display_name").and_then(Value::as_str).map(String::from);
        Ok((id, name))
    }

    /// Map a Spotify track object to the unified model. Returns `None` for
    /// local files and non-track items (episodes), which cannot be played via the API.
    pub fn map_track(v: &Value) -> Option<TrackUpsert> {
        if v.get("type").and_then(Value::as_str).is_some_and(|t| t != "track") {
            return None;
        }
        if v.get("is_local").and_then(Value::as_bool).unwrap_or(false) {
            return None;
        }
        let id = v.get("id").and_then(Value::as_str)?;
        let artists = v
            .get("artists")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|x| x.get("name").and_then(Value::as_str)).collect::<Vec<_>>().join(", "))
            .unwrap_or_default();
        let primary_artist = v
            .pointer("/artists/0/name")
            .and_then(Value::as_str)
            .map(String::from)
            .filter(|s| !s.trim().is_empty());
        let album = v.get("album");
        let album_artist = album
            .and_then(|a| a.pointer("/artists/0/name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let year = album
            .and_then(|a| a.get("release_date"))
            .and_then(Value::as_str)
            .and_then(|d| d.get(..4))
            .and_then(|y| y.parse().ok());
        let playable = v.get("is_playable").and_then(Value::as_bool).unwrap_or(true);
        let restriction = v.pointer("/restrictions/reason").and_then(Value::as_str);
        Some(TrackUpsert {
            provider_id: id.to_string(),
            provider_uri: v.get("uri").and_then(Value::as_str).map(String::from),
            title: v.get("name").and_then(Value::as_str).unwrap_or("Unknown").to_string(),
            artist: artists,
            primary_artist,
            album: album.and_then(|a| a.get("name")).and_then(Value::as_str).unwrap_or_default().to_string(),
            album_artist,
            track_number: v.get("track_number").and_then(Value::as_u64).map(|n| n as u32),
            disc_number: v.get("disc_number").and_then(Value::as_u64).map(|n| n as u32),
            year,
            genre: String::new(),
            duration_ms: v.get("duration_ms").and_then(Value::as_u64),
            artwork_url: album.and_then(|a| a.pointer("/images/0/url")).and_then(Value::as_str).map(String::from),
            availability: Some(if playable { Availability::Available } else { Availability::Unavailable }),
            unavailable_reason: if playable {
                None
            } else {
                Some(match restriction {
                    Some(r) => format!("Not playable on Spotify ({r})"),
                    None => "Not playable on Spotify in your market".into(),
                })
            },
            permalink_url: v.pointer("/external_urls/spotify").and_then(Value::as_str).map(String::from),
            ..Default::default()
        })
    }

    /// Follow `next` links, collecting mapped tracks from `items[].{item|track}`.
    async fn collect_paged_tracks(&self, client_id: &str, first: &str, query: &[(&str, String)], cap: usize) -> AppResult<Vec<TrackUpsert>> {
        let mut out = Vec::new();
        let mut v = self.get(client_id, first, query).await?;
        loop {
            if let Some(items) = v.get("items").and_then(Value::as_array) {
                for it in items {
                    let t = it.get("item").or_else(|| it.get("track")).unwrap_or(it);
                    if let Some(m) = Self::map_track(t) {
                        out.push(m);
                    }
                }
            }
            if out.len() >= cap {
                break;
            }
            match v.get("next").and_then(Value::as_str) {
                Some(next) => v = self.get(client_id, next, &[]).await?,
                None => break,
            }
        }
        Ok(out)
    }

    pub async fn saved_tracks(&self, client_id: &str, cap: usize) -> AppResult<Vec<TrackUpsert>> {
        self.collect_paged_tracks(client_id, "/me/tracks", &[("limit", "50".into()), ("market", "from_token".into())], cap).await
    }

    pub async fn my_playlists(&self, client_id: &str) -> AppResult<Vec<SpotifyPlaylistSummary>> {
        let mut out = Vec::new();
        let mut v = self.get(client_id, "/me/playlists", &[("limit", "50".into())]).await?;
        loop {
            for p in v.get("items").and_then(Value::as_array).into_iter().flatten() {
                if let (Some(id), Some(name)) = (p.get("id").and_then(Value::as_str), p.get("name").and_then(Value::as_str)) {
                    out.push(SpotifyPlaylistSummary {
                        id: id.into(),
                        name: name.into(),
                        owner_id: p.pointer("/owner/id").and_then(Value::as_str).map(String::from),
                        total: p
                            .pointer("/items/total")
                            .or_else(|| p.pointer("/tracks/total"))
                            .and_then(Value::as_u64)
                            .map(|n| n as u32),
                        collaborative: p.get("collaborative").and_then(Value::as_bool).unwrap_or(false),
                    });
                }
            }
            match v.get("next").and_then(Value::as_str) {
                Some(next) => v = self.get(client_id, next, &[]).await?,
                None => break,
            }
        }
        Ok(out)
    }

    /// Only playlists the user owns or collaborates on return items in Development Mode.
    pub async fn playlist_tracks(&self, client_id: &str, playlist_id: &str) -> AppResult<Vec<TrackUpsert>> {
        let path = format!("/playlists/{}/items", urlencoding::encode(playlist_id));
        self.collect_paged_tracks(client_id, &path, &[("limit", "50".into()), ("market", "from_token".into())], 10_000).await
    }

    pub async fn search(&self, client_id: &str, q: &str, offset: u32) -> AppResult<Vec<TrackUpsert>> {
        let v = self
            .get(
                client_id,
                "/search",
                &[
                    ("q", q.into()),
                    ("type", "track".into()),
                    ("limit", SEARCH_LIMIT.to_string()),
                    ("offset", offset.to_string()),
                    ("market", "from_token".into()),
                ],
            )
            .await?;
        Ok(v.pointer("/tracks/items")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Self::map_track).collect())
            .unwrap_or_default())
    }

    /// Scopes from `required` that the stored sign-in lacks.
    pub fn missing_granted_scopes(&self, required: &[&'static str]) -> Vec<&'static str> {
        match self.secrets.get(SPOTIFY_TOKENS) {
            Ok(Some(raw)) => match serde_json::from_str::<SpotifyTokens>(&raw) {
                Ok(t) => missing_scopes(&t.scope, required),
                Err(_) => required.to_vec(),
            },
            _ => required.to_vec(),
        }
    }

    // ------------------------------------------------- playlist sync

    fn playlist_path(id: &str, rest: &str) -> String {
        format!("/playlists/{}{rest}", urlencoding::encode(id))
    }

    fn snapshot_of(v: Option<Value>) -> Option<String> {
        v.and_then(|v| v.get("snapshot_id").and_then(Value::as_str).map(String::from))
    }

    /// Cheap metadata read: name, version, owner and cover, no items.
    pub async fn playlist_meta(&self, client_id: &str, playlist_id: &str) -> AppResult<PlaylistMeta> {
        let v = self
            .get(
                client_id,
                &Self::playlist_path(playlist_id, ""),
                &[("fields", "id,name,snapshot_id,collaborative,owner(id),images".into())],
            )
            .await?;
        let name = v.get("name").and_then(Value::as_str).ok_or_else(|| AppError::Http {
            provider: PROVIDER.into(),
            status: 502,
            message: "playlist response did not include a name".into(),
        })?;
        Ok(PlaylistMeta {
            id: v.get("id").and_then(Value::as_str).unwrap_or(playlist_id).to_string(),
            name: name.to_string(),
            snapshot_id: v.get("snapshot_id").and_then(Value::as_str).map(String::from),
            owner_id: v.pointer("/owner/id").and_then(Value::as_str).map(String::from),
            collaborative: v.get("collaborative").and_then(Value::as_bool).unwrap_or(false),
            images: v
                .get("images")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|i| i.get("url").and_then(Value::as_str).map(String::from)).collect())
                .unwrap_or_default(),
        })
    }

    /// Every entry of a playlist in order, including entries Branflakes
    /// can't hold. Fails (rather than returning a partial list) if the pages
    /// don't add up to Spotify's `total`, so a short response is never
    /// mistaken for removals.
    pub async fn playlist_items_all(&self, client_id: &str, playlist_id: &str) -> AppResult<Vec<RemoteItem>> {
        let path = Self::playlist_path(playlist_id, "/items");
        let mut out: Vec<RemoteItem> = Vec::new();
        let mut v = self.get(client_id, &path, &[("limit", "50".into()), ("market", "from_token".into())]).await?;
        let mut total: Option<u64> = None;
        loop {
            let page_total = v.get("total").and_then(Value::as_u64);
            match (total, page_total) {
                (None, t) => total = t,
                (Some(a), Some(b)) if a != b => {
                    return Err(AppError::Other("the Spotify playlist changed while it was being read; will retry".into()));
                }
                _ => {}
            }
            let Some(items) = v.get("items").and_then(Value::as_array) else {
                return Err(AppError::Http { provider: PROVIDER.into(), status: 502, message: "playlist items response had no items".into() });
            };
            for it in items {
                let obj = it.get("item").or_else(|| it.get("track")).filter(|o| !o.is_null());
                let offset = out.len();
                let uri = obj.and_then(|o| o.get("uri")).and_then(Value::as_str).map(String::from);
                let item = match (obj, uri) {
                    (Some(o), Some(uri)) => {
                        let track = if uri.starts_with("spotify:track:") { Self::map_track(o) } else { None };
                        // A track URI that we can't map is kept as foreign.
                        let uri = if uri.starts_with("spotify:track:") && track.is_none() && !uri.starts_with("spotify:local:") {
                            format!("spotify:unavailable:{offset}")
                        } else {
                            uri
                        };
                        let name = o.get("name").and_then(Value::as_str).map(String::from);
                        RemoteItem { uri, track, name }
                    }
                    _ => RemoteItem { uri: format!("spotify:unavailable:{offset}"), track: None, name: None },
                };
                out.push(item);
            }
            match v.get("next").and_then(Value::as_str) {
                Some(next) => v = self.get(client_id, next, &[]).await?,
                None => break,
            }
        }
        if let Some(t) = total {
            if out.len() as u64 != t {
                return Err(AppError::Other(format!(
                    "Spotify returned {} of {t} playlist items; nothing was changed and the sync will retry",
                    out.len()
                )));
            }
        }
        Ok(out)
    }

    pub async fn playlist_add_items(&self, client_id: &str, playlist_id: &str, uris: &[String], position: Option<usize>) -> AppResult<Option<String>> {
        let mut body = serde_json::json!({ "uris": uris });
        if let Some(p) = position {
            body["position"] = Value::from(p);
        }
        let v = self.request(client_id, reqwest::Method::POST, &Self::playlist_path(playlist_id, "/items"), &[], Some(&body)).await?;
        Ok(Self::snapshot_of(v))
    }

    /// Removes every occurrence of each URI.
    pub async fn playlist_remove_items(&self, client_id: &str, playlist_id: &str, uris: &[String], snapshot_id: Option<&str>) -> AppResult<Option<String>> {
        let items: Vec<Value> = uris.iter().map(|u| serde_json::json!({ "uri": u })).collect();
        let mut body = serde_json::json!({ "items": items });
        if let Some(s) = snapshot_id {
            body["snapshot_id"] = Value::from(s);
        }
        let v = self.request(client_id, reqwest::Method::DELETE, &Self::playlist_path(playlist_id, "/items"), &[], Some(&body)).await?;
        Ok(Self::snapshot_of(v))
    }

    pub async fn playlist_move_item(
        &self,
        client_id: &str,
        playlist_id: &str,
        range_start: usize,
        insert_before: usize,
        snapshot_id: Option<&str>,
    ) -> AppResult<Option<String>> {
        let mut body = serde_json::json!({ "range_start": range_start, "insert_before": insert_before, "range_length": 1 });
        if let Some(s) = snapshot_id {
            body["snapshot_id"] = Value::from(s);
        }
        let v = self.request(client_id, reqwest::Method::PUT, &Self::playlist_path(playlist_id, "/items"), &[], Some(&body)).await?;
        Ok(Self::snapshot_of(v))
    }

    /// Replace all items (≤ 100 URIs).
    pub async fn playlist_replace_items(&self, client_id: &str, playlist_id: &str, uris: &[String]) -> AppResult<Option<String>> {
        let body = serde_json::json!({ "uris": uris });
        let v = self.request(client_id, reqwest::Method::PUT, &Self::playlist_path(playlist_id, "/items"), &[], Some(&body)).await?;
        Ok(Self::snapshot_of(v))
    }

    /// Change Playlist Details (owner only).
    pub async fn playlist_rename(&self, client_id: &str, playlist_id: &str, name: &str) -> AppResult<()> {
        let body = serde_json::json!({ "name": name });
        self.request(client_id, reqwest::Method::PUT, &Self::playlist_path(playlist_id, ""), &[], Some(&body)).await.map(|_| ())
    }

    /// Upload a custom cover: base64 JPEG, at most 256 KB.
    pub async fn playlist_upload_cover(&self, client_id: &str, playlist_id: &str, jpeg_base64: &str) -> AppResult<()> {
        if jpeg_base64.len() > COVER_MAX_BASE64 {
            return Err(AppError::InvalidInput("cover image is larger than Spotify's 256 KB limit".into()));
        }
        self.request_with(client_id, reqwest::Method::PUT, &Self::playlist_path(playlist_id, "/images"), &[], Body::Raw("image/jpeg", jpeg_base64))
            .await
            .map(|_| ())
    }

    /// Download a cover image (public CDN URL; no token is sent).
    pub async fn fetch_image(&self, url: &str) -> AppResult<Vec<u8>> {
        if !url.starts_with("https://") && !url.starts_with(&self.api_base) {
            return Err(AppError::InvalidInput("refusing to fetch a non-https image".into()));
        }
        let resp = http::send(PROVIDER, self.policy, || self.http.get(url)).await?;
        let bytes = resp.bytes().await.map_err(|e| AppError::Network { provider: PROVIDER.into(), message: http::sanitize_reqwest_error(&e) })?;
        if bytes.len() > 10 * 1024 * 1024 {
            return Err(AppError::InvalidInput("cover image is unexpectedly large".into()));
        }
        Ok(bytes.to_vec())
    }

    // ------------------------------------------------ playlist lifecycle

    /// `POST /me/playlists`. Created **private** (Spotify's default is public).
    /// Returns (playlist id, snapshot id).
    pub async fn playlist_create(&self, client_id: &str, name: &str) -> AppResult<(String, Option<String>)> {
        let body = serde_json::json!({ "name": name, "public": false });
        let v = self.request(client_id, reqwest::Method::POST, "/me/playlists", &[], Some(&body)).await?.unwrap_or(Value::Null);
        let id = v.get("id").and_then(Value::as_str).ok_or_else(|| AppError::Http {
            provider: PROVIDER.into(),
            status: 502,
            message: "Spotify did not return the new playlist's id".into(),
        })?;
        Ok((id.to_string(), v.get("snapshot_id").and_then(Value::as_str).map(String::from)))
    }

    /// Remove a playlist from the user's library (`DELETE /me/library`).
    /// Spotify has no endpoint that permanently deletes a playlist; for the
    /// owner this is exactly what "Delete" does in Spotify's own apps.
    pub async fn library_remove_playlist(&self, client_id: &str, playlist_id: &str) -> AppResult<()> {
        let q = [("uris", format!("spotify:playlist:{playlist_id}"))];
        self.request(client_id, reqwest::Method::DELETE, "/me/library", &q, None).await.map(|_| ())
    }

    /// Is the playlist in the user's library (`GET /me/library/contains`)?
    /// An unexpected response shape is an error, never "false".
    pub async fn library_contains_playlist(&self, client_id: &str, playlist_id: &str) -> AppResult<bool> {
        let q = [("uris", format!("spotify:playlist:{playlist_id}"))];
        let v = self.get(client_id, "/me/library/contains", &q).await?;
        match v.as_array().map(|a| a.as_slice()) {
            Some([Value::Bool(b)]) => Ok(*b),
            _ => Err(AppError::Http { provider: PROVIDER.into(), status: 502, message: "unexpected response from Spotify's library check".into() }),
        }
    }

    // ---------------------------------------------------------- player

    pub async fn devices(&self, client_id: &str) -> AppResult<Vec<SpotifyDevice>> {
        let v = self.get(client_id, "/me/player/devices", &[]).await?;
        Ok(serde_json::from_value(v.get("devices").cloned().unwrap_or(Value::Array(vec![]))).unwrap_or_default())
    }

    pub async fn playback_state(&self, client_id: &str) -> AppResult<Option<SpotifyPlaybackState>> {
        // market=from_token makes Spotify report `item.linked_from` for relinked tracks.
        let q = [("market", "from_token".to_string())];
        let v = self.request(client_id, reqwest::Method::GET, "/me/player", &q, None).await?;
        let Some(v) = v else { return Ok(None) };
        let disallows = v
            .pointer("/actions/disallows")
            .and_then(Value::as_object)
            .map(|o| o.iter().filter(|(_, b)| b.as_bool() == Some(true)).map(|(k, _)| k.clone()).collect())
            .unwrap_or_default();
        Ok(Some(SpotifyPlaybackState {
            device: v.get("device").and_then(|d| serde_json::from_value(d.clone()).ok()),
            is_playing: v.get("is_playing").and_then(Value::as_bool).unwrap_or(false),
            progress_ms: v.get("progress_ms").and_then(Value::as_u64),
            item_uri: v.pointer("/item/uri").and_then(Value::as_str).map(String::from),
            linked_from_uri: v.pointer("/item/linked_from/uri").and_then(Value::as_str).map(String::from),
            item_duration_ms: v.pointer("/item/duration_ms").and_then(Value::as_u64),
            disallows,
        }))
    }

    fn device_query(device_id: Option<&str>) -> Vec<(&'static str, String)> {
        device_id.map(|d| vec![("device_id", d.to_string())]).unwrap_or_default()
    }

    pub async fn play_uri(&self, client_id: &str, device_id: Option<&str>, uri: &str, position_ms: u64) -> AppResult<()> {
        let body = serde_json::json!({ "uris": [uri], "position_ms": position_ms });
        self.request(client_id, reqwest::Method::PUT, "/me/player/play", &Self::device_query(device_id), Some(&body)).await.map(|_| ())
    }

    pub async fn resume(&self, client_id: &str, device_id: Option<&str>) -> AppResult<()> {
        self.request(client_id, reqwest::Method::PUT, "/me/player/play", &Self::device_query(device_id), None).await.map(|_| ())
    }

    pub async fn pause(&self, client_id: &str, device_id: Option<&str>) -> AppResult<()> {
        self.request(client_id, reqwest::Method::PUT, "/me/player/pause", &Self::device_query(device_id), None).await.map(|_| ())
    }

    pub async fn seek(&self, client_id: &str, device_id: Option<&str>, position_ms: u64) -> AppResult<()> {
        let mut q = Self::device_query(device_id);
        q.push(("position_ms", position_ms.to_string()));
        self.request(client_id, reqwest::Method::PUT, "/me/player/seek", &q, None).await.map(|_| ())
    }

    pub async fn set_volume(&self, client_id: &str, device_id: Option<&str>, percent: u32) -> AppResult<()> {
        let mut q = Self::device_query(device_id);
        q.push(("volume_percent", percent.min(100).to_string()));
        self.request(client_id, reqwest::Method::PUT, "/me/player/volume", &q, None).await.map(|_| ())
    }

    pub async fn add_to_queue(&self, client_id: &str, device_id: Option<&str>, uri: &str) -> AppResult<()> {
        let mut q = Self::device_query(device_id);
        q.push(("uri", uri.to_string()));
        self.request(client_id, reqwest::Method::POST, "/me/player/queue", &q, None).await.map(|_| ())
    }

    pub async fn transfer(&self, client_id: &str, device_id: &str, play: bool) -> AppResult<()> {
        let body = serde_json::json!({ "device_ids": [device_id], "play": play });
        self.request(client_id, reqwest::Method::PUT, "/me/player", &[], Some(&body)).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryStore;
    use mockito::Matcher;

    fn adapter(server: &mockito::Server) -> (SpotifyAdapter, Arc<MemoryStore>) {
        let store = Arc::new(MemoryStore::default());
        let a = SpotifyAdapter::with_base(
            http::client(),
            store.clone(),
            &format!("{}/v1", server.url()),
            &server.url(),
            RetryPolicy::immediate(),
        );
        (a, store)
    }

    fn seed_tokens(store: &MemoryStore, expires_in: i64) {
        let t = SpotifyTokens {
            access_token: Secret::new("AT"),
            refresh_token: Some(Secret::new("RT")),
            expires_at: chrono::Utc::now().timestamp() + expires_in,
            scope: String::new(),
        };
        store.set(SPOTIFY_TOKENS, &serde_json::to_string(&t).unwrap()).unwrap();
    }

    #[test]
    fn authorize_url_uses_pkce_and_scopes() {
        let server_url = "https://accounts.example";
        let a = SpotifyAdapter::with_base(http::client(), Arc::new(MemoryStore::default()), "x", server_url, RetryPolicy::immediate());
        let u = url::Url::parse(&a.authorize_url("cid", "http://127.0.0.1:43821/callback/spotify", "CH", "ST")).unwrap();
        let q: std::collections::HashMap<_, _> = u.query_pairs().into_owned().collect();
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["code_challenge"], "CH");
        assert_eq!(q["response_type"], "code");
        // Every scope, including the playlist-sync ones, is requested at sign-in.
        let requested: Vec<&str> = q["scope"].split(' ').collect();
        for s in SCOPES {
            assert!(requested.contains(s), "{s} not requested");
        }
        for s in ["playlist-modify-public", "playlist-modify-private", "ugc-image-upload"] {
            assert!(requested.contains(&s), "{s} not requested");
        }
        assert!(!q.contains_key("client_secret"));
    }

    #[tokio::test]
    async fn exchange_code_stores_tokens_in_secret_store() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/api/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("grant_type".into(), "authorization_code".into()),
                Matcher::UrlEncoded("code_verifier".into(), "VER".into()),
            ]))
            .with_body(r#"{"access_token":"AT1","token_type":"Bearer","expires_in":3600,"refresh_token":"RT1","scope":"x"}"#)
            .create_async()
            .await;
        let (a, store) = adapter(&server);
        a.exchange_code("cid", "CODE", "VER", "http://127.0.0.1/cb").await.unwrap();
        let raw = store.get(SPOTIFY_TOKENS).unwrap().unwrap();
        assert!(raw.contains("AT1") && raw.contains("RT1"));
        assert_eq!(a.access_token("cid").await.unwrap().expose(), "AT1");
    }

    #[tokio::test]
    async fn expired_token_is_refreshed_and_refresh_token_kept() {
        let mut server = mockito::Server::new_async().await;
        let refresh = server
            .mock("POST", "/api/token")
            .match_body(Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()))
            .with_body(r#"{"access_token":"AT2","expires_in":3600}"#)
            .expect(1)
            .create_async()
            .await;
        let (a, store) = adapter(&server);
        seed_tokens(&store, -10);
        assert_eq!(a.access_token("cid").await.unwrap().expose(), "AT2");
        refresh.assert_async().await;
        let saved: SpotifyTokens = serde_json::from_str(&store.get(SPOTIFY_TOKENS).unwrap().unwrap()).unwrap();
        assert_eq!(saved.refresh_token.unwrap().expose(), "RT", "old refresh token must be kept when none returned");
    }

    #[tokio::test]
    async fn retries_once_after_401() {
        let mut server = mockito::Server::new_async().await;
        let (a, store) = adapter(&server);
        seed_tokens(&store, 3600);
        server.mock("GET", "/v1/me").match_header("authorization", "Bearer AT").with_status(401).with_body(r#"{"error":{"status":401,"message":"expired"}}"#).create_async().await;
        server.mock("POST", "/api/token").with_body(r#"{"access_token":"NEW","expires_in":3600}"#).create_async().await;
        server.mock("GET", "/v1/me").match_header("authorization", "Bearer NEW").with_body(r#"{"id":"u1","display_name":"Ann"}"#).create_async().await;
        let (id, name) = a.me("cid").await.unwrap();
        assert_eq!((id.as_str(), name.as_deref()), ("u1", Some("Ann")));
    }

    #[test]
    fn maps_tracks_and_skips_local_and_episodes() {
        let v: Value = serde_json::from_str(
            r#"{"type":"track","id":"t1","uri":"spotify:track:t1","name":"Song","duration_ms":123000,
                "track_number":3,"disc_number":1,"is_playable":false,"restrictions":{"reason":"market"},
                "artists":[{"name":"A"},{"name":"B"}],
                "album":{"name":"Alb","release_date":"2019-05-01","artists":[{"name":"A"}],"images":[{"url":"https://i.scdn.co/x"}]},
                "external_urls":{"spotify":"https://open.spotify.com/track/t1"}}"#,
        )
        .unwrap();
        let t = SpotifyAdapter::map_track(&v).unwrap();
        assert_eq!(t.artist, "A, B");
        assert_eq!(t.primary_artist.as_deref(), Some("A"));
        assert_eq!(t.year, Some(2019));
        assert_eq!(t.availability, Some(Availability::Unavailable));
        assert!(t.unavailable_reason.unwrap().contains("market"));
        assert!(SpotifyAdapter::map_track(&serde_json::json!({"type":"track","id":"x","is_local":true})).is_none());
        assert!(SpotifyAdapter::map_track(&serde_json::json!({"type":"episode","id":"x"})).is_none());
    }

    #[tokio::test]
    async fn search_respects_dev_mode_limit_and_playlist_items_shape() {
        let mut server = mockito::Server::new_async().await;
        let (a, store) = adapter(&server);
        seed_tokens(&store, 3600);
        let m = server
            .mock("GET", "/v1/search")
            .match_query(Matcher::AllOf(vec![Matcher::UrlEncoded("limit".into(), "10".into()), Matcher::UrlEncoded("type".into(), "track".into())]))
            .with_body(r#"{"tracks":{"items":[{"type":"track","id":"s1","name":"S","artists":[],"album":{}}]}}"#)
            .create_async()
            .await;
        assert_eq!(a.search("cid", "q", 0).await.unwrap().len(), 1);
        m.assert_async().await;

        server
            .mock("GET", "/v1/playlists/p1/items")
            .match_query(Matcher::Any)
            .with_body(r#"{"items":[{"item":{"type":"track","id":"a","name":"A","artists":[],"album":{}}},{"track":{"type":"track","id":"b","name":"B","artists":[],"album":{}}}],"next":null}"#)
            .create_async()
            .await;
        let items = a.playlist_tracks("cid", "p1").await.unwrap();
        assert_eq!(items.iter().map(|t| t.provider_id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    }

    #[tokio::test]
    async fn player_premium_required_is_explicit() {
        let mut server = mockito::Server::new_async().await;
        let (a, store) = adapter(&server);
        seed_tokens(&store, 3600);
        server
            .mock("PUT", "/v1/me/player/play")
            .match_query(Matcher::UrlEncoded("device_id".into(), "D1".into()))
            .match_body(Matcher::PartialJson(serde_json::json!({"uris":["spotify:track:t1"],"position_ms":5000})))
            .with_status(403)
            .with_body(r#"{"error":{"status":403,"message":"Player command failed: Premium required","reason":"PREMIUM_REQUIRED"}}"#)
            .create_async()
            .await;
        let err = a.play_uri("cid", Some("D1"), "spotify:track:t1", 5000).await.unwrap_err();
        assert!(matches!(err, AppError::PremiumRequired { .. }));
    }

    #[tokio::test]
    async fn playback_state_handles_204_and_parses_state() {
        let mut server = mockito::Server::new_async().await;
        let (a, store) = adapter(&server);
        seed_tokens(&store, 3600);
        let m = server.mock("GET", "/v1/me/player").match_query(mockito::Matcher::Any).with_status(204).create_async().await;
        assert!(a.playback_state("cid").await.unwrap().is_none());
        m.remove_async().await;
        server
            .mock("GET", "/v1/me/player")
            .match_query(mockito::Matcher::UrlEncoded("market".into(), "from_token".into()))
            .with_body(r#"{"device":{"id":"D","name":"Phone","type":"Smartphone","is_active":true,"is_restricted":false,"supports_volume":false,"volume_percent":null},
                          "is_playing":true,"progress_ms":4200,"item":{"uri":"spotify:track:t","duration_ms":9000,"linked_from":{"uri":"spotify:track:orig"}},
                          "actions":{"disallows":{"seeking":true,"pausing":false}}}"#)
            .create_async()
            .await;
        let s = a.playback_state("cid").await.unwrap().unwrap();
        assert!(s.is_playing);
        assert_eq!(s.progress_ms, Some(4200));
        assert_eq!(s.linked_from_uri.as_deref(), Some("spotify:track:orig"));
        assert!(!s.device.unwrap().supports_volume);
        assert_eq!(s.disallows, vec!["seeking".to_string()]);
    }

    #[test]
    fn scope_checks() {
        assert!(SDK_SCOPES.iter().all(|s| SCOPES.contains(s)), "sign-in must request every SDK scope");
        assert!(missing_scopes("streaming user-read-email user-read-private", SDK_SCOPES).is_empty());
        assert_eq!(missing_scopes("streaming user-read-playback-state", SDK_SCOPES), vec!["user-read-email", "user-read-private"]);
        assert_eq!(missing_scopes("", SDK_SCOPES).len(), 3);
    }

    #[tokio::test]
    async fn sdk_token_refused_when_scopes_missing_and_status_flags_reconnect() {
        let server = mockito::Server::new_async().await;
        let (a, store) = adapter(&server);
        let t = SpotifyTokens {
            access_token: Secret::new("AT"),
            refresh_token: Some(Secret::new("RT")),
            expires_at: chrono::Utc::now().timestamp() + 3600,
            scope: "streaming user-read-playback-state".into(),
        };
        store.set(SPOTIFY_TOKENS, &serde_json::to_string(&t).unwrap()).unwrap();
        let err = a.sdk_access_token("cid").await.unwrap_err();
        assert!(matches!(err, AppError::Auth { ref message, .. } if message.contains("Reconnect")), "{err:?}");
        assert!(a.status(true, None, false).needs_reconnect);

        let full = SpotifyTokens { scope: SCOPES.join(" "), ..t };
        a.disconnect().await.unwrap();
        store.set(SPOTIFY_TOKENS, &serde_json::to_string(&full).unwrap()).unwrap();
        assert_eq!(a.sdk_access_token("cid").await.unwrap().expose(), "AT");
        assert!(!a.status(true, None, false).needs_reconnect);
    }

    #[test]
    fn pre_sync_sign_in_is_flagged_for_reconnect_without_breaking_the_sdk() {
        // Exactly the scopes requested before playlist sync existed.
        let old = "user-read-playback-state user-modify-playback-state user-read-currently-playing user-library-read playlist-read-private playlist-read-collaborative streaming user-read-email user-read-private";
        let store = Arc::new(MemoryStore::default());
        let t = SpotifyTokens { access_token: Secret::new("AT"), refresh_token: Some(Secret::new("RT")), expires_at: 0, scope: old.into() };
        store.set(SPOTIFY_TOKENS, &serde_json::to_string(&t).unwrap()).unwrap();
        let a = SpotifyAdapter::with_base(http::client(), store, "x", "y", RetryPolicy::immediate());
        let st = a.status(true, None, false);
        assert!(st.needs_reconnect);
        assert_eq!(st.missing_scopes, vec!["playlist-modify-public", "playlist-modify-private", "ugc-image-upload"]);
        assert!(st.limitations[0].contains("Reconnect Spotify"));
        assert!(st.missing_scopes.iter().all(|s| !SDK_SCOPES.contains(&s.as_str())), "in-app player still works");
    }

    #[tokio::test]
    async fn refresh_keeps_scope_when_omitted() {
        let mut server = mockito::Server::new_async().await;
        server.mock("POST", "/api/token").with_body(r#"{"access_token":"AT2","expires_in":3600}"#).create_async().await;
        let (a, store) = adapter(&server);
        let t = SpotifyTokens {
            access_token: Secret::new("AT"),
            refresh_token: Some(Secret::new("RT")),
            expires_at: chrono::Utc::now().timestamp() - 10,
            scope: SCOPES.join(" "),
        };
        store.set(SPOTIFY_TOKENS, &serde_json::to_string(&t).unwrap()).unwrap();
        a.access_token("cid").await.unwrap();
        let saved: SpotifyTokens = serde_json::from_str(&store.get(SPOTIFY_TOKENS).unwrap().unwrap()).unwrap();
        assert_eq!(saved.scope, SCOPES.join(" "));
    }

    #[tokio::test]
    async fn playlist_items_keep_foreign_entries_and_reject_short_reads() {
        let mut server = mockito::Server::new_async().await;
        let (a, store) = adapter(&server);
        seed_tokens(&store, 3600);
        let m = server
            .mock("GET", "/v1/playlists/p1/items")
            .match_query(Matcher::Any)
            .with_body(
                r#"{"total":4,"next":null,"items":[
                    {"item":{"type":"track","id":"a","uri":"spotify:track:a","name":"A","artists":[],"album":{}}},
                    {"is_local":true,"item":{"type":"track","id":null,"uri":"spotify:local:x:y:z:1","name":"L","is_local":true}},
                    {"item":null},
                    {"item":{"type":"episode","id":"e","uri":"spotify:episode:e","name":"E"}}]}"#,
            )
            .create_async()
            .await;
        let items = a.playlist_items_all("cid", "p1").await.unwrap();
        assert_eq!(
            items.iter().map(|i| i.uri.as_str()).collect::<Vec<_>>(),
            ["spotify:track:a", "spotify:local:x:y:z:1", "spotify:unavailable:2", "spotify:episode:e"]
        );
        assert!(items[0].track.is_some() && items[1].track.is_none());
        m.remove_async().await;

        // total says 3 but only 1 item came back: an error, never a partial list.
        server
            .mock("GET", "/v1/playlists/p1/items")
            .match_query(Matcher::Any)
            .with_body(r#"{"total":3,"next":null,"items":[{"item":{"type":"track","id":"a","uri":"spotify:track:a","name":"A","artists":[],"album":{}}}]}"#)
            .create_async()
            .await;
        assert!(a.playlist_items_all("cid", "p1").await.is_err());
    }

    #[tokio::test]
    async fn playlist_writes_use_items_endpoints() {
        let mut server = mockito::Server::new_async().await;
        let (a, store) = adapter(&server);
        seed_tokens(&store, 3600);
        let add = server
            .mock("POST", "/v1/playlists/p1/items")
            .match_body(Matcher::Json(serde_json::json!({"uris":["spotify:track:x"],"position":1})))
            .with_status(201)
            .with_body(r#"{"snapshot_id":"s2"}"#)
            .create_async()
            .await;
        assert_eq!(a.playlist_add_items("cid", "p1", &["spotify:track:x".into()], Some(1)).await.unwrap().as_deref(), Some("s2"));
        add.assert_async().await;
        let del = server
            .mock("DELETE", "/v1/playlists/p1/items")
            .match_body(Matcher::Json(serde_json::json!({"items":[{"uri":"spotify:track:x"}],"snapshot_id":"s2"})))
            .with_body(r#"{"snapshot_id":"s3"}"#)
            .create_async()
            .await;
        assert_eq!(a.playlist_remove_items("cid", "p1", &["spotify:track:x".into()], Some("s2")).await.unwrap().as_deref(), Some("s3"));
        del.assert_async().await;
        let mv = server
            .mock("PUT", "/v1/playlists/p1/items")
            .match_body(Matcher::PartialJson(serde_json::json!({"range_start":3,"insert_before":0})))
            .with_body(r#"{"snapshot_id":"s4"}"#)
            .create_async()
            .await;
        assert_eq!(a.playlist_move_item("cid", "p1", 3, 0, None).await.unwrap().as_deref(), Some("s4"));
        mv.assert_async().await;
        let ren = server
            .mock("PUT", "/v1/playlists/p1")
            .match_body(Matcher::Json(serde_json::json!({"name":"New"})))
            .with_status(200)
            .create_async()
            .await;
        a.playlist_rename("cid", "p1", "New").await.unwrap();
        ren.assert_async().await;
        let img = server
            .mock("PUT", "/v1/playlists/p1/images")
            .match_header("content-type", "image/jpeg")
            .match_body("QUJD")
            .with_status(202)
            .create_async()
            .await;
        a.playlist_upload_cover("cid", "p1", "QUJD").await.unwrap();
        img.assert_async().await;
        let big = "A".repeat(COVER_MAX_BASE64 + 1);
        assert!(a.playlist_upload_cover("cid", "p1", &big).await.is_err());
    }

    #[test]
    fn sign_in_requests_playlist_write_scopes() {
        for s in PLAYLIST_WRITE_SCOPES.iter().chain([IMAGE_UPLOAD_SCOPE].iter()) {
            assert!(SCOPES.contains(s), "{s}");
        }
    }

    #[test]
    fn capabilities_off_without_premium() {
        assert_eq!(SpotifyAdapter::capabilities(true, true), PlaybackCapabilities::NONE);
        assert_eq!(SpotifyAdapter::capabilities(false, false), PlaybackCapabilities::NONE);
        assert!(SpotifyAdapter::capabilities(true, false).device_selection);
    }
}
