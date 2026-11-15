//! Scrobble eligibility rules, queueing and flushing.

use crate::db::{library, scrobbles, settings};
use crate::error::{AppError, AppResult};
use crate::model::Source;
use crate::providers::lastfm::{LastFmAdapter, LastFmCredentials};
use crate::secrets::Secret;
use rusqlite::Connection;
use serde::Serialize;
use std::sync::Mutex;

pub const MIN_TRACK_MS: u64 = 30_000;
pub const MAX_REQUIRED_PLAY_MS: u64 = 240_000;
pub const BATCH_SIZE: usize = 50;

/// Last.fm rule: track longer than 30 s, and played for at least half its
/// duration or 4 minutes, whichever comes first.
pub fn check_eligible(duration_ms: Option<u64>, played_ms: u64) -> Result<(), String> {
    let duration = duration_ms.ok_or("track duration is unknown")?;
    if duration <= MIN_TRACK_MS {
        return Err("track is 30 seconds or shorter".into());
    }
    let required = (duration / 2).min(MAX_REQUIRED_PLAY_MS);
    if played_ms < required {
        return Err(format!("played {}s of the required {}s", played_ms / 1000, required / 1000));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SubmitOutcome {
    pub queued: bool,
    pub reason: Option<String>,
}

/// Source policy: local files and SoundCloud are always eligible, Spotify
/// unless the user turned it off. Returns the reason when refused.
pub fn source_allowed(conn: &Connection, source: Source) -> AppResult<Result<(), String>> {
    Ok(match source {
        Source::Local | Source::Soundcloud => Ok(()),
        Source::Spotify if settings::get_bool(conn, settings::LASTFM_SCROBBLE_SPOTIFY, true)? => Ok(()),
        Source::Spotify => Err("Spotify scrobbling is turned off".into()),
    })
}

/// Validate a finished play and add it to the persistent scrobble cache.
///
/// Enforces the source policy (see `source_allowed`) even if the UI asks.
pub fn submit_play(conn: &Connection, track_id: &str, started_at_unix: i64, played_ms: u64) -> AppResult<SubmitOutcome> {
    let track = library::get_track(conn, track_id)?;
    if let Err(reason) = source_allowed(conn, track.source)? {
        return Ok(SubmitOutcome { queued: false, reason: Some(reason) });
    }
    if !settings::get_bool(conn, settings::LASTFM_ENABLED, true)? {
        return Ok(SubmitOutcome { queued: false, reason: Some("scrobbling disabled".into()) });
    }
    let artist = library::scrobble_artist(conn, &track)?;
    if artist.trim().is_empty() {
        return Ok(SubmitOutcome { queued: false, reason: Some("track has no artist tag".into()) });
    }
    if let Err(r) = check_eligible(track.duration_ms, played_ms) {
        return Ok(SubmitOutcome { queued: false, reason: Some(r) });
    }
    let now = chrono::Utc::now().timestamp();
    if started_at_unix > now + 60 || started_at_unix < now - 14 * 24 * 3600 {
        return Err(AppError::InvalidInput("scrobble timestamp out of range".into()));
    }
    scrobbles::enqueue(
        conn,
        &scrobbles::NewScrobble {
            track_id,
            artist: &artist,
            track: &track.title,
            album: Some(&track.album).filter(|a| !a.is_empty()).map(|s| s.as_str()),
            album_artist: Some(&track.album_artist).filter(|a| !a.is_empty()).map(|s| s.as_str()),
            track_number: track.track_number,
            duration_s: track.duration_ms.map(|d| (d / 1000) as u32),
            timestamp: started_at_unix,
        },
    )?;
    Ok(SubmitOutcome { queued: true, reason: None })
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlushReport {
    pub accepted: u32,
    pub ignored: u32,
    pub failed: u32,
    pub remaining: u32,
    pub session_invalid: bool,
    pub error: Option<String>,
}

/// Send cached scrobbles oldest-first in batches of up to 50. Stops at the
/// first retryable failure so ordering is preserved.
pub async fn flush(
    db: &Mutex<Connection>,
    lastfm: &LastFmAdapter,
    creds: &LastFmCredentials,
    sk: &Secret,
) -> AppResult<FlushReport> {
    let mut report = FlushReport::default();
    loop {
        let batch = {
            let conn = db.lock().unwrap();
            scrobbles::next_batch(&conn, BATCH_SIZE)?
        };
        if batch.is_empty() {
            break;
        }
        let ids: Vec<i64> = batch.iter().map(|s| s.id).collect();
        match lastfm.scrobble(creds, sk, &batch).await {
            Ok(r) => {
                report.accepted += r.accepted;
                report.ignored += r.ignored;
                let conn = db.lock().unwrap();
                scrobbles::delete(&conn, &ids)?;
            }
            Err(e) => {
                let msg = e.to_string();
                let conn = db.lock().unwrap();
                if e.is_invalid_session() {
                    scrobbles::record_retryable_failure(&conn, &ids, &msg)?;
                    report.session_invalid = true;
                    report.error = Some(msg);
                    break;
                } else if e.is_retryable() {
                    scrobbles::record_retryable_failure(&conn, &ids, &msg)?;
                    report.error = Some(msg);
                    break;
                } else {
                    // Malformed request: Last.fm says don't retry.
                    scrobbles::mark_failed(&conn, &ids, &msg)?;
                    report.failed += ids.len() as u32;
                    report.error = Some(msg);
                }
            }
        }
    }
    let conn = db.lock().unwrap();
    report.remaining = scrobbles::status(&conn)?.pending;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::library::{tests::sample, upsert_track};
    use crate::db::open_in_memory;

    #[test]
    fn eligibility_rules() {
        assert!(check_eligible(None, 1_000_000).is_err());
        assert!(check_eligible(Some(30_000), 30_000).is_err(), "exactly 30s is too short");
        assert!(check_eligible(Some(31_000), 15_500).is_ok());
        assert!(check_eligible(Some(31_000), 15_000).is_err());
        assert!(check_eligible(Some(200_000), 100_000).is_ok());
        assert!(check_eligible(Some(200_000), 99_999).is_err());
        // Long track: 4 minutes is enough.
        assert!(check_eligible(Some(3_600_000), 240_000).is_ok());
        assert!(check_eligible(Some(3_600_000), 239_000).is_err());
    }

    #[test]
    fn scrobbles_soundcloud_with_artist_and_title() {
        let conn = open_in_memory().unwrap();
        let now = chrono::Utc::now().timestamp();
        let id = upsert_track(&conn, Source::Soundcloud, &sample("sc-1", "Song", "Some Artist", ""), true).unwrap();
        // A 30-second preview of a 200 s track is not enough.
        assert!(!submit_play(&conn, &id, now - 300, 30_000).unwrap().queued);
        let out = submit_play(&conn, &id, now - 300, 150_000).unwrap();
        assert!(out.queued, "{out:?}");
        let b = scrobbles::next_batch(&conn, 50).unwrap();
        assert_eq!((b[0].artist.as_str(), b[0].track.as_str()), ("Some Artist", "Song"));
        assert_eq!(b[0].album, None, "empty album is omitted");
    }

    #[test]
    fn scrobbles_spotify_with_primary_artist_unless_disabled() {
        let conn = open_in_memory().unwrap();
        let now = chrono::Utc::now().timestamp();
        let mut t = sample("sp-1", "Song", "A, B", "Album");
        t.primary_artist = Some("A".into());
        let id = upsert_track(&conn, Source::Spotify, &t, true).unwrap();

        let out = submit_play(&conn, &id, now - 300, 150_000).unwrap();
        assert!(out.queued, "{out:?}");
        let b = scrobbles::next_batch(&conn, 50).unwrap();
        assert_eq!(b[0].artist, "A", "only the primary artist is scrobbled");

        settings::set(&conn, settings::LASTFM_SCROBBLE_SPOTIFY, "false").unwrap();
        let out = submit_play(&conn, &id, now - 100, 150_000).unwrap();
        assert!(!out.queued);
        assert_eq!(scrobbles::status(&conn).unwrap().pending, 1);
    }

    #[test]
    fn scrobble_artist_fallbacks_and_override() {
        let conn = open_in_memory().unwrap();
        // Imported before primary_artist existed: album artist prefix is used.
        let mut t = sample("sp-2", "Song", "Tyler, The Creator, Kali Uchis", "Album");
        t.album_artist = "Tyler, The Creator".into();
        let id = upsert_track(&conn, Source::Spotify, &t, true).unwrap();
        let track = library::get_track(&conn, &id).unwrap();
        assert_eq!(library::scrobble_artist(&conn, &track).unwrap(), "Tyler, The Creator");

        // A user override always wins.
        library::set_override(&conn, &id, &library::TrackOverride { artist: Some("Someone".into()), ..Default::default() }).unwrap();
        let track = library::get_track(&conn, &id).unwrap();
        assert_eq!(library::scrobble_artist(&conn, &track).unwrap(), "Someone");
    }

    #[test]
    fn queues_eligible_local_play_with_snapshot_metadata() {
        let conn = open_in_memory().unwrap();
        let now = chrono::Utc::now().timestamp();
        let id = upsert_track(&conn, Source::Local, &sample("c:\\a.mp3", "Song", "Artist", "Album"), true).unwrap();
        assert!(!submit_play(&conn, &id, now - 10, 5_000).unwrap().queued);
        let out = submit_play(&conn, &id, now - 200, 150_000).unwrap();
        assert!(out.queued, "{out:?}");
        let b = scrobbles::next_batch(&conn, 50).unwrap();
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].artist, "Artist");
        assert_eq!(b[0].duration_s, Some(200));

        settings::set(&conn, settings::LASTFM_ENABLED, "false").unwrap();
        assert!(!submit_play(&conn, &id, now - 200, 150_000).unwrap().queued);
    }

    #[tokio::test]
    async fn flush_sends_in_order_and_handles_failures() {
        let conn = open_in_memory().unwrap();
        let now = chrono::Utc::now().timestamp();
        let id = upsert_track(&conn, Source::Local, &sample("c:\\a.mp3", "Song", "Artist", "Album"), true).unwrap();
        for i in 0..3 {
            submit_play(&conn, &id, now - 1000 + i, 150_000).unwrap();
        }
        let db = Mutex::new(conn);
        let mut server = mockito::Server::new_async().await;
        let lf = LastFmAdapter::with_base(crate::http::client(), &format!("{}/", server.url()), "x");
        let creds = LastFmCredentials { api_key: "k".into(), shared_secret: Secret::new("s") };
        let sk = Secret::new("sk");

        // Service offline: kept for retry.
        let m = server.mock("POST", "/").with_body(r#"{"error":11,"message":"Service Offline"}"#).create_async().await;
        let r = flush(&db, &lf, &creds, &sk).await.unwrap();
        assert_eq!(r.remaining, 3);
        assert!(r.error.is_some());
        m.remove_async().await;

        // Success: all three sent in one batch, oldest first.
        let m = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::UrlEncoded("timestamp[0]".into(), (now - 1000).to_string()))
            .with_body(r#"{"scrobbles":{"@attr":{"accepted":3,"ignored":0}}}"#)
            .expect(1)
            .create_async()
            .await;
        let r = flush(&db, &lf, &creds, &sk).await.unwrap();
        assert_eq!(r.accepted, 3);
        assert_eq!(r.remaining, 0);
        m.assert_async().await;
    }

    #[tokio::test]
    async fn flush_marks_malformed_as_failed() {
        let conn = open_in_memory().unwrap();
        let now = chrono::Utc::now().timestamp();
        let id = upsert_track(&conn, Source::Local, &sample("c:\\b.mp3", "Song", "Artist", "Album"), true).unwrap();
        submit_play(&conn, &id, now - 500, 150_000).unwrap();
        let db = Mutex::new(conn);
        let mut server = mockito::Server::new_async().await;
        server.mock("POST", "/").with_body(r#"{"error":6,"message":"Invalid parameters"}"#).create_async().await;
        let lf = LastFmAdapter::with_base(crate::http::client(), &format!("{}/", server.url()), "x");
        let creds = LastFmCredentials { api_key: "k".into(), shared_secret: Secret::new("s") };
        let r = flush(&db, &lf, &creds, &Secret::new("sk")).await.unwrap();
        assert_eq!(r.failed, 1);
        let st = scrobbles::status(&db.lock().unwrap()).unwrap();
        assert_eq!((st.pending, st.failed), (0, 1));
    }
}
