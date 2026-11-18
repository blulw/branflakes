//! LRCLIB (lrclib.net): free, keyless, community time-synced lyrics.

use super::{length_ok, match_score, net, primary_artist, LyricsClient, SyncLookup, TimedLyrics};
use crate::alternatives::title_fragment;
use crate::error::{AppError, AppResult};
use crate::http;
use crate::lyrics::parse_lrc;
use crate::model::Track;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
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

impl Record {
    fn duration_ms(&self) -> Option<u64> {
        self.duration.filter(|d| *d > 0.0).map(|d| (d * 1000.0).round() as u64)
    }
    fn accepted(&self, t: &Track) -> Option<u32> {
        if !length_ok(t, self.duration_ms()) {
            return None;
        }
        match_score(t, &self.track_name, &self.artist_name, self.duration_ms())
    }
    fn into_lookup(self) -> SyncLookup {
        let lines = self.synced_lyrics.as_deref().map(parse_lrc).unwrap_or_default();
        let plain = self.plain_lyrics.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
        SyncLookup {
            timed: lines.iter().any(|l| !l.text.is_empty()).then(|| TimedLyrics { source: "lrclib", lines }),
            plain,
            instrumental: self.instrumental,
        }
    }
}

pub(super) async fn lookup(c: &LyricsClient, t: &Track) -> AppResult<SyncLookup> {
    let primary = primary_artist(&t.artist);
    // Exact lookup first (needs the length); it is LRCLIB's preferred API.
    if let Some(d) = t.duration_ms {
        let url = format!("{}/api/get", c.bases.lrclib);
        let q = [
            ("track_name", t.title.clone()),
            ("artist_name", primary.clone()),
            ("album_name", t.album.clone()),
            ("duration", ((d + 500) / 1000).to_string()),
        ];
        match http::send("lrclib", c.policy, || c.http.get(&url).query(&q)).await {
            Ok(resp) => {
                let rec: Record = resp.json().await.map_err(|e| net("lrclib", &e))?;
                if rec.accepted(t).is_some() && rec.synced_lyrics.as_deref().is_some_and(|s| !s.trim().is_empty()) {
                    return Ok(rec.into_lookup());
                }
            }
            Err(AppError::Http { status: 404, .. }) => {}
            Err(e) => return Err(e),
        }
    }
    // Then a search with the cleaned-up title.
    let url = format!("{}/api/search", c.bases.lrclib);
    let q = [("track_name", title_fragment(&t.title)), ("artist_name", primary)];
    let resp = match http::send("lrclib", c.policy, || c.http.get(&url).query(&q)).await {
        Ok(r) => r,
        Err(AppError::Http { status: 404, .. }) => return Ok(SyncLookup::default()),
        Err(e) => return Err(e),
    };
    let records: Vec<Record> = resp.json().await.map_err(|e| net("lrclib", &e))?;
    Ok(best(t, records).map(Record::into_lookup).unwrap_or_default())
}

fn best(t: &Track, records: Vec<Record>) -> Option<Record> {
    records
        .into_iter()
        .filter_map(|r| {
            let s = r.accepted(t)?;
            let has_synced = r.synced_lyrics.as_deref().is_some_and(|s| !s.trim().is_empty());
            let has_any = has_synced || r.plain_lyrics.as_deref().is_some_and(|s| !s.trim().is_empty()) || r.instrumental;
            has_any.then_some(((has_synced, s), r))
        })
        .max_by_key(|(k, _)| *k)
        .map(|(_, r)| r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::tests::track;

    #[test]
    fn prefers_synced_matching_record() {
        let t = track("Karma Police", "Radiohead", Some(264_000));
        let rec = |name: &str, artist: &str, dur: f64, synced: Option<&str>| Record {
            track_name: name.into(),
            artist_name: artist.into(),
            duration: Some(dur),
            instrumental: false,
            plain_lyrics: Some("x".into()),
            synced_lyrics: synced.map(String::from),
        };
        let best = best(
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
}
