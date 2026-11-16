//! Persistence for Spotify-synced playlists (`spotify_playlist_links`).
//! See migration 0004 and `sync.rs` for how the base state is used.

use super::now_iso;
use crate::error::{AppError, AppResult};
use rusqlite::{params, Connection, OptionalExtension, Row};

#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub playlist_id: String,
    pub spotify_playlist_id: String,
    pub enabled: bool,
    pub owner_id: Option<String>,
    pub can_edit_details: bool,
    pub snapshot_id: Option<String>,
    pub base_name: String,
    pub base_items: Vec<String>,
    pub base_image_key: Option<String>,
    pub base_artwork_path: Option<String>,
    pub artwork_failed_path: Option<String>,
    pub status: String,
    pub last_synced_at: Option<String>,
    pub last_error: Option<String>,
    pub last_notice: Option<String>,
}

const COLS: &str = "playlist_id, spotify_playlist_id, enabled, owner_id, can_edit_details, snapshot_id, base_name, base_items,
                    base_image_key, base_artwork_path, artwork_failed_path, status, last_synced_at, last_error, last_notice";

fn from_row(r: &Row<'_>) -> rusqlite::Result<Link> {
    let items: String = r.get(7)?;
    Ok(Link {
        playlist_id: r.get(0)?,
        spotify_playlist_id: r.get(1)?,
        enabled: r.get::<_, i64>(2)? != 0,
        owner_id: r.get(3)?,
        can_edit_details: r.get::<_, i64>(4)? != 0,
        snapshot_id: r.get(5)?,
        base_name: r.get(6)?,
        // A corrupt base is treated as empty: the merge then keeps both
        // sides' items (never removes anything).
        base_items: serde_json::from_str(&items).unwrap_or_default(),
        base_image_key: r.get(8)?,
        base_artwork_path: r.get(9)?,
        artwork_failed_path: r.get(10)?,
        status: r.get(11)?,
        last_synced_at: r.get(12)?,
        last_error: r.get(13)?,
        last_notice: r.get(14)?,
    })
}

pub fn get(conn: &Connection, playlist_id: &str) -> AppResult<Option<Link>> {
    Ok(conn
        .query_row(&format!("SELECT {COLS} FROM spotify_playlist_links WHERE playlist_id = ?1"), params![playlist_id], from_row)
        .optional()?)
}

/// The enabled link for a playlist, i.e. "is this playlist Spotify Synced?".
pub fn get_enabled(conn: &Connection, playlist_id: &str) -> AppResult<Option<Link>> {
    Ok(get(conn, playlist_id)?.filter(|l| l.enabled))
}

pub fn find_by_spotify_id(conn: &Connection, spotify_playlist_id: &str) -> AppResult<Option<Link>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM spotify_playlist_links WHERE spotify_playlist_id = ?1"),
            params![spotify_playlist_id],
            from_row,
        )
        .optional()?)
}

pub fn list_enabled(conn: &Connection) -> AppResult<Vec<Link>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLS} FROM spotify_playlist_links WHERE enabled = 1 ORDER BY created_at"))?;
    let rows = stmt.query_map([], from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Insert or replace the link for a playlist (used when enabling sync).
pub fn upsert(conn: &Connection, l: &Link) -> AppResult<()> {
    let now = now_iso();
    conn.execute(
        "INSERT INTO spotify_playlist_links (playlist_id, spotify_playlist_id, enabled, owner_id, can_edit_details, snapshot_id,
             base_name, base_items, base_image_key, base_artwork_path, artwork_failed_path, status, last_synced_at, last_error,
             last_notice, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?16)
         ON CONFLICT (playlist_id) DO UPDATE SET
             spotify_playlist_id = ?2, enabled = ?3, owner_id = ?4, can_edit_details = ?5, snapshot_id = ?6, base_name = ?7,
             base_items = ?8, base_image_key = ?9, base_artwork_path = ?10, artwork_failed_path = ?11, status = ?12,
             last_synced_at = ?13, last_error = ?14, last_notice = ?15, updated_at = ?16",
        params![
            l.playlist_id,
            l.spotify_playlist_id,
            l.enabled as i64,
            l.owner_id,
            l.can_edit_details as i64,
            l.snapshot_id,
            l.base_name,
            serde_json::to_string(&l.base_items)?,
            l.base_image_key,
            l.base_artwork_path,
            l.artwork_failed_path,
            l.status,
            l.last_synced_at,
            l.last_error,
            l.last_notice,
            now,
        ],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, playlist_id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM spotify_playlist_links WHERE playlist_id = ?1", params![playlist_id])?;
    Ok(())
}

pub fn set_status(conn: &Connection, playlist_id: &str, status: &str, error: Option<&str>) -> AppResult<()> {
    let n = conn.execute(
        "UPDATE spotify_playlist_links SET status = ?2, last_error = ?3, last_attempt_at = ?4, updated_at = ?4 WHERE playlist_id = ?1",
        params![playlist_id, status, error, now_iso()],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("sync link for playlist {playlist_id}")));
    }
    Ok(())
}

// ------------------------------------------------ external removal state

