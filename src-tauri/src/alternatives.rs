//! Matching for "Find alternative": which local / SoundCloud tracks are the
//! same song as an unavailable Spotify track. Pure functions, no I/O.
//!
//! Titles and artists are normalised (case, punctuation, spacing, featured
//! artists, "(Official Video)"-style noise) before comparing. A candidate
//! must match the title *and* the artist; exact matches rank first. A shared
//! word is never enough.

use crate::model::Track;
use serde::Serialize;
use std::collections::HashSet;

/// Words that describe an upload or edition, not the song.
const NOISE: &[&str] = &[
    "official", "audio", "video", "music", "lyrics", "lyric", "visualizer", "visualiser", "hd", "hq", "4k", "explicit", "clean", "remastered",
    "remaster", "version", "mv",
];

fn strip_brackets_with(s: &str, keep: impl Fn(&str) -> bool) -> String {
    // Drop (...) / [...] groups whose content is noise or a featured credit.
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '(' || c == '[' {
            let close = if c == '(' { ')' } else { ']' };
            let mut inner = String::new();
            let mut closed = false;
            for d in chars.by_ref() {
                if d == close {
                    closed = true;
                    break;
                }
                inner.push(d);
            }
            if closed && keep(&inner) {
                out.push(' ');
                out.push_str(&inner);
                out.push(' ');
            } else if !closed {
                out.push_str(&inner);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn is_feat_word(w: &str) -> bool {
    matches!(w, "feat" | "feat." | "ft" | "ft." | "featuring" | "with")
}

/// Lowercase alphanumeric words, `&` → `and`.
fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .replace('&', " and ")
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '\'' { c } else { ' ' })
        .collect::<String>()
        .replace('\'', "")
        .split_whitespace()
        .map(String::from)
        .collect()
}

/// Normalised song title: no featured credits, no noise groups, no
/// " - Remastered 2011"-style suffixes.
pub fn norm_title(title: &str) -> String {
    let t = strip_brackets_with(title, |inner| {
        let w = words(inner);
        !(w.is_empty() || w.first().is_some_and(|f| is_feat_word(f)) || w.iter().all(|x| NOISE.contains(&x.as_str()) || x.chars().all(|c| c.is_ascii_digit())))
    });
    // "Song - Remastered 2011" / "Song - Radio Edit": drop a trailing noise segment.
    let t = match t.rsplit_once(" - ") {
        Some((head, tail)) if words(tail).iter().any(|w| NOISE.contains(&w.as_str()) || w == "edit" || w == "mix") && !words(head).is_empty() => {
            head.to_string()
        }
        _ => t,
    };
    let mut out = Vec::new();
    for w in words(&t) {
        if is_feat_word(&w) {
            break; // "Song feat. X"
        }
        out.push(w);
    }
    out.join(" ")
}

/// Individual normalised artist names from a credit like "A, B & C feat. D".
pub fn norm_artists(credit: &str) -> Vec<String> {
    let lowered = credit.to_lowercase();
    let mut parts = vec![lowered];
    for sep in [",", " & ", " x ", " and ", " feat. ", " feat ", " ft. ", " ft ", " featuring ", " with ", ";", "/"] {
        parts = parts.iter().flat_map(|p| p.split(sep).map(String::from).collect::<Vec<_>>()).collect();
    }
    parts.into_iter().map(|p| words(&p).join(" ")).filter(|p| !p.is_empty()).collect()
}

fn contains_phrase(hay: &str, needle: &str) -> bool {
    !needle.is_empty() && format!(" {hay} ").contains(&format!(" {needle} "))
}

fn jaccard(a: &str, b: &str) -> f64 {
    let sa: HashSet<&str> = a.split(' ').filter(|w| !w.is_empty()).collect();
    let sb: HashSet<&str> = b.split(' ').filter(|w| !w.is_empty()).collect();
    if sa.is_empty() || sb.is_empty() {
        return 0.0;
    }
    sa.intersection(&sb).count() as f64 / sa.union(&sb).count() as f64
}

/// Score a candidate for an original. `None` = not the same song.
///
/// | title        | artist                        | score |
/// |--------------|-------------------------------|-------|
/// | exact        | exact primary artist          | 100   |
/// | exact        | an original artist credited   | 80    |
/// | close        | exact primary artist          | 60    |
/// | close        | an original artist credited   | 50    |
/// | exact        | candidate has no artist (local untagged) | 30 |
///
/// "Close" = the same words in a slightly different form (≥ 75% overlap,
/// at least two words). Matching durations add a small bonus.
pub fn score(orig_title: &str, orig_artist: &str, orig_duration_ms: Option<u64>, cand: &Candidate) -> Option<u32> {
    let ot = norm_title(orig_title);
    if ot.is_empty() {
        return None;
    }
    let oartists = norm_artists(orig_artist);
    let primary = oartists.first().cloned().unwrap_or_default();

    // A SoundCloud upload often carries "Artist - Title" in its title.
    let mut readings: Vec<(String, String)> = vec![(cand.title.clone(), cand.artist.clone())];
    if let Some((a, t)) = cand.title.split_once(" - ") {
        readings.push((t.to_string(), a.to_string()));
    }
    let mut best: Option<u32> = None;
    for (title, artist) in readings {
        let ct = norm_title(&title);
        if ct.is_empty() {
            continue;
        }
        let title_exact = ct == ot;
        let title_close = !title_exact && ot.split(' ').count() >= 2 && jaccard(&ct, &ot) >= 0.75;
        if !title_exact && !title_close {
            continue;
        }
        let cartists = norm_artists(&artist);
        let credit_text = words(&format!("{} {} {}", artist, cand.uploader.as_deref().unwrap_or(""), title)).join(" ");
        let artist_exact = !primary.is_empty() && cartists.first() == Some(&primary);
        let artist_credited = oartists.iter().any(|a| cartists.contains(a) || contains_phrase(&credit_text, a));
        let s = match (title_exact, artist_exact, artist_credited) {
            (true, true, _) => 100,
            (true, false, true) => 80,
            (false, true, _) => 60,
            (false, false, true) => 50,
            (true, false, false) if cartists.is_empty() && cand.uploader.is_none() => 30,
            _ => continue,
        };
        let bonus = match (orig_duration_ms, cand.duration_ms) {
            (Some(a), Some(b)) if a.abs_diff(b) <= 3_000 => 5,
            (Some(a), Some(b)) if a.abs_diff(b) <= 10_000 => 2,
            _ => 0,
        };
        best = best.max(Some(s + bonus));
    }
    best
}

/// What `score` needs from a track.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub title: String,
    pub artist: String,
    pub uploader: Option<String>,
    pub duration_ms: Option<u64>,
}

