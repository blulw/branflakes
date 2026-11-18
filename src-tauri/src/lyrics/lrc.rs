//! LRC parsing, shared by every timed source.

use super::LyricLine;

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
        out.extend(times.into_iter().map(|time_ms| LyricLine { time_ms, end_ms: None, text: text.clone(), header: false }));
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

/// Drop credit lines that some sources put in the lyrics, such as NetEase's
/// "作词 : …" (lyricist) / "作曲 : …" (composer): a short label in a
/// non-Latin script followed by a colon.
pub fn drop_credit_lines(lines: Vec<LyricLine>) -> Vec<LyricLine> {
    lines
        .into_iter()
        .filter(|l| {
            let Some((label, _)) = l.text.split_once([':', '：']) else { return true };
            let label = label.trim();
            !(!label.is_empty() && label.chars().count() <= 8 && label.chars().all(|c| !c.is_ascii() || c.is_whitespace()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn drops_credit_lines() {
        let lines = parse_lrc("[00:00.000] 作词 : Yorke\n[00:01.000] 作曲 : Yorke\n[00:27.000]Karma police: arrest this man\n[00:30.000]Time: 3am");
        let texts: Vec<String> = drop_credit_lines(lines).into_iter().map(|l| l.text).collect();
        assert_eq!(texts, vec!["Karma police: arrest this man", "Time: 3am"]);
    }
}
