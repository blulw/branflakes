//! Shareable playlist files (`.bran`).
//!
//! A `.bran` file is a ZIP archive:
//!
//! ```text
//! playlist.json          manifest: name, description, cover, tracks in order
//! cover.<ext>            playlist cover (optional)
//! art/<hash>.<ext>       track artwork that only exists on the sharer's PC
//! media/<sha256>.<ext>   local audio files, named by their content hash
//! ```
//!
//! Spotify and SoundCloud tracks travel as metadata (provider ID, URI,
//! title, …); local tracks carry the audio file itself. Uncompressed audio
//! (WAV, AIFF) and the manifest are deflated; formats that are already
//! compressed (MP3, FLAC, AAC, Ogg, JPEG, …) are stored as-is, since
//! deflating them gains almost nothing and costs time.
//!
//! On import, audio goes to `<app data>/shared/<sha256>.<ext>`, so importing
//! the same file twice (or two playlists sharing a song) reuses one copy and
//! one library row. The hash in each name is checked against the content
//! before the file is kept. Entry names are never used as paths except
//! through that validated `<hex>.<ext>` form.

use crate::db::library::{self, TrackOverride};
use crate::db::{now_iso, playlists};
use crate::error::{AppError, AppResult};
use crate::model::{Availability, Source, TrackUpsert};
use crate::providers::local;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub const EXTENSION: &str = "bran";
pub const FORMAT: &str = "branflakes-playlist";
pub const FORMAT_VERSION: u32 = 1;
const MANIFEST: &str = "playlist.json";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;
const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

