//! Persisted unified play queue and playback state.

use super::now_iso;
use crate::error::{AppError, AppResult};
use crate::model::Track;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepeatMode {
    Off,
    All,
    One,
}

impl RepeatMode {
    fn as_str(&self) -> &'static str {
        match self {
            RepeatMode::Off => "off",
            RepeatMode::All => "all",
            RepeatMode::One => "one",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "all" => RepeatMode::All,
            "one" => RepeatMode::One,
            _ => RepeatMode::Off,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackState {
    pub current_index: Option<usize>,
    pub position_ms: u64,
    pub volume: f64,
    pub shuffle: bool,
    pub repeat_mode: RepeatMode,
    pub spotify_device_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueueSnapshot {
    pub items: Vec<Track>,
    pub state: PlaybackState,
}

pub fn get_state(conn: &Connection) -> AppResult<PlaybackState> {
    Ok(conn.query_row(
        "SELECT current_index, position_ms, volume, shuffle, repeat_mode, spotify_device_id FROM playback_state WHERE id = 1",
        [],
        |r| {
            Ok(PlaybackState {
                current_index: r.get::<_, Option<i64>>(0)?.map(|v| v.max(0) as usize),
                position_ms: r.get::<_, i64>(1)?.max(0) as u64,
                volume: r.get(2)?,
                shuffle: r.get::<_, i64>(3)? != 0,
                repeat_mode: RepeatMode::parse(&r.get::<_, String>(4)?),
                spotify_device_id: r.get(5)?,
            })
        },
    )?)
}

pub fn save_state(conn: &Connection, s: &PlaybackState) -> AppResult<()> {
    if !(0.0..=1.0).contains(&s.volume) {
        return Err(AppError::InvalidInput("volume must be between 0 and 1".into()));
    }
    conn.execute(
        "UPDATE playback_state SET current_index = ?1, position_ms = ?2, volume = ?3, shuffle = ?4,
            repeat_mode = ?5, spotify_device_id = ?6, updated_at = ?7 WHERE id = 1",
        params![
            s.current_index.map(|v| v as i64),
            s.position_ms as i64,
            s.volume,
            s.shuffle as i64,
            s.repeat_mode.as_str(),
            s.spotify_device_id,
            now_iso()
        ],
    )?;
    Ok(())
}

pub fn track_ids(conn: &Connection) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare("SELECT track_id FROM queue_items ORDER BY position")?;
    let rows = stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
    Ok(rows)
}

pub fn snapshot(conn: &Connection) -> AppResult<QueueSnapshot> {
    let items = super::library::get_tracks(conn, &track_ids(conn)?)?;
    let mut state = get_state(conn)?;
    if state.current_index.is_some_and(|i| i >= items.len()) {
        state.current_index = if items.is_empty() { None } else { Some(0) };
    }
    Ok(QueueSnapshot { items, state })
}

fn write_items(conn: &Connection, ids: &[String], current: Option<usize>) -> AppResult<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM queue_items", [])?;
    for (i, id) in ids.iter().enumerate() {
        tx.execute(
            "INSERT INTO queue_items (position, track_id) VALUES (?1, ?2)",
            params![i as i64, id],
        )?;
    }
    tx.execute(
        "UPDATE playback_state SET current_index = ?1, updated_at = ?2 WHERE id = 1",
        params![current.map(|v| v as i64), now_iso()],
    )?;
    tx.commit()?;
    Ok(())
}

/// Replace the queue and start at `start_index`.
pub fn replace(conn: &Connection, ids: &[String], start_index: usize) -> AppResult<QueueSnapshot> {
    let current = if ids.is_empty() { None } else { Some(start_index.min(ids.len() - 1)) };
    write_items(conn, ids, current)?;
    conn.execute("UPDATE playback_state SET position_ms = 0 WHERE id = 1", [])?;
    snapshot(conn)
}

/// Append tracks, or insert them right after the current track when `play_next`.
pub fn add(conn: &Connection, new_ids: &[String], play_next: bool) -> AppResult<QueueSnapshot> {
    let mut ids = track_ids(conn)?;
    let state = get_state(conn)?;
    let current = state.current_index.filter(|i| *i < ids.len());
    if play_next {
        let at = current.map(|i| i + 1).unwrap_or(0);
        for (k, id) in new_ids.iter().enumerate() {
            ids.insert(at + k, id.clone());
        }
    } else {
        ids.extend(new_ids.iter().cloned());
    }
    let current = current.or(if ids.is_empty() { None } else { Some(0) });
    write_items(conn, &ids, current)?;
    snapshot(conn)
}

pub fn remove_at(conn: &Connection, position: usize) -> AppResult<QueueSnapshot> {
    let mut ids = track_ids(conn)?;
    if position >= ids.len() {
        return Err(AppError::InvalidInput(format!("position {position} out of range")));
    }
    ids.remove(position);
    let current = get_state(conn)?.current_index.and_then(|c| {
        if ids.is_empty() {
            None
        } else if position < c {
            Some(c - 1)
        } else {
            Some(c.min(ids.len() - 1))
        }
    });
    write_items(conn, &ids, current)?;
    snapshot(conn)
}

pub fn move_item(conn: &Connection, from: usize, to: usize) -> AppResult<QueueSnapshot> {
    let mut ids = track_ids(conn)?;
    if from >= ids.len() || to >= ids.len() {
        return Err(AppError::InvalidInput("position out of range".into()));
    }
    let item = ids.remove(from);
    ids.insert(to, item);
    let current = get_state(conn)?.current_index.map(|c| moved_index(c, from, to));
    write_items(conn, &ids, current)?;
    snapshot(conn)
}

/// Where the item at `c` ends up after moving `from` to `to`.
pub fn moved_index(c: usize, from: usize, to: usize) -> usize {
    if c == from {
        to
    } else if from < c && to >= c {
        c - 1
    } else if from > c && to <= c {
        c + 1
    } else {
        c
    }
}

pub fn clear(conn: &Connection) -> AppResult<QueueSnapshot> {
    write_items(conn, &[], None)?;
    snapshot(conn)
}

/// Compute the next index given repeat mode. `None` means stop.
pub fn next_index(current: Option<usize>, len: usize, repeat: RepeatMode, user_initiated: bool) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let c = current?;
    if repeat == RepeatMode::One && !user_initiated {
        return Some(c);
    }
    if c + 1 < len {
        Some(c + 1)
    } else if repeat == RepeatMode::All || (repeat == RepeatMode::One && user_initiated) {
        Some(0)
    } else {
        None
    }
}

