//! Genius: the lyrics text (with `[Verse]`/`[Chorus]` headers).
//!
//! The song search works without an API key. The lyrics themselves are only
//! in the song page's HTML (`data-lyrics-container` blocks), so they are
//! extracted from there. Genius has no timestamps.

use super::{match_score, net, LyricsClient};
use crate::alternatives::search_query;
use crate::error::AppResult;
use crate::http;
use crate::model::Track;

#[derive(Debug, Default)]
pub(super) struct GeniusResult {
    pub text: Option<String>,
    pub url: Option<String>,
    pub instrumental: bool,
}

pub(super) async fn lookup(c: &LyricsClient, t: &Track) -> AppResult<GeniusResult> {
    let query = search_query(t);
    if query.is_empty() {
        return Ok(GeniusResult::default());
    }
    let url = format!("{}/api/search/song", c.bases.genius);
    let q = [("q", query), ("per_page", "10".to_string())];
    let resp = http::send("genius", c.policy, || c.http.get(&url).query(&q)).await?;
    let v: serde_json::Value = resp.json().await.map_err(|e| net("genius", &e))?;
    let Some(hit) = best_hit(t, &v) else { return Ok(GeniusResult::default()) };
    if hit.instrumental {
        return Ok(GeniusResult { instrumental: true, url: Some(hit.url), text: None });
    }
    // Only ever fetch pages on Genius itself.
    if !hit.url.starts_with(&format!("{}/", c.bases.genius)) {
        return Ok(GeniusResult::default());
    }
    let html = http::send("genius", c.policy, || c.http.get(&hit.url)).await?.text().await.map_err(|e| net("genius", &e))?;
    Ok(GeniusResult { text: lyrics_from_html(&html), url: Some(hit.url), instrumental: false })
}

struct Hit {
    url: String,
    instrumental: bool,
}

fn best_hit(t: &Track, v: &serde_json::Value) -> Option<Hit> {
    let hits = v.pointer("/response/sections").and_then(|s| s.as_array())?.iter().flat_map(|s| s.get("hits").and_then(|h| h.as_array()).into_iter().flatten());
    let mut best: Option<(u32, Hit)> = None;
    for h in hits {
        let r = h.get("result").unwrap_or(h);
        let str_of = |k: &str| r.get(k).and_then(|x| x.as_str()).unwrap_or("");
        let artist = match str_of("artist_names") {
            "" => r.pointer("/primary_artist/name").and_then(|x| x.as_str()).unwrap_or(""),
            a => a,
        };
        let url = str_of("url");
        if url.is_empty() {
            continue;
        }
        let Some(s) = match_score(t, str_of("title"), artist, None) else { continue };
        if best.as_ref().is_none_or(|(b, _)| s > *b) {
            let instrumental = r.get("instrumental").and_then(|x| x.as_bool()).unwrap_or(false);
            best = Some((s, Hit { url: url.to_string(), instrumental }));
        }
    }
    best.map(|(_, h)| h)
}

// ---------------------------------------------------------------- HTML

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
pub fn lyrics_from_html(html: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_lyrics() {
        let html = r#"<html><body><div class="x">Not lyrics</div>
            <div data-lyrics-container="true" class="Lyrics__Container"><div data-exclude-from-selection="true" class="Header"><span>164 Contributors</span><div>Karma Police Lyrics</div></div>[Verse 1]<br/>Karma police<br/><a href="/1" class="Ref"><span class="Hl">Arrest this man<br>He talks in maths</span></a><br/><br/>[Chorus]<br/>This is what you get &amp; it&#x27;s &quot;fine&quot;</div>
            <div data-exclude-from-selection="true"><div id="ad">AD TEXT</div></div>
            <div data-lyrics-container="true" class="Lyrics__Container">[Outro]<br/><i>For a minute there</i><br/>I lost myself</div>
            <div>Footer</div></body></html>"#;
        let text = lyrics_from_html(html).unwrap();
        assert_eq!(
            text,
            "[Verse 1]\nKarma police\nArrest this man\nHe talks in maths\n\n[Chorus]\nThis is what you get & it's \"fine\"\n[Outro]\nFor a minute there\nI lost myself"
        );
        assert_eq!(lyrics_from_html("<div>no lyrics here</div>"), None);
    }
}
