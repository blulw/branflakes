//! Persistent Last.fm scrobble cache.

use super::now_iso;
use crate::error::AppResult;
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingScrobble {
    pub id: i64,
    pub track_id: Option<String>,
    pub artist: String,
    pub track: String,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub duration_s: Option<u32>,
    pub timestamp: i64,
    pub attempts: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrobbleQueueStatus {
    pub pending: u32,
    pub failed: u32,
    pub last_error: Option<String>,
}

pub struct NewScrobble<'a> {
    pub track_id: &'a str,
    pub artist: &'a str,
    pub track: &'a str,
    pub album: Option<&'a str>,
    pub album_artist: Option<&'a str>,
    pub track_number: Option<u32>,
    pub duration_s: Option<u32>,
    pub timestamp: i64,
}

pub fn enqueue(conn: &Connection, s: &NewScrobble<'_>) -> AppResult<i64> {
    conn.execute(
        "INSERT INTO scrobble_queue (track_id, artist, track, album, album_artist, track_number, duration_s, timestamp, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![s.track_id, s.artist, s.track, s.album, s.album_artist, s.track_number, s.duration_s, s.timestamp, now_iso()],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Oldest pending scrobbles first (Last.fm requires chronological submission).
pub fn next_batch(conn: &Connection, limit: usize) -> AppResult<Vec<PendingScrobble>> {
    let mut stmt = conn.prepare(
        "SELECT id, track_id, artist, track, album, album_artist, track_number, duration_s, timestamp, attempts
         FROM scrobble_queue WHERE status = 'pending' ORDER BY timestamp, id LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit as i64], |r| {
            Ok(PendingScrobble {
                id: r.get(0)?,
                track_id: r.get(1)?,
                artist: r.get(2)?,
                track: r.get(3)?,
                album: r.get(4)?,
                album_artist: r.get(5)?,
                track_number: r.get(6)?,
                duration_s: r.get(7)?,
                timestamp: r.get(8)?,
                attempts: r.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn delete(conn: &Connection, ids: &[i64]) -> AppResult<()> {
    let tx = conn.unchecked_transaction()?;
    for id in ids {
        tx.execute("DELETE FROM scrobble_queue WHERE id = ?1", params![id])?;
    }
    tx.commit()?;
    Ok(())
}

pub fn record_retryable_failure(conn: &Connection, ids: &[i64], error: &str) -> AppResult<()> {
    let tx = conn.unchecked_transaction()?;
    for id in ids {
        tx.execute(
            "UPDATE scrobble_queue SET attempts = attempts + 1, last_error = ?2 WHERE id = ?1",
            params![id, error],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn mark_failed(conn: &Connection, ids: &[i64], error: &str) -> AppResult<()> {
    let tx = conn.unchecked_transaction()?;
    for id in ids {
        tx.execute(
            "UPDATE scrobble_queue SET status = 'failed', attempts = attempts + 1, last_error = ?2 WHERE id = ?1",
            params![id, error],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn status(conn: &Connection) -> AppResult<ScrobbleQueueStatus> {
    let (pending, failed): (u32, u32) = conn.query_row(
        "SELECT COALESCE(SUM(status = 'pending'), 0), COALESCE(SUM(status = 'failed'), 0) FROM scrobble_queue",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let last_error = conn
        .query_row(
            "SELECT last_error FROM scrobble_queue WHERE last_error IS NOT NULL ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .ok();
    Ok(ScrobbleQueueStatus { pending, failed, last_error })
}

pub fn clear_failed(conn: &Connection) -> AppResult<()> {
    conn.execute("DELETE FROM scrobble_queue WHERE status = 'failed'", [])?;
    Ok(())
}
