//! Two-way Spotify playlist sync.
//!
//! * `merge` — pure three-way merge and push planning (unit-tested).
//! * `enable` — link a Spotify playlist to a Branflakes playlist.
//! * `sync_playlist` — one reconciliation pass for one playlist.
//! * `add_tracks` — the playlist add rule: non-Spotify tracks added to a
//!   synced playlist create a Branflakes-only copy instead.
//! * `SyncEngine` — the backend scheduler (default every 5 minutes, plus
//!   debounced syncs after local edits and manual "sync now").
//!
//! Safety rules:
//! * Nothing is removed on either side unless the base (last agreed state)
//!   shows that the other side removed it, and only after reading Spotify's
//!   complete item list (a short read is an error, never a removal).
//! * Every pass is idempotent: an interrupted pass is simply repeated, and
//!   a pass after a successful one changes nothing (no sync loops).
//! * Local edits made while a pass is running are never overwritten: the
//!   local write checks the playlist is unchanged, otherwise the pass reruns.

pub mod merge;

use crate::db::{library, now_iso, playlists, settings, spotify_links as links};
use crate::error::{AppError, AppResult};
use crate::model::Source;
use crate::providers::local::cache_artwork;
use crate::providers::spotify::{self, PlaylistMeta, RemoteItem, SpotifyAdapter};
use merge::Op;
use rusqlite::Connection;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const DEFAULT_INTERVAL_SECS: u64 = 300;
pub const MIN_INTERVAL_SECS: u64 = 60;
/// Wait after a local edit before pushing, so a burst of edits is one sync.
pub const EDIT_DEBOUNCE: Duration = Duration::from_secs(3);
/// How many times a pass reruns when the playlist changes underneath it.
const MAX_PASSES: usize = 3;

/// Everything a sync pass needs. Kept free of Tauri so it can be tested.
pub struct SyncCtx<'a> {
    pub db: &'a Mutex<Connection>,
    pub spotify: &'a SpotifyAdapter,
    pub client_id: String,
    pub artwork_dir: PathBuf,
}

impl SyncCtx<'_> {
    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub playlist_id: String,
    /// Items added to / removed from Branflakes and Spotify by this pass.
    pub pulled_added: u32,
    pub pulled_removed: u32,
    pub pushed_ops: u32,
    pub renamed: bool,
    pub artwork_changed: bool,
    /// Human-readable notes (conflicts, limitations) for the UI.
    pub notices: Vec<String>,
    /// Set when the Spotify playlist was confirmed deleted or removed from
    /// the user's library and Branflakes removed its synced copy.
    pub removed: Option<String>,
}

impl SyncReport {
    pub fn changed_locally(&self) -> bool {
        self.pulled_added > 0 || self.pulled_removed > 0 || self.renamed || self.artwork_changed || self.removed.is_some()
    }
}

fn spotify_uri(t: &crate::model::Track) -> Option<String> {
    if t.source != Source::Spotify {
        return None;
    }
    Some(match &t.provider_uri {
        Some(u) if u.starts_with("spotify:track:") => u.clone(),
        _ => format!("spotify:track:{}", t.provider_id),
    })
}

/// Can this Branflakes track be added to a Spotify playlist?
pub fn is_spotify_compatible(t: &crate::model::Track) -> bool {
    spotify_uri(t).is_some()
}

/// Identity of a Spotify cover, from its URL. Spotify image ids are part of
/// the path (and change when the image changes, as far as observed), while
/// the host, size and query vary between responses. So the key is the path
/// without host or query, and for mosaic covers also without the size
/// segment. Not guaranteed by Spotify's docs: a manual sync additionally
/// compares the downloaded bytes (see `pass`).
pub fn image_key(url: &str) -> String {
    let no_query = url.split(['?', '#']).next().unwrap_or(url);
    let Ok(u) = url::Url::parse(no_query) else { return no_query.to_string() };
    let path = u.path();
    if u.host_str().is_some_and(|h| h.starts_with("mosaic.")) {
        // mosaic.scdn.co/<size>/<ids…>
        let mut segs = path.trim_start_matches('/').splitn(2, '/');
        let first = segs.next().unwrap_or("");
        if let (true, Some(rest)) = (first.chars().all(|c| c.is_ascii_digit()), segs.next()) {
            return format!("mosaic/{rest}");
        }
    }
    path.to_string()
}

fn permission_error(e: AppError, what: &str) -> AppError {
    match e {
        AppError::Http { status: 403, .. } => AppError::Unsupported(format!(
            "Spotify refused to {what}: you need to own this playlist (or be a collaborator for track changes)"
        )),
        AppError::Http { status: 404, .. } => {
            AppError::NotFound("the Spotify playlist no longer exists or is no longer shared with you".into())
        }
        other => other,
    }
}

fn require_write_scopes(sp: &SpotifyAdapter) -> AppResult<()> {
    let missing = sp.missing_granted_scopes(spotify::PLAYLIST_WRITE_SCOPES);
    if missing.is_empty() {
        Ok(())
    } else {
        Err(AppError::Auth {
            provider: "spotify".into(),
            message: format!(
                "your Spotify sign-in doesn't allow changing playlists ({}). Go to Settings > Accounts and click Reconnect Spotify",
                missing.join(", ")
            ),
        })
    }
}

/// Spotify user id of the signed-in account (cached in settings).
pub async fn my_user_id(ctx: &SyncCtx<'_>) -> AppResult<String> {
    if let Some(id) = settings::get_nonempty(&ctx.conn(), settings::SPOTIFY_USER_ID)? {
        return Ok(id);
    }
    let (id, _) = ctx.spotify.me(&ctx.client_id).await?;
    if id.is_empty() {
        return Err(AppError::Auth { provider: "spotify".into(), message: "could not read your Spotify user id".into() });
    }
    settings::set(&ctx.conn(), settings::SPOTIFY_USER_ID, &id)?;
    Ok(id)
}

/// Owner can do everything; a collaborator can change items only.
fn access(meta: &PlaylistMeta, me: &str) -> AppResult<bool> {
    if meta.owner_id.as_deref() == Some(me) {
        Ok(true)
    } else if meta.collaborative {
        Ok(false)
    } else {
        Err(AppError::Unsupported(format!(
            "“{}” can't be synced: it belongs to another Spotify user. Spotify only lets apps change playlists you own or collaborate on. You can still play it, or copy it into a Branflakes playlist.",
            meta.name
        )))
    }
}

fn upsert_remote_tracks(conn: &Connection, items: &[RemoteItem]) -> AppResult<HashMap<String, String>> {
    let tx = conn.unchecked_transaction()?;
    let mut map = HashMap::new();
    for it in items {
        if let Some(t) = &it.track {
            let id = library::upsert_track(&tx, Source::Spotify, t, false)?;
            map.insert(it.uri.clone(), id);
        }
    }
    tx.commit()?;
    Ok(map)
}

/// Download Spotify's current cover into the artwork cache.
async fn pull_cover(ctx: &SyncCtx<'_>, url: &str) -> AppResult<String> {
    let bytes = ctx.spotify.fetch_image(url).await?;
    let mime = match bytes.get(..4) {
        Some([0x89, b'P', b'N', b'G']) => Some("image/png"),
        Some([b'R', b'I', b'F', b'F']) => Some("image/webp"),
        Some([0xFF, 0xD8, ..]) => Some("image/jpeg"),
        _ => return Err(AppError::InvalidInput("Spotify returned a cover in an unsupported format".into())),
    };
    Ok(cache_artwork(&ctx.artwork_dir, &bytes, mime)?.to_string_lossy().into_owned())
}

