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
    /// Present only when the playlist is linked to a Spotify playlist with
    /// sync enabled ("Spotify Synced").
    pub sync: Option<PlaylistSync>,
    /// For a Branflakes-only copy of a synced playlist: the source playlist.
    pub forked_from: Option<String>,
    pub forked_from_name: Option<String>,
    /// For a copy: what it is based on and which Spotify entries were not copied.
    pub fork_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistSync {
    pub spotify_playlist_id: String,
    /// "pending" | "syncing" | "ok" | "error"
    pub status: String,
    pub last_synced_at: Option<String>,
    pub last_error: Option<String>,
    /// Non-error information from the last sync (e.g. a merged conflict).
    pub last_notice: Option<String>,
    /// False for collaborative playlists owned by someone else.
    pub can_edit_details: bool,
}

pub fn list(conn: &Connection) -> AppResult<Vec<Playlist>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.name, p.description, p.origin,
                (SELECT COUNT(*) FROM playlist_items i WHERE i.playlist_id = p.id),
                p.artwork_path, p.last_played_at, p.created_at, p.updated_at,
                l.spotify_playlist_id, l.status, l.last_synced_at, l.last_error, l.last_notice, l.can_edit_details,
                p.forked_from, (SELECT f.name FROM playlists f WHERE f.id = p.forked_from), p.fork_note
         FROM playlists p
         LEFT JOIN spotify_playlist_links l ON l.playlist_id = p.id AND l.enabled = 1
         ORDER BY p.name COLLATE NOCASE",
    )?;
    let rows = stmt
        .query_map([], |r| {
            let spotify_id: Option<String> = r.get(9)?;
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
                sync: match spotify_id {
                    Some(spotify_playlist_id) => Some(PlaylistSync {
                        spotify_playlist_id,
                        status: r.get(10)?,
                        last_synced_at: r.get(11)?,
                        last_error: r.get(12)?,
                        last_notice: r.get(13)?,
                        can_edit_details: r.get::<_, i64>(14)? != 0,
                    }),
                    None => None,
                },
                forked_from: r.get(15)?,
                forked_from_name: r.get(16)?,
                fork_note: r.get(17)?,
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
    write_items(&tx, id, track_ids)?;
    tx.commit()?;
    Ok(())
}

/// `set_items` without its own transaction, for callers that hold one.
pub fn write_items(conn: &Connection, id: &str, track_ids: &[String]) -> AppResult<()> {
    conn.execute("DELETE FROM playlist_items WHERE playlist_id = ?1", params![id])?;
    for (pos, t) in track_ids.iter().enumerate() {
        conn.execute(
            "INSERT INTO playlist_items (playlist_id, position, track_id) VALUES (?1, ?2, ?3)",
            params![id, pos as i64, t],
        )?;
    }
    conn.execute("UPDATE playlists SET updated_at = ?2 WHERE id = ?1", params![id, now_iso()])?;
    Ok(())
}

pub fn add_tracks(conn: &Connection, id: &str, new_ids: &[String]) -> AppResult<()> {
    insert_tracks(conn, id, new_ids, None)
}

/// Insert tracks at `position` (clamped; `None` appends), keeping order.
pub fn insert_tracks(conn: &Connection, id: &str, new_ids: &[String], position: Option<usize>) -> AppResult<()> {
    get(conn, id)?;
    let mut ids = track_ids(conn, id)?;
    let at = position.unwrap_or(ids.len()).min(ids.len());
    ids.splice(at..at, new_ids.iter().cloned());
    set_items(conn, id, &ids)
}

/// A name based on `base` that no other playlist uses yet:
/// "Mix (Branflakes)", then "Mix (Branflakes 2)", …
pub fn unique_fork_name(conn: &Connection, base: &str) -> AppResult<String> {
    let taken: std::collections::HashSet<String> = list(conn)?.into_iter().map(|p| p.name.to_lowercase()).collect();
    let mut n = 1;
    loop {
        let candidate = if n == 1 { format!("{base} (Branflakes)") } else { format!("{base} (Branflakes {n})") };
        if !taken.contains(&candidate.to_lowercase()) {
            return Ok(candidate);
        }
        n += 1;
    }
}

/// Create a local-only copy of `source` (its cover and description) holding
/// `items` (the caller decides which version of the source's contents).
/// The copy has no `origin` and no Spotify link, so it is never synced.
pub fn create_fork(conn: &Connection, source: &str, items: &[String], note: Option<&str>) -> AppResult<Playlist> {
    let src = get(conn, source)?;
    let ids = items.to_vec();
    let name = unique_fork_name(conn, &src.name)?;
    let tx = conn.unchecked_transaction()?;
    let fork = create(&tx, &name, None)?;
    tx.execute(
        "UPDATE playlists SET forked_from = ?2, artwork_path = ?3, description = ?4, fork_note = ?5 WHERE id = ?1",
        params![fork.id, src.id, src.artwork_path, src.description, note],
    )?;
    write_items(&tx, &fork.id, &ids)?;
    tx.commit()?;
    get(conn, &fork.id)
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

        // Positional insert.
        let before = track_ids(&conn, &p.id).unwrap();
        insert_tracks(&conn, &p.id, &[a.clone()], Some(1)).unwrap();
        let after = track_ids(&conn, &p.id).unwrap();
        assert_eq!(after[1], a);
        assert_eq!(after.len(), before.len() + 1);
        assert!(get(&conn, &p.id).unwrap().sync.is_none(), "ordinary playlists are not synced");

        rename(&conn, &p.id, "Renamed").unwrap();
        assert_eq!(get(&conn, &p.id).unwrap().name, "Renamed");
        assert!(create(&conn, "  ", None).is_err());
        delete(&conn, &p.id).unwrap();
        assert!(list(&conn).unwrap().is_empty());
    }
}

