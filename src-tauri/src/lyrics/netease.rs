//! NetEase Cloud Music: a very large LRC collection (strongest for Asian
//! music, but with plenty of Western releases too). Unofficial, keyless API.

use super::{length_ok, match_score, net, LyricsClient, SyncLookup, TimedLyrics};
use crate::alternatives::search_query;
use crate::error::AppResult;
use crate::http;
use crate::lyrics::{drop_credit_lines, parse_lrc};
use crate::model::Track;

/// NetEase's "pure music, please enjoy" marker for instrumentals.
const INSTRUMENTAL_MARKER: &str = "纯音乐";

pub(super) async fn lookup(c: &LyricsClient, t: &Track) -> AppResult<SyncLookup> {
    let query = search_query(t);
    if query.is_empty() {
        return Ok(SyncLookup::default());
    }
    let url = format!("{}/api/search/get", c.bases.netease);
    let q = [("s", query), ("type", "1".into()), ("limit", "10".into())];
    let v: serde_json::Value = http::send("netease", c.policy, || c.http.get(&url).query(&q)).await?.json().await.map_err(|e| net("netease", &e))?;
    let Some(id) = best_song(t, &v) else { return Ok(SyncLookup::default()) };

    let url = format!("{}/api/song/lyric", c.bases.netease);
    let q = [("id", id.to_string()), ("lv", "1".into()), ("kv", "1".into()), ("tv", "-1".into())];
    let v: serde_json::Value = http::send("netease", c.policy, || c.http.get(&url).query(&q)).await?.json().await.map_err(|e| net("netease", &e))?;
    let lrc = v.pointer("/lrc/lyric").and_then(|x| x.as_str()).unwrap_or("");
    if lrc.contains(INSTRUMENTAL_MARKER) {
        return Ok(SyncLookup { instrumental: true, ..Default::default() });
    }
    let lines = drop_credit_lines(parse_lrc(lrc));
    Ok(SyncLookup {
        timed: lines.iter().any(|l| !l.text.is_empty()).then(|| TimedLyrics { source: "netease", lines }),
        ..Default::default()
    })
}

fn best_song(t: &Track, v: &serde_json::Value) -> Option<u64> {
    let songs = v.pointer("/result/songs")?.as_array()?;
    songs
        .iter()
        .filter_map(|s| {
            let id = s.get("id")?.as_u64()?;
            let name = s.get("name")?.as_str()?;
            let artists = s
                .get("artists")
                .and_then(|a| a.as_array())
                .map(|a| a.iter().filter_map(|x| x.get("name")?.as_str()).collect::<Vec<_>>().join(", "))
                .unwrap_or_default();
            let duration = s.get("duration").and_then(|d| d.as_u64()).filter(|d| *d > 0);
            if !length_ok(t, duration) {
                return None;
            }
            Some((match_score(t, name, &artists, duration)?, id))
        })
        .max_by_key(|(s, _)| *s)
        .map(|(_, id)| id)
}
