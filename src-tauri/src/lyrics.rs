//! Lyrics for the playing track.
//!
//! * **Genius** supplies the text (with its `[Verse]`/`[Chorus]` headers). Its
//!   song search works without an API key; the lyrics themselves are only in
//!   the song page's HTML (`data-lyrics-container` blocks), so they are
//!   extracted from there. Genius has no timestamps.
//! * **LRCLIB** (free, no key) supplies time-synced lines in LRC format, which
//!   the player highlights as the song plays.
//!
//! Both are looked up in parallel by title, primary artist and length. A result
//! must pass the same title/artist matching as "Find alternative"
//! (`alternatives::score`), so a cover or a different song with a similar name
//! is not shown. Results (including "nothing found") are cached per track in
//! `lyrics_cache`; a lookup that failed on the network is not cached.

use crate::alternatives::{norm_artists, score, search_query, title_fragment, Candidate};
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
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Lyrics {
    /// Time-synced lines from LRCLIB, sorted by time. Empty text = a pause.
    pub synced: Option<Vec<LyricLine>>,
    /// Plain text: Genius when found, otherwise LRCLIB's.
    pub plain: Option<String>,
    /// `"genius"` or `"lrclib"`.
    pub plain_source: Option<String>,
    /// The Genius song page (attribution link).
    pub genius_url: Option<String>,
    /// Known instrumental and no lyrics found.
    pub instrumental: bool,
}

/// Minimum `alternatives::score`: the title must match (exactly or closely)
/// and one of the track's artists must be credited.
const MIN_SCORE: u32 = 50;
/// Synced lines are only trusted for a recording of (about) the same length;
/// a radio edit or live version would be out of time.
const MAX_SYNCED_LENGTH_DIFF_MS: u64 = 5_000;
/// "Nothing found" is retried after this long (lyrics get added over time).
const MISS_TTL: chrono::Duration = chrono::Duration::days(3);

pub struct LyricsClient {
    http: reqwest::Client,
    lrclib_base: String,
    genius_base: String,
    policy: RetryPolicy,
}

/// What a lookup produced. `complete` is false when a source failed (network,
/// 5xx), in which case the result must not be cached.
#[derive(Debug)]
pub struct Fetched {
    pub lyrics: Option<Lyrics>,
    pub complete: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LrcRecord {
    #[serde(default)]
    track_name: String,
    #[serde(default)]
    artist_name: String,
    duration: Option<f64>,
    #[serde(default)]
    instrumental: bool,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
}

impl LrcRecord {
    fn duration_ms(&self) -> Option<u64> {
        self.duration.filter(|d| *d > 0.0).map(|d| (d * 1000.0).round() as u64)
    }
}

#[derive(Debug, Default)]
struct GeniusResult {
    text: Option<String>,
    url: Option<String>,
    instrumental: bool,
}

impl LyricsClient {
    pub fn new(http: reqwest::Client) -> Self {
        LyricsClient {
            http,
            lrclib_base: "https://lrclib.net".into(),
            genius_base: "https://genius.com".into(),
            policy: RetryPolicy { max_attempts: 2, base_delay: Duration::from_millis(500), max_retry_after: Duration::from_secs(5), retry_ambiguous: true },
        }
    }

    /// For tests: point both sources at a mock server.
    pub fn with_bases(http: reqwest::Client, lrclib_base: &str, genius_base: &str) -> Self {
        LyricsClient { lrclib_base: lrclib_base.into(), genius_base: genius_base.into(), policy: RetryPolicy::immediate(), ..Self::new(http) }
    }

    pub async fn fetch(&self, t: &Track) -> AppResult<Fetched> {
        let (lrc, genius) = tokio::join!(self.lrclib(t), self.genius(t));
        let complete = lrc.is_ok() && genius.is_ok();
        if let Err(e) = &lrc {
            log::warn!("lrclib lyrics lookup failed: {e}");
        }
        if let Err(e) = &genius {
            log::warn!("genius lyrics lookup failed: {e}");
        }
        let (lrc, genius) = match (lrc, genius) {
            (Err(e), Err(_)) => return Err(e),
            (l, g) => (l.ok().flatten(), g.ok().unwrap_or_default()),
        };
        let lyrics = combine(lrc, genius);
        if lyrics.is_none() && !complete {
            // One source failed and the other had nothing: report the failure
            // instead of a "no lyrics" that might be wrong.
            return Err(AppError::Network { provider: "lyrics".into(), message: "a lyrics source could not be reached".into() });
        }
        Ok(Fetched { lyrics, complete })
    }

