//! Put another source's timing onto Genius's lines.
//!
//! Genius and the timed sources rarely break lines the same way ("Karma
//! police, arrest this man" vs "Karma police" / "Arrest this man"), so the
//! alignment works on words: both texts become word sequences, each timed
//! word gets a time (its line's start, spread across the line), and a longest
//! common subsequence pairs them up. A Genius line then starts at its first
//! matched word and ends where that word's last matched line ends. Lines with
//! no matched word (ad-libs) get times in between their neighbours.
//!
//! Too little overlap means the timing belongs to different words (another
//! version, a translation, scrambled text), and the alignment is refused.

use super::LyricLine;

/// Share of Genius's words that must be found in the timed text.
const MIN_GENIUS_COVERAGE: f64 = 0.6;
/// Share of the timed text's words that must be found in Genius.
const MIN_SOURCE_COVERAGE: f64 = 0.4;
/// A timed line is assumed to last until the next one, but no longer than this.
const MAX_LINE_SPAN_MS: u64 = 12_000;
/// Keeps the comparison table (n × m) to a few megabytes.
const MAX_CELLS: usize = 4_000_000;

/// Words for matching: lowercase letters/digits, apostrophes dropped,
/// "nothin'" = "nothing".
fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .replace('&', " and ")
        .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '\u{2019}'))
        .map(|w| {
            let mut w: String = w.chars().filter(|c| c.is_alphanumeric()).collect();
            if w.len() > 3 && w.ends_with("in") {
                w.push('g');
            }
            w
        })
        .filter(|w| !w.is_empty())
        .collect()
}

fn is_header(line: &str) -> bool {
    let l = line.trim();
    l.len() > 2 && l.starts_with('[') && l.ends_with(']') && !l[1..].contains('[')
}

struct SourceWord {
    word: String,
    time: u64,
    line_end: Option<u64>,
}

