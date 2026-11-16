//! LastFmAdapter — desktop authentication, Now Playing and scrobbling.
//!
//! Policy: local and SoundCloud tracks are always eligible; Spotify tracks
//! are eligible unless the user turns off "Scrobble Spotify tracks". Enforced in `scrobbler::source_allowed` regardless of what the
//! UI requests.
//!
//! Protocol reference: https://www.last.fm/api/scrobbling and
//! https://www.last.fm/api/desktopauth (desktop auth flow: auth.getToken →
//! user authorises in the browser → auth.getSession).

use crate::db::scrobbles::PendingScrobble;
use crate::error::AppError;
use crate::http::sanitize_reqwest_error;
use crate::model::{PlaybackCapabilities, ProviderStatus};
use crate::secrets::Secret;
use std::collections::BTreeMap;
use std::time::Duration;

pub const API_BASE: &str = "https://ws.audioscrobbler.com/2.0/";
pub const AUTH_BASE: &str = "https://www.last.fm/api/auth/";
const PROVIDER: &str = "lastfm";

#[derive(Clone)]
pub struct LastFmCredentials {
    pub api_key: String,
    pub shared_secret: Secret,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallError {
    /// Last.fm returned `{ "error": code, "message": ... }`.
    Api { code: i64, message: String },
    /// Transport or unparseable response.
    Transport(String),
}

impl CallError {
    /// Per Last.fm docs: 11 (service offline) and 16 (temporarily unavailable)
    /// are retryable; transport failures are treated the same way.
    pub fn is_retryable(&self) -> bool {
        matches!(self, CallError::Api { code: 11 | 16, .. } | CallError::Transport(_))
    }
    pub fn is_invalid_session(&self) -> bool {
        matches!(self, CallError::Api { code: 9, .. })
    }
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Api { code, message } => write!(f, "Last.fm error {code}: {message}"),
            CallError::Transport(m) => write!(f, "Last.fm unreachable: {m}"),
        }
    }
}

impl From<CallError> for AppError {
    fn from(e: CallError) -> Self {
        match e {
            CallError::Api { code: 9 | 4 | 14 | 15, message } => AppError::Auth { provider: PROVIDER.into(), message },
            CallError::Api { code: 29, .. } => AppError::RateLimited { provider: PROVIDER.into(), retry_after_secs: None, quota_exceeded: false },
            CallError::Api { code, message } => AppError::Http { provider: PROVIDER.into(), status: code as u16, message },
            CallError::Transport(message) => AppError::Network { provider: PROVIDER.into(), message },
        }
    }
}

/// Last.fm API signature: md5 of all parameters (except `format` and
/// `callback`) sorted by name and concatenated as `namevalue`, followed by
/// the shared secret. UTF-8 throughout.
pub fn sign(params: &BTreeMap<String, String>, secret: &str) -> String {
    let mut s = String::new();
    for (k, v) in params {
        if k == "format" || k == "callback" {
            continue;
        }
        s.push_str(k);
        s.push_str(v);
    }
    s.push_str(secret);
    format!("{:x}", md5::compute(s.as_bytes()))
}

#[derive(Debug, Clone, PartialEq)]
pub struct NowPlaying {
    pub artist: String,
    pub track: String,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub duration_s: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BatchResult {
    pub accepted: u32,
    pub ignored: u32,
}

pub struct LastFmAdapter {
    http: reqwest::Client,
    api_base: String,
    auth_base: String,
}

impl LastFmAdapter {
    pub fn new(http: reqwest::Client) -> Self {
        Self::with_base(http, API_BASE, AUTH_BASE)
    }

    pub fn with_base(http: reqwest::Client, api_base: &str, auth_base: &str) -> Self {
        LastFmAdapter { http, api_base: api_base.into(), auth_base: auth_base.into() }
    }

    pub fn status(configured: bool, connected: bool, account: Option<String>, enabled: bool, spotify: bool) -> ProviderStatus {
        ProviderStatus {
            provider: PROVIDER.into(),
            display_name: "Last.fm".into(),
            configured,
            connected,
            needs_reconnect: false,
            missing_scopes: Vec::new(),
            account_name: account,
            capabilities: PlaybackCapabilities::NONE,
            limitations: vec![
                if spotify {
                    "Scrobbling and Now Playing are sent for local files, Spotify and SoundCloud tracks.".into()
                } else {
                    "Scrobbling and Now Playing are sent for local files and SoundCloud tracks (Spotify scrobbling is turned off).".into()
                },
                "If your Spotify account is also linked to Last.fm in Spotify's own settings, Spotify plays will be scrobbled twice. Turn off one of the two.".into(),
                "A track is scrobbled when it is longer than 30 seconds and has played for half its length or 4 minutes, whichever comes first.".into(),
                "Scrobbles that cannot be sent are cached and retried (oldest first, in batches of up to 50).".into(),
                if enabled { "Scrobbling is enabled.".into() } else { "Scrobbling is currently disabled in Settings.".into() },
            ],
        }
    }