/// Audio extensions accepted inside a `.bran` (what the local scanner reads).
const AUDIO_EXTS: &[&str] = &["mp3", "flac", "m4a", "mp4", "aac", "ogg", "oga", "opus", "wav", "aif", "aiff", "ape", "wv", "mpc", "wma"];
/// Extensions whose content is already compressed: stored, not deflated.
const COMPRESSED_EXTS: &[&str] = &[
    "mp3", "flac", "m4a", "mp4", "aac", "ogg", "oga", "opus", "ape", "wv", "mpc", "wma", "jpg", "jpeg", "png", "webp", "gif",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    #[serde(default)]
    pub app_version: String,
    #[serde(default)]
    pub exported_at: String,
    pub playlist: SharedPlaylist,
    pub tracks: Vec<SharedTrack>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SharedPlaylist {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Archive entry of the cover image.
    #[serde(default)]
    pub cover: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SharedTrack {
    pub source: Option<Source>,
    /// Spotify track ID or SoundCloud URN. Empty for local files (the
    /// sharer's file path is never written).
    pub provider_id: String,
    pub provider_uri: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub year: Option<i32>,
    pub genre: String,
    pub duration_ms: Option<u64>,
    /// An http(s) URL, or an archive entry (`art/…`).
    pub artwork: Option<String>,
    pub permalink_url: Option<String>,
    pub uploader: Option<String>,
    pub uploader_url: Option<String>,
    pub primary_artist: Option<String>,
    /// Local tracks: archive entry of the audio (`media/<sha256>.<ext>`).
    /// `None` when the file was missing on the sharer's PC.
    pub file: Option<String>,
    /// The sharer's app-level edits (title, artist, cover, …). `artworkPath`
    /// is an archive entry.
    pub overrides: Option<TrackOverride>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub path: String,
    pub tracks: u32,
    pub local_files: u32,
    pub bytes: u64,
    /// Local tracks whose file couldn't be read (shared as metadata only).
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub playlist: playlists::Playlist,
    pub tracks: u32,
    pub local_files: u32,
    /// Entries that couldn't be added, with the reason.
    pub skipped: Vec<String>,
}

// ================================================================ export

/// Provider data for a track, without the app-level overrides.
fn raw_track(conn: &Connection, id: &str) -> AppResult<(Source, SharedTrack, Option<String>)> {
    conn.query_row(
        "SELECT source, provider_id, provider_uri, title, artist, album, album_artist, track_number, disc_number,
                year, genre, duration_ms, artwork_url, permalink_url, uploader, uploader_url, primary_artist, file_path
         FROM tracks WHERE id = ?1",
        params![id],
        |r| {
            let source: String = r.get(0)?;
            let source: Source = source.parse().map_err(|e: String| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, e.into()))?;
            Ok((
                source,
                SharedTrack {
                    source: Some(source),
                    provider_id: r.get(1)?,
                    provider_uri: r.get(2)?,
                    title: r.get(3)?,
                    artist: r.get(4)?,
                    album: r.get(5)?,
                    album_artist: r.get(6)?,
                    track_number: r.get(7)?,
                    disc_number: r.get(8)?,
                    year: r.get(9)?,
                    genre: r.get(10)?,
                    duration_ms: r.get::<_, Option<i64>>(11)?.map(|v| v.max(0) as u64),
                    artwork: r.get(12)?,
                    permalink_url: r.get(13)?,
                    uploader: r.get(14)?,
                    uploader_url: r.get(15)?,
                    primary_artist: r.get(16)?,
                    file: None,
                    overrides: None,
                },
                r.get(17)?,
            ))
        },
    )
    .optional()?
    .ok_or_else(|| AppError::NotFound(format!("track {id}")))
}

fn ext_of(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

fn is_remote(s: &str) -> bool {
    s.starts_with("https://") || s.starts_with("http://")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash_file(path: &Path) -> AppResult<String> {
    let mut f = BufReader::new(File::open(path)?);
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

fn options_for(ext: &str, size: u64) -> SimpleFileOptions {
    let method = if COMPRESSED_EXTS.contains(&ext) { CompressionMethod::Stored } else { CompressionMethod::Deflated };
    SimpleFileOptions::default().compression_method(method).large_file(size >= u32::MAX as u64)
}

/// Files collected for the archive: entry name → source path on disk.
#[derive(Default)]
struct Entries {
    files: Vec<(String, PathBuf)>,
    by_source: HashMap<PathBuf, String>,
}

impl Entries {
    /// Add an image file once; returns its entry name, or `None` if unreadable.
    fn image(&mut self, path: &Path, name_for: impl FnOnce(&str, &str) -> String) -> Option<String> {
        if let Some(e) = self.by_source.get(path) {
            return Some(e.clone());
        }
        let data = std::fs::read(path).ok()?;
        let ext = match ext_of(path).as_str() {
            "" => "jpg".to_string(),
            e => e.to_string(),
        };
        let name = name_for(&hex(&Sha256::digest(&data))[..32], &ext);
        self.by_source.insert(path.to_path_buf(), name.clone());
        self.files.push((name.clone(), path.to_path_buf()));
        Some(name)
    }
}

/// Write playlist `id` to `dest` as a `.bran` file. The archive is written
/// next to `dest` first and renamed into place, so a failed export never
/// leaves a half-written file under the chosen name.
pub fn export_playlist(conn: &Connection, id: &str, dest: &Path, app_version: &str) -> AppResult<ExportReport> {
    let pl = playlists::get(conn, id)?;
    let mut entries = Entries::default();
    let mut manifest = Manifest {
        format: FORMAT.into(),
        version: FORMAT_VERSION,
        app_version: app_version.into(),
        exported_at: now_iso(),
        playlist: SharedPlaylist { name: pl.name.clone(), description: pl.description.clone(), cover: None },
        tracks: Vec::new(),
    };
    if let Some(cover) = pl.artwork_path.as_deref() {
        manifest.playlist.cover = entries.image(Path::new(cover), |_, ext| format!("cover.{ext}"));
    }

    let mut missing = Vec::new();
    let mut local_files = 0u32;
    for track_id in playlists::track_ids(conn, id)? {
        let (source, mut t, file_path) = raw_track(conn, &track_id)?;
        let label = if t.artist.is_empty() { t.title.clone() } else { format!("{} – {}", t.artist, t.title) };
        if let Some(art) = t.artwork.clone().filter(|a| !is_remote(a)) {
            t.artwork = entries.image(Path::new(&art), |h, ext| format!("art/{h}.{ext}"));
        }
        if let Some(mut o) = library::get_override(conn, &track_id)? {
            if let Some(art) = o.artwork_path.clone() {
                o.artwork_path = entries.image(Path::new(&art), |h, ext| format!("art/{h}.{ext}"));
            }
            t.overrides = Some(o);
        }
        if source == Source::Local {
            t.provider_id = String::new();
            // Local artwork is embedded in the file and re-read on import.
            t.artwork = None;
            let path = file_path.map(PathBuf::from);
            match path.as_deref().filter(|p| p.is_file()) {
                Some(p) => {
                    if let Some(name) = entries.by_source.get(p) {
                        t.file = Some(name.clone());
                    } else {
                        let ext = ext_of(p);
                        if !AUDIO_EXTS.contains(&ext.as_str()) {
                            missing.push(format!("{label} (.{ext} files can't be shared)"));
                        } else {
                            let name = format!("media/{}.{ext}", hash_file(p)?);
                            entries.by_source.insert(p.to_path_buf(), name.clone());
                            entries.files.push((name.clone(), p.to_path_buf()));
                            t.file = Some(name);
                            local_files += 1;
                        }
                    }
                }
                None => missing.push(label),
            }
        }
        manifest.tracks.push(t);
    }

    let tmp = dest.with_extension("bran.partial");
    let result = (|| -> AppResult<()> {
        let mut zip = ZipWriter::new(BufWriter::new(File::create(&tmp)?));
        let json = serde_json::to_vec_pretty(&manifest).map_err(|e| AppError::Other(e.to_string()))?;
        zip.start_file(MANIFEST, options_for("json", json.len() as u64)).map_err(zip_err)?;
        zip.write_all(&json)?;
        // A name can appear twice only if two source files hash the same
        // (identical content); write it once.
        let mut written = HashSet::new();
        for (name, path) in &entries.files {
            if !written.insert(name.clone()) {
                continue;
            }
            let size = std::fs::metadata(path)?.len();
            zip.start_file(name.as_str(), options_for(&ext_of(Path::new(name)), size)).map_err(zip_err)?;
            std::io::copy(&mut BufReader::new(File::open(path)?), &mut zip)?;
        }
        zip.finish().map_err(zip_err)?.flush()?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if dest.exists() {
        std::fs::remove_file(dest)?;
    }
    std::fs::rename(&tmp, dest)?;
    Ok(ExportReport {
        path: dest.to_string_lossy().into_owned(),
        tracks: manifest.tracks.len() as u32,
        local_files,
        bytes: std::fs::metadata(dest)?.len(),
        missing,
    })
}

fn zip_err(e: zip::result::ZipError) -> AppError {
    match e {
        zip::result::ZipError::Io(io) => AppError::Io(io),
        other => AppError::InvalidInput(format!("not a valid .bran file: {other}")),
    }
}

// ================================================================ import

/// Read and check the manifest.
pub fn read_manifest<R: Read + Seek>(zip: &mut ZipArchive<R>) -> AppResult<Manifest> {
    let entry = zip
        .by_name(MANIFEST)
        .map_err(|_| AppError::InvalidInput("not a Branflakes playlist file (playlist.json is missing)".into()))?;
    let mut buf = Vec::new();
    entry.take(MAX_MANIFEST_BYTES).read_to_end(&mut buf)?;
    let m: Manifest = serde_json::from_slice(&buf).map_err(|e| AppError::InvalidInput(format!("the playlist file is damaged: {e}")))?;
    if m.format != FORMAT {
        return Err(AppError::InvalidInput("not a Branflakes playlist file".into()));
    }
    if m.version > FORMAT_VERSION {
        return Err(AppError::Unsupported(format!(
            "this playlist was shared from a newer Branflakes ({}); update Branflakes to open it",
            if m.app_version.is_empty() { "unknown version" } else { &m.app_version }
        )));
    }
    Ok(m)
}

/// `media/<64 hex>.<audio ext>` → (hash, ext).
fn parse_media_name(name: &str) -> Option<(String, String)> {
    let rest = name.strip_prefix("media/")?;
    let (hash, ext) = rest.split_once('.')?;
    let ext = ext.to_ascii_lowercase();
    (hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) && AUDIO_EXTS.contains(&ext.as_str()))
        .then(|| (hash.to_ascii_lowercase(), ext))
}

/// Extract an audio entry into `shared_dir` (content-addressed, verified).
fn extract_media<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str, shared_dir: &Path) -> AppResult<PathBuf> {
    let (hash, ext) = parse_media_name(name).ok_or_else(|| AppError::InvalidInput(format!("unexpected file name {name}")))?;
    let target = shared_dir.join(format!("{hash}.{ext}"));
    if target.is_file() {
        return Ok(target);
    }
    std::fs::create_dir_all(shared_dir)?;
    let mut entry = zip.by_name(name).map_err(|_| AppError::InvalidInput(format!("{name} is missing from the file")))?;
    let tmp = shared_dir.join(format!("{hash}.{ext}.partial"));
    let res = (|| -> AppResult<()> {
        let mut out = BufWriter::new(File::create(&tmp)?);
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = entry.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
            out.write_all(&buf[..n])?;
        }
        out.flush()?;
        if hex(&h.finalize()) != hash {
            return Err(AppError::InvalidInput(format!("{name} is damaged (checksum mismatch)")));
        }
        Ok(())
    })();
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, &target)?;
    Ok(target)
}

/// Extract an image entry into the artwork cache.
fn extract_image<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str, artwork_dir: &Path) -> AppResult<PathBuf> {
    let entry = zip.by_name(name).map_err(|_| AppError::InvalidInput(format!("{name} is missing from the file")))?;
    let mut data = Vec::new();
    entry.take(MAX_IMAGE_BYTES).read_to_end(&mut data)?;
    let mime = match data.get(..4) {
        Some([0x89, b'P', b'N', b'G']) => "image/png",
        Some([0xFF, 0xD8, ..]) => "image/jpeg",
        Some([b'R', b'I', b'F', b'F']) => "image/webp",
        Some([b'G', b'I', b'F', b'8']) => "image/gif",
        Some([b'B', b'M', ..]) => "image/bmp",
        _ => return Err(AppError::InvalidInput(format!("{name} is not an image"))),
    };
    local::cache_artwork(artwork_dir, &data, Some(mime))
}

/// Peek at a `.bran` without importing it.
pub fn inspect(path: &Path) -> AppResult<Manifest> {
    let mut zip = ZipArchive::new(BufReader::new(File::open(path)?)).map_err(zip_err)?;
    read_manifest(&mut zip)
}

/// Import a `.bran` as a new (local-only) playlist.
pub fn import_playlist(conn: &Connection, src: &Path, shared_dir: &Path, artwork_dir: &Path) -> AppResult<ImportReport> {
    let mut zip = ZipArchive::new(BufReader::new(File::open(src)?)).map_err(zip_err)?;
    let m = read_manifest(&mut zip)?;

    // Phase 1: files (no database).
    let mut images: HashMap<String, Option<String>> = HashMap::new();
    let mut image = |zip: &mut ZipArchive<_>, name: &str| -> Option<String> {
        images
            .entry(name.to_string())
            .or_insert_with(|| match extract_image(zip, name, artwork_dir) {
                Ok(p) => Some(p.to_string_lossy().into_owned()),
                Err(e) => {
                    log::warn!("bran import: {e}");
                    None
                }
            })
            .clone()
    };
    let cover = m.playlist.cover.as_deref().and_then(|c| image(&mut zip, c));

    enum Item {
        Local(TrackUpsert, Option<TrackOverride>),
        Remote(Source, TrackUpsert, Option<TrackOverride>),
    }
    let mut items = Vec::new();
    let mut skipped = Vec::new();
    let mut local_files = HashSet::new();
    for t in &m.tracks {
        let label = if t.artist.is_empty() { t.title.clone() } else { format!("{} – {}", t.artist, t.title) };
        let overrides = t.overrides.clone().map(|mut o| {
            o.artwork_path = o.artwork_path.as_deref().and_then(|a| image(&mut zip, a));
            o
        });
        match t.source {
            Some(Source::Local) => {
                let Some(file) = t.file.as_deref() else {
                    skipped.push(format!("{label}: the file wasn't included (it was missing on the sharer's PC)"));
                    continue;
                };
                let up = extract_media(&mut zip, file, shared_dir).and_then(|p| {
                    local_files.insert(p.clone());
                    let mut up = local::upsert_from_file(&p, artwork_dir)?;
                    // The file is stored under its hash, so a title that came
                    // from the sharer's file name would be lost: use the
                    // sharer's metadata (read from the same file).
                    if !t.title.trim().is_empty() {
                        up.title = t.title.clone();
                    }
                    up.artist = t.artist.clone();
                    up.album = t.album.clone();
                    up.album_artist = t.album_artist.clone();
                    up.genre = t.genre.clone();
                    up.year = t.year;
                    up.track_number = t.track_number;
                    up.disc_number = t.disc_number;
                    Ok(up)
                });
                match up {
                    Ok(up) => items.push(Item::Local(up, overrides)),
                    Err(e) => skipped.push(format!("{label}: {e}")),
                }
            }
            Some(source) => {
                if t.provider_id.trim().is_empty() {
                    skipped.push(format!("{label}: no {source} ID"));
                    continue;
                }
                let artwork_url = match t.artwork.as_deref() {
                    Some(a) if is_remote(a) => Some(a.to_string()),
                    Some(a) => image(&mut zip, a),
                    None => None,
                };
                items.push(Item::Remote(
                    source,
                    TrackUpsert {
                        provider_id: t.provider_id.clone(),
                        provider_uri: t.provider_uri.clone(),
                        title: t.title.clone(),
                        artist: t.artist.clone(),
                        album: t.album.clone(),
                        album_artist: t.album_artist.clone(),
                        track_number: t.track_number,
                        disc_number: t.disc_number,
                        year: t.year,
                        genre: t.genre.clone(),
                        duration_ms: t.duration_ms,
                        artwork_url,
                        availability: Some(Availability::Available),
                        permalink_url: t.permalink_url.clone(),
                        uploader: t.uploader.clone(),
                        uploader_url: t.uploader_url.clone(),
                        primary_artist: t.primary_artist.clone(),
                        ..Default::default()
                    },
                    overrides,
                ));
            }
            None => skipped.push(format!("{label}: unknown source")),
        }
    }

    // Phase 2: database, all or nothing.
    let tx = conn.unchecked_transaction()?;
    let mut ids = Vec::with_capacity(items.len());
    for item in items {
        let (id, overrides) = match item {
            Item::Local(up, o) => (library::upsert_track(&tx, Source::Local, &up, false)?, o),
            Item::Remote(source, up, o) => {
                // A track the user already has keeps its own (fresher) data.
                let id = match library::find_by_provider_id(&tx, source, &up.provider_id)? {
                    Some(id) => id,
                    None => library::upsert_track(&tx, source, &up, false)?,
                };
                (id, o)
            }
        };
        // The sharer's edits apply only where the user has none of their own.
        if let Some(o) = overrides {
            if library::get_override(&tx, &id)?.is_none() {
                library::set_override(&tx, &id, &o)?;
            }
        }
        ids.push(id);
    }
    let name = match m.playlist.name.trim() {
        "" => "Shared playlist",
        n => n,
    };
    let pl = playlists::create(&tx, name, None)?;
    tx.execute("UPDATE playlists SET description = ?2, artwork_path = ?3 WHERE id = ?1", params![pl.id, m.playlist.description, cover])?;
    playlists::write_items(&tx, &pl.id, &ids)?;
    tx.commit()?;

    Ok(ImportReport {
        playlist: playlists::get(conn, &pl.id)?,
        tracks: ids.len() as u32,
        local_files: local_files.len() as u32,
        skipped,
    })
}

// ======================================================= "Open with" files

/// A `.bran` the app was launched with (or that a second launch passed on),
/// waiting for the UI to pick it up.
static PENDING_OPEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `.bran` paths among command-line arguments.
pub fn bran_args<I: IntoIterator<Item = String>>(args: I) -> Vec<String> {
    args.into_iter()
        .skip(1)
        .filter(|a| Path::new(a).extension().is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION)))
        .collect()
}