impl From<&Track> for Candidate {
    fn from(t: &Track) -> Self {
        Candidate { title: t.title.clone(), artist: t.artist.clone(), uploader: t.uploader.clone(), duration_ms: t.duration_ms }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Ranked {
    pub track: Track,
    pub score: u32,
}

/// Keep matching candidates, drop duplicates (same id, or the same
/// title/artist/length), best first.
pub fn rank(original: &Track, candidates: Vec<Track>) -> Vec<Ranked> {
    let mut seen_ids = HashSet::new();
    let mut seen_keys = HashSet::new();
    let mut out: Vec<Ranked> = Vec::new();
    for t in candidates {
        if t.id == original.id || !seen_ids.insert(t.id.clone()) {
            continue;
        }
        if t.availability == crate::model::Availability::Unavailable {
            continue;
        }
        let Some(s) = score(&original.title, &original.artist, original.duration_ms, &Candidate::from(&t)) else { continue };
        let key = (norm_title(&t.title), norm_artists(&t.artist).join(","), t.uploader.clone(), t.duration_ms.map(|d| d / 2000));
        if !seen_keys.insert(key) {
            continue;
        }
        out.push(Ranked { track: t, score: s });
    }
    out.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.track.title.cmp(&b.track.title)));
    out
}

/// Search text for providers: "primary-artist title" without credits/noise.
pub fn search_query(t: &Track) -> String {
    let artist = norm_artists(&t.artist).into_iter().next().unwrap_or_default();
    format!("{artist} {}", norm_title(&t.title)).trim().to_string()
}