    pub fn auth_url(&self, api_key: &str, token: &str) -> String {
        format!(
            "{}?api_key={}&token={}",
            self.auth_base,
            urlencoding::encode(api_key),
            urlencoding::encode(token)
        )
    }

    async fn call(
        &self,
        creds: &LastFmCredentials,
        method: &str,
        mut params: BTreeMap<String, String>,
        session_key: Option<&Secret>,
    ) -> Result<serde_json::Value, CallError> {
        params.insert("method".into(), method.into());
        params.insert("api_key".into(), creds.api_key.clone());
        if let Some(sk) = session_key {
            params.insert("sk".into(), sk.expose().to_string());
        }
        let sig = sign(&params, creds.shared_secret.expose());
        params.insert("api_sig".into(), sig);
        params.insert("format".into(), "json".into());

        // POST with a form body for everything: keeps secrets/session keys
        // out of URLs (and therefore out of any proxy or error logs).
        let mut attempt = 0;
        let resp = loop {
            attempt += 1;
            match self.http.post(&self.api_base).form(&params).send().await {
                Ok(r) => break r,
                Err(e) if attempt < 2 => {
                    log::warn!("lastfm: transport error, retrying once");
                    let _ = e;
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
                Err(e) => return Err(CallError::Transport(sanitize_reqwest_error(&e))),
            }
        };
        let status = resp.status();
        let body = resp.text().await.map_err(|e| CallError::Transport(sanitize_reqwest_error(&e)))?;
        let v: serde_json::Value = serde_json::from_str(&body)
            .map_err(|_| CallError::Transport(format!("unexpected response (HTTP {status})")))?;
        if let Some(code) = v.get("error").and_then(|c| c.as_i64()) {
            let message = v.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error").to_string();
            return Err(CallError::Api { code, message });
        }
        if !status.is_success() {
            return Err(CallError::Transport(format!("HTTP {status}")));
        }
        Ok(v)
    }

    pub async fn get_token(&self, creds: &LastFmCredentials) -> Result<String, CallError> {
        let v = self.call(creds, "auth.getToken", BTreeMap::new(), None).await?;
        v.get("token")
            .and_then(|t| t.as_str())
            .map(String::from)
            .ok_or_else(|| CallError::Transport("auth.getToken returned no token".into()))
    }

    /// Returns `(session_key, username)`. Error 14 means the user has not yet
    /// authorised the token; callers poll.
    pub async fn get_session(&self, creds: &LastFmCredentials, token: &str) -> Result<(Secret, String), CallError> {
        let mut p = BTreeMap::new();
        p.insert("token".into(), token.into());
        let v = self.call(creds, "auth.getSession", p, None).await?;
        let key = v.pointer("/session/key").and_then(|k| k.as_str());
        let name = v.pointer("/session/name").and_then(|k| k.as_str());
        match (key, name) {
            (Some(k), Some(n)) => Ok((Secret::new(k), n.to_string())),
            _ => Err(CallError::Transport("auth.getSession returned no session".into())),
        }
    }

    pub async fn update_now_playing(&self, creds: &LastFmCredentials, sk: &Secret, np: &NowPlaying) -> Result<(), CallError> {
        let mut p = BTreeMap::new();
        p.insert("artist".into(), np.artist.clone());
        p.insert("track".into(), np.track.clone());
        if let Some(a) = np.album.as_ref().filter(|s| !s.is_empty()) {
            p.insert("album".into(), a.clone());
        }
        if let Some(a) = np.album_artist.as_ref().filter(|s| !s.is_empty()) {
            p.insert("albumArtist".into(), a.clone());
        }
        if let Some(n) = np.track_number {
            p.insert("trackNumber".into(), n.to_string());
        }
        if let Some(d) = np.duration_s {
            p.insert("duration".into(), d.to_string());
        }
        self.call(creds, "track.updateNowPlaying", p, Some(sk)).await.map(|_| ())
    }

    /// Build the indexed parameter map for `track.scrobble` (max 50).
    pub fn scrobble_params(batch: &[PendingScrobble]) -> BTreeMap<String, String> {
        let mut p = BTreeMap::new();
        for (i, s) in batch.iter().enumerate() {
            p.insert(format!("artist[{i}]"), s.artist.clone());
            p.insert(format!("track[{i}]"), s.track.clone());
            p.insert(format!("timestamp[{i}]"), s.timestamp.to_string());
            if let Some(a) = s.album.as_ref().filter(|a| !a.is_empty()) {
                p.insert(format!("album[{i}]"), a.clone());
            }
            if let Some(a) = s.album_artist.as_ref().filter(|a| !a.is_empty()) {
                p.insert(format!("albumArtist[{i}]"), a.clone());
            }
            if let Some(n) = s.track_number {
                p.insert(format!("trackNumber[{i}]"), n.to_string());
            }
            if let Some(d) = s.duration_s {
                p.insert(format!("duration[{i}]"), d.to_string());
            }
        }
        p
    }

    pub async fn scrobble(&self, creds: &LastFmCredentials, sk: &Secret, batch: &[PendingScrobble]) -> Result<BatchResult, CallError> {
        assert!(batch.len() <= 50, "Last.fm accepts at most 50 scrobbles per request");
        let v = self.call(creds, "track.scrobble", Self::scrobble_params(batch), Some(sk)).await?;
        let num = |p: &str| -> u32 {
            v.pointer(p)
                .and_then(|x| x.as_u64().or_else(|| x.as_str().and_then(|s| s.parse().ok())))
                .unwrap_or(0) as u32
        };
        Ok(BatchResult { accepted: num("/scrobbles/@attr/accepted"), ignored: num("/scrobbles/@attr/ignored") })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds() -> LastFmCredentials {
        LastFmCredentials { api_key: "KEY".into(), shared_secret: Secret::new("SECRET") }
    }

    #[test]
    fn signature_sorts_params_and_skips_format() {
        let mut p = BTreeMap::new();
        p.insert("method".to_string(), "auth.getSession".to_string());
        p.insert("api_key".to_string(), "KEY".to_string());
        p.insert("token".to_string(), "TOK".to_string());
        p.insert("format".to_string(), "json".to_string());
        let expected = format!("{:x}", md5::compute("api_keyKEYmethodauth.getSessiontokenTOKSECRET"));
        assert_eq!(sign(&p, "SECRET"), expected);
    }

    #[test]
    fn signature_handles_utf8_and_indexed_keys() {
        let mut p = BTreeMap::new();
        p.insert("artist[0]".to_string(), "Björk".to_string());
        p.insert("api_key".to_string(), "K".to_string());
        let expected = format!("{:x}", md5::compute("api_keyKartist[0]BjörkS".as_bytes()));
        assert_eq!(sign(&p, "S"), expected);
    }

    #[tokio::test]
    async fn get_session_parses_and_maps_not_authorized() {
        let mut server = mockito::Server::new_async().await;
        let lf = LastFmAdapter::with_base(crate::http::client(), &format!("{}/2.0/", server.url()), "x");
        let m = server
            .mock("POST", "/2.0/")
            .match_body(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("method".into(), "auth.getSession".into()),
                mockito::Matcher::UrlEncoded("token".into(), "T1".into()),
                mockito::Matcher::Regex("api_sig=[0-9a-f]{32}".into()),
            ]))
            .with_body(r#"{"session":{"name":"alice","key":"SK","subscriber":0}}"#)
            .create_async()
            .await;
        let (sk, name) = lf.get_session(&creds(), "T1").await.unwrap();
        assert_eq!(sk.expose(), "SK");
        assert_eq!(name, "alice");
        m.assert_async().await;

        server.reset();
        server
            .mock("POST", "/2.0/")
            .with_status(403)
            .with_body(r#"{"error":14,"message":"Unauthorized Token - This token has not been authorized"}"#)
            .create_async()
            .await;
        let err = lf.get_session(&creds(), "T1").await.unwrap_err();
        assert_eq!(err, CallError::Api { code: 14, message: "Unauthorized Token - This token has not been authorized".into() });
    }

    #[tokio::test]
    async fn scrobble_sends_indexed_batch_and_reads_counts() {
        let mut server = mockito::Server::new_async().await;
        let lf = LastFmAdapter::with_base(crate::http::client(), &format!("{}/2.0/", server.url()), "x");
        let m = server
            .mock("POST", "/2.0/")
            .match_body(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("method".into(), "track.scrobble".into()),
                mockito::Matcher::UrlEncoded("artist[0]".into(), "A".into()),
                mockito::Matcher::UrlEncoded("artist[1]".into(), "B".into()),
                mockito::Matcher::UrlEncoded("timestamp[1]".into(), "1700000100".into()),
                mockito::Matcher::UrlEncoded("sk".into(), "SK".into()),
            ]))
            .with_body(r#"{"scrobbles":{"scrobble":[],"@attr":{"accepted":1,"ignored":1}}}"#)
            .create_async()
            .await;
        let mk = |id, artist: &str, ts| PendingScrobble {
            id,
            track_id: None,
            artist: artist.into(),
            track: "t".into(),
            album: None,
            album_artist: None,
            track_number: None,
            duration_s: Some(200),
            timestamp: ts,
            attempts: 0,
        };
        let res = lf.scrobble(&creds(), &Secret::new("SK"), &[mk(1, "A", 1_700_000_000), mk(2, "B", 1_700_000_100)]).await.unwrap();
        assert_eq!(res, BatchResult { accepted: 1, ignored: 1 });
        m.assert_async().await;
    }

    #[test]
    fn retry_classification() {
        assert!(CallError::Api { code: 11, message: String::new() }.is_retryable());
        assert!(CallError::Api { code: 16, message: String::new() }.is_retryable());
        assert!(CallError::Transport("x".into()).is_retryable());
        assert!(!CallError::Api { code: 6, message: String::new() }.is_retryable());
        assert!(CallError::Api { code: 9, message: String::new() }.is_invalid_session());
    }
}
