//! "Find alternative" selections (`playlist_alternatives`, migration 0005).
//!
//! An alternative is an ordinary playlist item placed right after the
//! unavailable Spotify track it stands in for. The relation is remembered so
//! the UI can show it, re-clicks don't duplicate it, the sync engine keeps it
//! next to its original (for Spotify Synced playlists it exists only in
//! Branflakes), and a removed alternative is never added back automatically.

use super::{now_iso, playlists};
use crate::error::{AppError, AppResult};
use crate::model::Track;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Relation {
    pub original_track_id: String,
    pub alternative_track_id: String,
    /// The user removed it from the playlist later.
    pub removed: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum AddAlternativeOutcome {
    /// Inserted at `position` (right after the original where possible).
    Added { position: usize },
    /// Already in the playlist for this original: nothing changed.
    AlreadyAdded { position: usize },
}

pub fn list(conn: &Connection, playlist_id: &str) -> AppResult<Vec<Relation>> {
    let mut stmt = conn.prepare(
        "SELECT original_track_id, alternative_track_id, removed, created_at FROM playlist_alternatives
         WHERE playlist_id = ?1 ORDER BY created_at",
    )?;
    let rows = stmt
        .query_map(params![playlist_id], |r| {
            Ok(Relation { original_track_id: r.get(0)?, alternative_track_id: r.get(1)?, removed: r.get::<_, i64>(2)? != 0, created_at: r.get(3)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Active alternatives of a playlist: alternative id → original id. Only
/// those not removed by the user and currently in the playlist.
pub fn active(conn: &Connection, playlist_id: &str, current_items: &[String]) -> AppResult<HashMap<String, String>> {
    Ok(list(conn, playlist_id)?
        .into_iter()
        .filter(|r| !r.removed && current_items.contains(&r.alternative_track_id))
        .map(|r| (r.alternative_track_id, r.original_track_id))
        .collect())
}

/// Insert `alternative` right after the first occurrence of `original`
/// (or after `position_hint` if the original moved), and remember it.
pub fn add(conn: &Connection, playlist_id: &str, original: &Track, alternative: &Track, position_hint: Option<usize>) -> AppResult<AddAlternativeOutcome> {
    if original.id == alternative.id {
        return Err(AppError::InvalidInput("an alternative must be a different track".into()));
    }
    playlists::get(conn, playlist_id)?;
    let tx = conn.unchecked_transaction()?;
    let mut ids = playlists::track_ids(&tx, playlist_id)?;
    let orig_pos = match position_hint.filter(|&p| ids.get(p) == Some(&original.id)) {
        Some(p) => p,
        None => ids
            .iter()
            .position(|x| x == &original.id)
            .ok_or_else(|| AppError::NotFound("the original track is no longer in this playlist".into()))?,
    };
    // Repeated clicks / an earlier selection: never add the same alternative twice.
    let rel_active: bool = tx
        .query_row(
            "SELECT removed = 0 FROM playlist_alternatives WHERE playlist_id = ?1 AND original_track_id = ?2 AND alternative_track_id = ?3",
            params![playlist_id, original.id, alternative.id],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if rel_active {
        if let Some(p) = ids.iter().position(|x| x == &alternative.id) {
            return Ok(AddAlternativeOutcome::AlreadyAdded { position: p });
        }
    }
    let at = orig_pos + 1;
    ids.insert(at, alternative.id.clone());
    playlists::write_items(&tx, playlist_id, &ids)?;
    let now = now_iso();
    // An explicit new selection re-activates a previously removed one.
    tx.execute(
        "INSERT INTO playlist_alternatives (playlist_id, original_track_id, alternative_track_id, removed, created_at, updated_at)
         VALUES (?1, ?2, ?3, 0, ?4, ?4)
         ON CONFLICT (playlist_id, original_track_id, alternative_track_id) DO UPDATE SET removed = 0, updated_at = ?4",
        params![playlist_id, original.id, alternative.id, now],
    )?;
    tx.commit()?;
    Ok(AddAlternativeOutcome::Added { position: at })
}

/// The user removed `track_id` from the playlist: if it was a chosen
/// alternative, remember that so it is never re-added automatically.
pub fn mark_removed_if_gone(conn: &Connection, playlist_id: &str, track_id: &str) -> AppResult<()> {
    let still_there = playlists::track_ids(conn, playlist_id)?.iter().any(|x| x == track_id);
    if !still_there {
        conn.execute(
            "UPDATE playlist_alternatives SET removed = 1, updated_at = ?3 WHERE playlist_id = ?1 AND alternative_track_id = ?2",
            params![playlist_id, track_id, now_iso()],
        )?;
    }
    Ok(())
}

/// Put active alternatives back next to their originals after the sync
/// engine rebuilt the Spotify part of a playlist. `old` is the playlist as it
/// was (alternatives included), `new_spotify` the new Spotify items.
///
/// Each alternative goes right after its original if that is still present,
/// otherwise after the nearest item that preceded it before; alternatives
/// keep their relative order.
pub fn reinsert(new_spotify: &[String], old: &[String], active: &HashMap<String, String>) -> Vec<String> {
    let mut out: Vec<String> = new_spotify.to_vec();
    let is_alt = |id: &String| active.contains_key(id);
    for (i, id) in old.iter().enumerate() {
        let Some(original) = active.get(id) else { continue };
        if out.contains(id) {
            continue;
        }
        // Anchor: the original, else the closest earlier non-alternative item still present.
        let anchor = if out.contains(original) {
            Some(original.clone())
        } else {
            old[..i].iter().rev().find(|p| !is_alt(p) && out.contains(*p)).cloned()
        };
        let mut at = match anchor {
            Some(a) => out.iter().position(|x| *x == a).map(|p| p + 1).unwrap_or(out.len()),
            None => 0,
        };
        // After alternatives already placed behind the same anchor.
        while at < out.len() && is_alt(&out[at]) {
            at += 1;
        }
        out.insert(at, id.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::library::{tests::sample, upsert_track};
    use crate::db::open_in_memory;
    use crate::model::Source;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn reinsert_keeps_alternatives_after_their_originals() {
        let active: HashMap<String, String> = [("alt1".to_string(), "b".to_string())].into();
        // Spotify added x at the front; alt1 still follows b.
        assert_eq!(reinsert(&ids(&["x", "a", "b", "c"]), &ids(&["a", "b", "alt1", "c"]), &active), ids(&["x", "a", "b", "alt1", "c"]));
        // Original removed on Spotify: alt1 stays after a (its previous neighbour).
        assert_eq!(reinsert(&ids(&["a", "c"]), &ids(&["a", "b", "alt1", "c"]), &active), ids(&["a", "alt1", "c"]));
        // Two alternatives for the same original keep their order.
        let two: HashMap<String, String> = [("p".to_string(), "b".to_string()), ("q".to_string(), "b".to_string())].into();
        assert_eq!(reinsert(&ids(&["a", "b"]), &ids(&["a", "b", "p", "q"]), &two), ids(&["a", "b", "p", "q"]));
    }

    #[test]
    fn add_places_after_original_is_idempotent_and_remembers_removal() {
        let conn = open_in_memory().unwrap();
        let a = upsert_track(&conn, Source::Spotify, &sample("a", "A", "x", "y"), false).unwrap();
        let b = upsert_track(&conn, Source::Spotify, &sample("b", "B", "x", "y"), false).unwrap();
        let c = upsert_track(&conn, Source::Spotify, &sample("c", "C", "x", "y"), false).unwrap();
        let l = upsert_track(&conn, Source::Local, &sample("/l", "B", "x", "y"), true).unwrap();
        let p = playlists::create(&conn, "P", None).unwrap();
        playlists::set_items(&conn, &p.id, &[a.clone(), b.clone(), c.clone()]).unwrap();
        let orig = crate::db::library::get_track(&conn, &b).unwrap();
        let alt = crate::db::library::get_track(&conn, &l).unwrap();

        assert_eq!(add(&conn, &p.id, &orig, &alt, Some(1)).unwrap(), AddAlternativeOutcome::Added { position: 2 });
        assert_eq!(playlists::track_ids(&conn, &p.id).unwrap(), vec![a.clone(), b.clone(), l.clone(), c.clone()]);
        // Repeated click: no duplicate.
        assert_eq!(add(&conn, &p.id, &orig, &alt, Some(1)).unwrap(), AddAlternativeOutcome::AlreadyAdded { position: 2 });
        assert_eq!(playlists::track_ids(&conn, &p.id).unwrap().len(), 4);
        assert_eq!(active(&conn, &p.id, &playlists::track_ids(&conn, &p.id).unwrap()).unwrap().get(&l), Some(&b));

        // User removes the alternative: remembered, no longer active.
        playlists::remove_at(&conn, &p.id, 2).unwrap();
        mark_removed_if_gone(&conn, &p.id, &l).unwrap();
        let rel = list(&conn, &p.id).unwrap();
        assert!(rel[0].removed);
        assert!(active(&conn, &p.id, &playlists::track_ids(&conn, &p.id).unwrap()).unwrap().is_empty());
        // Explicit re-selection works again.
        assert!(matches!(add(&conn, &p.id, &orig, &alt, None).unwrap(), AddAlternativeOutcome::Added { position: 2 }));
        assert!(!list(&conn, &p.id).unwrap()[0].removed);
        assert!(add(&conn, &p.id, &orig, &orig, None).is_err());
    }
}