/// A plain, raw-ish title fragment for a SQL LIKE search of the local library.
pub fn title_fragment(title: &str) -> String {
    let cut = title.split(['(', '[']).next().unwrap_or(title);
    let cut = cut.split(" - ").next().unwrap_or(cut);
    let cut = cut.split(" feat").next().unwrap_or(cut);
    cut.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(title: &str, artist: &str) -> Candidate {
        Candidate { title: title.into(), artist: artist.into(), uploader: None, duration_ms: None }
    }

    #[test]
    fn normalises_harmless_differences() {
        assert_eq!(norm_title("Don't Stop Me Now - Remastered 2011"), "dont stop me now");
        assert_eq!(norm_title("Song Name (feat. Someone)"), "song name");
        assert_eq!(norm_title("Song Name [Official Video]"), "song name");
        assert_eq!(norm_title("SONG   name!!"), "song name");
        assert_eq!(norm_title("Song Name feat. X"), "song name");
        assert_eq!(norm_title("Song (Live at Wembley)"), "song live at wembley", "real version info is kept");
        assert_eq!(norm_artists("Queen, David Bowie"), vec!["queen", "david bowie"]);
        assert_eq!(norm_artists("A & B feat. C"), vec!["a", "b", "c"]);
    }

    #[test]
    fn exact_title_and_artist_rank_first() {
        let exact = score("Under Pressure", "Queen, David Bowie", None, &c("Under Pressure", "Queen")).unwrap();
        let credited = score("Under Pressure", "Queen, David Bowie", None, &c("Under Pressure", "David Bowie")).unwrap();
        let close = score("Under Pressure Remix Edit", "Queen", None, &c("Under Pressure Remix", "Queen")).unwrap();
        assert!(exact > credited && credited > close, "{exact} {credited} {close}");
        assert_eq!(score("Song", "Artist", None, &c("song", "ARTIST")), Some(100));
    }

    #[test]
    fn unrelated_results_are_filtered() {
        // Shares a word only.
        assert_eq!(score("Love Story", "Taylor Swift", None, &c("Love Me Do", "The Beatles")), None);
        // Same title, unrelated artist.
        assert_eq!(score("Hello", "Adele", None, &c("Hello", "Lionel Richie")), None);
        // Artist matches, different song.
        assert_eq!(score("Hello", "Adele", None, &c("Skyfall", "Adele")), None);
    }

    #[test]
    fn soundcloud_style_uploads_match() {
        // "Artist - Title" in the title, uploader as artist.
        let up = Candidate { title: "Adele - Hello (Official Audio)".into(), artist: "musicfan99".into(), uploader: Some("musicfan99".into()), duration_ms: None };
        assert_eq!(score("Hello", "Adele", None, &up), Some(100));
        // Artist only appears as uploader.
        let up = Candidate { title: "Hello".into(), artist: "".into(), uploader: Some("Adele".into()), duration_ms: None };
        assert!(score("Hello", "Adele", None, &up).unwrap() >= 80);
    }

    #[test]
    fn duration_breaks_ties_and_queries_are_clean() {
        let a = Candidate { duration_ms: Some(200_000), ..c("Song", "Artist") };
        let b = Candidate { duration_ms: Some(260_000), ..c("Song", "Artist") };
        assert!(score("Song", "Artist", Some(201_000), &a) > score("Song", "Artist", Some(201_000), &b));
        assert_eq!(title_fragment("Song Name (feat. X) - Remastered"), "Song Name");
    }
}
