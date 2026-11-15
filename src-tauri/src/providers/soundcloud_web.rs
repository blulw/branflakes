//! SoundCloudWebAdapter — **unofficial** SoundCloud access without an API app.
//!
//! This is opt-in (Settings → SoundCloud → "Web mode") and deliberately
//! outside the "official APIs only" rule, at the user's request. It uses the
//! same undocumented `api-v2.soundcloud.com` endpoints that soundcloud.com's own
//! web player uses, with the public `client_id` embedded in the site's
//! JavaScript (the approach used by yt-dlp).
//!
//! Guardrails:
//! * Only streams soundcloud.com plays for free in a browser. DRM-protected
//!   transcodings (`ctr-*` / `cbc-*` protocols, i.e. Go+ content) are never used.
//!   Snipped tracks stay previews.
//! * Streaming only — nothing is downloaded or saved.
//! * No user login; only public tracks, playlists and public profile likes.
//!
//! Risks (documented in the UI): breaks whenever SoundCloud changes its site,
//! may violate SoundCloud's Terms of Use, and SoundCloud can rate-limit or
//! block it at any time.

use crate::error::{AppError, AppResult};
use crate::http::{self, RetryPolicy};
use crate::model::{Availability, TrackUpsert};
use crate::providers::soundcloud::{safe_urn, SoundCloudAdapter, StreamInfo};
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

pub const SITE: &str = "https://soundcloud.com";
pub const API_V2: &str = "https://api-v2.soundcloud.com";
const PROVIDER: &str = "soundcloud";
/// A normal browser UA: the site serves its web app (and client_id) to browsers.
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36";
const HYDRATE_BATCH: usize = 50;

fn script_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r#"<script[^>]+src="([^"]+\.js)""#).unwrap())
}

fn client_id_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r#"client_id\s*[:=]\s*"([0-9a-zA-Z]{32})""#).unwrap())
}

/// Script URLs in page order.
pub fn extract_script_urls(html: &str) -> Vec<String> {
    script_re().captures_iter(html).map(|c| c[1].to_string()).collect()
}

pub fn extract_client_id(js: &str) -> Option<String> {
    client_id_re().captures(js).map(|c| c[1].to_string())
}

/// Result of importing a pasted SoundCloud URL.
#[derive(Debug, Clone, PartialEq)]
pub enum WebImport {
    Track(TrackUpsert),
    Playlist { urn: String, title: String, tracks: Vec<TrackUpsert> },
    UserLikes { username: String, user_id: u64, tracks: Vec<TrackUpsert> },
}

pub struct SoundCloudWebAdapter {
    http: reqwest::Client,
    site: String,
    api: String,
    policy: RetryPolicy,
    client_id: tokio::sync::Mutex<Option<String>>,
}

impl SoundCloudWebAdapter {
    pub fn new() -> Self {
        Self::with_base(SITE, API_V2, RetryPolicy::default())
    }

