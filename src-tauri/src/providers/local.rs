//! LocalMusicAdapter — scans user-selected folders, reads/writes tags in the
//! actual audio files, extracts embedded artwork and declares playback
//! capabilities for the built-in (WebView2/Chromium) audio element.
//!
//! Tag writes use lofty's *concrete* tag types (ID3v2, Vorbis Comments, MP4
//! ilst, APE) via `SplitTag`/`MergeTag`, so frames that the generic model
//! cannot represent (PRIV, GEOB, custom atoms, ...) survive an edit.

use crate::db::library;
use crate::error::{AppError, AppResult};
use crate::model::{Availability, PlaybackCapabilities, ProviderStatus, Source, TrackUpsert};
use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::{AudioFile, FileType, TaggedFile, TaggedFileExt};
use lofty::ogg::OggPictureStorage;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::{ItemKey, MergeTag, SplitTag, TagExt};
use lofty::probe::Probe;
use lofty::tag::{Tag, TagType};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

/// Extensions we attempt to read with lofty.
pub const SCANNED_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "m4a", "m4b", "aac", "ogg", "oga", "opus", "spx", "wav", "aif", "aiff", "ape", "wv", "mpc",
];

/// Audio extensions we recognise but cannot read (reported, not imported).
pub const KNOWN_UNSUPPORTED_EXTENSIONS: &[&str] = &["wma", "asf", "dsf", "dff", "mka", "ra", "tta", "mid", "midi"];

pub struct LocalMusicAdapter;

impl LocalMusicAdapter {
    pub fn capabilities() -> PlaybackCapabilities {
        PlaybackCapabilities {
            play: true,
            pause: true,
            next: true,     // via the unified app queue
            previous: true, // via the unified app queue
            seek: true,
            volume: true,
            queue: true,
            device_selection: false, // not implemented: output device follows Windows default
            playback_position: true,
            duration: true,
        }
    }

    pub fn status() -> ProviderStatus {
        ProviderStatus {
            provider: "local".into(),
            display_name: "Local files".into(),
            configured: true,
            connected: true,
            needs_reconnect: false,
            missing_scopes: Vec::new(),
            account_name: None,
            capabilities: Self::capabilities(),
            limitations: vec![
                "Playback uses the built-in Chromium (WebView2) decoder: MP3, FLAC, AAC/M4A, Ogg Vorbis, Opus and PCM WAV. AIFF, APE, WavPack, Musepack, Speex and ALAC files are imported for metadata but cannot be played.".into(),
                "WMA/ASF, DSD and Matroska audio cannot be read or edited and are skipped during scans.".into(),
                "Audio output device selection is not implemented; playback uses the Windows default device.".into(),
            ],
        }
    }
}

