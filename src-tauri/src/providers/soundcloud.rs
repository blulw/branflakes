//! SoundCloudAdapter — official SoundCloud API only.
//!
//! * Registration requires a SoundCloud **Artist Pro** account (current
//!   developer docs). The user supplies their own client ID and secret.
//! * Auth: OAuth 2.1 Authorization Code + PKCE at secure.soundcloud.com. All
//!   clients are confidential, so the client secret is sent on token
//!   requests. Access tokens last ~1 h; refresh tokens are **single-use**, so
//!   every refresh response's new refresh token is persisted immediately.
//! * Requests use `Authorization: OAuth <token>`.
//! * Playback: `GET /tracks/{urn}/streams` returns transcoding URLs. We prefer
//!   AAC HLS (`hls_aac_160_url`); MP3/Opus fields are deprecated. The API URL
//!   is resolved server-side (with the token header) to the CDN playlist URL so
//!   the token never reaches the webview. Tracks with `access = blocked` are
//!   unplayable; `preview` tracks play a preview only.
//! * Attribution (required for custom players): uploader credit, SoundCloud
//!   logo/source credit, and a link to the track's SoundCloud page.

use crate::error::{AppError, AppResult};
use crate::http::{self, RetryPolicy};
use crate::model::{Availability, PlaybackCapabilities, ProviderStatus, TrackUpsert};
use crate::secrets::{Secret, SecretStore, SOUNDCLOUD_TOKENS};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

pub const API_BASE: &str = "https://api.soundcloud.com";
pub const AUTH_BASE: &str = "https://secure.soundcloud.com";
const PROVIDER: &str = "soundcloud";

#[derive(Clone, Serialize, Deserialize)]
pub struct SoundCloudTokens {
    pub access_token: Secret,
    pub refresh_token: Option<Secret>,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StreamInfo {
    /// Playable URL (CDN, no credentials embedded by us).
    pub url: String,
    /// "hls" or "progressive".
    pub protocol: String,
    pub preview: bool,
    pub format: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SoundCloudPlaylistSummary {
    pub urn: String,
    pub title: String,
    pub track_count: Option<u32>,
}

/// Validate a SoundCloud URN (`soundcloud:<kind>:<digits>`) before putting it
/// in a URL path, so stored IDs can never alter the request path.
pub fn safe_urn<'a>(urn: &'a str, kind: &str) -> AppResult<&'a str> {
    let mut parts = urn.split(':');
    let ok = parts.next() == Some("soundcloud")
        && parts.next() == Some(kind)
        && parts.next().is_some_and(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()))
        && parts.next().is_none();
    if ok { Ok(urn) } else { Err(AppError::InvalidInput(format!("invalid SoundCloud {kind} URN"))) }
}

pub struct SoundCloudAdapter {
    http: reqwest::Client,
    /// Client that does not follow redirects (used to resolve stream URLs).
    no_redirect: reqwest::Client,
    api_base: String,
    auth_base: String,
    policy: RetryPolicy,
    secrets: Arc<dyn SecretStore>,
    cache: tokio::sync::Mutex<Option<SoundCloudTokens>>,
}

pub struct SoundCloudCredentials {
    pub client_id: String,
    pub client_secret: Secret,
}

impl SoundCloudAdapter {
    pub fn new(http: reqwest::Client, secrets: Arc<dyn SecretStore>) -> Self {
        Self::with_base(http, secrets, API_BASE, AUTH_BASE, RetryPolicy::default())
    }

    pub fn with_base(http: reqwest::Client, secrets: Arc<dyn SecretStore>, api: &str, auth: &str, policy: RetryPolicy) -> Self {
        let no_redirect = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .expect("http client");
        SoundCloudAdapter {
            http,
            no_redirect,
            api_base: api.trim_end_matches('/').into(),
            auth_base: auth.trim_end_matches('/').into(),
            policy,
            secrets,
            cache: tokio::sync::Mutex::new(None),
        }
    }