    // ------------------------------------------------------------ LRCLIB

    async fn lrclib(&self, t: &Track) -> AppResult<Option<LrcRecord>> {
        let primary = primary_artist(&t.artist);
        // Exact lookup first (needs the length); it is LRCLIB's preferred API.
        if let Some(d) = t.duration_ms {
            let url = format!("{}/api/get", self.lrclib_base);
            let q = [
                ("track_name", t.title.clone()),
                ("artist_name", primary.clone()),
                ("album_name", t.album.clone()),
                ("duration", ((d + 500) / 1000).to_string()),
            ];
            match http::send("lrclib", self.policy, || self.http.get(&url).query(&q)).await {
                Ok(resp) => {
                    let rec: LrcRecord = resp.json().await.map_err(|e| net("lrclib", &e))?;
                    if accept_lrc(t, &rec) {
                        return Ok(Some(rec));
                    }
                }
                Err(AppError::Http { status: 404, .. }) => {}
                Err(e) => return Err(e),
            }
        }
        // Then a search with the cleaned-up title.
        let url = format!("{}/api/search", self.lrclib_base);
        let q = [("track_name", title_fragment(&t.title)), ("artist_name", primary)];
        let resp = match http::send("lrclib", self.policy, || self.http.get(&url).query(&q)).await {
            Ok(r) => r,
            Err(AppError::Http { status: 404, .. }) => return Ok(None),
            Err(e) => return Err(e),
        };
        let records: Vec<LrcRecord> = resp.json().await.map_err(|e| net("lrclib", &e))?;
        Ok(best_lrc(t, records))
    }

    // ------------------------------------------------------------ Genius