/// Re-encode a Branflakes cover as a base64 JPEG within Spotify's 256 KB
/// payload limit (shrinking and lowering quality as needed).
pub fn cover_jpeg_base64(path: &Path) -> AppResult<String> {
    use base64::Engine;
    use image::codecs::jpeg::JpegEncoder;
    let img = image::open(path).map_err(|e| AppError::InvalidInput(format!("can't read the cover image: {e}")))?;
    let mut size = 640u32.min(img.width().max(img.height()).max(1));
    loop {
        let scaled = img.thumbnail(size, size).to_rgb8();
        for quality in [90u8, 80, 70, 60, 50] {
            let mut buf = Vec::new();
            JpegEncoder::new_with_quality(&mut buf, quality)
                .encode_image(&scaled)
                .map_err(|e| AppError::Other(format!("could not encode the cover as JPEG: {e}")))?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
            if b64.len() <= spotify::COVER_MAX_BASE64 {
                return Ok(b64);
            }
        }
        if size <= 160 {
            return Err(AppError::InvalidInput("cover image can't be made small enough for Spotify".into()));
        }
        size = size * 3 / 4;
    }
}

// ================================================================ enable

/// Link a Spotify playlist and make Branflakes match it. If Branflakes
/// already has an imported copy, that copy becomes the synced playlist.
/// Returns the Branflakes playlist id.
pub async fn enable(ctx: &SyncCtx<'_>, spotify_playlist_id: &str) -> AppResult<String> {
    require_write_scopes(ctx.spotify)?;
    let me = my_user_id(ctx).await?;
    let meta = ctx.spotify.playlist_meta(&ctx.client_id, spotify_playlist_id).await.map_err(|e| permission_error(e, "share this playlist"))?;
    let can_edit_details = access(&meta, &me)?;
    let items = ctx.spotify.playlist_items_all(&ctx.client_id, spotify_playlist_id).await.map_err(|e| permission_error(e, "read this playlist"))?;
    let cover = match meta.images.first() {
        Some(url) => match pull_cover(ctx, url).await {
            Ok(p) => Some((image_key(url), p)),
            Err(e) => {
                log::info!("spotify sync: cover not imported: {e}");
                None
            }
        },
        None => None,
    };

    let conn = ctx.conn();
    let origin = format!("spotify:playlist:{spotify_playlist_id}");
    let ids = upsert_remote_tracks(&conn, &items)?;
    let tx = conn.unchecked_transaction()?;
    let pid = match playlists::find_by_origin(&tx, &origin)? {
        Some(pid) => {
            let existing = playlists::tracks(&tx, &pid)?;
            let foreign = existing.iter().filter(|t| !is_spotify_compatible(t)).count();
            if foreign > 0 {
                return Err(AppError::InvalidInput(format!(
                    "this playlist has {foreign} non-Spotify track(s) in Branflakes, which can't be added to Spotify. Remove them first, or keep the playlist Branflakes-only"
                )));
            }
            pid
        }
        None => playlists::create(&tx, &meta.name, Some(&origin))?.id,
    };
    if let Some(other) = links::find_by_spotify_id(&tx, spotify_playlist_id)? {
        if other.playlist_id != pid {
            return Err(AppError::InvalidInput("this Spotify playlist is already synced with another Branflakes playlist".into()));
        }
    }
    // Spotify is the starting point: imported copies were always replaced
    // on re-import, so this matches what the user already had.
    let uris: Vec<String> = items.iter().map(|i| i.uri.clone()).collect();
    let local_ids: Vec<String> = uris.iter().filter_map(|u| ids.get(u).cloned()).collect();
    playlists::write_items(&tx, &pid, &local_ids)?;
    playlists::rename(&tx, &pid, &meta.name)?;
    if let Some((_, path)) = &cover {
        playlists::set_artwork(&tx, &pid, Some(path))?;
    }
    let artwork_now = playlists::get(&tx, &pid)?.artwork_path;
    links::upsert(
        &tx,
        &links::Link {
            playlist_id: pid.clone(),
            spotify_playlist_id: spotify_playlist_id.to_string(),
            enabled: true,
            owner_id: meta.owner_id.clone(),
            can_edit_details,
            snapshot_id: meta.snapshot_id.clone(),
            base_name: meta.name.clone(),
            base_items: uris,
            base_image_key: cover.as_ref().map(|(k, _)| k.clone()),
            base_artwork_path: artwork_now,
            artwork_failed_path: None,
            status: "ok".into(),
            last_synced_at: Some(now_iso()),
            last_error: None,
            last_notice: None,
        },
    )?;
    tx.commit()?;
    // Old imports of the same playlist must not linger as duplicates.
    remove_duplicates_of(&conn, &pid)?;
    Ok(pid)
}

pub fn disable(conn: &Connection, playlist_id: &str) -> AppResult<()> {
    links::delete(conn, playlist_id)
}

// ============================================================= add rule

/// A Spotify playlist entry that a Branflakes-only copy can't contain.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UncopyableEntry {
    /// "spotify_local_file" | "episode" | "unavailable"
    pub kind: String,
    pub name: Option<String>,
}

fn uncopyable_kind(uri: &str) -> &'static str {
    if uri.starts_with("spotify:local:") {
        "spotify_local_file"
    } else if uri.starts_with("spotify:episode:") {
        "episode"
    } else {
        "unavailable"
    }
}

/// The contents a Branflakes-only copy starts from.
#[derive(Debug, Clone, PartialEq)]
pub struct ForkSource {
    /// Branflakes track ids, in order.
    pub track_ids: Vec<String>,
    pub uncopyable: Vec<UncopyableEntry>,
    /// True when based on Spotify's current contents (reconciled with any
    /// unsynced Branflakes edits); false when based on the last sync.
    pub latest: bool,
    /// Why the latest contents could not be used.
    pub fallback_reason: Option<String>,
    pub last_synced_at: Option<String>,
}

/// Fallback: the synced playlist as Branflakes has it (last sync plus any
/// unsynced Branflakes edits). Never touches the network.
pub fn fork_source_last_sync(conn: &Connection, playlist_id: &str, reason: String) -> AppResult<ForkSource> {
    let link = links::get_enabled(conn, playlist_id)?;
    let uncopyable = link
        .as_ref()
        .map(|l| l.base_items.iter().filter(|u| !merge::is_track(u)).map(|u| UncopyableEntry { kind: uncopyable_kind(u).into(), name: None }).collect())
        .unwrap_or_default();
    Ok(ForkSource {
        track_ids: playlists::track_ids(conn, playlist_id)?,
        uncopyable,
        latest: false,
        fallback_reason: Some(reason),
        last_synced_at: link.and_then(|l| l.last_synced_at),
    })
}

