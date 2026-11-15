//! Track, override, folder and backup persistence.

use super::now_iso;
use crate::error::{AppError, AppResult};
use crate::model::{Availability, Source, Track, TrackUpsert};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

const TRACK_COLUMNS: &str = "id, source, provider_id, provider_uri, title, artist, album, album_artist, \
     track_number, disc_number, year, genre, duration_ms, artwork, file_path, file_format, \
     availability, unavailable_reason, permalink_url, uploader, uploader_url, in_library, \
     has_overrides, added_at";

fn row_to_track(r: &Row<'_>) -> rusqlite::Result<Track> {
    let source: String = r.get(1)?;
    let availability: String = r.get(16)?;
    Ok(Track {
        id: r.get(0)?,
        source: source.parse().map_err(|e: String| {
            rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, e.into())
        })?,
        provider_id: r.get(2)?,
        provider_uri: r.get(3)?,
        title: r.get(4)?,
        artist: r.get(5)?,
        album: r.get(6)?,
        album_artist: r.get(7)?,
        track_number: r.get(8)?,
        disc_number: r.get(9)?,
        year: r.get(10)?,
        genre: r.get(11)?,
        duration_ms: r.get::<_, Option<i64>>(12)?.map(|v| v.max(0) as u64),
        artwork: r.get(13)?,
        file_path: r.get(14)?,
        file_format: r.get(15)?,
        availability: availability.parse().unwrap_or(Availability::Unavailable),
        unavailable_reason: r.get(17)?,
        permalink_url: r.get(18)?,
        uploader: r.get(19)?,
        uploader_url: r.get(20)?,
        in_library: r.get::<_, i64>(21)? != 0,
        has_overrides: r.get::<_, i64>(22)? != 0,
        added_at: r.get(23)?,
    })
}

/// Insert or update a provider track, returning its stable internal ID.
///
/// The internal ID never changes once assigned. `in_library` can only be
/// promoted (0 -> 1) by an upsert, never demoted, so a search result never
/// removes a track the user already added.
pub fn upsert_track(
    conn: &Connection,
    source: Source,
    t: &TrackUpsert,
    in_library: bool,
) -> AppResult<String> {
    if t.provider_id.trim().is_empty() {
        return Err(AppError::InvalidInput("provider_id must not be empty".into()));
    }
    let now = now_iso();
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM tracks WHERE source = ?1 AND provider_id = ?2",
            params![source.as_str(), t.provider_id],
            |r| r.get(0),
        )
        .optional()?;
    let availability = t.availability.unwrap_or(Availability::Available);
    let id = existing.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if existing.is_some() {
        conn.execute(
            "UPDATE tracks SET provider_uri = ?2, title = ?3, artist = ?4, album = ?5, album_artist = ?6,
                track_number = ?7, disc_number = ?8, year = ?9, genre = ?10, duration_ms = ?11,
                artwork_url = ?12, file_path = ?13, file_format = ?14, file_size = ?15, file_mtime = ?16,
                availability = ?17, unavailable_reason = ?18, permalink_url = ?19, uploader = ?20,
                uploader_url = ?21, in_library = MAX(in_library, ?22), updated_at = ?23, primary_artist = ?24
             WHERE id = ?1",
            params![
                id, t.provider_uri, t.title, t.artist, t.album, t.album_artist, t.track_number,
                t.disc_number, t.year, t.genre, t.duration_ms.map(|v| v as i64), t.artwork_url,
                t.file_path, t.file_format, t.file_size.map(|v| v as i64), t.file_mtime,
                availability.as_str(), t.unavailable_reason, t.permalink_url, t.uploader,
                t.uploader_url, in_library as i64, now, t.primary_artist
            ],
        )?;
    } else {
        conn.execute(
            "INSERT INTO tracks (id, source, provider_id, provider_uri, title, artist, album, album_artist,
                track_number, disc_number, year, genre, duration_ms, artwork_url, file_path, file_format,
                file_size, file_mtime, availability, unavailable_reason, permalink_url, uploader,
                uploader_url, in_library, added_at, updated_at, primary_artist)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
                ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?25, ?26)",
            params![
                id, source.as_str(), t.provider_id, t.provider_uri, t.title, t.artist, t.album,
                t.album_artist, t.track_number, t.disc_number, t.year, t.genre,
                t.duration_ms.map(|v| v as i64), t.artwork_url, t.file_path, t.file_format,
                t.file_size.map(|v| v as i64), t.file_mtime, availability.as_str(),
                t.unavailable_reason, t.permalink_url, t.uploader, t.uploader_url,
                in_library as i64, now, t.primary_artist
            ],
        )?;
    }
    Ok(id)
}