fn ext_of(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// Provider ID for a local file: absolute path, lower-cased (Windows paths are
/// case-insensitive) with forward slashes normalised to backslashes.
pub fn provider_id_for(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

/// Can the built-in WebView2 audio element decode this file?
pub fn playback_support(file_type: Option<FileType>, mp4_is_alac: bool) -> Result<(), String> {
    match file_type {
        Some(FileType::Mpeg | FileType::Flac | FileType::Aac | FileType::Vorbis | FileType::Opus | FileType::Wav) => Ok(()),
        Some(FileType::Mp4) if !mp4_is_alac => Ok(()),
        Some(FileType::Mp4) => Err("ALAC (Apple Lossless) cannot be decoded by the built-in player".into()),
        Some(other) => Err(format!("{other:?} files cannot be decoded by the built-in player; metadata is still available")),
        None => Err("unknown audio format".into()),
    }
}

// ------------------------------------------------------------- reading

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileMetadata {
    pub path: String,
    pub format: String,
    pub tag_type: Option<String>,
    pub writable: bool,
    pub write_unsupported_reason: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub year: Option<i32>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub duration_ms: Option<u64>,
    pub has_artwork: bool,
    #[serde(skip)]
    pub has_front_cover: bool,
    #[serde(skip)]
    pub artwork: Option<(Vec<u8>, Option<String>)>,
    #[serde(skip)]
    pub file_type: Option<FileType>,
    #[serde(skip)]
    pub mp4_is_alac: bool,
}

fn leading_int(s: &str) -> Option<i64> {
    let digits: String = s.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn tag_type_name(t: TagType) -> String {
    format!("{t:?}")
}

fn writable_tag_type(ft: FileType) -> Option<TagType> {
    match ft {
        FileType::Mpeg | FileType::Wav | FileType::Aiff | FileType::Aac => Some(TagType::Id3v2),
        FileType::Flac | FileType::Vorbis | FileType::Opus | FileType::Speex => Some(TagType::VorbisComments),
        FileType::Mp4 => Some(TagType::Mp4Ilst),
        FileType::Ape | FileType::WavPack | FileType::Mpc => Some(TagType::Ape),
        _ => None,
    }
}

fn pick_cover(tag: &Tag) -> Option<(Vec<u8>, Option<String>)> {
    let pics = tag.pictures();
    pics.iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| pics.first())
        .map(|p| (p.data().to_vec(), p.mime_type().map(|m| m.as_str().to_string())))
}

pub fn read_file_metadata(path: &Path) -> AppResult<FileMetadata> {
    let tagged: TaggedFile = Probe::open(path)?.guess_file_type()?.read()?;
    let ft = tagged.file_type();
    let writable_type = writable_tag_type(ft);
    // Prefer the tag type we write to so edits read back consistently.
    let tag = writable_type
        .and_then(|t| tagged.tag(t))
        .or_else(|| tagged.primary_tag())
        .or_else(|| tagged.first_tag());

    let get = |k: ItemKey| tag.and_then(|t| t.get_string(k)).unwrap_or("").trim().to_string();
    let file_stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Unknown").to_string();
    let title = Some(get(ItemKey::TrackTitle)).filter(|s| !s.is_empty()).unwrap_or(file_stem);
    let year = tag
        .and_then(|t| t.get_string(ItemKey::RecordingDate).or_else(|| t.get_string(ItemKey::Year)))
        .and_then(leading_int)
        .map(|y| y as i32);
    let track_number = tag.and_then(|t| t.get_string(ItemKey::TrackNumber)).and_then(leading_int).map(|v| v as u32);
    let disc_number = tag.and_then(|t| t.get_string(ItemKey::DiscNumber)).and_then(leading_int).map(|v| v as u32);
    let artwork = tag.and_then(pick_cover).or_else(|| tagged.tags().iter().find_map(pick_cover));
    let duration = tagged.properties().duration();
    let mp4_is_alac = if ft == FileType::Mp4 {
        lofty::mp4::Mp4File::read_from(&mut File::open(path)?, ParseOptions::new().read_cover_art(false))
            .ok()
            .and_then(|f| f.properties().codec())
            .is_some_and(|c| matches!(c, lofty::mp4::Mp4Codec::ALAC))
    } else {
        false
    };

    Ok(FileMetadata {
        path: path.to_string_lossy().into_owned(),
        format: ext_of(path),
        tag_type: tag.map(|t| tag_type_name(t.tag_type())),
        writable: writable_type.is_some(),
        write_unsupported_reason: if writable_type.is_some() { None } else { Some(format!("writing tags to {ft:?} files is not supported")) },
        title,
        artist: get(ItemKey::TrackArtist),
        album: get(ItemKey::AlbumTitle),
        album_artist: get(ItemKey::AlbumArtist),
        genre: get(ItemKey::Genre),
        year,
        track_number,
        disc_number,
        duration_ms: if duration.is_zero() { None } else { Some(duration.as_millis() as u64) },
        has_artwork: artwork.is_some(),
        has_front_cover: tag.is_some_and(|t| t.pictures().iter().any(|p| p.pic_type() == PictureType::CoverFront)),
        artwork,
        file_type: Some(ft),
        mp4_is_alac,
    })
}

/// Persist embedded artwork into the cache directory (content-addressed).
pub fn cache_artwork(artwork_dir: &Path, data: &[u8], mime: Option<&str>) -> AppResult<PathBuf> {
    std::fs::create_dir_all(artwork_dir)?;
    let hash = Sha256::digest(data);
    let hex: String = hash.iter().take(16).map(|b| format!("{b:02x}")).collect();
    let ext = match mime {
        Some("image/png") => "png",
        Some("image/gif") => "gif",
        Some("image/bmp") => "bmp",
        Some("image/webp") => "webp",
        _ => "jpg",
    };
    let path = artwork_dir.join(format!("{hex}.{ext}"));
    if !path.exists() {
        std::fs::write(&path, data)?;
    }
    Ok(path)
}

fn file_stamp(path: &Path) -> (Option<u64>, Option<i64>) {
    match std::fs::metadata(path) {
        Ok(m) => (
            Some(m.len()),
            m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64),
        ),
        Err(_) => (None, None),
    }
}

/// Read a file and convert it to a library upsert.
pub fn upsert_from_file(path: &Path, artwork_dir: &Path) -> AppResult<TrackUpsert> {
    let md = read_file_metadata(path)?;
    let (size, mtime) = file_stamp(path);
    let artwork_url = match &md.artwork {
        Some((data, mime)) => cache_artwork(artwork_dir, data, mime.as_deref())
            .map(|p| p.to_string_lossy().into_owned())
            .ok(),
        None => None,
    };
    let (availability, reason) = match playback_support(md.file_type, md.mp4_is_alac) {
        Ok(()) => (Availability::Available, None),
        Err(r) => (Availability::Unavailable, Some(r)),
    };
    Ok(TrackUpsert {
        provider_id: provider_id_for(path),
        provider_uri: None,
        title: md.title,
        artist: md.artist,
        album: md.album,
        album_artist: md.album_artist,
        track_number: md.track_number,
        disc_number: md.disc_number,
        year: md.year,
        genre: md.genre,
        duration_ms: md.duration_ms,
        artwork_url,
        file_path: Some(path.to_string_lossy().into_owned()),
        file_format: Some(md.format),
        file_size: size,
        file_mtime: mtime,
        availability: Some(availability),
        unavailable_reason: reason,
        ..Default::default()
    })
}

// -------------------------------------------------------------- scanning

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub folder: String,
    pub files_seen: u32,
    pub added_or_updated: u32,
    pub unchanged: u32,
    pub missing: u32,
    pub unsupported: u32,
    pub errors: Vec<String>,
    pub unsupported_examples: Vec<String>,
}