pub fn push_pending(paths: Vec<String>) {
    PENDING_OPEN.lock().unwrap_or_else(|e| e.into_inner()).extend(paths);
}

pub fn take_pending() -> Vec<String> {
    std::mem::take(&mut *PENDING_OPEN.lock().unwrap_or_else(|e| e.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::library::{tests::sample, upsert_track};
    use crate::db::open_in_memory;
    use crate::providers::local::tests::{make_mp3, make_wav};

    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
    }
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        Fixture { _dir: dir, root }
    }

    fn png(path: &Path, seed: u8) {
        let mut data = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        data.extend_from_slice(&[seed; 32]);
        std::fs::write(path, data).unwrap();
    }

    #[test]
    fn round_trip_keeps_order_cover_metadata_and_files() {
        let f = fixture();
        let music = f.root.join("music");
        std::fs::create_dir_all(&music).unwrap();
        let mp3 = music.join("a.mp3");
        let wav = music.join("b.wav");
        make_mp3(&mp3);
        make_wav(&wav);
        let art_a = f.root.join("art_a");
        let cover = f.root.join("cover.png");
        png(&cover, 1);

        // Sharer.
        let src = open_in_memory().unwrap();
        let l1 = upsert_track(&src, Source::Local, &local::upsert_from_file(&mp3, &art_a).unwrap(), true).unwrap();
        let l2 = upsert_track(&src, Source::Local, &local::upsert_from_file(&wav, &art_a).unwrap(), true).unwrap();
        let mut sp = sample("sp1", "Spotty", "Artist", "Album");
        sp.provider_uri = Some("spotify:track:sp1".into());
        sp.artwork_url = Some("https://i.scdn.co/image/x".into());
        let s = upsert_track(&src, Source::Spotify, &sp, true).unwrap();
        let c = upsert_track(&src, Source::Soundcloud, &sample("soundcloud:tracks:9", "Cloudy", "SC", ""), false).unwrap();
        let ov_art = f.root.join("ov.png");
        png(&ov_art, 2);
        library::set_override(&src, &c, &TrackOverride { title: Some("Cloudy (edit)".into()), artwork_path: Some(ov_art.to_string_lossy().into()), ..Default::default() }).unwrap();
        let missing = upsert_track(&src, Source::Local, &sample("c:/gone.mp3", "Gone", "X", ""), true).unwrap();
        src.execute("UPDATE tracks SET file_path = 'c:/definitely/not/here.mp3' WHERE id = ?1", params![missing]).unwrap();

        let pl = playlists::create(&src, "Road trip", None).unwrap();
        playlists::set_artwork(&src, &pl.id, Some(&cover.to_string_lossy())).unwrap();
        src.execute("UPDATE playlists SET description = 'summer' WHERE id = ?1", params![pl.id]).unwrap();
        playlists::set_items(&src, &pl.id, &[s.clone(), l1.clone(), c.clone(), l2.clone(), l1.clone(), missing.clone()]).unwrap();

        let out = f.root.join("Road trip.bran");
        let rep = export_playlist(&src, &pl.id, &out, "0.6.0").unwrap();
        assert_eq!(rep.tracks, 6);
        assert_eq!(rep.local_files, 2, "the repeated track is stored once");
        assert_eq!(rep.missing, vec!["X – Gone".to_string()]);
        assert!(!f.root.join("Road trip.bran.partial").exists());

        let m = inspect(&out).unwrap();
        assert_eq!(m.playlist.name, "Road trip");
        let json = serde_json::to_string(&m).unwrap();
        assert!(!json.contains("music"), "local paths are never shared: {json}");

        // Recipient.
        let dst = open_in_memory().unwrap();
        let shared = f.root.join("shared");
        let art_b = f.root.join("art_b");
        let imp = import_playlist(&dst, &out, &shared, &art_b).unwrap();
        assert_eq!(imp.tracks, 5);
        assert_eq!(imp.local_files, 2);
        assert_eq!(imp.skipped.len(), 1);
        assert_eq!(imp.playlist.name, "Road trip");
        assert_eq!(imp.playlist.description, "summer");
        let cover2 = imp.playlist.artwork_path.clone().unwrap();
        assert!(cover2.starts_with(&*art_b.to_string_lossy()));
        assert_eq!(std::fs::read(&cover2).unwrap(), std::fs::read(&cover).unwrap());

        let tracks = playlists::tracks(&dst, &imp.playlist.id).unwrap();
        let titles: Vec<_> = tracks.iter().map(|t| (t.source, t.title.clone())).collect();
        let local_title = library::get_track(&src, &l1).unwrap().title;
        let wav_title = library::get_track(&src, &l2).unwrap().title;
        assert_eq!(
            titles,
            vec![
                (Source::Spotify, "Spotty".to_string()),
                (Source::Local, local_title.clone()),
                (Source::Soundcloud, "Cloudy (edit)".to_string()),
                (Source::Local, wav_title),
                (Source::Local, local_title),
            ]
        );
        assert_eq!(tracks[1].id, tracks[4].id, "same file → same track");
        assert_eq!(tracks[0].provider_uri.as_deref(), Some("spotify:track:sp1"));
        assert_eq!(tracks[0].artwork.as_deref(), Some("https://i.scdn.co/image/x"));
        let local_path = tracks[1].file_path.clone().unwrap();
        assert!(local_path.starts_with(&*shared.to_string_lossy()));
        assert_eq!(std::fs::read(&local_path).unwrap(), std::fs::read(&mp3).unwrap());
        let ov = std::fs::read(tracks[2].artwork.as_deref().unwrap()).unwrap();
        assert_eq!(ov, std::fs::read(&ov_art).unwrap());

        // Importing again reuses files and tracks.
        let again = import_playlist(&dst, &out, &shared, &art_b).unwrap();
        assert_ne!(again.playlist.id, imp.playlist.id);
        assert_eq!(playlists::track_ids(&dst, &again.playlist.id).unwrap(), playlists::track_ids(&dst, &imp.playlist.id).unwrap());
        assert_eq!(std::fs::read_dir(&shared).unwrap().count(), 2);
    }

    #[test]
    fn existing_tracks_and_overrides_win() {
        let f = fixture();
        let src = open_in_memory().unwrap();
        let s = upsert_track(&src, Source::Spotify, &sample("sp1", "Theirs", "A", "B"), true).unwrap();
        library::set_override(&src, &s, &TrackOverride { title: Some("Their edit".into()), ..Default::default() }).unwrap();
        let pl = playlists::create(&src, "P", None).unwrap();
        playlists::set_items(&src, &pl.id, &[s]).unwrap();
        let out = f.root.join("p.bran");
        export_playlist(&src, &pl.id, &out, "0.6.0").unwrap();

        let dst = open_in_memory().unwrap();
        let mine = upsert_track(&dst, Source::Spotify, &sample("sp1", "Mine", "A", "B"), true).unwrap();
        library::set_override(&dst, &mine, &TrackOverride { title: Some("My edit".into()), ..Default::default() }).unwrap();
        let imp = import_playlist(&dst, &out, &f.root.join("s"), &f.root.join("a")).unwrap();
        let t = &playlists::tracks(&dst, &imp.playlist.id).unwrap()[0];
        assert_eq!(t.id, mine);
        assert_eq!(t.title, "My edit");
        library::clear_override(&dst, &mine).unwrap();
        assert_eq!(library::get_track(&dst, &mine).unwrap().title, "Mine", "base data not overwritten");
    }

    #[test]
    fn rejects_bad_files() {
        let f = fixture();
        let dst = open_in_memory().unwrap();
        let junk = f.root.join("junk.bran");
        std::fs::write(&junk, b"not a zip").unwrap();
        assert!(import_playlist(&dst, &junk, &f.root, &f.root).is_err());

        // A tampered audio entry is refused and leaves nothing behind.
        let bad = f.root.join("bad.bran");
        let hash = "0".repeat(64);
        let manifest = Manifest {
            format: FORMAT.into(),
            version: FORMAT_VERSION,
            app_version: String::new(),
            exported_at: String::new(),
            playlist: SharedPlaylist { name: "Bad".into(), description: String::new(), cover: None },
            tracks: vec![SharedTrack { source: Some(Source::Local), title: "T".into(), file: Some(format!("media/{hash}.mp3")), ..Default::default() }],
        };
        let mut zip = ZipWriter::new(File::create(&bad).unwrap());
        zip.start_file(MANIFEST, SimpleFileOptions::default()).unwrap();
        zip.write_all(&serde_json::to_vec(&manifest).unwrap()).unwrap();
        zip.start_file(format!("media/{hash}.mp3"), SimpleFileOptions::default()).unwrap();
        zip.write_all(b"evil").unwrap();
        zip.finish().unwrap();
        let shared = f.root.join("shared");
        let imp = import_playlist(&dst, &bad, &shared, &f.root.join("art")).unwrap();
        assert_eq!(imp.tracks, 0);
        assert!(imp.skipped[0].contains("checksum"), "{:?}", imp.skipped);
        assert_eq!(std::fs::read_dir(&shared).unwrap().count(), 0);

        // Names that could escape the folder are never used.
        assert!(parse_media_name("media/../../x.mp3").is_none());
        assert!(parse_media_name(&format!("media/{hash}.exe")).is_none());
        assert!(parse_media_name(&format!("media/{hash}.mp3")).is_some());

        // Newer format.
        let newer = f.root.join("newer.bran");
        let mut zip = ZipWriter::new(File::create(&newer).unwrap());
        zip.start_file(MANIFEST, SimpleFileOptions::default()).unwrap();
        zip.write_all(serde_json::to_string(&Manifest { version: 99, app_version: "9.0.0".into(), ..manifest }).unwrap().as_bytes()).unwrap();
        zip.finish().unwrap();
        assert!(matches!(import_playlist(&dst, &newer, &shared, &f.root), Err(AppError::Unsupported(_))));
    }

    #[test]
    fn finds_bran_arguments() {
        let args = ["app.exe", "C:\\x\\Mix.BRAN", "--flag", "c:\\y.mp3"].map(String::from);
        assert_eq!(bran_args(args), vec!["C:\\x\\Mix.BRAN".to_string()]);
    }
}