    pub fn with_base(site: &str, api: &str, policy: RetryPolicy) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(BROWSER_UA)
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .expect("http client");
        SoundCloudWebAdapter {
            http,
            site: site.trim_end_matches('/').into(),
            api: api.trim_end_matches('/').into(),
            policy,
            client_id: tokio::sync::Mutex::new(None),
        }
    }

    pub fn limitations() -> Vec<String> {
        vec![
            "Web mode is UNOFFICIAL: it uses soundcloud.com's internal web API, not the public API. It may break without warning when SoundCloud changes its website, may be rate-limited or blocked, and may be against SoundCloud's Terms of Use.".into(),
            "Web mode plays public tracks only (no sign-in). Go+ tracks play the free 30-second preview; DRM-protected streams are never used. Nothing is downloaded.".into(),
            "Add music in web mode by searching, or by pasting a SoundCloud track, playlist or profile link (a profile imports that user's public likes).".into(),
        ]
    }

    // ---------------------------------------------------------- client_id

    async fn discover_client_id(&self) -> AppResult<String> {
        let net = |e: reqwest::Error| AppError::Network { provider: PROVIDER.into(), message: http::sanitize_reqwest_error(&e) };
        let html = self.http.get(format!("{}/", self.site)).send().await.map_err(net)?.text().await.map_err(net)?;
        // The client_id lives in one of the app bundles, usually a later one.
        for src in extract_script_urls(&html).into_iter().rev() {
            let url = if src.starts_with("http") { src } else { format!("{}{}", self.site, src) };
            let Ok(resp) = self.http.get(&url).send().await else { continue };
            let Ok(js) = resp.text().await else { continue };
            if let Some(id) = extract_client_id(&js) {
                return Ok(id);
            }
        }
        Err(AppError::Other(
            "SoundCloud web mode could not find a client ID on soundcloud.com (the website has probably changed). Web mode is unofficial and may stop working.".into(),
        ))
    }

    async fn client_id(&self, force_refresh: bool) -> AppResult<String> {
        let mut g = self.client_id.lock().await;
        if force_refresh {
            *g = None;
        }
        if let Some(id) = g.as_ref() {
            return Ok(id.clone());
        }
        let id = self.discover_client_id().await?;
        *g = Some(id.clone());
        Ok(id)
    }

    /// GET on api-v2 with `client_id`; on 401/403 refresh the client_id once.
    async fn get(&self, path_or_url: &str, query: &[(&str, String)]) -> AppResult<Value> {
        let url = if path_or_url.starts_with("http") { path_or_url.to_string() } else { format!("{}{}", self.api, path_or_url) };
        for attempt in 0..2 {
            let cid = self.client_id(attempt > 0).await?;
            let mut q: Vec<(&str, String)> = query.to_vec();
            q.push(("client_id", cid));
            let res = http::send(PROVIDER, self.policy, || {
                self.http.get(&url).header(reqwest::header::ACCEPT, "application/json").query(&q)
            })
            .await;
            match res {
                Ok(r) => {
                    return r.json().await.map_err(|e| AppError::Network { provider: PROVIDER.into(), message: http::sanitize_reqwest_error(&e) })
                }
                Err(AppError::Auth { .. } | AppError::Http { status: 403, .. }) if attempt == 0 => continue,
                Err(e) => return Err(e),
            }
        }
        Err(AppError::Other("SoundCloud web mode was refused even with a fresh client ID".into()))
    }

    async fn collect(&self, path_or_url: &str, query: &[(&str, String)], cap: usize) -> AppResult<Vec<Value>> {
        let mut out = Vec::new();
        let mut v = self.get(path_or_url, query).await?;
        loop {
            if let Some(items) = v.get("collection").and_then(Value::as_array) {
                out.extend(items.iter().cloned());
            }
            if out.len() >= cap {
                out.truncate(cap);
                break;
            }
            match v.get("next_href").and_then(Value::as_str) {
                // next_href carries its own paging params; get() adds client_id.
                Some(next) if !next.is_empty() => v = self.get(next, &[]).await?,
                _ => break,
            }
        }
        Ok(out)
    }

    // ------------------------------------------------------------ mapping

    /// Map an api-v2 track. Returns `None` for stubs (only id/policy) that
    /// still need hydration, and for non-track items.
    pub fn map_track(v: &Value) -> Option<TrackUpsert> {
        if v.get("kind").and_then(Value::as_str).is_some_and(|k| k != "track") {
            return None;
        }
        v.get("title").and_then(Value::as_str)?;
        let mut t = SoundCloudAdapter::map_track(v)?;
        let policy = v.get("policy").and_then(Value::as_str).unwrap_or("ALLOW");
        let streamable = v.get("streamable").and_then(Value::as_bool).unwrap_or(true);
        let (availability, reason) = match (policy, streamable) {
            ("BLOCK", _) => (Availability::Unavailable, Some("Blocked on SoundCloud in your region".to_string())),
            (_, false) => (Availability::Unavailable, Some("The uploader has disabled streaming for this track".to_string())),
            ("SNIP", _) => (Availability::PreviewOnly, Some("SoundCloud Go+ track: only the free 30-second preview can play".to_string())),
            _ => (Availability::Available, None),
        };
        t.availability = Some(availability);
        t.unavailable_reason = reason;
        // Snipped tracks report duration = preview length; full_duration is the real one.
        if let Some(full) = v.get("full_duration").and_then(Value::as_u64) {
            t.duration_ms = Some(full);
        }
        if let Some(a) = v.pointer("/publisher_metadata/artist").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
            t.artist = a.to_string();
        }
        if t.year.is_none() {
            t.year = v
                .get("release_date")
                .or_else(|| v.get("display_date"))
                .and_then(Value::as_str)
                .and_then(|d| d.get(..4))
                .and_then(|y| y.parse().ok());
        }
        Some(t)
    }

    /// Replace stub entries (id only) with full tracks via `GET /tracks?ids=`,
    /// preserving playlist order.
    async fn hydrate(&self, items: &[Value]) -> AppResult<Vec<TrackUpsert>> {
        let stub_ids: Vec<u64> = items
            .iter()
            .filter(|t| t.get("title").is_none())
            .filter_map(|t| t.get("id").and_then(Value::as_u64))
            .collect();
        let mut full: std::collections::HashMap<u64, Value> = std::collections::HashMap::new();
        for chunk in stub_ids.chunks(HYDRATE_BATCH) {
            let ids = chunk.iter().map(u64::to_string).collect::<Vec<_>>().join(",");
            let v = self.get("/tracks", &[("ids", ids)]).await?;
            for t in v.as_array().into_iter().flatten() {
                if let Some(id) = t.get("id").and_then(Value::as_u64) {
                    full.insert(id, t.clone());
                }
            }
        }
        Ok(items
            .iter()
            .filter_map(|t| {
                let resolved = match t.get("title") {
                    Some(_) => Some(t),
                    None => t.get("id").and_then(Value::as_u64).and_then(|id| full.get(&id)),
                };
                resolved.and_then(Self::map_track)
            })
            .collect())
    }

    // --------------------------------------------------------- operations

    pub async fn search(&self, q: &str, limit: u32, offset: u32) -> AppResult<Vec<TrackUpsert>> {
        let v = self
            .get(
                "/search/tracks",
                &[("q", q.into()), ("limit", limit.min(50).to_string()), ("offset", offset.to_string()), ("linked_partitioning", "1".into())],
            )
            .await?;
        Ok(v.get("collection").and_then(Value::as_array).map(|a| a.iter().filter_map(Self::map_track).collect()).unwrap_or_default())
    }

    /// Normalise a user-pasted link. Short `on.soundcloud.com` links are
    /// expanded by following their redirect.
    pub async fn normalize_url(&self, input: &str) -> AppResult<String> {
        let parsed = url::Url::parse(input.trim()).map_err(|_| AppError::InvalidInput("not a valid URL".into()))?;
        let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
        let mut u = match host.as_str() {
            "soundcloud.com" | "www.soundcloud.com" | "m.soundcloud.com" => parsed,
            "on.soundcloud.com" => {
                let resp = self.http.get(parsed.as_str()).send().await.map_err(|e| AppError::Network { provider: PROVIDER.into(), message: http::sanitize_reqwest_error(&e) })?;
                let fin = resp.url().clone();
                if !fin.host_str().is_some_and(|h| h.ends_with("soundcloud.com")) || fin.host_str() == Some("on.soundcloud.com") {
                    return Err(AppError::InvalidInput("that short link did not lead to a SoundCloud page".into()));
                }
                fin
            }
            _ => return Err(AppError::InvalidInput("paste a soundcloud.com link".into())),
        };
        u.set_host(Some("soundcloud.com")).ok();
        u.set_scheme("https").ok();
        u.set_query(None);
        u.set_fragment(None);
        Ok(u.to_string())
    }

    pub async fn import_url(&self, input: &str, likes_cap: usize) -> AppResult<WebImport> {
        let url = self.normalize_url(input).await?;
        let v = self.get("/resolve", &[("url", url)]).await?;
        match v.get("kind").and_then(Value::as_str) {
            Some("track") => Self::map_track(&v).map(WebImport::Track).ok_or_else(|| AppError::Other("SoundCloud returned an incomplete track".into())),
            Some("playlist") | Some("system-playlist") => {
                let items = v.get("tracks").and_then(Value::as_array).cloned().unwrap_or_default();
                let urn = v
                    .get("urn")
                    .and_then(Value::as_str)
                    .map(String::from)
                    .or_else(|| v.get("id").and_then(Value::as_u64).map(|id| format!("soundcloud:playlists:{id}")))
                    .unwrap_or_else(|| "soundcloud:playlists:unknown".into());
                Ok(WebImport::Playlist {
                    urn,
                    title: v.get("title").and_then(Value::as_str).unwrap_or("SoundCloud playlist").into(),
                    tracks: self.hydrate(&items).await?,
                })
            }
            Some("user") => {
                let user_id = v.get("id").and_then(Value::as_u64).ok_or_else(|| AppError::Other("SoundCloud user has no id".into()))?;
                let username = v.get("username").and_then(Value::as_str).unwrap_or("SoundCloud user").to_string();
                let items = self
                    .collect(&format!("/users/{user_id}/likes"), &[("limit", "200".into()), ("linked_partitioning", "1".into())], likes_cap)
                    .await?;
                let tracks: Vec<Value> = items.iter().filter_map(|i| i.get("track").cloned()).collect();
                Ok(WebImport::UserLikes { username, user_id, tracks: self.hydrate(&tracks).await? })
            }
            other => Err(AppError::Unsupported(format!("SoundCloud link type {other:?} is not supported; paste a track, playlist or profile link"))),
        }
    }

    /// Rank a transcoding; `None` = never use (DRM, adaptive presets, or
    /// formats WebView2/hls.js cannot play).
    pub fn rank_transcoding(t: &Value) -> Option<(u8, u8, u8)> {
        t.get("url").and_then(Value::as_str).filter(|u| !u.is_empty())?;
        let preset = t.get("preset").and_then(Value::as_str)?;
        if preset.starts_with("abr") {
            return None;
        }
        let protocol = t.pointer("/format/protocol").and_then(Value::as_str).unwrap_or("progressive");
        if protocol.starts_with("ctr-") || protocol.starts_with("cbc-") {
            return None; // DRM (Go+): never used
        }
        let mime = t.pointer("/format/mime_type").and_then(Value::as_str).unwrap_or("");
        let snipped = t.get("snipped").and_then(Value::as_bool).unwrap_or(false);
        let proto_rank = match protocol {
            "hls" => 0,
            "encrypted-hls" => 1, // standard AES-128 HLS, supported by hls.js
            "progressive" => 2,
            _ => return None,
        };
        let codec_rank = if mime.contains("mp4") || mime.contains("aac") {
            0
        } else if mime.contains("mpeg") {
            1
        } else if mime.contains("ogg") && protocol != "progressive" {
            return None; // hls.js can't demux Ogg segments
        } else {
            2
        };
        Some((snipped as u8, proto_rank, codec_rank))
    }

    pub async fn resolve_stream(&self, urn: &str) -> AppResult<StreamInfo> {
        safe_urn(urn, "tracks")?;
        let id = urn.rsplit(':').next().unwrap_or_default();
        let track = self.get(&format!("/tracks/{id}"), &[]).await?;
        let transcodings = track.pointer("/media/transcodings").and_then(Value::as_array).cloned().unwrap_or_default();
        let best = transcodings
            .iter()
            .filter_map(|t| Self::rank_transcoding(t).map(|r| (r, t)))
            .min_by_key(|(r, _)| *r)
            .map(|(_, t)| t)
            .ok_or_else(|| AppError::Unsupported("SoundCloud offers no stream this app can play for this track (it may be DRM-protected Go+ content)".into()))?;
        let mut q: Vec<(&str, String)> = Vec::new();
        if let Some(auth) = track.get("track_authorization").and_then(Value::as_str) {
            q.push(("track_authorization", auth.into()));
        }
        let info = self.get(best["url"].as_str().unwrap_or_default(), &q).await?;
        let url = info
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Other("SoundCloud returned no stream URL".into()))?
            .to_string();
        let protocol = best.pointer("/format/protocol").and_then(Value::as_str).unwrap_or("progressive");
        Ok(StreamInfo {
            url,
            protocol: if protocol.contains("hls") { "hls".into() } else { "progressive".into() },
            preview: best.get("snipped").and_then(Value::as_bool).unwrap_or(false),
            format: best.get("preset").and_then(Value::as_str).unwrap_or_default().into(),
        })
    }
}