/// Spotify's current contents merged with unsynced Branflakes edits, exactly
/// as the next sync would produce them, computed in memory. Reads Spotify;
/// writes nothing to Spotify, to the synced playlist or to its sync state
/// (only track metadata is cached, as search results are).
pub async fn fork_source_latest(ctx: &SyncCtx<'_>, playlist_id: &str) -> AppResult<ForkSource> {
    let (link, local) = {
        let conn = ctx.conn();
        read_local(&conn, playlist_id)?
    };
    let items = ctx
        .spotify
        .playlist_items_all(&ctx.client_id, &link.spotify_playlist_id)
        .await
        .map_err(|e| permission_error(e, "read this playlist"))?;
    let remote: Vec<String> = items.iter().map(|i| i.uri.clone()).collect();
    let names: HashMap<&str, Option<String>> = items.iter().map(|i| (i.uri.as_str(), i.name.clone())).collect();
    let m = merge::merge(&link.base_items, &local.uris, &remote);
    let conn = ctx.conn();
    let mut uri_to_id: HashMap<String, String> = local.uris.iter().cloned().zip(local.spotify_ids.iter().cloned()).collect();
    uri_to_id.extend(upsert_remote_tracks(&conn, &items)?);
    let mut track_ids = Vec::new();
    let mut uncopyable = Vec::new();
    for u in &m.merged {
        match (merge::is_track(u), uri_to_id.get(u)) {
            (true, Some(id)) => track_ids.push(id.clone()),
            _ => uncopyable.push(UncopyableEntry { kind: uncopyable_kind(u).into(), name: names.get(u.as_str()).cloned().flatten() }),
        }
    }
    let track_ids = crate::db::alternatives::reinsert(&track_ids, &local.ids, &local.alts);
    Ok(ForkSource { track_ids, uncopyable, latest: true, fallback_reason: None, last_synced_at: link.last_synced_at })
}