/// Recorded first sighting of a missing Spotify playlist: (kind, since).
pub fn missing_state(conn: &Connection, playlist_id: &str) -> AppResult<Option<(String, String)>> {
    Ok(conn
        .query_row(
            "SELECT missing_kind, missing_since FROM spotify_playlist_links WHERE playlist_id = ?1 AND missing_kind IS NOT NULL",
            params![playlist_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

pub fn set_missing(conn: &Connection, playlist_id: &str, kind: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE spotify_playlist_links SET missing_kind = ?2, missing_since = ?3 WHERE playlist_id = ?1",
        params![playlist_id, kind, now_iso()],
    )?;
    Ok(())
}

pub fn clear_missing(conn: &Connection, playlist_id: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE spotify_playlist_links SET missing_kind = NULL, missing_since = NULL WHERE playlist_id = ?1 AND missing_kind IS NOT NULL",
        params![playlist_id],
    )?;
    Ok(())
}

// ------------------------------------------------------ pending deletion

pub fn set_pending_delete(conn: &Connection, playlist_id: &str, on: bool) -> AppResult<()> {
    conn.execute("UPDATE spotify_playlist_links SET pending_delete = ?2 WHERE playlist_id = ?1", params![playlist_id, on as i64])?;
    Ok(())
}

pub fn is_pending_delete(conn: &Connection, playlist_id: &str) -> AppResult<bool> {
    Ok(conn
        .query_row("SELECT pending_delete FROM spotify_playlist_links WHERE playlist_id = ?1", params![playlist_id], |r| r.get::<_, i64>(0))
        .optional()?
        .unwrap_or(0)
        != 0)
}

/// Deletions interrupted by a restart.
pub fn pending_deletes(conn: &Connection) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare("SELECT playlist_id FROM spotify_playlist_links WHERE pending_delete = 1")?;
    let ids = stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
    Ok(ids)
}

// ------------------------------------------------ pending Spotify creates

/// Name of a Spotify playlist creation for this playlist whose outcome is unknown.
pub fn pending_create(conn: &Connection, playlist_id: &str) -> AppResult<Option<String>> {
    Ok(conn
        .query_row("SELECT name FROM spotify_pending_creates WHERE playlist_id = ?1", params![playlist_id], |r| r.get(0))
        .optional()?)
}

pub fn set_pending_create(conn: &Connection, playlist_id: &str, name: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO spotify_pending_creates (playlist_id, name, started_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (playlist_id) DO UPDATE SET name = ?2",
        params![playlist_id, name, now_iso()],
    )?;
    Ok(())
}

pub fn clear_pending_create(conn: &Connection, playlist_id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM spotify_pending_creates WHERE playlist_id = ?1", params![playlist_id])?;
    Ok(())
}

/// After a restart, a sync that was interrupted is marked pending so the
/// scheduler picks it up immediately. Returns the affected playlist ids.
pub fn recover_interrupted(conn: &Connection) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare("SELECT playlist_id FROM spotify_playlist_links WHERE status IN ('syncing', 'pending') AND enabled = 1")?;
    let ids = stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
    conn.execute("UPDATE spotify_playlist_links SET status = 'pending' WHERE status = 'syncing'", [])?;
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{open_in_memory, playlists};

    pub fn link(playlist_id: &str, spotify_id: &str) -> Link {
        Link {
            playlist_id: playlist_id.into(),
            spotify_playlist_id: spotify_id.into(),
            enabled: true,
            owner_id: Some("me".into()),
            can_edit_details: true,
            snapshot_id: Some("s1".into()),
            base_name: "Mix".into(),
            base_items: vec!["spotify:track:a".into()],
            base_image_key: None,
            base_artwork_path: None,
            artwork_failed_path: None,
            status: "ok".into(),
            last_synced_at: None,
            last_error: None,
            last_notice: None,
        }
    }

    #[test]
    fn link_round_trip_cascade_and_recovery() {
        let conn = open_in_memory().unwrap();
        let p = playlists::create(&conn, "Mix", Some("spotify:playlist:sp1")).unwrap();
        upsert(&conn, &link(&p.id, "sp1")).unwrap();
        let l = get_enabled(&conn, &p.id).unwrap().unwrap();
        assert_eq!(l.base_items, vec!["spotify:track:a".to_string()]);
        assert_eq!(find_by_spotify_id(&conn, "sp1").unwrap().unwrap().playlist_id, p.id);
        // Visible on the playlist model as "Spotify Synced".
        assert_eq!(playlists::get(&conn, &p.id).unwrap().sync.unwrap().spotify_playlist_id, "sp1");

        // Interrupted sync is recovered as pending.
        set_status(&conn, &p.id, "syncing", None).unwrap();
        assert_eq!(recover_interrupted(&conn).unwrap(), vec![p.id.clone()]);
        assert_eq!(get(&conn, &p.id).unwrap().unwrap().status, "pending");

        // A disabled link is not "synced".
        upsert(&conn, &Link { enabled: false, ..l.clone() }).unwrap();
        assert!(get_enabled(&conn, &p.id).unwrap().is_none());
        assert!(playlists::get(&conn, &p.id).unwrap().sync.is_none());

        // Deleting the Branflakes playlist removes the link (Spotify untouched).
        playlists::delete(&conn, &p.id).unwrap();
        assert!(get(&conn, &p.id).unwrap().is_none());
    }
}