/// Scan one folder. `progress` is called with the number of files processed.
pub fn scan_folder(
    conn: &Connection,
    folder_id: i64,
    folder: &Path,
    artwork_dir: &Path,
    mut progress: impl FnMut(u32),
) -> AppResult<ScanReport> {
    if !folder.is_dir() {
        return Err(AppError::NotFound(format!("folder {} does not exist", folder.display())));
    }
    let mut report = ScanReport { folder: folder.to_string_lossy().into_owned(), ..Default::default() };
    let folder_key = provider_id_for(folder);
    let folder_prefix = format!("{}\\", folder_key.trim_end_matches('\\'));
    let existing: HashMap<String, library::LocalFileRow> = library::local_file_rows(conn)?
        .into_iter()
        .map(|r| (provider_id_for(Path::new(&r.file_path)), r))
        .filter(|(k, _)| k.starts_with(&folder_prefix))
        .collect();
    let mut seen = std::collections::HashSet::new();

    for entry in walkdir::WalkDir::new(folder).follow_links(false).into_iter() {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                report.errors.push(e.to_string());
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let ext = ext_of(path);
        if KNOWN_UNSUPPORTED_EXTENSIONS.contains(&ext.as_str()) {
            report.unsupported += 1;
            if report.unsupported_examples.len() < 20 {
                report.unsupported_examples.push(path.to_string_lossy().into_owned());
            }
            continue;
        }
        if !SCANNED_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        report.files_seen += 1;
        let key = provider_id_for(path);
        seen.insert(key.clone());
        let (size, mtime) = file_stamp(path);
        if let Some(row) = existing.get(&key) {
            if row.file_size == size.map(|s| s as i64) && row.file_mtime == mtime {
                report.unchanged += 1;
                // File is back if it was previously marked missing.
                conn.execute(
                    "UPDATE tracks SET availability = 'available', unavailable_reason = NULL
                     WHERE id = ?1 AND unavailable_reason = 'File not found'",
                    [&row.id],
                )?;
                continue;
            }
        }
        match upsert_from_file(path, artwork_dir) {
            Ok(up) => {
                library::upsert_track(conn, Source::Local, &up, true)?;
                report.added_or_updated += 1;
            }
            Err(e) => {
                report.errors.push(format!("{}: {e}", path.display()));
            }
        }
        if report.files_seen % 25 == 0 {
            progress(report.files_seen);
        }
    }

    for (key, row) in &existing {
        if !seen.contains(key) {
            library::set_availability(conn, &row.id, Availability::Unavailable, Some("File not found"))?;
            report.missing += 1;
        }
    }
    library::mark_folder_scanned(conn, folder_id)?;
    progress(report.files_seen);
    Ok(report)
}

// --------------------------------------------------------------- writing

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum ArtworkEdit {
    /// Embed the image at `path` as the front cover (replacing any existing front cover).
    Replace { path: String },
    /// Remove the embedded front cover.
    Remove,
}

/// A metadata edit. `None` = leave unchanged, `Some("")` = remove the field.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MetadataEdit {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<String>,
    pub track_number: Option<String>,
    pub disc_number: Option<String>,
    pub artwork: Option<ArtworkEdit>,
}

impl MetadataEdit {
    fn text_fields(&self) -> Vec<(ItemKey, &Option<String>)> {
        vec![
            (ItemKey::TrackTitle, &self.title),
            (ItemKey::TrackArtist, &self.artist),
            (ItemKey::AlbumTitle, &self.album),
            (ItemKey::AlbumArtist, &self.album_artist),
            (ItemKey::Genre, &self.genre),
            (ItemKey::RecordingDate, &self.year),
            (ItemKey::TrackNumber, &self.track_number),
            (ItemKey::DiscNumber, &self.disc_number),
        ]
    }

