//! Followed Spotify artists and the tracks found by their syncs.

use crate::db::{library, now_iso};
use crate::error::{AppError, AppResult};
use crate::model::Track;
use crate::providers::spotify::{SpotifyArtist, SpotifyRelease};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FollowedArtist {
    pub id: String,
    pub name: String,
    pub image_url: Option<String>,
    pub genres: Vec<String>,
    pub auto_sync: bool,
    pub added_at: String,
    pub last_synced_at: Option<String>,
    pub last_attempt_at: Option<String>,
    pub last_error: Option<String>,
    pub track_count: u32,
    pub new_count: u32,
}

/// A track on a followed artist's page.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistTrack {
    #[serde(flatten)]
    pub track: Track,
    pub is_new: bool,
    pub album_type: String,
    pub release_date: String,
    pub found_at: String,
}

const COLUMNS: &str = "a.id, a.name, a.image_url, a.genres, a.auto_sync, a.added_at, a.last_synced_at, a.last_attempt_at, a.last_error,
     (SELECT COUNT(*) FROM followed_artist_tracks t WHERE t.artist_id = a.id),
     (SELECT COUNT(*) FROM followed_artist_tracks t WHERE t.artist_id = a.id AND t.is_new = 1)";

fn row_to_artist(r: &Row<'_>) -> rusqlite::Result<FollowedArtist> {
    let genres: String = r.get(3)?;
    Ok(FollowedArtist {
        id: r.get(0)?,
        name: r.get(1)?,
        image_url: r.get(2)?,
        genres: genres.split(',').map(str::trim).filter(|g| !g.is_empty()).map(String::from).collect(),
        auto_sync: r.get::<_, i64>(4)? != 0,
        added_at: r.get(5)?,
        last_synced_at: r.get(6)?,
        last_attempt_at: r.get(7)?,
        last_error: r.get(8)?,
        track_count: r.get(9)?,
        new_count: r.get(10)?,
    })
}

pub fn list(conn: &Connection) -> AppResult<Vec<FollowedArtist>> {
    let mut st = conn.prepare(&format!("SELECT {COLUMNS} FROM followed_artists a ORDER BY a.name COLLATE NOCASE"))?;
    let rows = st.query_map([], row_to_artist)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get(conn: &Connection, id: &str) -> AppResult<FollowedArtist> {
    conn.query_row(&format!("SELECT {COLUMNS} FROM followed_artists a WHERE a.id = ?1"), params![id], row_to_artist)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("followed artist {id}")))
}

pub fn is_followed(conn: &Connection, id: &str) -> AppResult<bool> {
    Ok(conn.query_row("SELECT 1 FROM followed_artists WHERE id = ?1", params![id], |_| Ok(())).optional()?.is_some())
}

/// Follow an artist (auto sync on). Following again only refreshes the profile.
pub fn follow(conn: &Connection, a: &SpotifyArtist) -> AppResult<FollowedArtist> {
    conn.execute(
        "INSERT INTO followed_artists (id, name, image_url, genres, added_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (id) DO UPDATE SET name = ?2, image_url = ?3, genres = ?4",
        params![a.id, a.name, a.image_url, a.genres.join(", "), now_iso()],
    )?;
    get(conn, &a.id)
}

pub fn update_profile(conn: &Connection, a: &SpotifyArtist) -> AppResult<()> {
    conn.execute(
        "UPDATE followed_artists SET name = ?2, image_url = ?3, genres = ?4 WHERE id = ?1",
        params![a.id, a.name, a.image_url, a.genres.join(", ")],
    )?;
    Ok(())
}