    pub fn capabilities(connected: bool) -> PlaybackCapabilities {
        if !connected {
            return PlaybackCapabilities::NONE;
        }
        PlaybackCapabilities {
            play: true,
            pause: true,
            next: true,     // app queue
            previous: true, // app queue
            seek: true,
            volume: true,
            queue: true, // app queue only; SoundCloud has no queue API
            device_selection: false,
            playback_position: true,
            duration: true,
        }
    }

    /// `web_mode`: the unofficial web adapter is enabled (see `soundcloud_web.rs`).
    pub fn status(&self, configured: bool, account: Option<String>, web_mode: bool) -> ProviderStatus {
        let connected = self.secrets.has(SOUNDCLOUD_TOKENS);
        let mut status = ProviderStatus {
            provider: PROVIDER.into(),
            display_name: "SoundCloud".into(),
            configured,
            connected,
            needs_reconnect: false,
            missing_scopes: Vec::new(),
            account_name: account,
            capabilities: Self::capabilities(connected || web_mode),
            limitations: vec![
                "Registering a SoundCloud API app requires a SoundCloud Artist Pro subscription.".into(),
                "Only tracks whose access is 'playable' stream in full; 'preview' tracks play a short preview and 'blocked' tracks cannot be played.".into(),
                "Streams are SoundCloud's transcoded AAC HLS streams played in-app; nothing is downloaded or saved.".into(),
                "Stream requests are limited to 15,000 per 24 hours per client ID.".into(),
                "Playback with live credentials has not been verified by the developers of this app; it follows the documented /tracks/{urn}/streams flow.".into(),
                "SoundCloud plays are never scrobbled to Last.fm by this app.".into(),
            ],
        };
        if web_mode && !connected {
            status.account_name = Some("Web mode (no account)".into());
        }
        if web_mode {
            let mut l = super::soundcloud_web::SoundCloudWebAdapter::limitations();
            l.extend(status.limitations);
            status.limitations = l;
        }
        status
    }

    // ------------------------------------------------------------ auth

