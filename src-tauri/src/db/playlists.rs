//! Mixed-source playlists. Items reference internal track IDs, so a playlist
//! can freely mix Local, Spotify and SoundCloud tracks.

use super::now_iso;
use crate::error::{AppError, AppResult};
use crate::model::Track;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub description: String,
    pub origin: Option<String>,
    pub track_count: u32,
    /// App-level cover image (absolute path in the artwork cache), if set.
    pub artwork_path: Option<String>,
    /// When the playlist was last started from its page (RFC 3339).
    pub last_played_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub fn list(conn: &Connection) -> AppResult<Vec<Playlist>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.name, p.description, p.origin,
                (SELECT COUNT(*) FROM playlist_items i WHERE i.playlist_id = p.id),
                p.artwork_path, p.last_played_at, p.created_at, p.updated_at
         FROM playlists p ORDER BY p.name COLLATE NOCASE",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Playlist {
                id: r.get(0)?,
                name: r.get(1)?,
                description: r.get(2)?,
                origin: r.get(3)?,
                track_count: r.get(4)?,
                artwork_path: r.get(5)?,
                last_played_at: r.get(6)?,
                created_at: r.get(7)?,
                updated_at: r.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get(conn: &Connection, id: &str) -> AppResult<Playlist> {
    list(conn)?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| AppError::NotFound(format!("playlist {id}")))
}

pub fn find_by_origin(conn: &Connection, origin: &str) -> AppResult<Option<String>> {
    Ok(conn
        .query_row("SELECT id FROM playlists WHERE origin = ?1", params![origin], |r| r.get(0))
        .optional()?)
}

pub fn create(conn: &Connection, name: &str, origin: Option<&str>) -> AppResult<Playlist> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::InvalidInput("playlist name must not be empty".into()));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_iso();
    conn.execute(
        "INSERT INTO playlists (id, name, origin, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
        params![id, name, origin, now],
    )?;
    get(conn, &id)
}

pub fn rename(conn: &Connection, id: &str, name: &str) -> AppResult<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::InvalidInput("playlist name must not be empty".into()));
    }
    let n = conn.execute(
        "UPDATE playlists SET name = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, name, now_iso()],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("playlist {id}")));
    }
    Ok(())
}

pub fn mark_played(conn: &Connection, id: &str) -> AppResult<()> {
    let n = conn.execute("UPDATE playlists SET last_played_at = ?2 WHERE id = ?1", params![id, now_iso()])?;
    if n == 0 {
        return Err(AppError::NotFound(format!("playlist {id}")));
    }
    Ok(())
}

/// Set (or clear with `None`) the playlist cover image path.
pub fn set_artwork(conn: &Connection, id: &str, artwork_path: Option<&str>) -> AppResult<()> {
    let n = conn.execute(
        "UPDATE playlists SET artwork_path = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, artwork_path, now_iso()],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("playlist {id}")));
    }
    Ok(())
}

pub fn delete(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM playlists WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn track_ids(conn: &Connection, id: &str) -> AppResult<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT track_id FROM playlist_items WHERE playlist_id = ?1 ORDER BY position")?;
    let rows = stmt
        .query_map(params![id], |r| r.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(rows)
}

pub fn tracks(conn: &Connection, id: &str) -> AppResult<Vec<Track>> {
    get(conn, id)?;
    super::library::get_tracks(conn, &track_ids(conn, id)?)
}

/// Rewrite all items for a playlist in a single transaction with dense positions.
pub fn set_items(conn: &Connection, id: &str, track_ids: &[String]) -> AppResult<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM playlist_items WHERE playlist_id = ?1", params![id])?;
    for (pos, t) in track_ids.iter().enumerate() {
        tx.execute(
            "INSERT INTO playlist_items (playlist_id, position, track_id) VALUES (?1, ?2, ?3)",
            params![id, pos as i64, t],
        )?;
    }
    tx.execute("UPDATE playlists SET updated_at = ?2 WHERE id = ?1", params![id, now_iso()])?;
    tx.commit()?;
    Ok(())
}

pub fn add_tracks(conn: &Connection, id: &str, new_ids: &[String]) -> AppResult<()> {
    get(conn, id)?;
    let mut ids = track_ids(conn, id)?;
    ids.extend(new_ids.iter().cloned());
    set_items(conn, id, &ids)
}

pub fn remove_at(conn: &Connection, id: &str, position: usize) -> AppResult<()> {
    let mut ids = track_ids(conn, id)?;
    if position >= ids.len() {
        return Err(AppError::InvalidInput(format!("position {position} out of range")));
    }
    ids.remove(position);
    set_items(conn, id, &ids)
}

pub fn move_item(conn: &Connection, id: &str, from: usize, to: usize) -> AppResult<()> {
    let mut ids = track_ids(conn, id)?;
    if from >= ids.len() || to >= ids.len() {
        return Err(AppError::InvalidInput("position out of range".into()));
    }
    let item = ids.remove(from);
    ids.insert(to, item);
    set_items(conn, id, &ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::library::{tests::sample, upsert_track};
    use crate::db::open_in_memory;
    use crate::model::Source;

    #[test]
    fn mixed_source_playlist_crud() {
        let conn = open_in_memory().unwrap();
        let a = upsert_track(&conn, Source::Local, &sample("/a", "A", "x", "y"), true).unwrap();
        let b = upsert_track(&conn, Source::Spotify, &sample("b", "B", "x", "y"), true).unwrap();
        let c = upsert_track(&conn, Source::Soundcloud, &sample("c", "C", "x", "y"), true).unwrap();
        let p = create(&conn, "Mix", None).unwrap();
        add_tracks(&conn, &p.id, &[a.clone(), b.clone(), c.clone()]).unwrap();
        let t = tracks(&conn, &p.id).unwrap();
        assert_eq!(t.iter().map(|t| t.source).collect::<Vec<_>>(), [Source::Local, Source::Spotify, Source::Soundcloud]);

        move_item(&conn, &p.id, 2, 0).unwrap();
        assert_eq!(track_ids(&conn, &p.id).unwrap(), vec![c.clone(), a.clone(), b.clone()]);
        remove_at(&conn, &p.id, 1).unwrap();
        assert_eq!(track_ids(&conn, &p.id).unwrap(), vec![c.clone(), b.clone()]);
        assert!(remove_at(&conn, &p.id, 9).is_err());

        // Duplicates are allowed in playlists.
        add_tracks(&conn, &p.id, &[b.clone()]).unwrap();
        assert_eq!(get(&conn, &p.id).unwrap().track_count, 3);

        // Fresh playlists have no art and have never been played.
        let fresh = get(&conn, &p.id).unwrap();
        assert!(fresh.artwork_path.is_none() && fresh.last_played_at.is_none());
        mark_played(&conn, &p.id).unwrap();
        assert!(get(&conn, &p.id).unwrap().last_played_at.is_some());
        set_artwork(&conn, &p.id, Some("C:/art/x.png")).unwrap();
        assert_eq!(get(&conn, &p.id).unwrap().artwork_path.as_deref(), Some("C:/art/x.png"));
        set_artwork(&conn, &p.id, None).unwrap();
        assert!(get(&conn, &p.id).unwrap().artwork_path.is_none());
        assert!(mark_played(&conn, "missing").is_err());

        rename(&conn, &p.id, "Renamed").unwrap();
        assert_eq!(get(&conn, &p.id).unwrap().name, "Renamed");
        assert!(create(&conn, "  ", None).is_err());
        delete(&conn, &p.id).unwrap();
        assert!(list(&conn).unwrap().is_empty());
    }
}