pub fn unfollow(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM followed_artists WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn set_auto_sync(conn: &Connection, id: &str, on: bool) -> AppResult<()> {
    let n = conn.execute("UPDATE followed_artists SET auto_sync = ?2 WHERE id = ?1", params![id, on as i64])?;
    if n == 0 {
        return Err(AppError::NotFound(format!("followed artist {id}")));
    }
    Ok(())
}

pub fn mark_seen(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("UPDATE followed_artist_tracks SET is_new = 0 WHERE artist_id = ?1", params![id])?;
    Ok(())
}

pub fn record_attempt(conn: &Connection, id: &str, error: Option<&str>) -> AppResult<()> {
    let now = now_iso();
    match error {
        None => conn.execute(
            "UPDATE followed_artists SET last_attempt_at = ?2, last_synced_at = ?2, last_error = NULL WHERE id = ?1",
            params![id, now],
        )?,
        Some(e) => conn.execute("UPDATE followed_artists SET last_attempt_at = ?2, last_error = ?3 WHERE id = ?1", params![id, now, e])?,
    };
    Ok(())
}

/// Releases already read completely for this artist.
pub fn known_releases(conn: &Connection, artist_id: &str) -> AppResult<HashSet<String>> {
    let mut st = conn.prepare("SELECT album_id FROM followed_artist_releases WHERE artist_id = ?1")?;
    let rows = st.query_map(params![artist_id], |r| r.get::<_, String>(0))?.collect::<Result<HashSet<_>, _>>()?;
    Ok(rows)
}

/// Normalised title + duration in seconds: the same song released as a single
/// and again on an album has a different Spotify id but the same identity.
fn song_key(title: &str, duration_ms: Option<u64>) -> (String, u64) {
    let t: String = title.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect();
    (t, duration_ms.unwrap_or(0) / 1000)
}

fn existing_song_keys(conn: &Connection, artist_id: &str) -> AppResult<Vec<(String, u64)>> {
    let mut st = conn.prepare(
        "SELECT tr.title, tr.duration_ms FROM followed_artist_tracks f JOIN tracks tr ON tr.id = f.track_id WHERE f.artist_id = ?1",
    )?;
    let rows = st
        .query_map(params![artist_id], |r| Ok(song_key(&r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?.map(|d| d as u64))))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn same_song(a: &(String, u64), b: &(String, u64)) -> bool {
    a.0 == b.0 && a.1.abs_diff(b.1) <= 2
}

/// Store one release's tracks and mark the release as read, atomically.
/// Songs the artist already has (e.g. a single that is now on an album) are
/// skipped. Returns how many tracks were added.
pub fn add_release(
    conn: &Connection,
    artist_id: &str,
    release: &SpotifyRelease,
    tracks: &[crate::model::TrackUpsert],
    mark_new: bool,
) -> AppResult<u32> {
    let tx = conn.unchecked_transaction()?;
    let mut keys = existing_song_keys(&tx, artist_id)?;
    let now = now_iso();
    let mut added = 0;
    for t in tracks {
        let key = song_key(&t.title, t.duration_ms);
        if keys.iter().any(|k| same_song(k, &key)) {
            continue;
        }
        let id = library::upsert_track(&tx, crate::model::Source::Spotify, t, false)?;
        added += tx.execute(
            "INSERT OR IGNORE INTO followed_artist_tracks (artist_id, track_id, album_id, album_type, release_date, is_new, found_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![artist_id, id, release.id, release.album_type, release.release_date, mark_new as i64, now],
        )? as u32;
        keys.push(key);
    }
    tx.execute(
        "INSERT OR REPLACE INTO followed_artist_releases (artist_id, album_id, name, album_type, release_date) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![artist_id, release.id, release.name, release.album_type, release.release_date],
    )?;
    tx.commit()?;
    Ok(added)
}

/// The artist's tracks: new ones first, then newest release first, in album order.
pub fn tracks(conn: &Connection, artist_id: &str) -> AppResult<Vec<ArtistTrack>> {
    let mut st = conn.prepare(
        "SELECT f.track_id, f.is_new, f.album_type, f.release_date, f.found_at FROM followed_artist_tracks f
         JOIN tracks tr ON tr.id = f.track_id
         WHERE f.artist_id = ?1
         ORDER BY f.is_new DESC, f.release_date DESC, f.album_id, COALESCE(tr.disc_number, 1), COALESCE(tr.track_number, 0)",
    )?;
    let rows = st
        .query_map(params![artist_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(id, is_new, album_type, release_date, found_at)| {
            Ok(ArtistTrack { track: library::get_track(conn, &id)?, is_new, album_type, release_date, found_at })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::library::tests::sample;
    use crate::db::open_in_memory;

    fn artist() -> SpotifyArtist {
        SpotifyArtist { id: "ar1".into(), name: "Ar".into(), image_url: None, genres: vec!["pop".into(), "rock".into()] }
    }

    fn release(id: &str, date: &str) -> SpotifyRelease {
        SpotifyRelease { id: id.into(), name: id.into(), album_type: "album".into(), release_date: date.into() }
    }

    #[test]
    fn follow_add_releases_and_mark_seen() {
        let conn = open_in_memory().unwrap();
        let a = follow(&conn, &artist()).unwrap();
        assert!(a.auto_sync && a.track_count == 0 && a.genres == ["pop", "rock"]);

        // First release (baseline): not new.
        let n = add_release(&conn, "ar1", &release("al1", "2020-01-01"), &[sample("t1", "Song", "Ar", "al1"), sample("t2", "Other", "Ar", "al1")], false).unwrap();
        assert_eq!(n, 2);
        // A later release repeats "Song" (same title and length): skipped.
        let n = add_release(&conn, "ar1", &release("al2", "2024-05-01"), &[sample("t3", "song", "Ar", "al2"), sample("t4", "Fresh", "Ar", "al2")], true).unwrap();
        assert_eq!(n, 1);
        assert_eq!(known_releases(&conn, "ar1").unwrap().len(), 2);

        let a = get(&conn, "ar1").unwrap();
        assert_eq!((a.track_count, a.new_count), (3, 1));
        let list = tracks(&conn, "ar1").unwrap();
        assert_eq!(list[0].track.title, "Fresh");
        assert!(list[0].is_new);
        // Not added to the user's library.
        assert!(!list[0].track.in_library);

        mark_seen(&conn, "ar1").unwrap();
        assert_eq!(get(&conn, "ar1").unwrap().new_count, 0);

        set_auto_sync(&conn, "ar1", false).unwrap();
        assert!(!get(&conn, "ar1").unwrap().auto_sync);
        unfollow(&conn, "ar1").unwrap();
        assert!(list_is_empty(&conn));
        let left: i64 = conn.query_row("SELECT COUNT(*) FROM followed_artist_tracks", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0);
    }

    fn list_is_empty(conn: &Connection) -> bool {
        list(conn).unwrap().is_empty()
    }

    #[test]
    fn attempts_record_errors_and_success() {
        let conn = open_in_memory().unwrap();
        follow(&conn, &artist()).unwrap();
        record_attempt(&conn, "ar1", Some("offline")).unwrap();
        let a = get(&conn, "ar1").unwrap();
        assert!(a.last_synced_at.is_none() && a.last_error.as_deref() == Some("offline") && a.last_attempt_at.is_some());
        record_attempt(&conn, "ar1", None).unwrap();
        let a = get(&conn, "ar1").unwrap();
        assert!(a.last_synced_at.is_some() && a.last_error.is_none());
    }
}