/// Text stored on the copy so its page never implies more than it holds.
pub fn fork_note(source_name: &str, src: &ForkSource) -> String {
    let mut parts = Vec::new();
    if src.latest {
        parts.push(format!("Copied from “{source_name}” using Spotify's current version of the playlist."));
    } else {
        let when = src.last_synced_at.as_deref().map(|t| format!(" (last synced {})", t.get(..16).unwrap_or(t).replace('T', " "))).unwrap_or_default();
        parts.push(format!(
            "Copied from “{source_name}” as Branflakes last synced it{when}, because Spotify couldn't be checked: {}. Changes made on Spotify since then are not included.",
            src.fallback_reason.as_deref().unwrap_or("unknown reason")
        ));
    }
    if !src.uncopyable.is_empty() {
        let count = |k: &str| src.uncopyable.iter().filter(|e| e.kind == k).count();
        let mut kinds = Vec::new();
        for (k, one, many) in [("episode", "podcast episode", "podcast episodes"), ("spotify_local_file", "Spotify local file", "Spotify local files"), ("unavailable", "unavailable Spotify entry", "unavailable Spotify entries")] {
            match count(k) {
                0 => {}
                1 => kinds.push(format!("1 {one}")),
                n => kinds.push(format!("{n} {many}")),
            }
        }
        let named: Vec<String> = src.uncopyable.iter().filter_map(|e| e.name.as_ref().map(|n| format!("“{n}”"))).take(5).collect();
        let names = if named.is_empty() { String::new() } else { format!(": {}", named.join(", ")) };
        parts.push(format!(
            "Not copied: {} ({}){names}. Branflakes can't hold these, so this copy is not complete.",
            src.uncopyable.len(),
            kinds.join(", ")
        ));
    }
    parts.join(" ")
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum AddOutcome {
    /// Added to the requested playlist.
    Added { added: u32, skipped_duplicates: u32 },
    /// The playlist is Spotify Synced and a track isn't on Spotify: a
    /// Branflakes-only copy was created instead; the original is untouched.
    Forked {
        playlist: playlists::Playlist,
        added: u32,
        not_on_spotify: u32,
        /// Spotify entries that are not in the copy.
        uncopyable: Vec<UncopyableEntry>,
        /// Based on Spotify's current contents (false = last sync).
        from_latest: bool,
        note: String,
    },
}

/// Does adding these tracks require a Branflakes-only copy?
pub fn needs_fork(conn: &Connection, playlist_id: &str, track_ids: &[String]) -> AppResult<bool> {
    if track_ids.is_empty() || links::get_enabled(conn, playlist_id)?.is_none() {
        return Ok(false);
    }
    Ok(library::get_tracks(conn, track_ids)?.iter().any(|t| !is_spotify_compatible(t)))
}

/// Where `position` (an index into the synced playlist as the user saw it)
/// lands in the copy: right after the track the user inserted after.
fn fork_position(shown: &[String], copy: &[String], position: Option<usize>) -> usize {
    match position {
        None => copy.len(),
        Some(0) => 0,
        Some(p) => match shown.get(p.min(shown.len()) - 1).and_then(|prev| copy.iter().position(|x| x == prev)) {
            Some(i) => i + 1,
            None => p.min(copy.len()),
        },
    }
}

/// The single backend entry point for adding tracks to a playlist. Every UI
/// path (menus, dialogs, multi-select) goes through this. `fork` is the
/// contents to copy when a Branflakes-only copy is needed; `None` falls
/// back to the last sync (used when the caller couldn't reach Spotify).
pub fn add_tracks(conn: &Connection, playlist_id: &str, track_ids: &[String], position: Option<usize>, fork: Option<ForkSource>) -> AppResult<AddOutcome> {
    let pl = playlists::get(conn, playlist_id)?;
    if track_ids.is_empty() {
        return Ok(AddOutcome::Added { added: 0, skipped_duplicates: 0 });
    }
    let new_tracks = library::get_tracks(conn, track_ids)?;
    if new_tracks.len() != track_ids.len() {
        return Err(AppError::NotFound("one or more tracks no longer exist".into()));
    }
    let Some(_link) = links::get_enabled(conn, playlist_id)? else {
        playlists::insert_tracks(conn, playlist_id, track_ids, position)?;
        return Ok(AddOutcome::Added { added: track_ids.len() as u32, skipped_duplicates: 0 });
    };
    let not_on_spotify = new_tracks.iter().filter(|t| !is_spotify_compatible(t)).count() as u32;
    if not_on_spotify > 0 {
        // Never touch the synced playlist: copy it, including the new tracks.
        let src = match fork {
            Some(f) => f,
            None => fork_source_last_sync(conn, playlist_id, "Spotify was not checked".into())?,
        };
        let shown = playlists::track_ids(conn, playlist_id)?;
        let mut items = src.track_ids.clone();
        let at = fork_position(&shown, &items, position);
        items.splice(at..at, track_ids.iter().cloned());
        let note = fork_note(&pl.name, &src);
        let copy = playlists::create_fork(conn, playlist_id, &items, Some(&note))?;
        return Ok(AddOutcome::Forked {
            playlist: copy,
            added: track_ids.len() as u32,
            not_on_spotify,
            uncopyable: src.uncopyable,
            from_latest: src.latest,
            note,
        });
    }
    // Spotify Synced: don't create duplicates (they'd sync to Spotify too).
    let existing: HashSet<String> = playlists::track_ids(conn, playlist_id)?.into_iter().collect();
    let mut seen = HashSet::new();
    let fresh: Vec<String> = track_ids.iter().filter(|id| !existing.contains(*id) && seen.insert((*id).clone())).cloned().collect();
    let skipped = (track_ids.len() - fresh.len()) as u32;
    if !fresh.is_empty() {
        playlists::insert_tracks(conn, playlist_id, &fresh, position)?;
        links::set_status(conn, playlist_id, "pending", None)?;
    }
    Ok(AddOutcome::Added { added: fresh.len() as u32, skipped_duplicates: skipped })
}

/// How long a fork waits for Spotify before using the last sync instead.
pub const FORK_FETCH_TIMEOUT: Duration = Duration::from_secs(20);

/// `add_tracks` with the fork reconciled against Spotify first. `spotify` is
/// `Err(reason)` when Spotify can't be used at all (e.g. not connected).
/// Any Spotify failure falls back to the last sync and says so in the copy.
pub async fn add_tracks_reconciled(
    db: &Mutex<Connection>,
    spotify: Result<&SyncCtx<'_>, String>,
    run_lock: Option<&tokio::sync::Mutex<()>>,
    playlist_id: &str,
    track_ids: &[String],
    position: Option<usize>,
    limit: Duration,
) -> AppResult<AddOutcome> {
    let needs = needs_fork(&db.lock().unwrap_or_else(|e| e.into_inner()), playlist_id, track_ids)?;
    let fork = if !needs {
        None
    } else {
        let latest = match spotify {
            Err(reason) => Err(reason),
            Ok(ctx) => {
                let work = async {
                    // Don't read Spotify halfway through a running sync's writes.
                    let _guard = match run_lock {
                        Some(l) => Some(l.lock().await),
                        None => None,
                    };
                    fork_source_latest(ctx, playlist_id).await
                };
                match tokio::time::timeout(limit, work).await {
                    Ok(Ok(src)) => Ok(src),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(_) => Err("Spotify didn't respond in time".to_string()),
                }
            }
        };
        let conn = db.lock().unwrap_or_else(|e| e.into_inner());
        Some(match latest {
            Ok(src) => src,
            Err(reason) => {
                log::info!("fork of {playlist_id} uses the last sync: {reason}");
                fork_source_last_sync(&conn, playlist_id, reason)?
            }
        })
    };
    let conn = db.lock().unwrap_or_else(|e| e.into_inner());
    add_tracks(&conn, playlist_id, track_ids, position, fork)
}

// ============================================================ sync pass

struct LocalState {
    name: String,
    artwork: Option<String>,
    /// Every item, alternatives included (used to detect concurrent edits).
    ids: Vec<String>,
    /// Spotify items only, in order, and their URIs (what the merge sees).
    spotify_ids: Vec<String>,
    uris: Vec<String>,
    /// Chosen alternatives in this playlist: alternative id -> original id.
    /// They exist only in Branflakes and are never sent to Spotify.
    alts: HashMap<String, String>,
}

fn read_local(conn: &Connection, playlist_id: &str) -> AppResult<(links::Link, LocalState)> {
    let link = links::get_enabled(conn, playlist_id)?.ok_or_else(|| AppError::NotFound("this playlist is not synced with Spotify".into()))?;
    let pl = playlists::get(conn, playlist_id)?;
    let tracks = playlists::tracks(conn, playlist_id)?;
    let ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
    let alts = crate::db::alternatives::active(conn, playlist_id, &ids)?;
    let mut uris = Vec::with_capacity(tracks.len());
    let mut spotify_ids = Vec::with_capacity(tracks.len());
    for t in &tracks {
        if alts.contains_key(&t.id) {
            continue; // a Branflakes-only alternative, not part of the Spotify playlist
        }
        match spotify_uri(t) {
            Some(u) => {
                uris.push(u);
                spotify_ids.push(t.id.clone());
            }
            None => {
                return Err(AppError::InvalidInput(format!(
                    "“{}” isn't a Spotify track, so this playlist can't be synced. Remove it from the playlist",
                    t.title
                )))
            }
        }
    }
    Ok((link, LocalState { name: pl.name, artwork: pl.artwork_path, ids, spotify_ids, uris, alts }))
}

enum Pass {
    Done(SyncReport),
    Retry,
}

/// One full reconciliation of a synced playlist. `manual` = user pressed
/// sync (retries a cover upload Spotify rejected before).
pub async fn sync_playlist(ctx: &SyncCtx<'_>, playlist_id: &str, manual: bool) -> AppResult<SyncReport> {
    {
        let conn = ctx.conn();
        links::get_enabled(&conn, playlist_id)?.ok_or_else(|| AppError::NotFound("this playlist is not synced with Spotify".into()))?;
        if links::is_pending_delete(&conn, playlist_id)? {
            return Err(AppError::Other("this playlist is being deleted from Spotify".into()));
        }
        links::set_status(&conn, playlist_id, "syncing", None)?;
    }
    let mut result = Err(AppError::Other("the playlist kept changing during sync; it will be retried".into()));
    for _ in 0..MAX_PASSES {
        match pass(ctx, playlist_id, manual).await {
            Ok(Pass::Done(r)) => {
                result = Ok(r);
                break;
            }
            Ok(Pass::Retry) => continue,
            Err(e) => {
                result = Err(e);
                break;
            }
        }
    }
    let conn = ctx.conn();
    match &result {
        Ok(_) => {} // pass() stored the new base and status
        Err(e) => {
            if links::get(&conn, playlist_id)?.is_some() {
                links::set_status(&conn, playlist_id, "error", Some(&e.to_string()))?;
            }
        }
    }
    result
}

async fn pass(ctx: &SyncCtx<'_>, playlist_id: &str, manual: bool) -> AppResult<Pass> {
    let (link, local) = {
        let conn = ctx.conn();
        read_local(&conn, playlist_id)?
    };
    let cid = &ctx.client_id;
    let sp_id = link.spotify_playlist_id.clone();
    let mut report = SyncReport { playlist_id: playlist_id.into(), ..Default::default() };

    // Is the Spotify playlist still there and in the user's library? Only a
    // definite answer counts; any error leaves everything as it is.
    let meta = match ctx.spotify.playlist_meta(cid, &sp_id).await {
        Ok(m) => m,
        Err(AppError::Http { status: 404, .. }) => return missing(ctx, playlist_id, &local.name, link.can_edit_details, MISSING_NOT_FOUND),
        Err(e) => return Err(permission_error(e, "share this playlist")),
    };
    if !ctx.spotify.library_contains_playlist(cid, &sp_id).await? {
        return missing(ctx, playlist_id, &local.name, link.can_edit_details, MISSING_NOT_IN_LIBRARY);
    }
    links::clear_missing(&ctx.conn(), playlist_id)?;
    // Ownership can change (playlist transferred, collaboration turned off).
    let me = my_user_id(ctx).await?;
    let can_edit_details = access(&meta, &me)?;

    // ---- items
    let base_tracks: Vec<String> = link.base_items.iter().filter(|u| merge::is_track(u)).cloned().collect();
    let local_changed = local.uris != base_tracks;
    let remote_unchanged = link.snapshot_id.is_some() && meta.snapshot_id == link.snapshot_id;
    let mut remote_items: Option<Vec<RemoteItem>> = None;
    let remote: Vec<String> = if remote_unchanged && !local_changed {
        link.base_items.clone() // nothing to read: avoids a full fetch every 5 minutes
    } else {
        let items = ctx.spotify.playlist_items_all(cid, &sp_id).await.map_err(|e| permission_error(e, "read this playlist"))?;
        let uris = items.iter().map(|i| i.uri.clone()).collect();
        remote_items = Some(items);
        uris
    };
    let m = merge::merge(&link.base_items, &local.uris, &remote);
    if m.conflict {
        report.notices.push("This playlist changed in both Branflakes and Spotify since the last sync. Both sets of additions were kept; Spotify's track order was kept.".into());
    }

    let final_items: Vec<String>;
    let mut wrote_remote = false;
    if m.merged != remote {
        require_write_scopes(ctx.spotify)?;
        // Optimistic concurrency: abort if Spotify changed since we read it.
        let check = ctx.spotify.playlist_meta(cid, &sp_id).await?;
        if check.snapshot_id != meta.snapshot_id {
            return Ok(Pass::Retry);
        }
        let ops = merge::plan_push(&remote, &m.merged).map_err(|e| AppError::Other(e.0))?;
        let mut snap = meta.snapshot_id.clone();
        for op in &ops {
            let r = match op {
                Op::Remove(uris) => ctx.spotify.playlist_remove_items(cid, &sp_id, uris, snap.as_deref()).await,
                Op::Move { range_start, insert_before } => {
                    ctx.spotify.playlist_move_item(cid, &sp_id, *range_start, *insert_before, snap.as_deref()).await
                }
                Op::Add { uris, position } => ctx.spotify.playlist_add_items(cid, &sp_id, uris, Some(*position)).await,
                Op::Replace(uris) => ctx.spotify.playlist_replace_items(cid, &sp_id, uris).await,
            }
            .map_err(|e| permission_error(e, "change this playlist"))?;
            if r.is_some() {
                snap = r;
            }
            report.pushed_ops += 1;
        }
        wrote_remote = true;
        // Read back what Spotify actually has; that becomes the truth.
        let items = ctx.spotify.playlist_items_all(cid, &sp_id).await?;
        final_items = items.iter().map(|i| i.uri.clone()).collect();
        if final_items != m.merged {
            log::warn!("spotify sync: Spotify's result differs from the planned merge; adopting Spotify's version");
            report.notices.push("Spotify applied the changes slightly differently; Branflakes now matches Spotify.".into());
        }
        remote_items = Some(items);
    } else {
        final_items = m.merged.clone();
    }

    // ---- name (3-way)
    let mut name_target = if local.name == link.base_name {
        meta.name.clone()
    } else if meta.name == link.base_name {
        local.name.clone()
    } else {
        report.notices.push(format!("Renamed in both apps; kept the Branflakes name “{}”.", local.name));
        local.name.clone()
    };
    if name_target != meta.name {
        if can_edit_details {
            require_write_scopes(ctx.spotify)?;
            ctx.spotify.playlist_rename(cid, &sp_id, &name_target).await.map_err(|e| permission_error(e, "rename this playlist"))?;
            wrote_remote = true;
        } else {
            report.notices.push("Only the playlist owner can rename it on Spotify; kept Spotify's name.".into());
            name_target = meta.name.clone();
        }
    }

    // ---- cover (3-way, identities: local path / Spotify URL)
    let remote_key = meta.images.first().map(|u| image_key(u));
    let mut base_image_key = link.base_image_key.clone();
    let mut base_artwork = link.base_artwork_path.clone();
    let mut failed_path = link.artwork_failed_path.clone();
    let mut new_local_artwork = local.artwork.clone();
    let remote_art_changed = match base_image_key.as_deref().and_then(|k| k.strip_prefix("uploaded:")) {
        // We uploaded a cover; the first new key Spotify reports is ours.
        Some(prev) => {
            if remote_key.as_deref() != Some(prev) && remote_key.is_some() {
                base_image_key = remote_key.clone();
            }
            false
        }
        None => remote_key.is_some() && remote_key != base_image_key,
    };
    let local_art_changed = local.artwork != link.base_artwork_path;
    if local_art_changed {
        if remote_art_changed {
            report.notices.push("The cover changed in both apps; kept the Branflakes cover.".into());
        }
        match &local.artwork {
            None => {
                report.notices.push("Spotify doesn't let apps remove a playlist cover, so Spotify keeps its cover.".into());
                base_artwork = None;
                base_image_key = remote_key.clone();
            }
            Some(path) if !can_edit_details => {
                report.notices.push("Only the playlist owner can change the cover on Spotify.".into());
                base_artwork = Some(path.clone());
                base_image_key = remote_key.clone();
            }
            Some(path) if !ctx.spotify.missing_granted_scopes(&[spotify::IMAGE_UPLOAD_SCOPE]).is_empty() => {
                let _ = path;
                report.notices.push("Reconnect Spotify (Settings > Accounts) to allow cover uploads; the cover will sync after that.".into());
            }
            Some(path) if failed_path.as_deref() == Some(path) && !manual => {
                // Spotify rejected this exact image before; don't retry automatically.
            }
            Some(path) => {
                let upload = match cover_jpeg_base64(Path::new(path)) {
                    Ok(b64) => ctx.spotify.playlist_upload_cover(cid, &sp_id, &b64).await.map_err(|e| permission_error(e, "change the cover")),
                    Err(e) => Err(e),
                };
                match upload {
                    Ok(()) => {
                        base_artwork = Some(path.clone());
                        base_image_key = Some(format!("uploaded:{}", remote_key.clone().unwrap_or_default()));
                        failed_path = None;
                        wrote_remote = true;
                    }
                    Err(e) => {
                        // Never touch the Branflakes cover when an upload fails.
                        failed_path = Some(path.clone());
                        report.notices.push(format!("The cover couldn't be uploaded to Spotify ({e}). The Branflakes cover is unchanged."));
                    }
                }
            }
        }
    } else if remote_art_changed {
        let url = meta.images.first().expect("remote key implies an image");
        match pull_cover(ctx, url).await {
            Ok(p) => {
                report.artwork_changed = new_local_artwork.as_deref() != Some(p.as_str());
                new_local_artwork = Some(p.clone());
                base_artwork = Some(p);
                base_image_key = remote_key.clone();
            }
            Err(e) => report.notices.push(format!("Couldn't download the new Spotify cover ({e}); will retry.")),
        }
    } else if manual && !base_image_key.as_deref().is_some_and(|k| k.starts_with("uploaded:")) {
        // The URL looks unchanged. On a manual sync, check the image itself
        // (content-addressed cache: same bytes = same file), which catches a
        // cover replaced without a URL change. Not done automatically, to
        // avoid a download every few minutes.
        if let Some(url) = meta.images.first() {
            match pull_cover(ctx, url).await {
                Ok(p) if base_artwork.as_deref() != Some(p.as_str()) => {
                    report.artwork_changed = new_local_artwork.as_deref() != Some(p.as_str());
                    new_local_artwork = Some(p.clone());
                    base_artwork = Some(p);
                    base_image_key = remote_key.clone();
                }
                Ok(_) => {}
                Err(e) => log::info!("spotify sync: cover check skipped: {e}"),
            }
        }
    }

    let final_snapshot = if wrote_remote { ctx.spotify.playlist_meta(cid, &sp_id).await?.snapshot_id } else { meta.snapshot_id.clone() };

    // ---- write Branflakes, unless the user edited it meanwhile
    let conn = ctx.conn();
    let mut uri_to_id: HashMap<String, String> = local.uris.iter().cloned().zip(local.spotify_ids.iter().cloned()).collect();
    if let Some(items) = &remote_items {
        uri_to_id.extend(upsert_remote_tracks(&conn, items)?);
    }
    let new_spotify_ids: Vec<String> = final_items.iter().filter(|u| merge::is_track(u)).filter_map(|u| uri_to_id.get(u).cloned()).collect();
    // Alternatives aren't Spotify items: keep them beside their originals.
    let new_ids = crate::db::alternatives::reinsert(&new_spotify_ids, &local.ids, &local.alts);
    let tx = conn.unchecked_transaction()?;
    let (cur_link, cur) = read_local(&tx, playlist_id)?;
    if cur.ids != local.ids || cur.name != local.name || cur.artwork != local.artwork || cur_link.base_items != link.base_items {
        return Ok(Pass::Retry);
    }
    let before: HashSet<&String> = local.ids.iter().collect();
    let after: HashSet<&String> = new_ids.iter().collect();
    report.pulled_added = after.difference(&before).count() as u32;
    report.pulled_removed = before.difference(&after).count() as u32;
    if new_ids != local.ids {
        playlists::write_items(&tx, playlist_id, &new_ids)?;
    }
    if name_target != local.name {
        playlists::rename(&tx, playlist_id, &name_target)?;
        report.renamed = true;
    }
    if new_local_artwork != local.artwork {
        playlists::set_artwork(&tx, playlist_id, new_local_artwork.as_deref())?;
    }
    links::upsert(
        &tx,
        &links::Link {
            owner_id: meta.owner_id.clone(),
            can_edit_details,
            snapshot_id: final_snapshot,
            base_name: name_target,
            base_items: final_items,
            base_image_key,
            base_artwork_path: base_artwork,
            artwork_failed_path: failed_path,
            status: "ok".into(),
            last_synced_at: Some(now_iso()),
            last_error: None,
            last_notice: if report.notices.is_empty() { None } else { Some(report.notices.join(" ")) },
            ..cur_link
        },
    )?;
    tx.commit()?;
    Ok(Pass::Done(report))
}

// ============================== auto-sync existing imports + dedupe

/// After `keep` became the synced copy of a Spotify playlist, remove stale
/// duplicates so old imports don't linger next to it:
/// * other unsynced copies imported from the same Spotify playlist that
///   hold only Spotify tracks (old import snapshots), and
/// * unsynced playlists with the same name and exactly the same tracks.
/// Never removes forks, synced playlists, or anything with local or
/// SoundCloud tracks (those contain the user's own additions).
pub fn remove_duplicates_of(conn: &Connection, keep: &str) -> AppResult<Vec<String>> {
    let kept = playlists::get(conn, keep)?;
    let kept_items = playlists::track_ids(conn, keep)?;
    let mut removed = Vec::new();
    for p in playlists::list(conn)? {
        if p.id == keep || p.sync.is_some() || p.forked_from.is_some() {
            continue;
        }
        let tracks = playlists::tracks(conn, &p.id)?;
        if tracks.iter().any(|t| !is_spotify_compatible(t)) {
            continue;
        }
        let same_origin = p.origin.is_some() && p.origin == kept.origin;
        let identical = p.name.trim().eq_ignore_ascii_case(kept.name.trim()) && tracks.iter().map(|t| t.id.clone()).collect::<Vec<_>>() == kept_items;
        if same_origin || identical {
            playlists::delete(conn, &p.id)?;
            removed.push(p.name);
        }
    }
    if !removed.is_empty() {
        log::info!("spotify sync: removed {} duplicate(s) of “{}”", removed.len(), kept.name);
    }
    Ok(removed)
}

/// Make every previously imported Spotify playlist (origin
/// `spotify:playlist:…`) Spotify Synced, when Spotify allows it. Playlists
/// that can't be synced (not yours, missing permission, non-Spotify tracks)
/// are left as they are and listed in `skipped` so the caller can avoid
/// retrying them every pass. Returns the newly synced playlist ids.
pub async fn auto_link_imported(ctx: &SyncCtx<'_>, skipped: &mut HashSet<String>) -> AppResult<Vec<String>> {
    if !ctx.spotify.missing_granted_scopes(spotify::PLAYLIST_WRITE_SCOPES).is_empty() {
        return Ok(Vec::new()); // needs a reconnect first; nothing to try yet
    }
    let candidates: Vec<String> = {
        let conn = ctx.conn();
        let mut seen = HashSet::new();
        playlists::list(&conn)?
            .into_iter()
            .filter(|p| p.sync.is_none())
            .filter_map(|p| p.origin.and_then(|o| o.strip_prefix("spotify:playlist:").map(String::from)))
            .filter(|sp| !skipped.contains(sp) && seen.insert(sp.clone()))
            .filter(|sp| matches!(links::find_by_spotify_id(&conn, sp), Ok(None)))
            .collect()
    };
    let mut linked = Vec::new();
    for sp in candidates {
        match enable(ctx, &sp).await {
            Ok(pid) => linked.push(pid),
            Err(e @ (AppError::RateLimited { .. } | AppError::Network { .. })) => return Err(e),
            Err(e) => {
                log::info!("spotify sync: imported playlist {sp} stays unsynced: {e}");
                skipped.insert(sp);
            }
        }
    }
    Ok(linked)
}

// ======================================================= import (auto-sync)

/// What happened to one Spotify playlist during "Import".
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ImportResult {
    /// Imported (or refreshed) and Spotify Synced from now on.
    Synced { playlist_id: String },
    /// Already synced: left to the sync engine (nothing overwritten).
    AlreadySynced { playlist_id: String },
    /// Imported as an ordinary Branflakes playlist; `reason` says why sync
    /// isn't available.
    Plain { playlist_id: Option<String>, reason: String },
    /// Spotify doesn't share this playlist's tracks with apps.
    Skipped { reason: String },
}

const NOT_YOURS: &str = "it belongs to another Spotify user, and Spotify only lets apps change playlists you own or collaborate on";

/// Import one playlist from the user's Spotify playlists, making it Spotify
/// Synced when possible. Re-importing never creates a duplicate: an imported
/// copy is found by its origin and a link by its Spotify id.
pub async fn import_playlist(ctx: &SyncCtx<'_>, p: &spotify::SpotifyPlaylistSummary, me: &str) -> AppResult<ImportResult> {
    if let Some(l) = links::find_by_spotify_id(&ctx.conn(), &p.id)?.filter(|l| l.enabled) {
        return Ok(ImportResult::AlreadySynced { playlist_id: l.playlist_id });
    }
    let owned = p.owner_id.as_deref() == Some(me);
    let missing = ctx.spotify.missing_granted_scopes(spotify::PLAYLIST_WRITE_SCOPES);
    let mut reason = if !(owned || p.collaborative) {
        Some(NOT_YOURS.to_string())
    } else if !missing.is_empty() {
        Some("your Spotify sign-in doesn't allow changing playlists yet (Settings > Accounts > Reconnect Spotify)".to_string())
    } else {
        None
    };
    if reason.is_none() {
        match enable(ctx, &p.id).await {
            Ok(pid) => return Ok(ImportResult::Synced { playlist_id: pid }),
            Err(e @ AppError::RateLimited { .. }) => return Err(e),
            Err(e) => reason = Some(e.to_string()),
        }
    }
    let reason = reason.unwrap_or_default();
    // Ordinary import (the previous behaviour).
    let items = match ctx.spotify.playlist_tracks(&ctx.client_id, &p.id).await {
        Ok(items) => items,
        Err(AppError::Http { status: 403 | 404, .. }) => {
            return Ok(ImportResult::Skipped { reason: format!("Spotify doesn't share its tracks with apps: {NOT_YOURS}") })
        }
        Err(e) => return Err(e),
    };
    let conn = ctx.conn();
    let tx = conn.unchecked_transaction()?;
    let mut ids = Vec::with_capacity(items.len());
    for t in &items {
        ids.push(library::upsert_track(&tx, Source::Spotify, t, false)?);
    }
    let origin = format!("spotify:playlist:{}", p.id);
    let pid = match playlists::find_by_origin(&tx, &origin)? {
        Some(pid) => {
            // Never overwrite a copy the user extended with non-Spotify tracks.
            if playlists::tracks(&tx, &pid)?.iter().any(|t| !is_spotify_compatible(t)) {
                tx.commit()?;
                return Ok(ImportResult::Plain {
                    playlist_id: Some(pid),
                    reason: format!("{reason}. Your Branflakes copy has non-Spotify tracks, so it was left unchanged"),
                });
            }
            pid
        }
        None => playlists::create(&tx, &p.name, Some(&origin))?.id,
    };
    playlists::write_items(&tx, &pid, &ids)?;
    tx.commit()?;
    Ok(ImportResult::Plain { playlist_id: Some(pid), reason })
}

// ============================================== external removal (A/B/C)

pub const MISSING_NOT_FOUND: &str = "not_found";
pub const MISSING_NOT_IN_LIBRARY: &str = "not_in_library";
/// A removal is acted on only when seen again at least this long after the
/// first sighting (i.e. by a later, separate check).
pub const MISSING_CONFIRM_AFTER: Duration = Duration::from_secs(60);

fn missing_message(name: &str, owned: bool, kind: &str, confirmed: bool) -> String {
    let what = match (kind, owned) {
        (MISSING_NOT_FOUND, _) => format!("Spotify says “{name}” no longer exists"),
        (_, true) => format!("“{name}” was deleted in Spotify (Spotify removes deleted playlists from your library)"),
        _ => format!("“{name}” was removed from your Spotify library"),
    };
    if confirmed {
        format!("{what}, so Branflakes removed its synced copy. Tracks stay in your library.")
    } else {
        format!("{what}. Branflakes will check again at the next sync before removing its synced copy.")
    }
}

/// Handle a definite "missing" answer from Spotify: the first sighting is
/// recorded (and reported as a sync error); a second sighting of the same
/// kind, at least `MISSING_CONFIRM_AFTER` later, removes the Branflakes copy.
fn missing(ctx: &SyncCtx<'_>, playlist_id: &str, name: &str, owned: bool, kind: &str) -> AppResult<Pass> {
    let conn = ctx.conn();
    let confirmed = match links::missing_state(&conn, playlist_id)? {
        Some((k, since)) if k == kind => chrono::DateTime::parse_from_rfc3339(&since)
            .map(|t| chrono::Utc::now().signed_duration_since(t).to_std().unwrap_or_default() >= MISSING_CONFIRM_AFTER)
            .unwrap_or(false),
        _ => {
            links::set_missing(&conn, playlist_id, kind)?;
            false
        }
    };
    let msg = missing_message(name, owned, kind, confirmed);
    if !confirmed {
        return Err(AppError::Other(msg));
    }
    // Only a synced playlist ever gets here (local-only playlists and forks
    // have no link and are never checked).
    playlists::delete(&conn, playlist_id)?; // removes the link too (cascade)
    log::info!("spotify sync: removed playlist {playlist_id}: {kind}");
    Ok(Pass::Done(SyncReport { playlist_id: playlist_id.into(), removed: Some(msg), ..Default::default() }))
}

// ============================================= delete from Branflakes

/// Remove the linked Spotify playlist from the user's Spotify library
/// (owner: Spotify's "delete"; someone else's playlist: unfollow), then
/// delete the Branflakes playlist and its link. If anything fails, nothing
/// local is removed. Returns a message describing what happened.
pub async fn delete_synced(ctx: &SyncCtx<'_>, playlist_id: &str) -> AppResult<String> {
    let (link, name) = {
        let conn = ctx.conn();
        let link = links::get_enabled(&conn, playlist_id)?.ok_or_else(|| AppError::NotFound("this playlist is not synced with Spotify".into()))?;
        links::set_pending_delete(&conn, playlist_id, true)?;
        (link, playlists::get(&conn, playlist_id)?.name)
    };
    let cid = &ctx.client_id;
    let sp = &link.spotify_playlist_id;
    let result: AppResult<()> = async {
        require_write_scopes(ctx.spotify)?;
        // Already gone from the library (e.g. an earlier attempt succeeded):
        // don't remove it twice.
        if ctx.spotify.library_contains_playlist(cid, sp).await? {
            ctx.spotify.library_remove_playlist(cid, sp).await.map_err(|e| permission_error(e, "remove this playlist"))?;
            if ctx.spotify.library_contains_playlist(cid, sp).await? {
                return Err(AppError::Other("Spotify still lists the playlist in your library; nothing was deleted. Try again.".into()));
            }
        }
        Ok(())
    }
    .await;
    let conn = ctx.conn();
    if let Err(e) = result {
        links::set_pending_delete(&conn, playlist_id, false)?;
        return Err(e);
    }
    playlists::delete(&conn, playlist_id)?;
    Ok(if link.can_edit_details {
        format!("“{name}” was deleted from Spotify and Branflakes.")
    } else {
        format!("“{name}” was removed from your Spotify library (the owner's playlist is unchanged) and deleted from Branflakes.")
    })
}

/// Finish a deletion interrupted by a restart: if Spotify no longer lists
/// the playlist in the library, delete the Branflakes copy; if it still
/// does, the deletion never happened, so keep syncing. Errors keep the flag
/// for a later retry. Returns true if the playlist was removed.
pub async fn resume_delete(ctx: &SyncCtx<'_>, playlist_id: &str) -> AppResult<bool> {
    let link = links::get(&ctx.conn(), playlist_id)?.ok_or_else(|| AppError::NotFound("no such link".into()))?;
    let in_library = ctx.spotify.library_contains_playlist(&ctx.client_id, &link.spotify_playlist_id).await?;
    let conn = ctx.conn();
    if in_library {
        links::set_pending_delete(&conn, playlist_id, false)?;
        Ok(false)
    } else {
        playlists::delete(&conn, playlist_id)?;
        Ok(true)
    }
}

// ================================== create a Spotify playlist and link

/// Explicit user choice: create a Spotify playlist for a Branflakes
/// playlist and link them (two-way sync from then on). Safe to retry: an
/// earlier attempt whose outcome was unknown is adopted instead of creating
/// a duplicate. A playlist imported from Spotify is linked to its original
/// instead. Returns the Spotify playlist id. Pushing the contents is left to
/// the next sync pass.
pub async fn create_and_link(ctx: &SyncCtx<'_>, playlist_id: &str) -> AppResult<String> {
    let (pl, tracks, existing) = {
        let conn = ctx.conn();
        (playlists::get(&conn, playlist_id)?, playlists::tracks(&conn, playlist_id)?, links::get_enabled(&conn, playlist_id)?)
    };
    if let Some(l) = existing {
        return Ok(l.spotify_playlist_id); // already linked: nothing to create
    }
    if let Some(sp) = pl.origin.as_deref().and_then(|o| o.strip_prefix("spotify:playlist:")) {
        enable(ctx, sp).await?;
        return Ok(sp.to_string());
    }
    let foreign = tracks.iter().filter(|t| !is_spotify_compatible(t)).count();
    if foreign > 0 {
        return Err(AppError::InvalidInput(format!(
            "“{}” has {foreign} track(s) that aren't on Spotify (local files or SoundCloud), so it can't be synced. Spotify playlists can only contain Spotify tracks",
            pl.name
        )));
    }
    require_write_scopes(ctx.spotify)?;
    let me = my_user_id(ctx).await?;
    let cid = &ctx.client_id;

    let pending = links::pending_create(&ctx.conn(), playlist_id)?;
    let mut created: Option<(String, Option<String>)> = None;
    if let Some(pending_name) = pending {
        // A previous create may have succeeded without us seeing the answer.
        let lists = ctx.spotify.my_playlists(cid).await?;
        let conn = ctx.conn();
        let candidates: Vec<_> = lists
            .into_iter()
            .filter(|p| p.owner_id.as_deref() == Some(me.as_str()) && p.name == pending_name && p.total.unwrap_or(0) == 0)
            .filter(|p| matches!(links::find_by_spotify_id(&conn, &p.id), Ok(None)))
            .collect();
        match candidates.len() {
            0 => {}
            1 => created = Some((candidates[0].id.clone(), None)),
            n => {
                return Err(AppError::Other(format!(
                    "found {n} empty Spotify playlists named “{pending_name}” from an earlier attempt; delete the extras in Spotify, then try again"
                )))
            }
        }
    }
    let (sp_id, snapshot) = match created {
        Some(c) => c,
        None => {
            links::set_pending_create(&ctx.conn(), playlist_id, &pl.name)?;
            match ctx.spotify.playlist_create(cid, &pl.name).await {
                Ok(c) => c,
                Err(e) => {
                    // A definite refusal means nothing was created; anything
                    // else (network, timeout, 5xx, rate limit) is unknown and
                    // the marker stays so a retry adopts instead of duplicating.
                    let definite = matches!(&e, AppError::Http { status, .. } if (400..500).contains(status) && *status != 429)
                        || matches!(&e, AppError::Auth { .. } | AppError::NotConnected { .. } | AppError::PremiumRequired { .. });
                    if definite {
                        links::clear_pending_create(&ctx.conn(), playlist_id)?;
                    }
                    return Err(permission_error(e, "create the playlist"));
                }
            }
        }
    };
    let conn = ctx.conn();
    let tx = conn.unchecked_transaction()?;
    links::upsert(
        &tx,
        &links::Link {
            playlist_id: playlist_id.to_string(),
            spotify_playlist_id: sp_id.clone(),
            enabled: true,
            owner_id: Some(me),
            can_edit_details: true,
            snapshot_id: snapshot,
            base_name: pl.name.clone(),
            // Spotify starts empty: everything in Branflakes is a local
            // addition, so the next sync pushes it (in order).
            base_items: Vec::new(),
            base_image_key: None,
            base_artwork_path: None,
            artwork_failed_path: None,
            status: "pending".into(),
            last_synced_at: None,
            last_error: None,
            last_notice: None,
        },
    )?;
    links::clear_pending_create(&tx, playlist_id)?;
    tx.commit()?;
    Ok(sp_id)
}

// ============================================================ scheduler

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncEvent {
    pub playlist_id: String,
    /// "syncing" | "ok" | "error"
    pub status: String,
    pub changed: bool,
    pub message: Option<String>,
}

/// Backend scheduler state. The loop itself lives in `lib.rs` (it needs the
/// Tauri app handle); this holds the shared coordination primitives.
pub struct SyncEngine {
    pub wake: tokio::sync::Notify,
    shutdown: std::sync::atomic::AtomicBool,
    /// One sync at a time (keeps request bursts and rate limits in check).
    pub run_lock: tokio::sync::Mutex<()>,
    in_flight: Mutex<HashSet<String>>,
    due: Mutex<HashMap<String, Instant>>,
    backoff_until: Mutex<Option<Instant>>,
}

impl Default for SyncEngine {
    fn default() -> Self {
        SyncEngine {
            wake: tokio::sync::Notify::new(),
            shutdown: std::sync::atomic::AtomicBool::new(false),
            run_lock: tokio::sync::Mutex::new(()),
            in_flight: Mutex::new(HashSet::new()),
            due: Mutex::new(HashMap::new()),
            backoff_until: Mutex::new(None),
        }
    }
}

/// RAII marker: "this playlist is syncing right now".
pub struct InFlight<'a> {
    engine: &'a SyncEngine,
    id: String,
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.engine.in_flight.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.id);
    }
}

