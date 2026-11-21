//! Followed Spotify artists: gather every track they released and flag the
//! ones that appeared since the first sync as new.
//!
//! A sync reads the artist's albums and singles, and only fetches the tracks
//! of releases it hasn't read before, so a daily sync costs a few requests.
//! Each release is stored atomically, so an interrupted sync just continues.

use crate::db::{artists, settings};
use crate::error::AppResult;
use crate::providers::spotify::SpotifyAdapter;
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::Serialize;
use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

/// Automatic artist sync: every 24 hours unless changed in the app.
pub const DEFAULT_INTERVAL_SECS: u64 = 24 * 3600;
pub const MIN_INTERVAL_SECS: u64 = 3600;
/// After a failed automatic sync, wait this long before trying again.
pub const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArtistSyncReport {
    pub artist_id: String,
    /// Tracks added by this sync (flagged new unless it was the first sync).
    pub added: u32,
    pub new_tracks: u32,
    pub total_tracks: u32,
    pub first_sync: bool,
}

/// Event sent to the UI as `artist-sync`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistSyncEvent {
    pub artist_id: String,
    /// "syncing", "ok" or "error".
    pub status: String,
    pub new_tracks: u32,
    pub message: Option<String>,
}

#[derive(Default)]
pub struct ArtistSyncEngine {
    /// One artist sync at a time (keeps request bursts in check).
    pub run_lock: tokio::sync::Mutex<()>,
    pub wake: tokio::sync::Notify,
    in_flight: Mutex<HashSet<String>>,
}

pub struct InFlight<'a> {
    engine: &'a ArtistSyncEngine,
    id: String,
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.engine.in_flight.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.id);
    }
}

impl ArtistSyncEngine {
    /// Mark an artist as syncing; `None` if a sync of it is already running.
    pub fn begin(&self, id: &str) -> Option<InFlight<'_>> {
        let mut set = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        set.insert(id.to_string()).then(|| InFlight { engine: self, id: id.to_string() })
    }

    pub fn syncing(&self) -> Vec<String> {
        self.in_flight.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect()
    }
}

pub fn interval(conn: &Connection) -> Duration {
    let secs = settings::get_nonempty(conn, settings::ARTIST_SYNC_INTERVAL)
        .ok()
        .flatten()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_INTERVAL_SECS)
        .max(MIN_INTERVAL_SECS);
    Duration::from_secs(secs)
}

fn parse(ts: &Option<String>) -> Option<DateTime<Utc>> {
    ts.as_deref().and_then(|t| DateTime::parse_from_rfc3339(t).ok()).map(|t| t.with_timezone(&Utc))
}

/// Should the scheduler sync this artist now?
pub fn is_due(a: &artists::FollowedArtist, now: DateTime<Utc>, interval: Duration) -> bool {
    if !a.auto_sync {
        return false;
    }
    let since = |t: DateTime<Utc>| (now - t).to_std().unwrap_or(Duration::ZERO);
    let synced_due = parse(&a.last_synced_at).is_none_or(|t| since(t) >= interval);
    let attempt_ok = parse(&a.last_attempt_at).is_none_or(|t| since(t) >= RETRY_AFTER_FAILURE.min(interval));
    synced_due && attempt_ok
}

fn lock(db: &Mutex<Connection>) -> std::sync::MutexGuard<'_, Connection> {
    db.lock().unwrap_or_else(|e| e.into_inner())
}

/// One sync of one artist. Records the outcome on the artist either way.
pub async fn sync_artist(db: &Mutex<Connection>, sp: &SpotifyAdapter, client_id: &str, artist_id: &str) -> AppResult<ArtistSyncReport> {
    let result = sync_inner(db, sp, client_id, artist_id).await;
    let err = result.as_ref().err().map(|e| e.to_string());
    artists::record_attempt(&lock(db), artist_id, err.as_deref())?;
    let mut report = result?;
    let a = artists::get(&lock(db), artist_id)?;
    report.new_tracks = a.new_count;
    report.total_tracks = a.track_count;
    Ok(report)
}

async fn sync_inner(db: &Mutex<Connection>, sp: &SpotifyAdapter, client_id: &str, artist_id: &str) -> AppResult<ArtistSyncReport> {
    let (first_sync, known) = {
        let conn = lock(db);
        let a = artists::get(&conn, artist_id)?;
        (a.last_synced_at.is_none(), artists::known_releases(&conn, artist_id)?)
    };
    let profile = sp.artist(client_id, artist_id).await?;
    artists::update_profile(&lock(db), &profile)?;

    let mut releases = sp.artist_releases(client_id, artist_id).await?;
    // Oldest first, so a song's original release is the one that's kept.
    releases.sort_by(|a, b| a.release_date.cmp(&b.release_date).then(a.id.cmp(&b.id)));
    let mut added = 0;
    for r in releases.iter().filter(|r| !known.contains(&r.id)) {
        let tracks = sp.release_tracks(client_id, &r.id, artist_id).await?;
        added += artists::add_release(&lock(db), artist_id, r, &tracks, !first_sync)?;
    }
    Ok(ArtistSyncReport { artist_id: artist_id.to_string(), added, first_sync, ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::providers::spotify::SpotifyArtist;

    #[test]
    fn due_rules() {
        let conn = open_in_memory().unwrap();
        let mut a = artists::follow(&conn, &SpotifyArtist { id: "a".into(), name: "A".into(), image_url: None, genres: vec![] }).unwrap();
        let now = Utc::now();
        let day = Duration::from_secs(DEFAULT_INTERVAL_SECS);
        assert!(is_due(&a, now, day), "never synced");
        a.last_synced_at = Some((now - chrono::Duration::hours(3)).to_rfc3339());
        a.last_attempt_at = a.last_synced_at.clone();
        assert!(!is_due(&a, now, day));
        a.last_synced_at = Some((now - chrono::Duration::hours(25)).to_rfc3339());
        assert!(is_due(&a, now, day));
        // A recent failed attempt waits before retrying.
        a.last_attempt_at = Some((now - chrono::Duration::minutes(5)).to_rfc3339());
        assert!(!is_due(&a, now, day));
        a.last_attempt_at = Some((now - chrono::Duration::minutes(31)).to_rfc3339());
        assert!(is_due(&a, now, day));
        a.auto_sync = false;
        assert!(!is_due(&a, now, day));
    }

    #[test]
    fn default_interval_is_a_day() {
        let conn = open_in_memory().unwrap();
        assert_eq!(interval(&conn), Duration::from_secs(86_400));
        settings::set(&conn, settings::ARTIST_SYNC_INTERVAL, "60").unwrap();
        assert_eq!(interval(&conn), Duration::from_secs(MIN_INTERVAL_SECS));
    }
}