pub fn get_track(conn: &Connection, id: &str) -> AppResult<Track> {
    conn.query_row(
        &format!("SELECT {TRACK_COLUMNS} FROM tracks_effective WHERE id = ?1"),
        params![id],
        row_to_track,
    )
    .optional()?
    .ok_or_else(|| AppError::NotFound(format!("track {id}")))
}

/// The artist name to send to Last.fm for a track.
///
/// A user override always wins. Otherwise streaming tracks use their stored
/// primary artist, so "A, B" from Spotify scrobbles as "A". Spotify tracks
/// imported before the primary artist was stored fall back to the album
/// artist when the joined string starts with it.
pub fn scrobble_artist(conn: &Connection, track: &Track) -> AppResult<String> {
    let overridden = get_override(conn, &track.id)?.and_then(|o| o.artist).is_some();
    if overridden || track.source == Source::Local {
        return Ok(track.artist.clone());
    }
    let primary: Option<String> = conn
        .query_row("SELECT primary_artist FROM tracks WHERE id = ?1", params![track.id], |r| r.get(0))
        .optional()?
        .flatten();
    if let Some(p) = primary.map(|p| p.trim().to_string()).filter(|p| !p.is_empty()) {
        return Ok(p);
    }
    let aa = track.album_artist.trim();
    if !aa.is_empty() && track.artist.starts_with(&format!("{aa}, ")) {
        return Ok(aa.to_string());
    }
    Ok(track.artist.clone())
}

pub fn get_tracks(conn: &Connection, ids: &[String]) -> AppResult<Vec<Track>> {
    ids.iter().map(|id| get_track(conn, id)).collect()
}

pub fn find_by_provider_id(
    conn: &Connection,
    source: Source,
    provider_id: &str,
) -> AppResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT id FROM tracks WHERE source = ?1 AND provider_id = ?2",
            params![source.as_str(), provider_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// Raw (non-override) file info for a local track.
#[derive(Debug, Clone)]
pub struct LocalFileRow {
    pub id: String,
    pub file_path: String,
    pub file_size: Option<i64>,
    pub file_mtime: Option<i64>,
}

pub fn local_file_rows(conn: &Connection) -> AppResult<Vec<LocalFileRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, file_path, file_size, file_mtime FROM tracks WHERE source = 'local' AND file_path IS NOT NULL",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(LocalFileRow {
                id: r.get(0)?,
                file_path: r.get(1)?,
                file_size: r.get(2)?,
                file_mtime: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn set_availability(
    conn: &Connection,
    id: &str,
    availability: Availability,
    reason: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "UPDATE tracks SET availability = ?2, unavailable_reason = ?3, updated_at = ?4 WHERE id = ?1",
        params![id, availability.as_str(), reason, now_iso()],
    )?;
    Ok(())
}