/// Genius lines (headers included, blank lines dropped) timed from `timed`.
/// `None` when the texts don't match well enough.
pub fn align(genius_text: &str, timed: &[LyricLine]) -> Option<Vec<LyricLine>> {
    // Timed words.
    let mut src: Vec<SourceWord> = Vec::new();
    for (i, l) in timed.iter().enumerate() {
        let w = words(&l.text);
        if w.is_empty() {
            continue;
        }
        let next = timed.get(i + 1).map(|n| n.time_ms).filter(|&n| n > l.time_ms);
        let span = next.map(|n| n - l.time_ms).unwrap_or(4_000).min(MAX_LINE_SPAN_MS);
        let line_end = next.map(|n| n.min(l.time_ms + MAX_LINE_SPAN_MS));
        let count = w.len() as u64;
        for (k, word) in w.into_iter().enumerate() {
            src.push(SourceWord { word, time: l.time_ms + span * k as u64 / count, line_end });
        }
    }

    // Genius lines and words.
    struct GLine {
        text: String,
        header: bool,
        first_word: usize,
        word_count: usize,
    }
    let mut glines: Vec<GLine> = Vec::new();
    let mut gwords: Vec<String> = Vec::new();
    for line in genius_text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if is_header(line) {
            glines.push(GLine { text: line.to_string(), header: true, first_word: gwords.len(), word_count: 0 });
            continue;
        }
        let w = words(line);
        glines.push(GLine { text: line.to_string(), header: false, first_word: gwords.len(), word_count: w.len() });
        gwords.extend(w);
    }

    let (n, m) = (gwords.len(), src.len());
    if n == 0 || m == 0 || (n + 1) * (m + 1) > MAX_CELLS {
        return None;
    }
    // dp[i][j] = longest common subsequence of gwords[i..] and src[j..].
    let w = m + 1;
    let mut dp = vec![0u32; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i * w + j] = if gwords[i] == src[j].word { dp[(i + 1) * w + j + 1] + 1 } else { dp[(i + 1) * w + j].max(dp[i * w + j + 1]) };
        }
    }
    let matched = dp[0] as usize;
    if (matched as f64) < MIN_GENIUS_COVERAGE * n as f64 || (matched as f64) < MIN_SOURCE_COVERAGE * m as f64 {
        return None;
    }
    let mut pair: Vec<Option<usize>> = vec![None; n];
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if gwords[i] == src[j].word && dp[i * w + j] == dp[(i + 1) * w + j + 1] + 1 {
            pair[i] = Some(j);
            i += 1;
            j += 1;
        } else if dp[(i + 1) * w + j] >= dp[i * w + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }

    // Times for lyric lines from their matched words.
    let mut start: Vec<Option<u64>> = Vec::with_capacity(glines.len());
    let mut end: Vec<Option<u64>> = Vec::with_capacity(glines.len());
    for g in &glines {
        let ms: Vec<usize> = (g.first_word..g.first_word + g.word_count).filter_map(|k| pair[k]).collect();
        start.push(ms.first().map(|&k| src[k].time));
        end.push(ms.last().and_then(|&k| src[k].line_end));
    }
    // Unmatched lyric lines: spread between the neighbouring timed lines.
    let lyric_idx: Vec<usize> = (0..glines.len()).filter(|&k| !glines[k].header).collect();
    let mut p = 0;
    while p < lyric_idx.len() {
        if start[lyric_idx[p]].is_some() {
            p += 1;
            continue;
        }
        let run_start = p;
        while p < lyric_idx.len() && start[lyric_idx[p]].is_none() {
            p += 1;
        }
        let before = run_start.checked_sub(1).map(|q| lyric_idx[q]);
        let after = lyric_idx.get(p).copied();
        let lo = before.and_then(|b| start[b]);
        let hi = after.and_then(|a| start[a]);
        let run = p - run_start;
        for (r, q) in (run_start..p).enumerate() {
            let t = match (lo, hi) {
                (Some(lo), Some(hi)) => lo + (hi.saturating_sub(lo)) * (r as u64 + 1) / (run as u64 + 1),
                (Some(lo), None) => before.and_then(|b| end[b]).unwrap_or(lo) + 2_000 * r as u64,
                (None, Some(hi)) => hi,
                (None, None) => 0,
            };
            start[lyric_idx[q]] = Some(t);
        }
    }
    // Headers take the time of the line after them.
    for k in (0..glines.len()).rev() {
        if glines[k].header {
            start[k] = start.get(k + 1).copied().flatten().or_else(|| k.checked_sub(1).and_then(|b| start[b]));
        }
    }

    let mut out = Vec::with_capacity(glines.len());
    let mut last = 0u64;
    for (k, g) in glines.into_iter().enumerate() {
        let t = start[k].unwrap_or(last).max(last);
        last = t;
        let end_ms = if g.header { None } else { end[k].filter(|&e| e > t) };
        out.push(LyricLine { time_ms: t, end_ms, text: g.text, header: g.header });
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::parse_lrc;

    #[test]
    fn maps_timing_onto_genius_lines() {
        let timed = parse_lrc(
            "[00:10.00]Karma police\n[00:13.00]Arrest this man\n[00:16.00]He talks in maths\n[00:19.00]He buzzes like a fridge\n[00:22.00]\n[00:30.00]This is what you get\n[00:34.00]When you mess with us",
        );
        let genius = "[Verse 1]\nKarma police, arrest this man\nHe talks in maths, he buzzes like a fridge\n\n[Chorus]\nThis is what you get (Yeah)\nWhen you mess with us";
        let out = align(genius, &timed).unwrap();
        let got: Vec<(u64, Option<u64>, &str, bool)> = out.iter().map(|l| (l.time_ms, l.end_ms, l.text.as_str(), l.header)).collect();
        assert_eq!(
            got,
            vec![
                (10_000, None, "[Verse 1]", true),
                (10_000, Some(16_000), "Karma police, arrest this man", false),
                (16_000, Some(22_000), "He talks in maths, he buzzes like a fridge", false),
                (30_000, None, "[Chorus]", true),
                (30_000, Some(34_000), "This is what you get (Yeah)", false),
                (34_000, None, "When you mess with us", false),
            ]
        );
    }

    #[test]
    fn unmatched_lines_fall_between_neighbours() {
        let timed = parse_lrc("[00:10.00]one two three\n[00:20.00]four five six\n[00:30.00]seven eight nine\n[00:40.00]ten eleven twelve");
        let genius = "one two three\nla la la\nfour five six\nseven eight nine\nten eleven twelve";
        let out = align(genius, &timed).unwrap();
        let times: Vec<u64> = out.iter().map(|l| l.time_ms).collect();
        assert_eq!(times, vec![10_000, 15_000, 20_000, 30_000, 40_000]);
    }

    #[test]
    fn refuses_unrelated_text() {
        let timed = parse_lrc("[00:12.00]Wob gopini den\n[00:16.00]Tefe woxica fero\n[00:20.00]Gogoh vudob wiya");
        assert!(align("Karma police, arrest this man\nHe talks in maths", &timed).is_none());
    }

    #[test]
    fn dropped_g_matches() {
        assert_eq!(words("Nothin' Goin' on, rock & roll"), vec!["nothing", "going", "on", "rock", "and", "roll"]);
    }
}