    async fn genius(&self, t: &Track) -> AppResult<GeniusResult> {
        let query = search_query(t);
        if query.is_empty() {
            return Ok(GeniusResult::default());
        }
        let url = format!("{}/api/search/song", self.genius_base);
        let q = [("q", query), ("per_page", "10".to_string())];
        let resp = http::send("genius", self.policy, || self.http.get(&url).query(&q)).await?;
        let v: serde_json::Value = resp.json().await.map_err(|e| net("genius", &e))?;
        let Some(hit) = best_genius_hit(t, &v) else { return Ok(GeniusResult::default()) };
        if hit.instrumental {
            return Ok(GeniusResult { instrumental: true, url: Some(hit.url), text: None });
        }
        // Only ever fetch pages on Genius itself.
        if !hit.url.starts_with(&format!("{}/", self.genius_base)) {
            return Ok(GeniusResult::default());
        }
        let html = http::send("genius", self.policy, || self.http.get(&hit.url))
            .await?
            .text()
            .await
            .map_err(|e| net("genius", &e))?;
        Ok(GeniusResult { text: genius_lyrics_from_html(&html), url: Some(hit.url), instrumental: false })
    }
}

fn net(provider: &str, e: &reqwest::Error) -> AppError {
    AppError::Network { provider: provider.into(), message: http::sanitize_reqwest_error(e) }
}

/// The first credited artist, as written ("A, B & C" → "A").
fn primary_artist(credit: &str) -> String {
    let lower = credit.to_lowercase();
    let mut cut = credit.len();
    for sep in [",", " & ", " x ", " feat. ", " feat ", " ft. ", " featuring ", " with ", ";", "/"] {
        if let Some(i) = lower.find(sep) {
            cut = cut.min(i);
        }
    }
    credit.get(..cut).unwrap_or(credit).trim().to_string()
}

fn match_score(t: &Track, title: &str, artist: &str, duration_ms: Option<u64>) -> Option<u32> {
    score(&t.title, &t.artist, t.duration_ms, &Candidate { title: title.into(), artist: artist.into(), uploader: None, duration_ms })
        .filter(|s| *s >= MIN_SCORE)
}

fn length_ok(t: &Track, rec: &LrcRecord) -> bool {
    match (t.duration_ms, rec.duration_ms()) {
        (Some(a), Some(b)) => a.abs_diff(b) <= MAX_SYNCED_LENGTH_DIFF_MS,
        _ => true,
    }
}

fn accept_lrc(t: &Track, rec: &LrcRecord) -> bool {
    length_ok(t, rec) && match_score(t, &rec.track_name, &rec.artist_name, rec.duration_ms()).is_some()
}

fn best_lrc(t: &Track, records: Vec<LrcRecord>) -> Option<LrcRecord> {
    records
        .into_iter()
        .filter(|r| length_ok(t, r))
        .filter_map(|r| {
            let s = match_score(t, &r.track_name, &r.artist_name, r.duration_ms())?;
            let has_synced = r.synced_lyrics.as_deref().is_some_and(|s| !s.trim().is_empty());
            let has_any = has_synced || r.plain_lyrics.as_deref().is_some_and(|s| !s.trim().is_empty()) || r.instrumental;
            has_any.then_some(((has_synced, s), r))
        })
        .max_by_key(|(k, _)| *k)
        .map(|(_, r)| r)
}

struct GeniusHit {
    url: String,
    instrumental: bool,
}

fn best_genius_hit(t: &Track, v: &serde_json::Value) -> Option<GeniusHit> {
    let hits = v.pointer("/response/sections").and_then(|s| s.as_array())?.iter().flat_map(|s| s.get("hits").and_then(|h| h.as_array()).into_iter().flatten());
    let mut best: Option<(u32, GeniusHit)> = None;
    for h in hits {
        let r = h.get("result").unwrap_or(h);
        let str_of = |k: &str| r.get(k).and_then(|x| x.as_str()).unwrap_or("");
        let title = str_of("title");
        let artist = match str_of("artist_names") {
            "" => r.pointer("/primary_artist/name").and_then(|x| x.as_str()).unwrap_or(""),
            a => a,
        };
        let url = str_of("url");
        if url.is_empty() {
            continue;
        }
        let Some(s) = match_score(t, title, artist, None) else { continue };
        if best.as_ref().is_none_or(|(b, _)| s > *b) {
            let instrumental = r.get("instrumental").and_then(|x| x.as_bool()).unwrap_or(false);
            best = Some((s, GeniusHit { url: url.to_string(), instrumental }));
        }
    }
    best.map(|(_, h)| h)
}

fn combine(lrc: Option<LrcRecord>, genius: GeniusResult) -> Option<Lyrics> {
    let nonempty = |s: Option<String>| s.map(|x| x.trim().to_string()).filter(|x| !x.is_empty());
    let synced = lrc.as_ref().and_then(|r| r.synced_lyrics.as_deref()).map(parse_lrc).filter(|v| v.iter().any(|l| !l.text.is_empty()));
    let (plain, plain_source) = match nonempty(genius.text) {
        Some(g) => (Some(g), Some("genius".to_string())),
        None => match nonempty(lrc.as_ref().and_then(|r| r.plain_lyrics.clone())) {
            Some(p) => (Some(p), Some("lrclib".to_string())),
            None => (None, None),
        },
    };
    let instrumental = synced.is_none() && plain.is_none() && (genius.instrumental || lrc.as_ref().is_some_and(|r| r.instrumental));
    let out = Lyrics { synced, plain, plain_source, genius_url: genius.url, instrumental };
    (out.synced.is_some() || out.plain.is_some() || out.instrumental).then_some(out)
}

// ---------------------------------------------------------------- LRC

/// Parse LRC text: `[mm:ss.xx] line`, several timestamps per line allowed,
/// metadata tags (`[ar:…]`) and word timings (`<mm:ss.xx>`) ignored.
pub fn parse_lrc(s: &str) -> Vec<LyricLine> {
    let mut out = Vec::new();
    for raw in s.lines() {
        let mut rest = raw.trim();
        let mut times = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            let Some(ms) = parse_timestamp(&stripped[..end]) else { break };
            times.push(ms);
            rest = &stripped[end + 1..];
        }
        if times.is_empty() {
            continue;
        }
        let text = strip_word_timings(rest).trim().to_string();
        out.extend(times.into_iter().map(|time_ms| LyricLine { time_ms, text: text.clone() }));
    }
    out.sort_by_key(|l| l.time_ms);
    out
}

fn parse_timestamp(tag: &str) -> Option<u64> {
    let (m, s) = tag.split_once(':')?;
    let minutes: u64 = m.trim().parse().ok()?;
    // "ss.xx", "ss" or the rarer "ss:xx".
    let secs: f64 = s.trim().replacen(':', ".", 1).parse().ok()?;
    if !(0.0..60.0).contains(&secs) {
        return None;
    }
    Some(minutes * 60_000 + (secs * 1000.0).round() as u64)
}