pub fn set_in_library(conn: &Connection, ids: &[String], in_library: bool) -> AppResult<()> {
    let tx = conn.unchecked_transaction()?;
    for id in ids {
        tx.execute(
            "UPDATE tracks SET in_library = ?2 WHERE id = ?1",
            params![id, in_library as i64],
        )?;
    }
    tx.commit()?;
    Ok(())
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackQuery {
    pub search: Option<String>,
    pub source: Option<Source>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub sort: Option<String>,
    pub descending: Option<bool>,
    pub include_unavailable: Option<bool>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

pub fn list_tracks(conn: &Connection, q: &TrackQuery) -> AppResult<Vec<Track>> {
    let mut sql = format!("SELECT {TRACK_COLUMNS} FROM tracks_effective WHERE in_library = 1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(s) = q.search.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let pattern = format!("%{}%", s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        sql.push_str(
            " AND (title LIKE ? ESCAPE '\\' OR artist LIKE ? ESCAPE '\\' OR album LIKE ? ESCAPE '\\' OR genre LIKE ? ESCAPE '\\')",
        );
        for _ in 0..4 {
            args.push(Box::new(pattern.clone()));
        }
    }
    if let Some(src) = q.source {
        sql.push_str(" AND source = ?");
        args.push(Box::new(src.as_str().to_string()));
    }
    if let Some(a) = &q.artist {
        sql.push_str(" AND (artist = ? COLLATE NOCASE OR album_artist = ? COLLATE NOCASE)");
        args.push(Box::new(a.clone()));
        args.push(Box::new(a.clone()));
    }
    if let Some(a) = &q.album {
        sql.push_str(" AND album = ? COLLATE NOCASE");
        args.push(Box::new(a.clone()));
    }
    if !q.include_unavailable.unwrap_or(true) {
        sql.push_str(" AND availability != 'unavailable'");
    }
    // Sort column comes from a whitelist — never interpolate user input.
    let sort = match q.sort.as_deref() {
        Some("artist") => "artist COLLATE NOCASE, album COLLATE NOCASE, disc_number, track_number",
        Some("album") => "album COLLATE NOCASE, disc_number, track_number",
        Some("duration") => "duration_ms",
        Some("added") => "added_at",
        Some("year") => "year",
        Some("source") => "source, artist COLLATE NOCASE",
        _ => "title COLLATE NOCASE",
    };
    let dir = if q.descending.unwrap_or(false) { "DESC" } else { "ASC" };
    let sort_sql: String = sort
        .split(", ")
        .map(|c| format!("{c} {dir}"))
        .collect::<Vec<_>>()
        .join(", ");
    sql.push_str(&format!(" ORDER BY {sort_sql}, id LIMIT ? OFFSET ?"));
    args.push(Box::new(q.limit.unwrap_or(5000).min(20000) as i64));
    args.push(Box::new(q.offset.unwrap_or(0) as i64));

    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    let rows = stmt
        .query_map(params.as_slice(), row_to_track)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArtistSummary {
    pub name: String,
    pub track_count: u32,
    pub sources: Vec<Source>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AlbumSummary {
    pub name: String,
    pub artist: String,
    pub track_count: u32,
    pub year: Option<i32>,
    pub artwork: Option<String>,
    pub sources: Vec<Source>,
}

fn parse_sources(s: Option<String>) -> Vec<Source> {
    let mut v: Vec<Source> = s
        .unwrap_or_default()
        .split(',')
        .filter_map(|x| x.parse().ok())
        .collect();
    v.sort_by_key(|s| s.as_str());
    v.dedup();
    v
}

pub fn list_artists(conn: &Connection) -> AppResult<Vec<ArtistSummary>> {
    let mut stmt = conn.prepare(
        "SELECT CASE WHEN album_artist != '' THEN album_artist ELSE artist END AS a,
                COUNT(*), GROUP_CONCAT(DISTINCT source)
         FROM tracks_effective WHERE in_library = 1 AND a != ''
         GROUP BY a COLLATE NOCASE ORDER BY a COLLATE NOCASE",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(ArtistSummary {
                name: r.get(0)?,
                track_count: r.get(1)?,
                sources: parse_sources(r.get(2)?),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn list_albums(conn: &Connection, artist: Option<&str>) -> AppResult<Vec<AlbumSummary>> {
    let mut sql = String::from(
        "SELECT album, CASE WHEN MAX(album_artist) != '' THEN MAX(album_artist) ELSE MAX(artist) END,
                COUNT(*), MAX(year), MAX(artwork), GROUP_CONCAT(DISTINCT source)
         FROM tracks_effective WHERE in_library = 1 AND album != ''",
    );
    if artist.is_some() {
        sql.push_str(" AND (artist = ?1 COLLATE NOCASE OR album_artist = ?1 COLLATE NOCASE)");
    }
    sql.push_str(" GROUP BY album COLLATE NOCASE, CASE WHEN album_artist != '' THEN album_artist ELSE artist END COLLATE NOCASE ORDER BY album COLLATE NOCASE");
    let mut stmt = conn.prepare(&sql)?;
    let map = |r: &Row<'_>| {
        Ok(AlbumSummary {
            name: r.get(0)?,
            artist: r.get(1)?,
            track_count: r.get(2)?,
            year: r.get(3)?,
            artwork: r.get(4)?,
            sources: parse_sources(r.get(5)?),
        })
    };
    let rows = match artist {
        Some(a) => stmt.query_map(params![a], map)?.collect::<Result<Vec<_>, _>>()?,
        None => stmt.query_map([], map)?.collect::<Result<Vec<_>, _>>()?,
    };
    Ok(rows)
}

// ---------------------------------------------------------------- overrides

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrackOverride {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<i32>,
    pub track_number: Option<u32>,
    pub artwork_path: Option<String>,
}

fn blank_to_none(s: &Option<String>) -> Option<String> {
    s.as_ref().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

pub fn get_override(conn: &Connection, track_id: &str) -> AppResult<Option<TrackOverride>> {
    Ok(conn
        .query_row(
            "SELECT title, artist, album, album_artist, genre, year, track_number, artwork_path
             FROM track_overrides WHERE track_id = ?1",
            params![track_id],
            |r| {
                Ok(TrackOverride {
                    title: r.get(0)?,
                    artist: r.get(1)?,
                    album: r.get(2)?,
                    album_artist: r.get(3)?,
                    genre: r.get(4)?,
                    year: r.get(5)?,
                    track_number: r.get(6)?,
                    artwork_path: r.get(7)?,
                })
            },
        )
        .optional()?)
}

/// Replace the override row for a track. Empty strings clear a field; an
/// override with no fields set removes the row entirely.
pub fn set_override(conn: &Connection, track_id: &str, o: &TrackOverride) -> AppResult<()> {
    get_track(conn, track_id)?; // existence check
    let o = TrackOverride {
        title: blank_to_none(&o.title),
        artist: blank_to_none(&o.artist),
        album: blank_to_none(&o.album),
        album_artist: blank_to_none(&o.album_artist),
        genre: blank_to_none(&o.genre),
        year: o.year,
        track_number: o.track_number,
        artwork_path: blank_to_none(&o.artwork_path),
    };
    if o == TrackOverride::default() {
        return clear_override(conn, track_id);
    }
    conn.execute(
        "INSERT INTO track_overrides (track_id, title, artist, album, album_artist, genre, year, track_number, artwork_path, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT (track_id) DO UPDATE SET title = ?2, artist = ?3, album = ?4, album_artist = ?5,
            genre = ?6, year = ?7, track_number = ?8, artwork_path = ?9, updated_at = ?10",
        params![track_id, o.title, o.artist, o.album, o.album_artist, o.genre, o.year, o.track_number, o.artwork_path, now_iso()],
    )?;
    Ok(())
}

pub fn clear_override(conn: &Connection, track_id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM track_overrides WHERE track_id = ?1", params![track_id])?;
    Ok(())
}

// ------------------------------------------------------------------ folders

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFolder {
    pub id: i64,
    pub path: String,
    pub added_at: String,
    pub last_scanned_at: Option<String>,
}

pub fn add_folder(conn: &Connection, path: &str) -> AppResult<LibraryFolder> {
    conn.execute(
        "INSERT OR IGNORE INTO library_folders (path, added_at) VALUES (?1, ?2)",
        params![path, now_iso()],
    )?;
    conn.query_row(
        "SELECT id, path, added_at, last_scanned_at FROM library_folders WHERE path = ?1",
        params![path],
        |r| {
            Ok(LibraryFolder {
                id: r.get(0)?,
                path: r.get(1)?,
                added_at: r.get(2)?,
                last_scanned_at: r.get(3)?,
            })
        },
    )
    .map_err(Into::into)
}

pub fn list_folders(conn: &Connection) -> AppResult<Vec<LibraryFolder>> {
    let mut stmt =
        conn.prepare("SELECT id, path, added_at, last_scanned_at FROM library_folders ORDER BY path")?;
    let rows = stmt
        .query_map([], |r| {
            Ok(LibraryFolder {
                id: r.get(0)?,
                path: r.get(1)?,
                added_at: r.get(2)?,
                last_scanned_at: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Remove a folder and every local track under it that is not referenced by
/// a playlist or the queue (referenced tracks are kept but marked unavailable).
pub fn remove_folder(conn: &Connection, id: i64) -> AppResult<()> {
    let path: String = conn
        .query_row("SELECT path FROM library_folders WHERE id = ?1", params![id], |r| r.get(0))
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("folder {id}")))?;
    let prefix = format!("{}%", path.trim_end_matches(['\\', '/']));
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM tracks WHERE source = 'local' AND file_path LIKE ?1
           AND id NOT IN (SELECT track_id FROM playlist_items)
           AND id NOT IN (SELECT track_id FROM queue_items)",
        params![prefix],
    )?;
    tx.execute(
        "UPDATE tracks SET availability = 'unavailable', unavailable_reason = 'Library folder removed', in_library = 0
         WHERE source = 'local' AND file_path LIKE ?1",
        params![prefix],
    )?;
    tx.execute("DELETE FROM library_folders WHERE id = ?1", params![id])?;
    tx.commit()?;
    Ok(())
}

pub fn mark_folder_scanned(conn: &Connection, id: i64) -> AppResult<()> {
    conn.execute(
        "UPDATE library_folders SET last_scanned_at = ?2 WHERE id = ?1",
        params![id, now_iso()],
    )?;
    Ok(())
}

// ------------------------------------------------------------------ backups

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MetadataBackup {
    pub id: i64,
    pub track_id: String,
    pub file_path: String,
    pub backup_path: String,
    pub created_at: String,
}

pub fn insert_backup(conn: &Connection, track_id: &str, file_path: &str, backup_path: &str) -> AppResult<i64> {
    conn.execute(
        "INSERT INTO metadata_backups (track_id, file_path, backup_path, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![track_id, file_path, backup_path, now_iso()],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn list_backups(conn: &Connection, track_id: &str) -> AppResult<Vec<MetadataBackup>> {
    let mut stmt = conn.prepare(
        "SELECT id, track_id, file_path, backup_path, created_at FROM metadata_backups
         WHERE track_id = ?1 ORDER BY id DESC",
    )?;
    let rows = stmt
        .query_map(params![track_id], |r| {
            Ok(MetadataBackup {
                id: r.get(0)?,
                track_id: r.get(1)?,
                file_path: r.get(2)?,
                backup_path: r.get(3)?,
                created_at: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_backup(conn: &Connection, id: i64) -> AppResult<MetadataBackup> {
    conn.query_row(
        "SELECT id, track_id, file_path, backup_path, created_at FROM metadata_backups WHERE id = ?1",
        params![id],
        |r| {
            Ok(MetadataBackup {
                id: r.get(0)?,
                track_id: r.get(1)?,
                file_path: r.get(2)?,
                backup_path: r.get(3)?,
                created_at: r.get(4)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| AppError::NotFound(format!("backup {id}")))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::db::open_in_memory;

    pub fn sample(provider_id: &str, title: &str, artist: &str, album: &str) -> TrackUpsert {
        TrackUpsert {
            provider_id: provider_id.into(),
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            duration_ms: Some(200_000),
            ..Default::default()
        }
    }

    #[test]
    fn upsert_keeps_stable_internal_id() {
        let conn = open_in_memory().unwrap();
        let id1 = upsert_track(&conn, Source::Spotify, &sample("abc", "One", "A", "X"), true).unwrap();
        let id2 = upsert_track(&conn, Source::Spotify, &sample("abc", "One (Remaster)", "A", "X"), true).unwrap();
        assert_eq!(id1, id2);
        assert_eq!(get_track(&conn, &id1).unwrap().title, "One (Remaster)");
        // Same provider ID under a different source is a different track.
        let id3 = upsert_track(&conn, Source::Soundcloud, &sample("abc", "One", "A", "X"), true).unwrap();
        assert_ne!(id1, id3);
    }

    #[test]
    fn search_upsert_never_demotes_library_membership() {
        let conn = open_in_memory().unwrap();
        let id = upsert_track(&conn, Source::Spotify, &sample("s1", "Song", "A", "X"), true).unwrap();
        upsert_track(&conn, Source::Spotify, &sample("s1", "Song", "A", "X"), false).unwrap();
        assert!(get_track(&conn, &id).unwrap().in_library);
        let hidden = upsert_track(&conn, Source::Spotify, &sample("s2", "Other", "A", "X"), false).unwrap();
        let listed = list_tracks(&conn, &TrackQuery::default()).unwrap();
        assert!(listed.iter().all(|t| t.id != hidden));
    }

    #[test]
    fn list_filters_sorts_and_escapes() {
        let conn = open_in_memory().unwrap();
        upsert_track(&conn, Source::Local, &sample("/a.mp3", "Beta", "Zed", "Alb"), true).unwrap();
        upsert_track(&conn, Source::Spotify, &sample("sp", "alpha", "Amy", "Alb2"), true).unwrap();
        upsert_track(&conn, Source::Soundcloud, &sample("sc", "100% Gamma", "Bob", ""), true).unwrap();

        let all = list_tracks(&conn, &TrackQuery::default()).unwrap();
        assert_eq!(all.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["100% Gamma", "alpha", "Beta"]);

        let q = TrackQuery { source: Some(Source::Local), ..Default::default() };
        assert_eq!(list_tracks(&conn, &q).unwrap().len(), 1);

        let q = TrackQuery { search: Some("100%".into()), ..Default::default() };
        assert_eq!(list_tracks(&conn, &q).unwrap().len(), 1);
        let q = TrackQuery { search: Some("%".into()), ..Default::default() };
        assert_eq!(list_tracks(&conn, &q).unwrap().len(), 1, "% must be escaped, not a wildcard");

        let q = TrackQuery { sort: Some("artist".into()), descending: Some(true), ..Default::default() };
        assert_eq!(list_tracks(&conn, &q).unwrap()[0].artist, "Zed");

        let q = TrackQuery { sort: Some("title; DROP TABLE tracks".into()), ..Default::default() };
        assert_eq!(list_tracks(&conn, &q).unwrap().len(), 3);
    }

    #[test]
    fn overrides_apply_and_clear() {
        let conn = open_in_memory().unwrap();
        let id = upsert_track(&conn, Source::Spotify, &sample("o1", "Orig", "Art", "Alb"), true).unwrap();
        set_override(&conn, &id, &TrackOverride { title: Some("Custom".into()), artwork_path: Some("C:\\art.png".into()), ..Default::default() }).unwrap();
        let t = get_track(&conn, &id).unwrap();
        assert_eq!(t.title, "Custom");
        assert_eq!(t.artist, "Art");
        assert_eq!(t.artwork.as_deref(), Some("C:\\art.png"));
        assert!(t.has_overrides);
        // Provider refresh does not clobber the override.
        upsert_track(&conn, Source::Spotify, &sample("o1", "Orig v2", "Art", "Alb"), true).unwrap();
        assert_eq!(get_track(&conn, &id).unwrap().title, "Custom");
        // All-blank override removes the row.
        set_override(&conn, &id, &TrackOverride { title: Some("  ".into()), ..Default::default() }).unwrap();
        let t = get_track(&conn, &id).unwrap();
        assert_eq!(t.title, "Orig v2");
        assert!(!t.has_overrides);
    }

    #[test]
    fn artists_and_albums_group_across_sources() {
        let conn = open_in_memory().unwrap();
        upsert_track(&conn, Source::Local, &sample("/1", "a", "Same", "Album"), true).unwrap();
        upsert_track(&conn, Source::Spotify, &sample("2", "b", "same", "album"), true).unwrap();
        let artists = list_artists(&conn).unwrap();
        assert_eq!(artists.len(), 1);
        assert_eq!(artists[0].track_count, 2);
        assert_eq!(artists[0].sources, vec![Source::Local, Source::Spotify]);
        let albums = list_albums(&conn, Some("SAME")).unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].track_count, 2);
    }

    #[test]
    fn removing_folder_keeps_referenced_tracks_as_unavailable() {
        let conn = open_in_memory().unwrap();
        let f = add_folder(&conn, "C:\\Music").unwrap();
        let mut a = sample("c:\\music\\a.mp3", "A", "x", "y");
        a.file_path = Some("C:\\Music\\a.mp3".into());
        let mut b = sample("c:\\music\\b.mp3", "B", "x", "y");
        b.file_path = Some("C:\\Music\\b.mp3".into());
        let ida = upsert_track(&conn, Source::Local, &a, true).unwrap();
        let idb = upsert_track(&conn, Source::Local, &b, true).unwrap();
        conn.execute("INSERT INTO queue_items (position, track_id) VALUES (0, ?1)", params![idb]).unwrap();
        remove_folder(&conn, f.id).unwrap();
        assert!(get_track(&conn, &ida).is_err());
        let kept = get_track(&conn, &idb).unwrap();
        assert_eq!(kept.availability, Availability::Unavailable);
        assert!(list_folders(&conn).unwrap().is_empty());
    }
}
