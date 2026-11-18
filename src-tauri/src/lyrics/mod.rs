//! Lyrics for the playing track.
//!
//! **The words always come from Genius** when Genius has the song. Timing
//! comes from the first source that has it and lines up with Genius's words
//! (`align`):
//!
//! 1. LRCLIB (free community database; looked up alongside Genius),
//! 2. Musixmatch, then 3. NetEase — the "extra sources" (Settings, on by
//!    default), tried only when the previous one didn't give usable timing.
//!
//! If no timing lines up, Genius's text is shown unsynced. Only when Genius
//! has no lyrics are another source's own words used.
//!
//! Every source's match must pass the same title/artist check as "Find
//! alternative" (`alternatives::score`), and timed lyrics must be for a
//! recording within 5 s of the track's length. Results are cached per track
//! in `lyrics_cache`.

mod align;
mod genius;
mod lrc;
mod lrclib;
mod musixmatch;
mod netease;

pub use align::align;
pub use genius::lyrics_from_html as genius_lyrics_from_html;
pub use lrc::{drop_credit_lines, parse_lrc};

use crate::alternatives::{norm_artists, score, Candidate};
use crate::error::{AppError, AppResult};
use crate::http::{self, RetryPolicy};
use crate::model::Track;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LyricLine {
    pub time_ms: u64,
    /// When the line's singing ends (start of a gap), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<u64>,
    /// Empty = an instrumental gap (only in a non-Genius source's own lines).
    pub text: String,
    /// A Genius section header such as "[Chorus]".
    #[serde(default)]
    pub header: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Lyrics {
    /// Timed lines, sorted by time. Genius's lines whenever Genius has text.
    pub synced: Option<Vec<LyricLine>>,
    /// Where the timing came from: `"lrclib"`, `"musixmatch"` or `"netease"`.
    #[serde(default)]
    pub sync_source: Option<String>,
    /// Plain text: Genius when found, otherwise another source's.
    pub plain: Option<String>,
    /// Who wrote the words shown: `"genius"`, or a timing source when Genius has none.
    pub plain_source: Option<String>,
    /// The Genius song page (attribution link).
    pub genius_url: Option<String>,
    /// Known instrumental and no lyrics found.
    pub instrumental: bool,
}

/// Minimum `alternatives::score`: the title must match (exactly or closely)
/// and one of the track's artists must be credited.
const MIN_SCORE: u32 = 50;
/// Timed lyrics are only trusted for a recording of (about) the same length;
/// a radio edit or live version would be out of time.
const MAX_LENGTH_DIFF_MS: u64 = 5_000;

pub(crate) struct Bases {
    pub lrclib: String,
    pub genius: String,
    pub netease: String,
    pub musixmatch: String,
}

pub struct LyricsClient {
    http: reqwest::Client,
    bases: Bases,
    policy: RetryPolicy,
    mxm: musixmatch::TokenCache,
}

/// What a lookup produced. `complete` is false when a source that was asked
/// failed (network, 5xx, refused); such results are cached only briefly.
#[derive(Debug)]
pub struct Fetched {
    pub lyrics: Option<Lyrics>,
    pub complete: bool,
}

/// Timed lines from one source, before aligning them to Genius.
#[derive(Debug, Clone)]
pub(crate) struct TimedLyrics {
    pub source: &'static str,
    pub lines: Vec<LyricLine>,
}

#[derive(Debug, Default)]
pub(crate) struct SyncLookup {
    pub timed: Option<TimedLyrics>,
    /// The source's own untimed text (LRCLIB only).
    pub plain: Option<String>,
    pub instrumental: bool,
}

impl LyricsClient {
    pub fn new(http: reqwest::Client) -> Self {
        LyricsClient {
            http,
            bases: Bases {
                lrclib: "https://lrclib.net".into(),
                genius: "https://genius.com".into(),
                netease: "https://music.163.com".into(),
                musixmatch: "https://apic-desktop.musixmatch.com".into(),
            },
            policy: RetryPolicy { max_attempts: 2, base_delay: Duration::from_millis(500), max_retry_after: Duration::from_secs(5), retry_ambiguous: true },
            mxm: Default::default(),
        }
    }

    /// For tests: every source on one mock server.
    pub fn with_base(http: reqwest::Client, base: &str) -> Self {
        LyricsClient {
            bases: Bases { lrclib: base.into(), genius: base.into(), netease: base.into(), musixmatch: base.into() },
            policy: RetryPolicy::immediate(),
            ..Self::new(http)
        }
    }

    /// `extra_sources`: also try Musixmatch and NetEase for timing.
    pub async fn fetch(&self, t: &Track, extra_sources: bool) -> AppResult<Fetched> {
        let (genius, lrclib) = tokio::join!(genius::lookup(self, t), lrclib::lookup(self, t));
        let mut complete = true;
        let genius = genius.unwrap_or_else(|e| {
            log::warn!("genius lyrics lookup failed: {e}");
            complete = false;
            Default::default()
        });
        let genius_failed = !complete;
        let genius_text = genius.text.as_deref();
        let mut instrumental = genius.instrumental;
        let mut synced: Option<(Vec<LyricLine>, &'static str)>;
        let mut fallback_timed: Option<TimedLyrics> = None;
        let mut fallback_plain: Option<(String, &'static str)> = None;

        let mut consider = |look: AppResult<SyncLookup>, name: &str, complete: &mut bool| -> Option<(Vec<LyricLine>, &'static str)> {
            let look = match look {
                Ok(l) => l,
                Err(e) => {
                    log::info!("{name} lyrics lookup failed: {e}");
                    *complete = false;
                    return None;
                }
            };
            instrumental |= look.instrumental;
            if let (Some(p), None) = (look.plain, &fallback_plain) {
                fallback_plain = Some((p, "lrclib"));
            }
            let timed = look.timed?;
            match genius_text {
                Some(text) => {
                    let aligned = align(text, &timed.lines);
                    if aligned.is_none() {
                        log::info!("{name} timing does not line up with the Genius lyrics");
                    }
                    aligned.map(|l| (l, timed.source))
                }
                // No Genius text: the first timed source's own lines are used.
                None if fallback_timed.is_none() => {
                    fallback_timed = Some(timed.clone());
                    Some((timed.lines, timed.source))
                }
                None => None,
            }
        };

        synced = consider(lrclib, "lrclib", &mut complete);
        if synced.is_none() && extra_sources {
            synced = consider(musixmatch::lookup(self, t).await, "musixmatch", &mut complete);
        }
        if synced.is_none() && extra_sources {
            synced = consider(netease::lookup(self, t).await, "netease", &mut complete);
        }

        let (plain, plain_source) = match (genius.text.clone(), &synced, fallback_plain) {
            (Some(g), _, _) => (Some(g), "genius"),
            (None, _, Some((p, src))) => (Some(p), src),
            (None, Some((lines, src)), None) => {
                let text = lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n").trim().to_string();
                (Some(text).filter(|t| !t.is_empty()), *src)
            }
            (None, None, None) => (None, ""),
        };
        let lyrics = Lyrics {
            instrumental: synced.is_none() && plain.is_none() && instrumental,
            sync_source: synced.as_ref().map(|(_, s)| s.to_string()),
            synced: synced.map(|(l, _)| l),
            plain_source: plain.as_ref().map(|_| plain_source.to_string()),
            plain,
            genius_url: genius.url,
        };
        let found = lyrics.synced.is_some() || lyrics.plain.is_some() || lyrics.instrumental;
        if !found && genius_failed {
            // Genius failed and nothing else had words: report it rather than
            // a "no lyrics" that might be wrong.
            return Err(AppError::Network { provider: "genius".into(), message: "Genius could not be reached".into() });
        }
        Ok(Fetched { lyrics: found.then_some(lyrics), complete })
    }
}

pub(crate) fn net(provider: &str, e: &reqwest::Error) -> AppError {
    AppError::Network { provider: provider.into(), message: http::sanitize_reqwest_error(e) }
}

/// The first credited artist, as written ("A, B & C" → "A").
pub(crate) fn primary_artist(credit: &str) -> String {
    let lower = credit.to_lowercase();
    let mut cut = credit.len();
    for sep in [",", " & ", " x ", " feat. ", " feat ", " ft. ", " featuring ", " with ", ";", "/"] {
        if let Some(i) = lower.find(sep) {
            cut = cut.min(i);
        }
    }
    credit.get(..cut).unwrap_or(credit).trim().to_string()
}

pub(crate) fn match_score(t: &Track, title: &str, artist: &str, duration_ms: Option<u64>) -> Option<u32> {
    score(&t.title, &t.artist, t.duration_ms, &Candidate { title: title.into(), artist: artist.into(), uploader: None, duration_ms })
        .filter(|s| *s >= MIN_SCORE)
}

pub(crate) fn length_ok(t: &Track, candidate_ms: Option<u64>) -> bool {
    match (t.duration_ms, candidate_ms) {
        (Some(a), Some(b)) => a.abs_diff(b) <= MAX_LENGTH_DIFF_MS,
        _ => true,
    }
}

// ---------------------------------------------------------------- cache

/// "Nothing found" / "no timing yet" is retried after this long.
const UNSYNCED_TTL: chrono::Duration = chrono::Duration::days(3);
/// A lookup where a source failed is retried after this long.
const INCOMPLETE_TTL: chrono::Duration = chrono::Duration::hours(1);

/// What a cached result was looked up with; a metadata edit or a change of
/// sources refetches.
pub fn lookup_key(t: &Track, extra_sources: bool) -> String {
    format!(
        "v2\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        t.title,
        norm_artists(&t.artist).join(","),
        t.duration_ms.map(|d| d / 1000).unwrap_or(0),
        if extra_sources { "extra" } else { "lrclib" }
    )
}

/// `Some(result)` when a usable cache entry exists (`result` may be `None`
/// = "nothing found", still fresh). Synced results are kept for good; others
/// are looked up again later, sooner when a source had failed.
pub fn cached(conn: &Connection, track_id: &str, key: &str) -> AppResult<Option<Option<Lyrics>>> {
    let row: Option<(String, Option<String>, String, bool)> = conn
        .query_row("SELECT lookup_key, payload, fetched_at, complete FROM lyrics_cache WHERE track_id = ?1", [track_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .optional()?;
    let Some((k, payload, fetched_at, complete)) = row else { return Ok(None) };
    if k != key {
        return Ok(None);
    }
    let lyrics = match payload {
        Some(json) => match serde_json::from_str::<Lyrics>(&json) {
            Ok(l) => Some(l),
            Err(_) => return Ok(None),
        },
        None => None,
    };
    if lyrics.as_ref().is_some_and(|l| l.synced.is_some()) {
        return Ok(Some(lyrics));
    }
    let ttl = if complete { UNSYNCED_TTL } else { INCOMPLETE_TTL };
    let fresh = chrono::DateTime::parse_from_rfc3339(&fetched_at).is_ok_and(|t| chrono::Utc::now().signed_duration_since(t) < ttl);
    Ok(fresh.then_some(lyrics))
}

pub fn store(conn: &Connection, track_id: &str, key: &str, lyrics: Option<&Lyrics>, complete: bool) -> AppResult<()> {
    let payload = lyrics.map(serde_json::to_string).transpose().map_err(|e| AppError::Other(e.to_string()))?;
    conn.execute(
        "INSERT INTO lyrics_cache (track_id, lookup_key, payload, fetched_at, complete) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(track_id) DO UPDATE SET lookup_key = excluded.lookup_key, payload = excluded.payload,
             fetched_at = excluded.fetched_at, complete = excluded.complete",
        params![track_id, key, payload, crate::db::now_iso(), complete],
    )?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{Availability, Source};
    use mockito::Matcher;

    pub(crate) fn track(title: &str, artist: &str, duration_ms: Option<u64>) -> Track {
        Track {
            id: "t1".into(),
            source: Source::Spotify,
            provider_id: "x".into(),
            provider_uri: None,
            title: title.into(),
            artist: artist.into(),
            album: "Album".into(),
            album_artist: String::new(),
            track_number: None,
            disc_number: None,
            year: None,
            genre: String::new(),
            duration_ms,
            artwork: None,
            file_path: None,
            file_format: None,
            availability: Availability::Available,
            unavailable_reason: None,
            permalink_url: None,
            uploader: None,
            uploader_url: None,
            in_library: true,
            has_overrides: false,
            added_at: String::new(),
        }
    }

    #[test]
    fn primary_artist_keeps_case() {
        assert_eq!(primary_artist("Simon & Garfunkel"), "Simon");
        assert_eq!(primary_artist("Drake feat. Rihanna"), "Drake");
        assert_eq!(primary_artist("Radiohead"), "Radiohead");
    }

    async fn genius_mocks(server: &mut mockito::ServerGuard) {
        let base = server.url();
        let search_body = format!(
            r#"{{"response":{{"sections":[{{"type":"song","hits":[{{"result":{{"title":"Karma Police (Translation)","artist_names":"Genius Translations","url":"{base}/translation"}}}},{{"result":{{"title":"Karma Police","artist_names":"Radiohead","url":"{base}/Radiohead-karma-police-lyrics","instrumental":false}}}}]}}]}}}}"#
        );
        server.mock("GET", "/api/search/song").match_query(Matcher::Any).with_status(200).with_body(search_body).create_async().await;
        server
            .mock("GET", "/Radiohead-karma-police-lyrics")
            .with_status(200)
            .with_body(r#"<div data-lyrics-container="true">[Verse 1]<br/>Karma police, arrest this man<br/>He talks in maths</div>"#)
            .create_async()
            .await;
    }

    #[tokio::test]
    async fn genius_words_with_lrclib_timing() {
        let mut server = mockito::Server::new_async().await;
        genius_mocks(&mut server).await;
        server
            .mock("GET", "/api/get")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(r#"{"trackName":"Karma Police","artistName":"Radiohead","duration":264.0,"instrumental":false,"plainLyrics":"x","syncedLyrics":"[00:10.00]Karma police\n[00:14.50]Arrest this man\n[00:18.00]He talks in maths"}"#)
            .create_async()
            .await;
        let mxm = server.mock("GET", "/ws/1.1/token.get").match_query(Matcher::Any).expect(0).create_async().await;

        let client = LyricsClient::with_base(http::client(), &server.url());
        let got = client.fetch(&track("Karma Police", "Radiohead", Some(264_000)), true).await.unwrap();
        assert!(got.complete);
        let l = got.lyrics.unwrap();
        let lines: Vec<(u64, &str)> = l.synced.as_ref().unwrap().iter().map(|l| (l.time_ms, l.text.as_str())).collect();
        assert_eq!(lines, vec![(10_000, "[Verse 1]"), (10_000, "Karma police, arrest this man"), (18_000, "He talks in maths")]);
        assert_eq!(l.sync_source.as_deref(), Some("lrclib"));
        assert_eq!(l.plain_source.as_deref(), Some("genius"));
        mxm.assert_async().await; // fallbacks are not asked when LRCLIB lines up
    }

    #[tokio::test]
    async fn falls_back_to_netease_when_others_have_nothing() {
        let mut server = mockito::Server::new_async().await;
        genius_mocks(&mut server).await;
        server.mock("GET", "/api/get").match_query(Matcher::Any).with_status(404).create_async().await;
        server.mock("GET", "/api/search").match_query(Matcher::Any).with_status(200).with_body("[]").create_async().await;
        // Musixmatch refuses (all-zero token) and must be skipped, not retried.
        let refused = server
            .mock("GET", "/ws/1.1/token.get")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(r#"{"message":{"header":{"status_code":200},"body":{"user_token":"0000000000"}}}"#)
            .expect(1)
            .create_async()
            .await;
        server
            .mock("GET", "/api/search/get")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(r#"{"result":{"songs":[{"id":7,"name":"Karma Police","artists":[{"name":"Radiohead"}],"duration":264066},{"id":8,"name":"Karma Police","artists":[{"name":"Cover Band"}],"duration":264000}]}}"#)
            .create_async()
            .await;
        server
            .mock("GET", "/api/song/lyric")
            .match_query(Matcher::UrlEncoded("id".into(), "7".into()))
            .with_status(200)
            .with_body(r#"{"lrc":{"lyric":"[00:00.000] 作词 : Yorke\n[00:27.000]Karma police arrest this man\n[00:31.000]He talks in maths"}}"#)
            .create_async()
            .await;

        let client = LyricsClient::with_base(http::client(), &server.url());
        let t = track("Karma Police", "Radiohead", Some(264_000));
        let got = client.fetch(&t, true).await.unwrap();
        assert!(!got.complete, "Musixmatch refused, so this is cached only briefly");
        let l = got.lyrics.unwrap();
        assert_eq!(l.sync_source.as_deref(), Some("netease"));
        let lines: Vec<(u64, &str)> = l.synced.as_ref().unwrap().iter().filter(|l| !l.header).map(|l| (l.time_ms, l.text.as_str())).collect();
        assert_eq!(lines, vec![(27_000, "Karma police, arrest this man"), (31_000, "He talks in maths")], "Genius's words, NetEase's timing");

        // Second lookup inside the back-off: Musixmatch is not asked again.
        client.fetch(&t, true).await.unwrap();
        refused.assert_async().await;

        // Without extra sources: Genius text, unsynced.
        let l = client.fetch(&t, false).await.unwrap().lyrics.unwrap();
        assert!(l.synced.is_none());
        assert_eq!(l.plain.as_deref(), Some("[Verse 1]\nKarma police, arrest this man\nHe talks in maths"));
    }

    #[tokio::test]
    async fn nothing_found_is_complete_none_and_genius_failure_is_error() {
        let mut server = mockito::Server::new_async().await;
        let base = server.url();
        server.mock("GET", "/api/get").match_query(Matcher::Any).with_status(404).create_async().await;
        server.mock("GET", "/api/search").match_query(Matcher::Any).with_status(200).with_body("[]").create_async().await;
        let genius = server
            .mock("GET", "/api/search/song")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(r#"{"response":{"sections":[{"hits":[]}]}}"#)
            .create_async()
            .await;
        let client = LyricsClient::with_base(http::client(), &base);
        let t = track("Unknown Song", "Nobody", Some(100_000));
        let got = client.fetch(&t, false).await.unwrap();
        assert!(got.complete && got.lyrics.is_none());

        genius.remove_async().await;
        server.mock("GET", "/api/search/song").match_query(Matcher::Any).with_status(503).create_async().await;
        assert!(client.fetch(&t, false).await.is_err(), "a failed source must not look like 'no lyrics'");
    }

    #[test]
    fn cache_roundtrip_ttl_and_key_change() {
        let conn = crate::db::open_in_memory().unwrap();
        conn.execute("INSERT INTO tracks (id, source, provider_id, title, added_at, updated_at) VALUES ('t1', 'spotify', 'x', 'T', 't', 't')", []).unwrap();
        let t = track("T", "A", Some(1000));
        let key = lookup_key(&t, true);
        assert_eq!(cached(&conn, "t1", &key).unwrap(), None);
        store(&conn, "t1", &key, None, true).unwrap();
        assert_eq!(cached(&conn, "t1", &key).unwrap(), Some(None), "a fresh miss is cached");
        let l = Lyrics { plain: Some("la".into()), plain_source: Some("genius".into()), ..Default::default() };
        store(&conn, "t1", &key, Some(&l), false).unwrap();
        assert_eq!(cached(&conn, "t1", &key).unwrap(), Some(Some(l.clone())));
        // An unsynced result from an incomplete lookup expires after an hour.
        conn.execute("UPDATE lyrics_cache SET fetched_at = ?1", [(chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339()]).unwrap();
        assert_eq!(cached(&conn, "t1", &key).unwrap(), None);
        // A synced result is kept.
        let s = Lyrics { synced: Some(vec![LyricLine { time_ms: 0, end_ms: None, text: "la".into(), header: false }]), ..l };
        store(&conn, "t1", &key, Some(&s), false).unwrap();
        conn.execute("UPDATE lyrics_cache SET fetched_at = '2000-01-01T00:00:00Z'", []).unwrap();
        assert_eq!(cached(&conn, "t1", &key).unwrap(), Some(Some(s)));
        assert_eq!(cached(&conn, "t1", &lookup_key(&t, false)).unwrap(), None, "changing sources refetches");
        assert_eq!(cached(&conn, "t1", &lookup_key(&track("T2", "A", Some(1000)), true)).unwrap(), None, "edited metadata refetches");
    }
}