    pub fn authorize_url(&self, client_id: &str, redirect_uri: &str, code_challenge: &str, state: &str) -> String {
        let mut u = url::Url::parse(&format!("{}/authorize", self.auth_base)).expect("valid url");
        u.query_pairs_mut()
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("code_challenge", code_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", state);
        u.into()
    }

    async fn token_request(&self, form: Vec<(&str, String)>) -> AppResult<SoundCloudTokens> {
        let url = format!("{}/oauth/token", self.auth_base);
        let resp = http::send(PROVIDER, self.policy, || {
            self.http.post(&url).header(reqwest::header::ACCEPT, "application/json; charset=utf-8").form(&form)
        })
        .await
        .map_err(|e| match e {
            AppError::Http { status: 400, message, .. } => AppError::Auth { provider: PROVIDER.into(), message },
            other => other,
        })?;
        let v: Value = resp.json().await.map_err(|e| AppError::Network { provider: PROVIDER.into(), message: http::sanitize_reqwest_error(&e) })?;
        let access = v.get("access_token").and_then(Value::as_str).ok_or_else(|| AppError::Auth {
            provider: PROVIDER.into(),
            message: "token response missing access_token".into(),
        })?;
        Ok(SoundCloudTokens {
            access_token: Secret::new(access),
            refresh_token: v.get("refresh_token").and_then(Value::as_str).map(Secret::new),
            expires_at: chrono::Utc::now().timestamp() + v.get("expires_in").and_then(Value::as_i64).unwrap_or(3600),
        })
    }

    pub async fn exchange_code(&self, creds: &SoundCloudCredentials, code: &str, verifier: &str, redirect_uri: &str) -> AppResult<()> {
        let t = self
            .token_request(vec![
                ("grant_type", "authorization_code".into()),
                ("client_id", creds.client_id.clone()),
                ("client_secret", creds.client_secret.expose().into()),
                ("redirect_uri", redirect_uri.into()),
                ("code_verifier", verifier.into()),
                ("code", code.into()),
            ])
            .await?;
        self.store_tokens(t).await
    }

    async fn store_tokens(&self, t: SoundCloudTokens) -> AppResult<()> {
        self.secrets.set(SOUNDCLOUD_TOKENS, &serde_json::to_string(&t)?)?;
        *self.cache.lock().await = Some(t);
        Ok(())
    }

    pub async fn disconnect(&self) -> AppResult<()> {
        *self.cache.lock().await = None;
        self.secrets.delete(SOUNDCLOUD_TOKENS)
    }

    async fn load_tokens(&self) -> AppResult<SoundCloudTokens> {
        let mut c = self.cache.lock().await;
        if let Some(t) = c.as_ref() {
            return Ok(t.clone());
        }
        let raw = self.secrets.get(SOUNDCLOUD_TOKENS)?.ok_or(AppError::NotConnected { provider: PROVIDER.into() })?;
        let t: SoundCloudTokens = serde_json::from_str(&raw)?;
        *c = Some(t.clone());
        Ok(t)
    }

    async fn refresh(&self, creds: &SoundCloudCredentials) -> AppResult<SoundCloudTokens> {
        let current = self.load_tokens().await?;
        let rt = current.refresh_token.ok_or_else(|| AppError::Auth {
            provider: PROVIDER.into(),
            message: "no refresh token; please reconnect SoundCloud".into(),
        })?;
        let t = self
            .token_request(vec![
                ("grant_type", "refresh_token".into()),
                ("client_id", creds.client_id.clone()),
                ("client_secret", creds.client_secret.expose().into()),
                ("refresh_token", rt.expose().into()),
            ])
            .await?;
        // Refresh tokens are single-use: persist the replacement immediately.
        self.store_tokens(t.clone()).await?;
        Ok(t)
    }

    async fn access_token(&self, creds: &SoundCloudCredentials) -> AppResult<Secret> {
        let t = self.load_tokens().await?;
        if t.expires_at - 60 > chrono::Utc::now().timestamp() {
            return Ok(t.access_token);
        }
        Ok(self.refresh(creds).await?.access_token)
    }

    async fn get(&self, creds: &SoundCloudCredentials, path_or_url: &str, query: &[(&str, String)]) -> AppResult<Value> {
        let url = if path_or_url.starts_with("http") { path_or_url.to_string() } else { format!("{}{}", self.api_base, path_or_url) };
        for attempt in 0..2 {
            let token = self.access_token(creds).await?;
            let res = http::send(PROVIDER, self.policy, || {
                self.http
                    .get(&url)
                    .header(reqwest::header::AUTHORIZATION, format!("OAuth {}", token.expose()))
                    .header(reqwest::header::ACCEPT, "application/json; charset=utf-8")
                    .query(query)
            })
            .await;
            match res {
                Ok(r) => {
                    return r.json().await.map_err(|e| AppError::Network { provider: PROVIDER.into(), message: http::sanitize_reqwest_error(&e) })
                }
                Err(AppError::Auth { .. }) if attempt == 0 => {
                    self.refresh(creds).await?;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }

    pub async fn me(&self, creds: &SoundCloudCredentials) -> AppResult<String> {
        let v = self.get(creds, "/me", &[]).await?;
        Ok(v.get("username").and_then(Value::as_str).unwrap_or("SoundCloud user").to_string())
    }

    pub fn track_urn(v: &Value) -> Option<String> {
        v.get("urn")
            .and_then(Value::as_str)
            .map(String::from)
            .or_else(|| v.get("id").and_then(Value::as_u64).map(|id| format!("soundcloud:tracks:{id}")))
    }

    pub fn map_track(v: &Value) -> Option<TrackUpsert> {
        if v.get("kind").and_then(Value::as_str).is_some_and(|k| k != "track") {
            return None;
        }
        let urn = Self::track_urn(v)?;
        let uploader = v.pointer("/user/username").and_then(Value::as_str).unwrap_or_default().to_string();
        let artist = v
            .get("metadata_artist")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(String::from)
            .unwrap_or_else(|| uploader.clone());
        let access = v.get("access").and_then(Value::as_str).unwrap_or("playable");
        let streamable = v.get("streamable").and_then(Value::as_bool).unwrap_or(true);
        let (availability, reason) = match (access, streamable) {
            ("blocked", _) => (Availability::Unavailable, Some("Blocked on SoundCloud for your account or region".to_string())),
            (_, false) => (Availability::Unavailable, Some("The uploader has disabled streaming for this track".to_string())),
            ("preview", _) => (Availability::PreviewOnly, Some("Only a preview is available on SoundCloud".to_string())),
            _ => (Availability::Available, None),
        };
        let artwork = v
            .get("artwork_url")
            .and_then(Value::as_str)
            .or_else(|| v.pointer("/user/avatar_url").and_then(Value::as_str))
            .map(|u| u.replace("-large.", "-t500x500."));
        Some(TrackUpsert {
            provider_id: urn,
            provider_uri: v.get("permalink_url").and_then(Value::as_str).map(String::from),
            title: v.get("title").and_then(Value::as_str).unwrap_or("Untitled").to_string(),
            artist,
            album: v.pointer("/publisher_metadata/album_title").and_then(Value::as_str).unwrap_or_default().to_string(),
            album_artist: String::new(),
            year: v.get("release_year").and_then(Value::as_i64).map(|y| y as i32),
            genre: v.get("genre").and_then(Value::as_str).unwrap_or_default().to_string(),
            duration_ms: v.get("duration").and_then(Value::as_u64),
            artwork_url: artwork,
            availability: Some(availability),
            unavailable_reason: reason,
            permalink_url: v.get("permalink_url").and_then(Value::as_str).map(String::from),
            uploader: Some(uploader),
            uploader_url: v.pointer("/user/permalink_url").and_then(Value::as_str).map(String::from),
            ..Default::default()
        })
    }

    async fn collect(&self, creds: &SoundCloudCredentials, path: &str, query: &[(&str, String)], cap: usize) -> AppResult<Vec<Value>> {
        let mut out = Vec::new();
        let mut q: Vec<(&str, String)> = query.to_vec();
        q.push(("linked_partitioning", "true".into()));
        let mut v = self.get(creds, path, &q).await?;
        loop {
            match v.get("collection").and_then(Value::as_array) {
                Some(items) => out.extend(items.iter().cloned()),
                None => {
                    if let Some(arr) = v.as_array() {
                        out.extend(arr.iter().cloned());
                    }
                }
            }
            if out.len() >= cap {
                break;
            }
            match v.get("next_href").and_then(Value::as_str) {
                Some(next) => v = self.get(creds, next, &[]).await?,
                None => break,
            }
        }
        Ok(out)
    }

    pub async fn liked_tracks(&self, creds: &SoundCloudCredentials, cap: usize) -> AppResult<Vec<TrackUpsert>> {
        let items = self.collect(creds, "/me/likes/tracks", &[("limit", "200".into())], cap).await?;
        Ok(items.iter().filter_map(|i| Self::map_track(i.get("track").unwrap_or(i))).collect())
    }

    pub async fn my_playlists(&self, creds: &SoundCloudCredentials) -> AppResult<Vec<SoundCloudPlaylistSummary>> {
        let items = self.collect(creds, "/me/playlists", &[("limit", "50".into()), ("show_tracks", "false".into())], 2000).await?;
        Ok(items
            .iter()
            .filter_map(|p| {
                let urn = p
                    .get("urn")
                    .and_then(Value::as_str)
                    .map(String::from)
                    .or_else(|| p.get("id").and_then(Value::as_u64).map(|id| format!("soundcloud:playlists:{id}")))?;
                Some(SoundCloudPlaylistSummary {
                    urn,
                    title: p.get("title").and_then(Value::as_str).unwrap_or("Untitled").into(),
                    track_count: p.get("track_count").and_then(Value::as_u64).map(|n| n as u32),
                })
            })
            .collect())
    }

    pub async fn playlist_tracks(&self, creds: &SoundCloudCredentials, urn: &str) -> AppResult<Vec<TrackUpsert>> {
        let path = format!("/playlists/{}/tracks", safe_urn(urn, "playlists")?);
        let items = self.collect(creds, &path, &[("limit", "200".into())], 10_000).await?;
        Ok(items.iter().filter_map(Self::map_track).collect())
    }

    pub async fn search(&self, creds: &SoundCloudCredentials, q: &str, limit: u32) -> AppResult<Vec<TrackUpsert>> {
        let v = self
            .get(creds, "/tracks", &[("q", q.into()), ("limit", limit.min(50).to_string()), ("linked_partitioning", "true".into())])
            .await?;
        let arr = v.get("collection").and_then(Value::as_array).or_else(|| v.as_array());
        Ok(arr.map(|a| a.iter().filter_map(Self::map_track).collect()).unwrap_or_default())
    }

    /// Choose the best documented stream field. Order: AAC HLS (preferred,
    /// current), then deprecated MP3/Opus variants if still present, then preview.
    pub fn pick_stream(v: &Value) -> Option<(String, &'static str, bool, &'static str)> {
        const ORDER: &[(&str, &str, bool, &str)] = &[
            ("hls_aac_160_url", "hls", false, "aac_160"),
            ("hls_aac_96_url", "hls", false, "aac_96"),
            ("hls_mp3_128_url", "hls", false, "mp3_128"),
            ("http_mp3_128_url", "progressive", false, "mp3_128"),
            ("hls_opus_64_url", "hls", false, "opus_64"),
            ("preview_mp3_128_url", "progressive", true, "preview_mp3_128"),
        ];
        ORDER.iter().find_map(|(field, proto, preview, fmt)| {
            v.get(*field).and_then(Value::as_str).filter(|s| !s.is_empty()).map(|u| (u.to_string(), *proto, *preview, *fmt))
        })
    }

    /// Resolve a playable stream for a track URN.
    pub async fn resolve_stream(&self, creds: &SoundCloudCredentials, urn: &str) -> AppResult<StreamInfo> {
        let path = format!("/tracks/{}/streams", safe_urn(urn, "tracks")?);
        let v = self.get(creds, &path, &[]).await?;
        let (api_url, protocol, preview, format) = Self::pick_stream(&v).ok_or_else(|| AppError::Unsupported("SoundCloud returned no playable stream for this track".into()))?;
        // The stream URL on api.soundcloud.com requires the OAuth header and
        // redirects to a signed CDN URL; resolve it here so the token never
        // leaves the backend.
        let url = if api_url.starts_with(&self.api_base) {
            let token = self.access_token(creds).await?;
            let resp = self
                .no_redirect
                .get(&api_url)
                .header(reqwest::header::AUTHORIZATION, format!("OAuth {}", token.expose()))
                .send()
                .await
                .map_err(|e| AppError::Network { provider: PROVIDER.into(), message: http::sanitize_reqwest_error(&e) })?;
            let status = resp.status();
            if status.is_redirection() {
                resp.headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|l| l.to_str().ok())
                    .map(String::from)
                    .ok_or_else(|| AppError::Other("SoundCloud stream redirect had no location".into()))?
            } else if status.is_success() {
                // Some responses return JSON `{ "url": ... }` instead of a redirect.
                let body: Value = resp.json().await.unwrap_or(Value::Null);
                body.get("url")
                    .and_then(Value::as_str)
                    .map(String::from)
                    .ok_or_else(|| AppError::Unsupported("SoundCloud stream URL requires authenticated access that cannot be handed to the player".into()))?
            } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(AppError::RateLimited { provider: PROVIDER.into(), retry_after_secs: None, quota_exceeded: true });
            } else {
                return Err(AppError::Http { provider: PROVIDER.into(), status: status.as_u16(), message: "stream could not be resolved".into() });
            }
        } else {
            api_url
        };
        Ok(StreamInfo { url, protocol: protocol.into(), preview, format: format.into() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryStore;
    use mockito::Matcher;

    fn creds() -> SoundCloudCredentials {
        SoundCloudCredentials { client_id: "cid".into(), client_secret: Secret::new("csecret") }
    }

    fn adapter(server: &mockito::Server) -> (SoundCloudAdapter, Arc<MemoryStore>) {
        let store = Arc::new(MemoryStore::default());
        (SoundCloudAdapter::with_base(http::client(), store.clone(), &server.url(), &server.url(), RetryPolicy::immediate()), store)
    }

    fn seed(store: &MemoryStore, expires_in: i64) {
        let t = SoundCloudTokens { access_token: Secret::new("AT"), refresh_token: Some(Secret::new("RT")), expires_at: chrono::Utc::now().timestamp() + expires_in };
        store.set(SOUNDCLOUD_TOKENS, &serde_json::to_string(&t).unwrap()).unwrap();
    }

    #[test]
    fn maps_access_levels() {
        let base = serde_json::json!({"kind":"track","urn":"soundcloud:tracks:1","title":"T","duration":61000,
            "user":{"username":"up","permalink_url":"https://soundcloud.com/up"},"permalink_url":"https://soundcloud.com/up/t",
            "artwork_url":"https://i1.sndcdn.com/artworks-x-large.jpg","access":"playable","streamable":true});
        let t = SoundCloudAdapter::map_track(&base).unwrap();
        assert_eq!(t.availability, Some(Availability::Available));
        assert_eq!(t.artwork_url.as_deref(), Some("https://i1.sndcdn.com/artworks-x-t500x500.jpg"));
        assert_eq!(t.uploader.as_deref(), Some("up"));
        let mut p = base.clone();
        p["access"] = "preview".into();
        assert_eq!(SoundCloudAdapter::map_track(&p).unwrap().availability, Some(Availability::PreviewOnly));
        let mut b = base.clone();
        b["access"] = "blocked".into();
        assert_eq!(SoundCloudAdapter::map_track(&b).unwrap().availability, Some(Availability::Unavailable));
        // Legacy numeric id falls back to a URN.
        let legacy = serde_json::json!({"kind":"track","id":42,"title":"x","user":{"username":"u"}});
        assert_eq!(SoundCloudAdapter::map_track(&legacy).unwrap().provider_id, "soundcloud:tracks:42");
    }

    #[test]
    fn urn_validation() {
        assert!(safe_urn("soundcloud:tracks:123", "tracks").is_ok());
        assert!(safe_urn("soundcloud:tracks:12/../me", "tracks").is_err());
        assert!(safe_urn("soundcloud:playlists:1", "tracks").is_err());
        assert!(safe_urn("soundcloud:tracks:", "tracks").is_err());
    }

    #[test]
    fn prefers_aac_hls_then_falls_back() {
        let v = serde_json::json!({"http_mp3_128_url":"m","hls_aac_160_url":"a","preview_mp3_128_url":"p"});
        assert_eq!(SoundCloudAdapter::pick_stream(&v).unwrap().0, "a");
        let v = serde_json::json!({"preview_mp3_128_url":"p"});
        let s = SoundCloudAdapter::pick_stream(&v).unwrap();
        assert!(s.2, "preview flag");
        assert!(SoundCloudAdapter::pick_stream(&serde_json::json!({})).is_none());
    }

    #[tokio::test]
    async fn exchange_sends_secret_and_pkce_verifier() {
        let mut server = mockito::Server::new_async().await;
        let m = server
            .mock("POST", "/oauth/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("grant_type".into(), "authorization_code".into()),
                Matcher::UrlEncoded("client_secret".into(), "csecret".into()),
                Matcher::UrlEncoded("code_verifier".into(), "V".into()),
            ]))
            .with_body(r#"{"access_token":"A","refresh_token":"R","expires_in":3599,"scope":""}"#)
            .create_async()
            .await;
        let (a, store) = adapter(&server);
        a.exchange_code(&creds(), "C", "V", "http://127.0.0.1/cb").await.unwrap();
        m.assert_async().await;
        assert!(store.get(SOUNDCLOUD_TOKENS).unwrap().unwrap().contains("\"R\""));
    }

    #[tokio::test]
    async fn single_use_refresh_token_is_replaced() {
        let mut server = mockito::Server::new_async().await;
        let (a, store) = adapter(&server);
        seed(&store, -5);
        server
            .mock("POST", "/oauth/token")
            .match_body(Matcher::UrlEncoded("refresh_token".into(), "RT".into()))
            .with_body(r#"{"access_token":"A2","refresh_token":"RT2","expires_in":3600}"#)
            .create_async()
            .await;
        server.mock("GET", "/me").match_header("authorization", "OAuth A2").with_body(r#"{"username":"bob"}"#).create_async().await;
        assert_eq!(a.me(&creds()).await.unwrap(), "bob");
        let saved: SoundCloudTokens = serde_json::from_str(&store.get(SOUNDCLOUD_TOKENS).unwrap().unwrap()).unwrap();
        assert_eq!(saved.refresh_token.unwrap().expose(), "RT2");
    }

    #[tokio::test]
    async fn paginates_likes_with_next_href() {
        let mut server = mockito::Server::new_async().await;
        let (a, store) = adapter(&server);
        seed(&store, 3600);
        let next = format!("{}/me/likes/tracks?page=2", server.url());
        server
            .mock("GET", "/me/likes/tracks")
            .match_query(Matcher::UrlEncoded("linked_partitioning".into(), "true".into()))
            .with_body(serde_json::json!({"collection":[{"kind":"track","urn":"soundcloud:tracks:1","title":"a","user":{"username":"u"}}],"next_href":next}).to_string())
            .create_async()
            .await;
        server
            .mock("GET", "/me/likes/tracks")
            .match_query(Matcher::UrlEncoded("page".into(), "2".into()))
            .with_body(r#"{"collection":[{"kind":"track","urn":"soundcloud:tracks:2","title":"b","user":{"username":"u"}}]}"#)
            .create_async()
            .await;
        let t = a.liked_tracks(&creds(), 1000).await.unwrap();
        assert_eq!(t.len(), 2);
    }

    #[tokio::test]
    async fn resolves_stream_redirect_without_leaking_token() {
        let mut server = mockito::Server::new_async().await;
        let (a, store) = adapter(&server);
        seed(&store, 3600);
        let api_stream = format!("{}/tracks/soundcloud:tracks:1/streams/hls", server.url());
        server
            .mock("GET", "/tracks/soundcloud:tracks:1/streams")
            .with_body(serde_json::json!({ "hls_aac_160_url": api_stream }).to_string())
            .create_async()
            .await;
        server
            .mock("GET", "/tracks/soundcloud:tracks:1/streams/hls")
            .match_header("authorization", "OAuth AT")
            .with_status(302)
            .with_header("location", "https://cf-hls-media.sndcdn.com/playlist/abc.m3u8?Policy=x")
            .create_async()
            .await;
        let s = a.resolve_stream(&creds(), "soundcloud:tracks:1").await.unwrap();
        assert_eq!(s.protocol, "hls");
        assert!(s.url.starts_with("https://cf-hls-media.sndcdn.com/"));
        assert!(!s.url.contains("AT"), "token must not be in the playable URL");
    }
}