impl SyncEngine {
    /// Schedule a sync of one playlist after `delay` (coalescing repeats).
    pub fn request(&self, playlist_id: &str, delay: Duration) {
        let at = Instant::now() + delay;
        self.due.lock().unwrap_or_else(|e| e.into_inner()).insert(playlist_id.to_string(), at);
        self.wake.notify_one();
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
        self.wake.notify_one();
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Mark a playlist as syncing; `None` if it already is (duplicate request).
    pub fn begin(&self, playlist_id: &str) -> Option<InFlight<'_>> {
        let mut set = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        if !set.insert(playlist_id.to_string()) {
            return None;
        }
        Some(InFlight { engine: self, id: playlist_id.to_string() })
    }

    pub fn is_syncing(&self, playlist_id: &str) -> bool {
        self.in_flight.lock().unwrap_or_else(|e| e.into_inner()).contains(playlist_id)
    }

    /// Take the playlists whose debounced sync is due now.
    pub fn take_due(&self, now: Instant) -> Vec<String> {
        let mut due = self.due.lock().unwrap_or_else(|e| e.into_inner());
        let ready: Vec<String> = due.iter().filter(|(_, t)| **t <= now).map(|(k, _)| k.clone()).collect();
        for k in &ready {
            due.remove(k);
        }
        ready
    }