    pub fn validate(&self) -> AppResult<()> {
        for (name, v) in [("year", &self.year), ("track number", &self.track_number), ("disc number", &self.disc_number)] {
            if let Some(v) = v.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                if !v.chars().all(|c| c.is_ascii_digit()) || v.len() > 4 {
                    return Err(AppError::InvalidInput(format!("{name} must be a number (got '{v}')")));
                }
            }
        }
        if let Some(t) = &self.title {
            if t.trim().is_empty() {
                return Err(AppError::InvalidInput("title cannot be empty".into()));
            }
        }
        if self.text_fields().iter().all(|(_, v)| v.is_none()) && self.artwork.is_none() {
            return Err(AppError::InvalidInput("no changes".into()));
        }
        Ok(())
    }
}

fn apply_to_generic(tag: &mut Tag, edit: &MetadataEdit, artwork: Option<&Picture>) {
    for (key, value) in edit.text_fields() {
        match value.as_deref().map(str::trim) {
            None => {}
            Some("") => tag.remove_key(key),
            Some(v) => {
                tag.insert_text(key, v.to_string());
            }
        }
    }
    match &edit.artwork {
        None => {}
        Some(ArtworkEdit::Remove) => tag.remove_picture_type(PictureType::CoverFront),
        Some(ArtworkEdit::Replace { .. }) => {
            tag.remove_picture_type(PictureType::CoverFront);
            if let Some(p) = artwork {
                tag.push_picture(p.clone());
            }
        }
    }
}

/// Split a concrete tag, apply the edit to the generic part, merge back.
fn edit_concrete<T>(tag: T, edit: &MetadataEdit, artwork: Option<&Picture>) -> T
where
    T: SplitTag,
    T::Remainder: MergeTag<Merged = T>,
{
    let (remainder, mut generic) = tag.split_tag();
    apply_to_generic(&mut generic, edit, artwork);
    remainder.merge_tag(generic)
}

fn load_artwork(edit: &MetadataEdit) -> AppResult<Option<Picture>> {
    let Some(ArtworkEdit::Replace { path }) = &edit.artwork else { return Ok(None) };
    let data = std::fs::read(path)?;
    if data.len() > 16 * 1024 * 1024 {
        return Err(AppError::InvalidInput("artwork image is larger than 16 MB".into()));
    }
    let mime = match data.get(..4) {
        Some([0x89, b'P', b'N', b'G']) => MimeType::Png,
        Some([0xFF, 0xD8, ..]) => MimeType::Jpeg,
        _ => return Err(AppError::InvalidInput("artwork must be a PNG or JPEG image".into())),
    };
    Ok(Some(Picture::unchecked(data).pic_type(PictureType::CoverFront).mime_type(mime).build()))
}

