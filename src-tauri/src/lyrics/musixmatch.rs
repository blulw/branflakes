//! Musixmatch through the desktop app's API (the catalogue behind most
//! streaming services' lyrics).
//!
//! The API hands out an anonymous user token. When Musixmatch doesn't want to
//! serve this client (it signals that with an all-zero token or a 401), the
//! source is skipped and not asked again for a while; it is never worked
//! around. Text that doesn't match Genius (Musixmatch returns scrambled
//! placeholder lyrics to refused clients) is also rejected by the alignment.

use super::{length_ok, match_score, net, primary_artist, LyricsClient, SyncLookup, TimedLyrics};
use crate::error::{AppError, AppResult};
use crate::http;
use crate::lyrics::parse_lrc;
use crate::model::Track;
use std::time::{Duration, Instant};

const APP_ID: &str = "web-desktop-app-v1.0";
/// After a refusal, don't ask again for this long.
const REFUSED_BACKOFF: Duration = Duration::from_secs(600);

#[derive(Default)]
pub(super) struct TokenCache {
    state: tokio::sync::Mutex<TokenState>,
}

#[derive(Default)]
struct TokenState {
    token: Option<String>,
    refused_until: Option<Instant>,
}

fn refused() -> AppError {
    AppError::Network { provider: "musixmatch".into(), message: "Musixmatch is not serving lyrics to this app right now".into() }
}

fn usable_token(tok: &str) -> bool {
    !tok.is_empty() && !tok.chars().all(|c| c == '0')
}

async fn token(c: &LyricsClient) -> AppResult<String> {
    let mut st = c.mxm.state.lock().await;
    if let Some(t) = &st.token {
        return Ok(t.clone());
    }
    if st.refused_until.is_some_and(|u| Instant::now() < u) {
        return Err(refused());
    }
    let url = format!("{}/ws/1.1/token.get", c.bases.musixmatch);
    let now = chrono::Utc::now().timestamp_millis().to_string();
    let q = [("app_id", APP_ID), ("user_language", "en"), ("t", now.as_str())];
    let v: serde_json::Value = http::send("musixmatch", c.policy, || c.http.get(&url).query(&q)).await?.json().await.map_err(|e| net("musixmatch", &e))?;
    let status = v.pointer("/message/header/status_code").and_then(|s| s.as_u64());
    let tok = v.pointer("/message/body/user_token").and_then(|t| t.as_str()).unwrap_or("");
    if status != Some(200) || !usable_token(tok) {
        st.refused_until = Some(Instant::now() + REFUSED_BACKOFF);
        return Err(refused());
    }
    st.token = Some(tok.to_string());
    Ok(tok.to_string())
}

async fn forget_token(c: &LyricsClient) {
    let mut st = c.mxm.state.lock().await;
    st.token = None;
    st.refused_until = Some(Instant::now() + REFUSED_BACKOFF);
}

pub(super) async fn lookup(c: &LyricsClient, t: &Track) -> AppResult<SyncLookup> {
    let token = token(c).await?;
    let url = format!("{}/ws/1.1/macro.subtitles.get", c.bases.musixmatch);
    let q = [
        ("format", "json".to_string()),
        ("namespace", "lyrics_richsynched".into()),
        ("subtitle_format", "lrc".into()),
        ("app_id", APP_ID.into()),
        ("q_artist", primary_artist(&t.artist)),
        ("q_track", t.title.clone()),
        ("q_album", t.album.clone()),
        ("q_duration", t.duration_ms.map(|d| ((d + 500) / 1000).to_string()).unwrap_or_default()),
        ("usertoken", token),
    ];
    let v: serde_json::Value = http::send("musixmatch", c.policy, || c.http.get(&url).query(&q)).await?.json().await.map_err(|e| net("musixmatch", &e))?;
    let status = |p: &str| v.pointer(p).and_then(|s| s.as_u64());
    if status("/message/header/status_code") == Some(401) || status("/message/body/macro_calls/matcher.track.get/message/header/status_code") == Some(401) {
        forget_token(c).await;
        return Err(refused());
    }
    let Some(track) = v.pointer("/message/body/macro_calls/matcher.track.get/message/body/track") else {
        return Ok(SyncLookup::default());
    };
    let s = |k: &str| track.get(k).and_then(|x| x.as_str()).unwrap_or("");
    let length = track.get("track_length").and_then(|x| x.as_u64()).filter(|l| *l > 0).map(|l| l * 1000);
    if !length_ok(t, length) || match_score(t, s("track_name"), s("artist_name"), length).is_none() {
        return Ok(SyncLookup::default());
    }
    let instrumental = track.get("instrumental").and_then(|x| x.as_u64()) == Some(1);
    let body = v
        .pointer("/message/body/macro_calls/track.subtitles.get/message/body/subtitle_list/0/subtitle/subtitle_body")
        .and_then(|b| b.as_str())
        .unwrap_or("");
    let lines = parse_lrc(body);
    Ok(SyncLookup {
        timed: lines.iter().any(|l| !l.text.is_empty()).then(|| TimedLyrics { source: "musixmatch", lines }),
        plain: None,
        instrumental,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_token_is_a_refusal() {
        assert!(!usable_token("00000000000000000000000000000000000000000000000000000000"));
        assert!(!usable_token(""));
        assert!(usable_token("2203269256ff7abcb649269df00e14c833dbf4ddfb5b36a1aae8b0"));
    }
}