impl Default for SoundCloudWebAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Matcher;

    const CID: &str = "abcdefghijklmnopqrstuvwxyz012345";

    async fn setup() -> (mockito::ServerGuard, SoundCloudWebAdapter) {
        let mut server = mockito::Server::new_async().await;
        let html = format!(
            r#"<html><script crossorigin src="{0}/assets/0-a.js"></script><script crossorigin src="{0}/assets/49-b.js"></script></html>"#,
            server.url()
        );
        server.mock("GET", "/").with_body(html).create_async().await;
        server.mock("GET", "/assets/49-b.js").with_body(format!(r#"...,client_id:"{CID}",env:"production"..."#)).create_async().await;
        server.mock("GET", "/assets/0-a.js").with_body("no id here").create_async().await;
        let a = SoundCloudWebAdapter::with_base(&server.url(), &format!("{}/v2", server.url()), RetryPolicy::immediate());
        (server, a)
    }

    #[test]
    fn extracts_scripts_and_client_id() {
        let html = r#"<script crossorigin src="https://a-v2.sndcdn.com/assets/1.js"></script><script src="https://a-v2.sndcdn.com/assets/2.js"></script>"#;
        assert_eq!(extract_script_urls(html).len(), 2);
        assert_eq!(extract_client_id(&format!(r#"x={{client_id:"{CID}"}}"#)).as_deref(), Some(CID));
        assert_eq!(extract_client_id(&format!(r#"?client_id="{CID}""#)).as_deref(), Some(CID));
        assert!(extract_client_id(r#"client_id:"short""#).is_none());
    }

    #[tokio::test]
    async fn search_discovers_client_id_and_maps_tracks() {
        let (mut server, a) = setup().await;
        let m = server
            .mock("GET", "/v2/search/tracks")
            .match_query(Matcher::AllOf(vec![Matcher::UrlEncoded("client_id".into(), CID.into()), Matcher::UrlEncoded("q".into(), "lofi".into())]))
            .with_body(r#"{"collection":[
                {"kind":"track","id":1,"urn":"soundcloud:tracks:1","title":"Free","user":{"username":"u"},"policy":"ALLOW","duration":200000,"full_duration":200000},
                {"kind":"track","id":2,"urn":"soundcloud:tracks:2","title":"GoPlus","user":{"username":"u"},"policy":"SNIP","duration":30000,"full_duration":250000},
                {"kind":"track","id":3,"urn":"soundcloud:tracks:3","title":"Blocked","user":{"username":"u"},"policy":"BLOCK"}]}"#)
            .create_async()
            .await;
        let r = a.search("lofi", 20, 0).await.unwrap();
        m.assert_async().await;
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].availability, Some(Availability::Available));
        assert_eq!(r[1].availability, Some(Availability::PreviewOnly));
        assert_eq!(r[1].duration_ms, Some(250000), "full duration, not preview length");
        assert_eq!(r[2].availability, Some(Availability::Unavailable));
    }

    #[tokio::test]
    async fn refreshes_client_id_after_401() {
        let (mut server, a) = setup().await;
        *a.client_id.lock().await = Some("stalestalestalestalestalestale12".into());
        server
            .mock("GET", "/v2/search/tracks")
            .match_query(Matcher::UrlEncoded("client_id".into(), "stalestalestalestalestalestale12".into()))
            .with_status(401)
            .create_async()
            .await;
        server
            .mock("GET", "/v2/search/tracks")
            .match_query(Matcher::UrlEncoded("client_id".into(), CID.into()))
            .with_body(r#"{"collection":[]}"#)
            .create_async()
            .await;
        assert!(a.search("x", 10, 0).await.unwrap().is_empty());
        assert_eq!(a.client_id.lock().await.as_deref(), Some(CID));
    }

    #[test]
    fn never_selects_drm_and_prefers_full_aac_hls() {
        let ts: Vec<Value> = serde_json::from_str(
            r#"[
            {"url":"u1","preset":"aac_160k","snipped":false,"format":{"protocol":"ctr-encrypted-hls","mime_type":"audio/mp4"}},
            {"url":"u2","preset":"mp3_1_0","snipped":false,"format":{"protocol":"progressive","mime_type":"audio/mpeg"}},
            {"url":"u3","preset":"aac_160k","snipped":false,"format":{"protocol":"hls","mime_type":"audio/mp4; codecs=\"mp4a.40.2\""}},
            {"url":"u4","preset":"aac_160k","snipped":true,"format":{"protocol":"hls","mime_type":"audio/mp4"}},
            {"url":"u5","preset":"abr_sq","snipped":false,"format":{"protocol":"hls","mime_type":"audio/mp4"}},
            {"url":"u6","preset":"opus_0_0","snipped":false,"format":{"protocol":"hls","mime_type":"audio/ogg; codecs=\"opus\""}}
        ]"#,
        )
        .unwrap();
        assert!(SoundCloudWebAdapter::rank_transcoding(&ts[0]).is_none(), "DRM must be skipped");
        assert!(SoundCloudWebAdapter::rank_transcoding(&ts[4]).is_none());
        assert!(SoundCloudWebAdapter::rank_transcoding(&ts[5]).is_none());
        let best = ts.iter().filter_map(|t| SoundCloudWebAdapter::rank_transcoding(t).map(|r| (r, t))).min_by_key(|(r, _)| *r).unwrap().1;
        assert_eq!(best["url"], "u3");
        let only_drm_and_snip = [ts[0].clone(), ts[3].clone()];
        let best = only_drm_and_snip.iter().filter_map(|t| SoundCloudWebAdapter::rank_transcoding(t).map(|r| (r, t))).min_by_key(|(r, _)| *r).unwrap().1;
        assert_eq!(best["url"], "u4", "snipped preview beats DRM");
    }

    #[tokio::test]
    async fn resolves_stream_with_track_authorization() {
        let (mut server, a) = setup().await;
        let tc = format!("{}/v2/media/soundcloud:tracks:7/abc/stream/hls", server.url());
        server
            .mock("GET", "/v2/tracks/7")
            .match_query(Matcher::Any)
            .with_body(
                serde_json::json!({"kind":"track","id":7,"title":"t","track_authorization":"TA",
                  "media":{"transcodings":[{"url":tc,"preset":"aac_160k","snipped":false,"format":{"protocol":"hls","mime_type":"audio/mp4"}}]}})
                .to_string(),
            )
            .create_async()
            .await;
        server
            .mock("GET", "/v2/media/soundcloud:tracks:7/abc/stream/hls")
            .match_query(Matcher::AllOf(vec![Matcher::UrlEncoded("track_authorization".into(), "TA".into()), Matcher::UrlEncoded("client_id".into(), CID.into())]))
            .with_body(r#"{"url":"https://playback.media-streaming.soundcloud.cloud/x/playlist.m3u8"}"#)
            .create_async()
            .await;
        let s = a.resolve_stream("soundcloud:tracks:7").await.unwrap();
        assert_eq!(s.protocol, "hls");
        assert!(!s.preview);
        assert!(s.url.ends_with("playlist.m3u8"));
        assert!(a.resolve_stream("soundcloud:tracks:7/../x").await.is_err());
    }

    #[tokio::test]
    async fn imports_playlist_hydrating_stubs_in_order() {
        let (mut server, a) = setup().await;
        server
            .mock("GET", "/v2/resolve")
            .match_query(Matcher::UrlEncoded("url".into(), "https://soundcloud.com/u/sets/mix".into()))
            .with_body(r#"{"kind":"playlist","id":9,"title":"Mix","tracks":[
                {"kind":"track","id":1,"title":"Full","user":{"username":"u"}},
                {"kind":"track","id":2},{"kind":"track","id":3}]}"#)
            .create_async()
            .await;
        server
            .mock("GET", "/v2/tracks")
            .match_query(Matcher::UrlEncoded("ids".into(), "2,3".into()))
            .with_body(r#"[{"kind":"track","id":3,"title":"Three","user":{"username":"u"}},{"kind":"track","id":2,"title":"Two","user":{"username":"u"}}]"#)
            .create_async()
            .await;
        match a.import_url("https://m.soundcloud.com/u/sets/mix?si=abc", 100).await.unwrap() {
            WebImport::Playlist { title, tracks, urn } => {
                assert_eq!(title, "Mix");
                assert_eq!(urn, "soundcloud:playlists:9");
                assert_eq!(tracks.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["Full", "Two", "Three"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_non_soundcloud_links() {
        let a = SoundCloudWebAdapter::with_base("http://127.0.0.1:9", "http://127.0.0.1:9", RetryPolicy::immediate());
        assert!(a.normalize_url("https://evil.example.com/u/t").await.is_err());
        assert!(a.normalize_url("not a url").await.is_err());
        assert_eq!(a.normalize_url("https://www.soundcloud.com/u/t?in=x#y").await.unwrap(), "https://soundcloud.com/u/t");
    }
}