fn strip_word_timings(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        match rest[i + 1..].find('>') {
            Some(j) if parse_timestamp(&rest[i + 1..i + 1 + j]).is_some() => rest = &rest[i + 2 + j..],
            _ => {
                out.push('<');
                rest = &rest[i + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------- Genius HTML

const VOID_ELEMENTS: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"];

/// Find the end of the tag starting at `start` (a `<`), honouring quoted
/// attribute values. Returns the index just past `>`.
fn tag_end(html: &str, start: usize) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (i, &b) in html.as_bytes().iter().enumerate().skip(start + 1) {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'>' => return Some(i + 1),
            None => {}
        }
    }
    None
}

fn tag_name(tag: &str) -> String {
    tag.trim_start_matches('<').trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase()
}

/// The lyrics text of a Genius song page: every `data-lyrics-container`
/// block, `<br>` as line breaks, markup removed, and elements marked
/// `data-exclude-from-selection` (contributor header, ads) skipped.
pub fn genius_lyrics_from_html(html: &str) -> Option<String> {
    const MARKER: &str = "data-lyrics-container=\"true\"";
    let mut text = String::new();
    let mut search_from = 0;
    while let Some(rel) = html[search_from..].find(MARKER) {
        let marker_at = search_from + rel;
        let Some(open) = html[..marker_at].rfind('<') else { break };
        let Some(open_end) = tag_end(html, open) else { break };
        // Walk the container's children until its closing tag.
        let mut stack: Vec<String> = vec![tag_name(&html[open..open_end])];
        let mut excluded_at: Option<usize> = None;
        let mut pos = open_end;
        while !stack.is_empty() && pos < html.len() {
            let Some(lt) = html[pos..].find('<').map(|i| pos + i) else {
                pos = html.len();
                break;
            };
            if excluded_at.is_none() {
                text.push_str(&html[pos..lt]);
            }
            let Some(end) = tag_end(html, lt) else {
                pos = html.len();
                break;
            };
            let tag = &html[lt..end];
            pos = end;
            if tag.starts_with("<!") {
                continue; // comment / doctype
            }
            let name = tag_name(tag);
            if tag.starts_with("</") {
                if let Some(i) = stack.iter().rposition(|n| *n == name) {
                    stack.truncate(i);
                    if excluded_at.is_some_and(|d| stack.len() <= d) {
                        excluded_at = None;
                    }
                }
            } else if name == "br" {
                if excluded_at.is_none() {
                    text.push('\n');
                }
            } else if !VOID_ELEMENTS.contains(&name.as_str()) && !tag.ends_with("/>") {
                if excluded_at.is_none() && tag.contains("data-exclude-from-selection=\"true\"") {
                    excluded_at = Some(stack.len());
                }
                stack.push(name);
            }
        }
        text.push('\n');
        search_from = pos.max(marker_at + MARKER.len());
    }
    let decoded = decode_entities(&text);
    // Trim lines, at most one blank line in a row, none at the ends.
    let mut out: Vec<&str> = Vec::new();
    for line in decoded.lines().map(str::trim) {
        if line.is_empty() && out.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        out.push(line);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    (!out.is_empty()).then(|| out.join("\n"))
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let decoded = rest.find(';').filter(|&j| j <= 10).and_then(|j| {
            let ent = &rest[1..j];
            let c = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => {
                    let n = if let Some(h) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
                        u32::from_str_radix(h, 16).ok()
                    } else {
                        ent.strip_prefix('#').and_then(|d| d.parse().ok())
                    };
                    n.and_then(char::from_u32)
                }
            };
            c.map(|c| (c, j))
        });
        match decoded {
            Some((c, j)) => {
                out.push(c);
                rest = &rest[j + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------- cache

/// What a cached result was looked up with; a metadata edit refetches.
pub fn lookup_key(t: &Track) -> String {
    format!("v1\u{1f}{}\u{1f}{}\u{1f}{}", t.title, norm_artists(&t.artist).join(","), t.duration_ms.map(|d| d / 1000).unwrap_or(0))
}

/// `Some(result)` when a usable cache entry exists (`result` may be `None`
/// = "nothing found", still fresh).
pub fn cached(conn: &Connection, track_id: &str, key: &str) -> AppResult<Option<Option<Lyrics>>> {
    let row: Option<(String, Option<String>, String)> = conn
        .query_row("SELECT lookup_key, payload, fetched_at FROM lyrics_cache WHERE track_id = ?1", [track_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    let Some((k, payload, fetched_at)) = row else { return Ok(None) };
    if k != key {
        return Ok(None);
    }
    match payload {
        Some(json) => Ok(serde_json::from_str::<Lyrics>(&json).ok().map(Some)),
        None => {
            let fresh = chrono::DateTime::parse_from_rfc3339(&fetched_at)
                .is_ok_and(|t| chrono::Utc::now().signed_duration_since(t) < MISS_TTL);
            Ok(fresh.then_some(None))
        }
    }
}

pub fn store(conn: &Connection, track_id: &str, key: &str, lyrics: Option<&Lyrics>) -> AppResult<()> {
    let payload = lyrics.map(|l| serde_json::to_string(l)).transpose().map_err(|e| AppError::Other(e.to_string()))?;
    conn.execute(
        "INSERT INTO lyrics_cache (track_id, lookup_key, payload, fetched_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(track_id) DO UPDATE SET lookup_key = excluded.lookup_key, payload = excluded.payload, fetched_at = excluded.fetched_at",
        params![track_id, key, payload, crate::db::now_iso()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Availability, Source};

    fn track(title: &str, artist: &str, duration_ms: Option<u64>) -> Track {
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
    fn parses_lrc() {
        let lrc = "[ar:Someone]\n[00:12.34]First line\n[00:05.00][01:00.50]Repeated <00:05.10>word <00:05.40>timed\n[00:20.00]\nnot a lyric\n[03:01:20]Colon fraction";
        let lines = parse_lrc(lrc);
        let got: Vec<(u64, &str)> = lines.iter().map(|l| (l.time_ms, l.text.as_str())).collect();
        assert_eq!(
            got,
            vec![(5_000, "Repeated word timed"), (12_340, "First line"), (20_000, ""), (60_500, "Repeated word timed"), (181_200, "Colon fraction")]
        );
    }

    #[test]
    fn extracts_genius_lyrics() {
        let html = r#"<html><body><div class="x">Not lyrics</div>
            <div data-lyrics-container="true" class="Lyrics__Container"><div data-exclude-from-selection="true" class="Header"><span>164 Contributors</span><div>Karma Police Lyrics</div></div>[Verse 1]<br/>Karma police<br/><a href="/1" class="Ref"><span class="Hl">Arrest this man<br>He talks in maths</span></a><br/><br/>[Chorus]<br/>This is what you get &amp; it&#x27;s &quot;fine&quot;</div>
            <div data-exclude-from-selection="true"><div id="ad">AD TEXT</div></div>
            <div data-lyrics-container="true" class="Lyrics__Container">[Outro]<br/><i>For a minute there</i><br/>I lost myself</div>
            <div>Footer</div></body></html>"#;
        let text = genius_lyrics_from_html(html).unwrap();
        assert_eq!(
            text,
            "[Verse 1]\nKarma police\nArrest this man\nHe talks in maths\n\n[Chorus]\nThis is what you get & it's \"fine\"\n[Outro]\nFor a minute there\nI lost myself"
        );
        assert_eq!(genius_lyrics_from_html("<div>no lyrics here</div>"), None);
    }

    #[test]
    fn primary_artist_keeps_case() {
        assert_eq!(primary_artist("Simon & Garfunkel"), "Simon");
        assert_eq!(primary_artist("Drake feat. Rihanna"), "Drake");
        assert_eq!(primary_artist("Radiohead"), "Radiohead");
    }

    #[test]
    fn prefers_synced_matching_record() {
        let t = track("Karma Police", "Radiohead", Some(264_000));
        let rec = |name: &str, artist: &str, dur: f64, synced: Option<&str>| LrcRecord {
            track_name: name.into(),
            artist_name: artist.into(),
            duration: Some(dur),
            instrumental: false,
            plain_lyrics: Some("x".into()),
            synced_lyrics: synced.map(String::from),
        };
        let best = best_lrc(
            &t,
            vec![
                rec("Karma Police", "Radiohead", 264.0, None),
                rec("Karma Police", "Radiohead", 264.5, Some("[00:01.00]a")),
                rec("Karma Police (Live)", "Radiohead", 300.0, Some("[00:01.00]live")),
                rec("Karma Police", "Some Cover Band", 264.0, Some("[00:01.00]cover")),
            ],
        )
        .unwrap();
        assert_eq!(best.synced_lyrics.as_deref(), Some("[00:01.00]a"));
    }

    #[tokio::test]
    async fn fetches_from_both_sources() {
        let mut server = mockito::Server::new_async().await;
        let base = server.url();
        server
            .mock("GET", "/api/get")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_body(r#"{"trackName":"Karma Police","artistName":"Radiohead","duration":264.0,"instrumental":false,"plainLyrics":"Karma police","syncedLyrics":"[00:10.00]Karma police\n[00:14.50]Arrest this man"}"#)
            .create_async()
            .await;
        let search_body = format!(
            r#"{{"response":{{"sections":[{{"type":"song","hits":[{{"result":{{"title":"Karma Police (Translation)","artist_names":"Genius Translations","url":"{base}/translation"}}}},{{"result":{{"title":"Karma Police","artist_names":"Radiohead","url":"{base}/Radiohead-karma-police-lyrics","instrumental":false}}}}]}}]}}}}"#
        );
        server.mock("GET", "/api/search/song").match_query(mockito::Matcher::Any).with_status(200).with_body(search_body).create_async().await;
        server
            .mock("GET", "/Radiohead-karma-police-lyrics")
            .with_status(200)
            .with_body(r#"<div data-lyrics-container="true">[Verse 1]<br/>Karma police<br/>Arrest this man</div>"#)
            .create_async()
            .await;

        let client = LyricsClient::with_bases(http::client(), &base, &base);
        let got = client.fetch(&track("Karma Police", "Radiohead", Some(264_000))).await.unwrap();
        assert!(got.complete);
        let l = got.lyrics.unwrap();
        assert_eq!(l.synced.as_ref().map(|s| s.len()), Some(2));
        assert_eq!(l.synced.as_ref().unwrap()[1], LyricLine { time_ms: 14_500, text: "Arrest this man".into() });
        assert_eq!(l.plain.as_deref(), Some("[Verse 1]\nKarma police\nArrest this man"));
        assert_eq!(l.plain_source.as_deref(), Some("genius"));
        assert_eq!(l.genius_url, Some(format!("{base}/Radiohead-karma-police-lyrics")));
    }

    #[tokio::test]
    async fn nothing_found_is_complete_none_and_failure_is_error() {
        let mut server = mockito::Server::new_async().await;
        let base = server.url();
        server.mock("GET", "/api/get").match_query(mockito::Matcher::Any).with_status(404).create_async().await;
        server.mock("GET", "/api/search").match_query(mockito::Matcher::Any).with_status(200).with_body("[]").create_async().await;
        let genius = server
            .mock("GET", "/api/search/song")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_body(r#"{"response":{"sections":[{"hits":[]}]}}"#)
            .create_async()
            .await;
        let client = LyricsClient::with_bases(http::client(), &base, &base);
        let t = track("Unknown Song", "Nobody", Some(100_000));
        let got = client.fetch(&t).await.unwrap();
        assert!(got.complete && got.lyrics.is_none());

        genius.remove_async().await;
        server.mock("GET", "/api/search/song").match_query(mockito::Matcher::Any).with_status(503).create_async().await;
        assert!(client.fetch(&t).await.is_err(), "a failed source must not look like 'no lyrics'");
    }

    #[test]
    fn cache_roundtrip_and_key_change() {
        let conn = crate::db::open_in_memory().unwrap();
        conn.execute("INSERT INTO tracks (id, source, provider_id, title, added_at, updated_at) VALUES ('t1', 'spotify', 'x', 'T', 't', 't')", []).unwrap();
        let t = track("T", "A", Some(1000));
        let key = lookup_key(&t);
        assert_eq!(cached(&conn, "t1", &key).unwrap(), None);
        store(&conn, "t1", &key, None).unwrap();
        assert_eq!(cached(&conn, "t1", &key).unwrap(), Some(None), "a fresh miss is cached");
        let l = Lyrics { plain: Some("la".into()), plain_source: Some("genius".into()), ..Default::default() };
        store(&conn, "t1", &key, Some(&l)).unwrap();
        assert_eq!(cached(&conn, "t1", &key).unwrap(), Some(Some(l)));
        assert_eq!(cached(&conn, "t1", &lookup_key(&track("T2", "A", Some(1000)))).unwrap(), None, "edited metadata refetches");
    }
}