#[cfg(test)]
mod fork_tests {
    use super::*;
    use crate::db::library::{tests::sample, upsert_track};
    use crate::db::open_in_memory;
    use crate::model::Source;

    #[test]
    fn fork_copies_everything_and_is_local_only() {
        let conn = open_in_memory().unwrap();
        let s1 = upsert_track(&conn, Source::Spotify, &sample("s1", "S1", "x", "y"), false).unwrap();
        let s2 = upsert_track(&conn, Source::Spotify, &sample("s2", "S2", "x", "y"), false).unwrap();
        let local = upsert_track(&conn, Source::Local, &sample("/l", "L", "x", "y"), true).unwrap();
        let src = create(&conn, "Road trip", Some("spotify:playlist:abc")).unwrap();
        set_items(&conn, &src.id, &[s1.clone(), s2.clone()]).unwrap();
        set_artwork(&conn, &src.id, Some("C:/art/cover.jpg")).unwrap();

        let fork = create_fork(&conn, &src.id, &[s1.clone(), local.clone(), s2.clone()], Some("note")).unwrap();
        assert_eq!(fork.fork_note.as_deref(), Some("note"));
        assert_eq!(fork.name, "Road trip (Branflakes)");
        assert_eq!(track_ids(&conn, &fork.id).unwrap(), vec![s1.clone(), local.clone(), s2.clone()]);
        assert_eq!(fork.origin, None);
        assert_eq!(fork.forked_from.as_deref(), Some(src.id.as_str()));
        assert_eq!(fork.forked_from_name.as_deref(), Some("Road trip"));
        assert_eq!(fork.artwork_path.as_deref(), Some("C:/art/cover.jpg"));
        assert!(fork.sync.is_none());
        // Source untouched.
        assert_eq!(track_ids(&conn, &src.id).unwrap(), vec![s1.clone(), s2.clone()]);
        // Second fork gets a distinct name; forks are independently editable.
        let fork2 = create_fork(&conn, &src.id, &[s1.clone(), s2.clone(), local.clone()], None).unwrap();
        assert_eq!(fork2.name, "Road trip (Branflakes 2)");
        remove_at(&conn, &fork.id, 0).unwrap();
        assert_eq!(track_ids(&conn, &fork2.id).unwrap().len(), 3);
        // Deleting the source keeps the fork (no cascade), name lookup becomes empty.
        delete(&conn, &src.id).unwrap();
        let f = get(&conn, &fork.id).unwrap();
        assert_eq!(f.forked_from_name, None);
    }
}