    pub fn next_due(&self) -> Option<Instant> {
        self.due.lock().unwrap_or_else(|e| e.into_inner()).values().min().copied()
    }

    /// After a rate limit or network failure, pause automatic syncing.
    pub fn back_off(&self, d: Duration) {
        *self.backoff_until.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now() + d);
    }

    pub fn backoff_until(&self) -> Option<Instant> {
        let mut b = self.backoff_until.lock().unwrap_or_else(|e| e.into_inner());
        if b.is_some_and(|t| t <= Instant::now()) {
            *b = None;
        }
        *b
    }
}

/// How long automatic syncing should pause after an error.
pub fn backoff_for(e: &AppError, interval: Duration) -> Option<Duration> {
    match e {
        AppError::RateLimited { retry_after_secs, quota_exceeded, .. } => {
            if *quota_exceeded {
                Some(Duration::from_secs(3600))
            } else {
                Some(Duration::from_secs(retry_after_secs.unwrap_or(60).max(5)))
            }
        }
        // Offline: try again sooner than the full interval.
        AppError::Network { .. } => Some(Duration::from_secs(60).min(interval)),
        _ => None,
    }
}

pub fn interval(conn: &Connection) -> Duration {
    let secs = settings::get_nonempty(conn, settings::SPOTIFY_SYNC_INTERVAL)
        .ok()
        .flatten()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_INTERVAL_SECS)
        .max(MIN_INTERVAL_SECS);
    Duration::from_secs(secs)
}

#[cfg(test)]
mod tests;