/// Write `edit` into the file's native tag. Does not back up or verify — see
/// [`write_metadata_safely`].
pub fn write_tags(path: &Path, edit: &MetadataEdit) -> AppResult<()> {
    let artwork = load_artwork(edit)?;
    let art = artwork.as_ref();
    let ft = Probe::open(path)?.guess_file_type()?.file_type().ok_or_else(|| AppError::Unsupported("unknown file type".into()))?;
    let opts = ParseOptions::new();
    let wopts = WriteOptions::default();
    let mut f = File::open(path)?;
    use lofty::id3::v2::Id3v2Tag;
    match ft {
        FileType::Mpeg => {
            let mut file = lofty::mpeg::MpegFile::read_from(&mut f, opts)?;
            drop(f);
            let tag = file.remove_id3v2().unwrap_or_else(Id3v2Tag::new);
            edit_concrete(tag, edit, art).save_to_path(path, wopts)?;
        }
        FileType::Aac => {
            let mut file = lofty::aac::AacFile::read_from(&mut f, opts)?;
            drop(f);
            let tag = file.remove_id3v2().unwrap_or_else(Id3v2Tag::new);
            edit_concrete(tag, edit, art).save_to_path(path, wopts)?;
        }
        FileType::Wav => {
            let mut file = lofty::iff::wav::WavFile::read_from(&mut f, opts)?;
            drop(f);
            let tag = file.remove_id3v2().unwrap_or_else(Id3v2Tag::new);
            edit_concrete(tag, edit, art).save_to_path(path, wopts)?;
        }
        FileType::Aiff => {
            let mut file = lofty::iff::aiff::AiffFile::read_from(&mut f, opts)?;
            drop(f);
            let tag = file.remove_id3v2().unwrap_or_else(Id3v2Tag::new);
            edit_concrete(tag, edit, art).save_to_path(path, wopts)?;
        }
        FileType::Flac => {
            let mut file = lofty::flac::FlacFile::read_from(&mut f, opts)?;
            drop(f);
            // lofty's FLAC writer replaces *all* PICTURE blocks with the
            // pictures held by the tag being saved. FlacFile keeps pictures
            // outside the Vorbis comments, so carry them over or they'd be lost.
            let pictures = file.pictures().to_vec();
            let mut tag = file.remove_vorbis_comments().unwrap_or_default();
            for (pic, info) in pictures {
                tag.insert_picture(pic, Some(info))?;
            }
            edit_concrete(tag, edit, art).save_to_path(path, wopts)?;
        }
        FileType::Vorbis => {
            let mut file = lofty::ogg::VorbisFile::read_from(&mut f, opts)?;
            drop(f);
            edit_concrete(file.remove_vorbis_comments(), edit, art).save_to_path(path, wopts)?;
        }
        FileType::Opus => {
            let mut file = lofty::ogg::OpusFile::read_from(&mut f, opts)?;
            drop(f);
            edit_concrete(file.remove_vorbis_comments(), edit, art).save_to_path(path, wopts)?;
        }
        FileType::Speex => {
            let mut file = lofty::ogg::SpeexFile::read_from(&mut f, opts)?;
            drop(f);
            edit_concrete(file.remove_vorbis_comments(), edit, art).save_to_path(path, wopts)?;
        }
        FileType::Mp4 => {
            let mut file = lofty::mp4::Mp4File::read_from(&mut f, opts)?;
            drop(f);
            let tag = file.remove_ilst().unwrap_or_default();
            edit_concrete(tag, edit, art).save_to_path(path, wopts)?;
        }
        FileType::Ape => {
            let mut file = lofty::ape::ApeFile::read_from(&mut f, opts)?;
            drop(f);
            let tag = file.remove_ape().unwrap_or_default();
            edit_concrete(tag, edit, art).save_to_path(path, wopts)?;
        }
        FileType::WavPack => {
            let mut file = lofty::wavpack::WavPackFile::read_from(&mut f, opts)?;
            drop(f);
            let tag = file.remove_ape().unwrap_or_default();
            edit_concrete(tag, edit, art).save_to_path(path, wopts)?;
        }
        FileType::Mpc => {
            let mut file = lofty::musepack::MpcFile::read_from(&mut f, opts)?;
            drop(f);
            let tag = file.remove_ape().unwrap_or_default();
            edit_concrete(tag, edit, art).save_to_path(path, wopts)?;
        }
        other => return Err(AppError::Unsupported(format!("writing tags to {other:?} files is not supported"))),
    }
    Ok(())
}