pub fn previous_index(current: Option<usize>, len: usize, repeat: RepeatMode) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let c = current?;
    if c > 0 {
        Some(c - 1)
    } else if repeat == RepeatMode::All {
        Some(len - 1)
    } else {
        Some(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::library::{tests::sample, upsert_track};
    use crate::db::open_in_memory;
    use crate::model::Source;

    fn three(conn: &Connection) -> Vec<String> {
        vec![
            upsert_track(conn, Source::Local, &sample("/1", "1", "a", "b"), true).unwrap(),
            upsert_track(conn, Source::Spotify, &sample("2", "2", "a", "b"), true).unwrap(),
            upsert_track(conn, Source::Soundcloud, &sample("3", "3", "a", "b"), true).unwrap(),
        ]
    }

    #[test]
    fn queue_persists_and_tracks_current_index() {
        let conn = open_in_memory().unwrap();
        let ids = three(&conn);
        let s = replace(&conn, &ids, 1).unwrap();
        assert_eq!(s.items.len(), 3);
        assert_eq!(s.state.current_index, Some(1));

        // Removing an item before current shifts current back.
        let s = remove_at(&conn, 0).unwrap();
        assert_eq!(s.state.current_index, Some(0));
        assert_eq!(s.items[0].id, ids[1]);

        // play_next inserts right after current.
        let s = add(&conn, &[ids[0].clone()], true).unwrap();
        assert_eq!(s.items.iter().map(|t| t.id.clone()).collect::<Vec<_>>(), vec![ids[1].clone(), ids[0].clone(), ids[2].clone()]);

        let s = move_item(&conn, 0, 2).unwrap();
        assert_eq!(s.state.current_index, Some(2));
        assert_eq!(s.items[2].id, ids[1]);

        let s = clear(&conn).unwrap();
        assert!(s.items.is_empty());
        assert_eq!(s.state.current_index, None);
    }

    #[test]
    fn state_roundtrip_and_validation() {
        let conn = open_in_memory().unwrap();
        let st = PlaybackState {
            current_index: Some(0),
            position_ms: 12_345,
            volume: 0.5,
            shuffle: true,
            repeat_mode: RepeatMode::All,
            spotify_device_id: Some("dev".into()),
        };
        save_state(&conn, &st).unwrap();
        assert_eq!(get_state(&conn).unwrap(), st);
        let bad = PlaybackState { volume: 1.5, ..st };
        assert!(save_state(&conn, &bad).is_err());
    }

    #[test]
    fn next_previous_rules() {
        assert_eq!(next_index(Some(0), 3, RepeatMode::Off, false), Some(1));
        assert_eq!(next_index(Some(2), 3, RepeatMode::Off, false), None);
        assert_eq!(next_index(Some(2), 3, RepeatMode::All, false), Some(0));
        assert_eq!(next_index(Some(1), 3, RepeatMode::One, false), Some(1));
        assert_eq!(next_index(Some(1), 3, RepeatMode::One, true), Some(2));
        assert_eq!(next_index(None, 3, RepeatMode::Off, false), None);
        assert_eq!(previous_index(Some(0), 3, RepeatMode::Off), Some(0));
        assert_eq!(previous_index(Some(0), 3, RepeatMode::All), Some(2));
        assert_eq!(previous_index(Some(2), 3, RepeatMode::Off), Some(1));
        assert_eq!(moved_index(1, 0, 2), 0);
        assert_eq!(moved_index(1, 2, 0), 2);
        assert_eq!(moved_index(1, 1, 2), 2);
    }
}