/// Compare an edit against freshly read metadata. Returns mismatching field names.
pub fn verify_edit(md: &FileMetadata, edit: &MetadataEdit, expected_art: Option<&[u8]>) -> Vec<String> {
    let mut bad = Vec::new();
    let mut text = |name: &str, want: &Option<String>, got: &str| {
        if let Some(w) = want {
            if w.trim() != got {
                bad.push(name.to_string());
            }
        }
    };
    text("artist", &edit.artist, &md.artist);
    text("album", &edit.album, &md.album);
    text("albumArtist", &edit.album_artist, &md.album_artist);
    text("genre", &edit.genre, &md.genre);
    if let Some(t) = &edit.title {
        if t.trim() != md.title {
            bad.push("title".into());
        }
    }
    let mut num = |name: &str, want: &Option<String>, got: Option<i64>| {
        if let Some(w) = want {
            let w = w.trim();
            let want_v = if w.is_empty() { None } else { w.parse::<i64>().ok() };
            if want_v != got {
                bad.push(name.to_string());
            }
        }
    };
    num("year", &edit.year, md.year.map(i64::from));
    num("trackNumber", &edit.track_number, md.track_number.map(i64::from));
    num("discNumber", &edit.disc_number, md.disc_number.map(i64::from));
    match (&edit.artwork, expected_art) {
        // Other picture types (back cover, artist...) may legitimately remain.
        (Some(ArtworkEdit::Remove), _) if md.has_front_cover => bad.push("artwork".into()),
        (Some(ArtworkEdit::Replace { .. }), Some(data)) => {
            if md.artwork.as_ref().map(|(d, _)| d.as_slice()) != Some(data) {
                bad.push("artwork".into());
            }
        }
        _ => {}
    }
    bad
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WriteReport {
    pub track_id: String,
    pub backup_id: i64,
    pub backup_path: String,
    pub verified: bool,
    pub metadata: FileMetadata,
}

/// Back up → write → read back → verify → (restore on failure) → update DB.
pub fn write_metadata_safely(
    conn: &Connection,
    track_id: &str,
    edit: &MetadataEdit,
    backup_dir: &Path,
    artwork_dir: &Path,
) -> AppResult<WriteReport> {
    edit.validate()?;
    let track = library::get_track(conn, track_id)?;
    if track.source != Source::Local {
        return Err(AppError::Unsupported("only local files can have their tags written; use app overrides for streaming tracks".into()));
    }
    let path = PathBuf::from(track.file_path.clone().ok_or_else(|| AppError::NotFound("track has no file path".into()))?);
    if !path.is_file() {
        return Err(AppError::NotFound(format!("{} no longer exists", path.display())));
    }
    let before = read_file_metadata(&path)?;
    if !before.writable {
        return Err(AppError::Unsupported(before.write_unsupported_reason.unwrap_or_else(|| "format not writable".into())));
    }
    let expected_art = load_artwork(edit)?.map(|p| p.data().to_vec());

    // 1. Backup
    std::fs::create_dir_all(backup_dir)?;
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let backup_path = backup_dir.join(format!("{stamp}-{}-{fname}", &track_id[..8.min(track_id.len())]));
    std::fs::copy(&path, &backup_path)?;
    let backup_id = library::insert_backup(conn, track_id, &path.to_string_lossy(), &backup_path.to_string_lossy())?;

    // 2. Write
    if let Err(e) = write_tags(&path, edit) {
        restore_file(&backup_path, &path)?;
        return Err(e);
    }

    // 3. Verify by reading back
    let after = match read_file_metadata(&path) {
        Ok(m) => m,
        Err(e) => {
            restore_file(&backup_path, &path)?;
            return Err(AppError::VerificationFailed(format!("file could not be re-read after writing ({e}); original restored")));
        }
    };
    let mismatches = verify_edit(&after, edit, expected_art.as_deref());
    if !mismatches.is_empty() {
        restore_file(&backup_path, &path)?;
        return Err(AppError::VerificationFailed(format!(
            "fields did not read back as written: {}; original file restored",
            mismatches.join(", ")
        )));
    }

    // 4. Update library row from the file on disk
    let up = upsert_from_file(&path, artwork_dir)?;
    library::upsert_track(conn, Source::Local, &up, true)?;
    Ok(WriteReport {
        track_id: track_id.to_string(),
        backup_id,
        backup_path: backup_path.to_string_lossy().into_owned(),
        verified: true,
        metadata: after,
    })
}

fn restore_file(backup: &Path, target: &Path) -> AppResult<()> {
    std::fs::copy(backup, target).map(|_| ()).map_err(|e| {
        AppError::Other(format!(
            "CRITICAL: could not restore backup {} to {} ({e}). Your original file is preserved at the backup path.",
            backup.display(),
            target.display()
        ))
    })
}

pub fn restore_backup(conn: &Connection, backup_id: i64, artwork_dir: &Path) -> AppResult<()> {
    let b = library::get_backup(conn, backup_id)?;
    let backup = Path::new(&b.backup_path);
    if !backup.is_file() {
        return Err(AppError::NotFound(format!("backup file {} is missing", backup.display())));
    }
    restore_file(backup, Path::new(&b.file_path))?;
    let up = upsert_from_file(Path::new(&b.file_path), artwork_dir)?;
    library::upsert_track(conn, Source::Local, &up, true)?;
    Ok(())
}

/// Copy a user-chosen image into the app artwork directory for use as an
/// application-level artwork override (never touches the audio file).
pub fn import_override_artwork(src: &Path, artwork_dir: &Path) -> AppResult<PathBuf> {
    let data = std::fs::read(src)?;
    let mime = match data.get(..4) {
        Some([0x89, b'P', b'N', b'G']) => "image/png",
        Some([0xFF, 0xD8, ..]) => "image/jpeg",
        Some([b'R', b'I', b'F', b'F']) => "image/webp",
        _ => return Err(AppError::InvalidInput("artwork must be a PNG, JPEG or WebP image".into())),
    };
    cache_artwork(artwork_dir, &data, Some(mime))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::db::open_in_memory;

    /// 50 frames of MPEG-1 Layer III 128 kbps 44.1 kHz (silent payload).
    pub fn make_mp3(path: &Path) {
        let mut data = Vec::new();
        for _ in 0..50 {
            data.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x44]);
            data.extend(std::iter::repeat_n(0u8, 413));
        }
        std::fs::write(path, data).unwrap();
    }

    /// 1 s of 8 kHz mono 16-bit PCM silence.
    pub fn make_wav(path: &Path) {
        let samples = 8000u32;
        let data_len = samples * 2;
        let mut d = Vec::new();
        d.extend_from_slice(b"RIFF");
        d.extend_from_slice(&(36 + data_len).to_le_bytes());
        d.extend_from_slice(b"WAVEfmt ");
        d.extend_from_slice(&16u32.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes()); // PCM
        d.extend_from_slice(&1u16.to_le_bytes()); // mono
        d.extend_from_slice(&8000u32.to_le_bytes());
        d.extend_from_slice(&16000u32.to_le_bytes());
        d.extend_from_slice(&2u16.to_le_bytes());
        d.extend_from_slice(&16u16.to_le_bytes());
        d.extend_from_slice(b"data");
        d.extend_from_slice(&data_len.to_le_bytes());
        d.extend(std::iter::repeat_n(0u8, data_len as usize));
        std::fs::write(path, d).unwrap();
    }

    /// Minimal FLAC: STREAMINFO only (1 s, 44.1 kHz, stereo, 16-bit).
    pub fn make_flac(path: &Path) {
        let mut d = Vec::new();
        d.extend_from_slice(b"fLaC");
        d.extend_from_slice(&[0x80, 0x00, 0x00, 34]); // last block, STREAMINFO, len 34
        d.extend_from_slice(&4096u16.to_be_bytes());
        d.extend_from_slice(&4096u16.to_be_bytes());
        d.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // min/max frame size
        let packed: u64 = (44100u64 << 44) | (1u64 << 41) | (15u64 << 36) | 44100u64;
        d.extend_from_slice(&packed.to_be_bytes());
        d.extend_from_slice(&[0u8; 16]); // MD5
        std::fs::write(path, d).unwrap();
    }

    /// 1×1 PNG.
    pub const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00,
        0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00,
        0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0, 0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF,
        0x89, 0x99, 0x3D, 0x1D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    fn edit_all() -> MetadataEdit {
        MetadataEdit {
            title: Some("New Title".into()),
            artist: Some("New Artist".into()),
            album: Some("New Album".into()),
            album_artist: Some("Album Artist".into()),
            genre: Some("Jazz".into()),
            year: Some("1999".into()),
            track_number: Some("7".into()),
            disc_number: Some("2".into()),
            artwork: None,
        }
    }

    #[test]
    fn write_and_read_back_each_format() {
        let dir = tempfile::tempdir().unwrap();
        for (name, make) in [("a.mp3", make_mp3 as fn(&Path)), ("b.wav", make_wav), ("c.flac", make_flac)] {
            let p = dir.path().join(name);
            make(&p);
            write_tags(&p, &edit_all()).unwrap_or_else(|e| panic!("{name}: {e}"));
            let md = read_file_metadata(&p).unwrap();
            assert!(verify_edit(&md, &edit_all(), None).is_empty(), "{name}: {md:?}");
            assert_eq!(md.title, "New Title");
            assert_eq!(md.year, Some(1999));
            assert_eq!(md.track_number, Some(7));
        }
    }

    #[test]
    fn edit_preserves_unrelated_frames_and_fields() {
        use lofty::id3::v2::{Frame, FrameId, Id3v2Tag, UnsynchronizedTextFrame};
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("keep.mp3");
        make_mp3(&p);
        // Seed: a comment-like custom text frame (TXXX) and a lyrics frame (USLT)
        let mut tag = Id3v2Tag::new();
        tag.set_title("Old".into());
        tag.set_artist("Keep Me".into());
        tag.insert_user_text("MY_CUSTOM".into(), "custom value".into());
        tag.insert(Frame::UnsynchronizedText(UnsynchronizedTextFrame::new(
            lofty::TextEncoding::UTF8,
            *b"eng",
            String::new(),
            "la la la".to_string(),
        )));
        tag.save_to_path(&p, WriteOptions::default()).unwrap();

        let edit = MetadataEdit { title: Some("Changed".into()), ..Default::default() };
        write_tags(&p, &edit).unwrap();

        let mut f = File::open(&p).unwrap();
        let file = lofty::mpeg::MpegFile::read_from(&mut f, ParseOptions::new()).unwrap();
        let t = file.id3v2().unwrap();
        use lofty::prelude::Accessor;
        assert_eq!(t.title().as_deref(), Some("Changed"));
        assert_eq!(t.artist().as_deref(), Some("Keep Me"));
        assert_eq!(t.get_user_text("MY_CUSTOM"), Some("custom value"));
        assert!(t.get(&FrameId::Valid("USLT".into())).is_some(), "USLT frame was dropped");
    }

    #[test]
    fn flac_edit_preserves_existing_pictures() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("pic.flac");
        let art = dir.path().join("cover.png");
        std::fs::write(&art, PNG).unwrap();
        make_flac(&p);
        write_tags(&p, &MetadataEdit { artwork: Some(ArtworkEdit::Replace { path: art.to_string_lossy().into() }), ..Default::default() }).unwrap();
        assert!(read_file_metadata(&p).unwrap().has_artwork);
        // An unrelated text edit must not drop the PICTURE block.
        write_tags(&p, &MetadataEdit { title: Some("T".into()), ..Default::default() }).unwrap();
        let md = read_file_metadata(&p).unwrap();
        assert!(md.has_artwork, "FLAC picture lost after text edit");
        assert_eq!(md.artwork.unwrap().0, PNG);
    }

    #[test]
    fn safe_write_backs_up_verifies_and_updates_db() {
        let dir = tempfile::tempdir().unwrap();
        let music = dir.path().join("music");
        std::fs::create_dir_all(&music).unwrap();
        let p = music.join("song.mp3");
        make_mp3(&p);
        let conn = open_in_memory().unwrap();
        let folder = library::add_folder(&conn, &music.to_string_lossy()).unwrap();
        let art_dir = dir.path().join("art");
        let rep = scan_folder(&conn, folder.id, &music, &art_dir, |_| {}).unwrap();
        assert_eq!(rep.added_or_updated, 1);
        let track_id = library::find_by_provider_id(&conn, Source::Local, &provider_id_for(&p)).unwrap().unwrap();
        let original = std::fs::read(&p).unwrap();

        let cover = dir.path().join("c.png");
        std::fs::write(&cover, PNG).unwrap();
        let mut edit = edit_all();
        edit.artwork = Some(ArtworkEdit::Replace { path: cover.to_string_lossy().into() });
        let report = write_metadata_safely(&conn, &track_id, &edit, &dir.path().join("bk"), &art_dir).unwrap();
        assert!(report.verified);
        assert_eq!(std::fs::read(&report.backup_path).unwrap(), original, "backup must be the pre-edit file");
        let t = library::get_track(&conn, &track_id).unwrap();
        assert_eq!(t.title, "New Title");
        assert!(t.artwork.is_some());

        // Restore brings back the original bytes and metadata.
        restore_backup(&conn, report.backup_id, &art_dir).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), original);
        assert_eq!(library::get_track(&conn, &track_id).unwrap().title, "song");
    }

    #[test]
    fn rejects_invalid_edits_and_non_local_tracks() {
        assert!(MetadataEdit { year: Some("19x9".into()), ..Default::default() }.validate().is_err());
        assert!(MetadataEdit { title: Some(" ".into()), ..Default::default() }.validate().is_err());
        assert!(MetadataEdit::default().validate().is_err());
        let conn = open_in_memory().unwrap();
        let id = library::upsert_track(&conn, Source::Spotify, &crate::db::library::tests::sample("x", "t", "a", "b"), true).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let err = write_metadata_safely(&conn, &id, &edit_all(), dir.path(), dir.path()).unwrap_err();
        assert!(matches!(err, AppError::Unsupported(_)));
    }

    #[test]
    fn scan_handles_unsupported_corrupt_and_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let music = dir.path().join("m");
        std::fs::create_dir_all(music.join("sub")).unwrap();
        make_mp3(&music.join("ok.mp3"));
        make_wav(&music.join("sub").join("ok.wav"));
        std::fs::write(music.join("legacy.wma"), b"not really").unwrap();
        std::fs::write(music.join("broken.flac"), b"garbage").unwrap();
        std::fs::write(music.join("notes.txt"), b"ignored").unwrap();
        let conn = open_in_memory().unwrap();
        let folder = library::add_folder(&conn, &music.to_string_lossy()).unwrap();
        let art = dir.path().join("art");

        let r = scan_folder(&conn, folder.id, &music, &art, |_| {}).unwrap();
        assert_eq!(r.files_seen, 3);
        assert_eq!(r.added_or_updated, 2);
        assert_eq!(r.unsupported, 1);
        assert_eq!(r.errors.len(), 1, "{:?}", r.errors);

        // Second scan: nothing changed.
        let r = scan_folder(&conn, folder.id, &music, &art, |_| {}).unwrap();
        assert_eq!(r.unchanged, 2);

        // Delete a file: it is marked unavailable, not silently dropped.
        std::fs::remove_file(music.join("ok.mp3")).unwrap();
        let r = scan_folder(&conn, folder.id, &music, &art, |_| {}).unwrap();
        assert_eq!(r.missing, 1);
        let id = library::find_by_provider_id(&conn, Source::Local, &provider_id_for(&music.join("ok.mp3"))).unwrap().unwrap();
        assert_eq!(library::get_track(&conn, &id).unwrap().availability, Availability::Unavailable);
    }

    #[test]
    fn playback_support_matrix() {
        assert!(playback_support(Some(FileType::Mpeg), false).is_ok());
        assert!(playback_support(Some(FileType::Mp4), false).is_ok());
        assert!(playback_support(Some(FileType::Mp4), true).is_err());
        assert!(playback_support(Some(FileType::Aiff), false).is_err());
        assert!(playback_support(Some(FileType::Ape), false).is_err());
    }
}
