//! Tauri command layer. Thin: validates input, delegates to db/providers,
//! and converts results. No provider logic lives in the UI.

use crate::db::{self, library, playlists, queue, scrobbles, settings, spotify_links};
use crate::error::{AppError, AppResult};
use crate::media::MediaUpdate;
use crate::model::{ProviderStatus, Source, Track};
use crate::oauth;
use crate::providers::{
    lastfm::{CallError, LastFmAdapter, LastFmCredentials, NowPlaying},
    local::{self, LocalMusicAdapter, MetadataEdit},
    soundcloud::{SoundCloudCredentials, StreamInfo},
    spotify::{SpotifyDevice, SpotifyPlaybackState},
};
use crate::scrobbler::{self, FlushReport, SubmitOutcome};
use crate::secrets::{self, Secret};
use crate::state::{require, AppState};
use crate::sync::{self, AddOutcome, SyncCtx, SyncEvent, SyncReport};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

type S<'a> = State<'a, AppState>;

const OAUTH_TIMEOUT: Duration = Duration::from_secs(300);

fn oauth_port(state: &AppState) -> AppResult<u16> {
    Ok(settings::get_nonempty(&state.conn(), settings::OAUTH_PORT)?
        .and_then(|p| p.parse().ok())
        .unwrap_or(oauth::DEFAULT_LOOPBACK_PORT))
}

fn spotify_client_id(state: &AppState) -> AppResult<String> {
    require(settings::get_nonempty(&state.conn(), settings::SPOTIFY_CLIENT_ID)?, "spotify", "Spotify client ID")
}

fn soundcloud_creds(state: &AppState) -> AppResult<SoundCloudCredentials> {
    let client_id = require(settings::get_nonempty(&state.conn(), settings::SOUNDCLOUD_CLIENT_ID)?, "soundcloud", "SoundCloud client ID")?;
    let secret = require(state.secrets.get(secrets::SOUNDCLOUD_CLIENT_SECRET)?, "soundcloud", "SoundCloud client secret")?;
    Ok(SoundCloudCredentials { client_id, client_secret: Secret::new(secret) })
}

fn lastfm_creds(state: &AppState) -> AppResult<LastFmCredentials> {
    let api_key = require(settings::get_nonempty(&state.conn(), settings::LASTFM_API_KEY)?, "lastfm", "Last.fm API key")?;
    let secret = require(state.secrets.get(secrets::LASTFM_SHARED_SECRET)?, "lastfm", "Last.fm shared secret")?;
    Ok(LastFmCredentials { api_key, shared_secret: Secret::new(secret) })
}

fn lastfm_session(state: &AppState) -> AppResult<Secret> {
    state.secrets.get(secrets::LASTFM_SESSION_KEY)?.map(Secret::new).ok_or(AppError::NotConnected { provider: "lastfm".into() })
}

fn open_browser(app: &AppHandle, url: &str) -> AppResult<()> {
    app.opener().open_url(url, None::<&str>).map_err(|e| AppError::Other(format!("could not open the browser: {e}")))
}

// ================================================================ library

#[tauri::command]
pub fn library_list_tracks(state: S<'_>, query: library::TrackQuery) -> AppResult<Vec<Track>> {
    library::list_tracks(&state.conn(), &query)
}

#[tauri::command]
pub fn library_get_track(state: S<'_>, id: String) -> AppResult<Track> {
    library::get_track(&state.conn(), &id)
}

#[tauri::command]
pub fn library_list_artists(state: S<'_>) -> AppResult<Vec<library::ArtistSummary>> {
    library::list_artists(&state.conn())
}

#[tauri::command]
pub fn library_list_albums(state: S<'_>, artist: Option<String>) -> AppResult<Vec<library::AlbumSummary>> {
    library::list_albums(&state.conn(), artist.as_deref())
}

#[tauri::command]
pub fn library_set_membership(state: S<'_>, track_ids: Vec<String>, in_library: bool) -> AppResult<()> {
    library::set_in_library(&state.conn(), &track_ids, in_library)
}

// ============================================================ local files

#[tauri::command]
pub fn local_list_folders(state: S<'_>) -> AppResult<Vec<library::LibraryFolder>> {
    library::list_folders(&state.conn())
}

#[tauri::command]
pub fn local_add_folder(app: AppHandle, state: S<'_>, path: String) -> AppResult<library::LibraryFolder> {
    let p = PathBuf::from(&path);
    if !p.is_absolute() || !p.is_dir() {
        return Err(AppError::InvalidInput(format!("{path} is not a folder")));
    }
    allow_asset_dir(&app, &p);
    library::add_folder(&state.conn(), &path)
}

#[tauri::command]
pub fn local_remove_folder(state: S<'_>, id: i64) -> AppResult<()> {
    library::remove_folder(&state.conn(), id)
}

pub fn allow_asset_dir(app: &AppHandle, dir: &Path) {
    if let Err(e) = app.asset_protocol_scope().allow_directory(dir, true) {
        log::warn!("could not allow asset access to {}: {e}", dir.display());
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanProgress {
    folder: String,
    processed: u32,
}

#[tauri::command]
pub async fn local_scan(app: AppHandle, state: S<'_>, folder_id: Option<i64>) -> AppResult<Vec<local::ScanReport>> {
    let folders: Vec<_> = library::list_folders(&state.conn())?
        .into_iter()
        .filter(|f| folder_id.is_none_or(|id| f.id == id))
        .collect();
    let conn = state.open_worker_connection()?;
    let artwork_dir = state.paths.artwork_dir.clone();
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut reports = Vec::new();
        for f in folders {
            let folder = f.path.clone();
            let r = local::scan_folder(&conn, f.id, Path::new(&f.path), &artwork_dir, |processed| {
                let _ = app2.emit("scan-progress", ScanProgress { folder: folder.clone(), processed });
            });
            match r {
                Ok(rep) => reports.push(rep),
                Err(e) => reports.push(local::ScanReport { folder: f.path, errors: vec![e.to_string()], ..Default::default() }),
            }
        }
        Ok(reports)
    })
    .await
    .map_err(|e| AppError::Other(format!("scan task failed: {e}")))?
}

#[tauri::command]
pub async fn local_read_metadata(state: S<'_>, track_id: String) -> AppResult<local::FileMetadata> {
    let t = library::get_track(&state.conn(), &track_id)?;
    let path = t.file_path.ok_or_else(|| AppError::Unsupported("not a local file".into()))?;
    tauri::async_runtime::spawn_blocking(move || local::read_file_metadata(Path::new(&path)))
        .await
        .map_err(|e| AppError::Other(e.to_string()))?
}

#[tauri::command]
pub async fn local_write_metadata(state: S<'_>, track_id: String, edit: MetadataEdit) -> AppResult<local::WriteReport> {
    let conn = state.open_worker_connection()?;
    let (bk, art) = (state.paths.backup_dir.clone(), state.paths.artwork_dir.clone());
    tauri::async_runtime::spawn_blocking(move || local::write_metadata_safely(&conn, &track_id, &edit, &bk, &art))
        .await
        .map_err(|e| AppError::Other(e.to_string()))?
}

#[tauri::command]
pub fn local_list_backups(state: S<'_>, track_id: String) -> AppResult<Vec<library::MetadataBackup>> {
    library::list_backups(&state.conn(), &track_id)
}

#[tauri::command]
pub fn local_restore_backup(state: S<'_>, backup_id: i64) -> AppResult<()> {
    local::restore_backup(&state.conn(), backup_id, &state.paths.artwork_dir)
}

// ============================================================== overrides

#[tauri::command]
pub fn override_get(state: S<'_>, track_id: String) -> AppResult<Option<library::TrackOverride>> {
    library::get_override(&state.conn(), &track_id)
}

#[tauri::command]
pub fn override_set(state: S<'_>, track_id: String, value: library::TrackOverride) -> AppResult<Track> {
    let conn = state.conn();
    library::set_override(&conn, &track_id, &value)?;
    library::get_track(&conn, &track_id)
}

#[tauri::command]
pub fn override_set_artwork(state: S<'_>, track_id: String, image_path: String) -> AppResult<Track> {
    let stored = local::import_override_artwork(Path::new(&image_path), &state.paths.artwork_dir)?;
    let conn = state.conn();
    let mut o = library::get_override(&conn, &track_id)?.unwrap_or_default();
    o.artwork_path = Some(stored.to_string_lossy().into_owned());
    library::set_override(&conn, &track_id, &o)?;
    library::get_track(&conn, &track_id)
}

#[tauri::command]
pub fn override_clear(state: S<'_>, track_id: String) -> AppResult<Track> {
    let conn = state.conn();
    library::clear_override(&conn, &track_id)?;
    library::get_track(&conn, &track_id)
}

// ============================================================== playlists

#[tauri::command]
pub fn playlist_list(state: S<'_>) -> AppResult<Vec<playlists::Playlist>> {
    playlists::list(&state.conn())
}

#[tauri::command]
pub async fn playlist_create(app: AppHandle, state: S<'_>, name: String, sync_with_spotify: Option<bool>) -> AppResult<CreatePlaylistResult> {
    // Always local first: the Branflakes playlist exists whatever Spotify does.
    let pl = playlists::create(&state.conn(), &name, None)?;
    if sync_with_spotify != Some(true) {
        return Ok(CreatePlaylistResult { playlist: pl, spotify_error: None });
    }
    let spotify_error = link_to_spotify(&app, &state, &pl.id).await.err().map(|e| e.to_string());
    Ok(CreatePlaylistResult { playlist: playlists::get(&state.conn(), &pl.id)?, spotify_error })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePlaylistResult {
    pub playlist: playlists::Playlist,
    /// Set when Spotify sync was requested but couldn't be enabled; the
    /// Branflakes playlist was still created (local-only for now).
    pub spotify_error: Option<String>,
}

/// Create (or adopt) the Spotify playlist, link it, then push the contents.
/// A failure after linking is a normal sync error (retried automatically).
async fn link_to_spotify(app: &AppHandle, state: &AppState, playlist_id: &str) -> AppResult<()> {
    let ctx = sync_ctx(state).map_err(|e| match e {
        AppError::NotConnected { .. } => AppError::NotConnected { provider: "spotify".into() },
        other => other,
    })?;
    {
        let _run = state.sync.run_lock.lock().await;
        sync::create_and_link(&ctx, playlist_id).await?;
    }
    emit_sync(app, SyncEvent { playlist_id: playlist_id.into(), status: "ok".into(), changed: true, message: None });
    if let Err(e) = run_sync(app, state, playlist_id, true).await {
        log::info!("first sync after linking {playlist_id} failed: {e}");
    }
    Ok(())
}

/// Explicitly link an existing local-only playlist to a new Spotify
/// playlist (or an imported one to its original). Safe to retry.
#[tauri::command]
pub async fn playlist_link_spotify(app: AppHandle, state: S<'_>, id: String) -> AppResult<playlists::Playlist> {
    link_to_spotify(&app, &state, &id).await?;
    playlists::get(&state.conn(), &id)
}

/// For a Spotify Synced playlist that only the owner may edit (rename,
/// cover), refuse up front with an explanation.
fn require_details_editable(conn: &rusqlite::Connection, id: &str, what: &str) -> AppResult<()> {
    if let Some(l) = spotify_links::get_enabled(conn, id)? {
        if !l.can_edit_details {
            return Err(AppError::Unsupported(format!(
                "this is a collaborative Spotify playlist owned by someone else; only the owner can {what}"
            )));
        }
    }
    Ok(())
}

/// After a local edit to a synced playlist, push it soon (debounced).
fn sync_after_edit(state: &AppState, id: &str) {
    if matches!(spotify_links::get_enabled(&state.conn(), id), Ok(Some(_))) {
        let _ = spotify_links::set_status(&state.conn(), id, "pending", None);
        state.sync.request(id, sync::EDIT_DEBOUNCE);
    }
}

#[tauri::command]
pub fn playlist_rename(state: S<'_>, id: String, name: String) -> AppResult<()> {
    {
        let conn = state.conn();
        require_details_editable(&conn, &id, "rename it")?;
        playlists::rename(&conn, &id, &name)?;
    }
    sync_after_edit(&state, &id);
    Ok(())
}

/// Delete a playlist. Ordinary playlists and Branflakes-only copies are
/// deleted locally and never touch Spotify. A Spotify Synced playlist needs
/// `confirm_spotify: true` (the user confirmed it also goes from Spotify) or
/// `keep_on_spotify: true` (stop syncing and delete only the Branflakes copy).
#[tauri::command]
pub async fn playlist_delete(
    app: AppHandle,
    state: S<'_>,
    id: String,
    confirm_spotify: Option<bool>,
    keep_on_spotify: Option<bool>,
) -> AppResult<Option<String>> {
    let linked = spotify_links::get_enabled(&state.conn(), &id)?.is_some();
    if !linked {
        playlists::delete(&state.conn(), &id)?;
        return Ok(None);
    }
    if keep_on_spotify == Some(true) {
        let conn = state.conn();
        let tx = conn.unchecked_transaction()?;
        spotify_links::delete(&tx, &id)?;
        playlists::delete(&tx, &id)?;
        tx.commit()?;
        return Ok(Some("Deleted from Branflakes. The Spotify playlist was not changed.".into()));
    }
    if confirm_spotify != Some(true) {
        return Err(AppError::InvalidInput(
            "this playlist is Spotify Synced: confirm that it should also be removed from Spotify, or choose to keep it on Spotify".into(),
        ));
    }
    let ctx = sync_ctx(&state)?;
    let Some(_guard) = state.sync.begin(&id) else {
        return Err(AppError::Other("this playlist is syncing right now; try again in a moment".into()));
    };
    let _run = state.sync.run_lock.lock().await;
    let msg = sync::delete_synced(&ctx, &id).await?;
    emit_sync(&app, SyncEvent { playlist_id: id.clone(), status: "deleted".into(), changed: true, message: Some(msg.clone()) });
    Ok(Some(msg))
}

#[tauri::command]
pub fn playlist_mark_played(state: S<'_>, id: String) -> AppResult<()> {
    playlists::mark_played(&state.conn(), &id)
}

/// Set a cover image (copied into the artwork cache) or clear it with `null`.
#[tauri::command]
pub fn playlist_set_artwork(state: S<'_>, id: String, image_path: Option<String>) -> AppResult<playlists::Playlist> {
    require_details_editable(&state.conn(), &id, "change its cover")?;
    let stored = match image_path {
        Some(p) => Some(local::import_override_artwork(Path::new(&p), &state.paths.artwork_dir)?.to_string_lossy().into_owned()),
        None => None,
    };
    playlists::set_artwork(&state.conn(), &id, stored.as_deref())?;
    sync_after_edit(&state, &id);
    playlists::get(&state.conn(), &id)
}

#[tauri::command]
pub fn playlist_tracks(state: S<'_>, id: String) -> AppResult<Vec<Track>> {
    playlists::tracks(&state.conn(), &id)
}

/// Add tracks (optionally at a position). Enforces the Spotify Synced rule:
/// a non-Spotify track creates a Branflakes-only copy instead.
/// Before a copy is made, the synced playlist is reconciled with Spotify's
/// current contents (read-only); if Spotify can't be reached the copy is
/// based on the last sync and says so.
#[tauri::command]
pub async fn playlist_add_tracks(state: S<'_>, id: String, track_ids: Vec<String>, position: Option<usize>) -> AppResult<AddOutcome> {
    let ctx = sync_ctx(&state).map_err(|e| match e {
        AppError::NotConnected { .. } => "Spotify isn't connected".to_string(),
        other => other.to_string(),
    });
    let out = sync::add_tracks_reconciled(&state.db, ctx.as_ref().map_err(Clone::clone), Some(&state.sync.run_lock), &id, &track_ids, position, sync::FORK_FETCH_TIMEOUT)
        .await?;
    if matches!(out, AddOutcome::Added { added, .. } if added > 0) {
        sync_after_edit(&state, &id);
    }
    Ok(out)
}

#[tauri::command]
pub fn playlist_remove_item(state: S<'_>, id: String, position: usize) -> AppResult<()> {
    {
        let conn = state.conn();
        let removed = playlists::track_ids(&conn, &id)?.get(position).cloned();
        playlists::remove_at(&conn, &id, position)?;
        if let Some(track) = removed {
            // A removed alternative is never re-added automatically.
            crate::db::alternatives::mark_removed_if_gone(&conn, &id, &track)?;
        }
    }
    sync_after_edit(&state, &id);
    Ok(())
}

#[tauri::command]
pub fn playlist_move_item(state: S<'_>, id: String, from: usize, to: usize) -> AppResult<()> {
    playlists::move_item(&state.conn(), &id, from, to)?;
    sync_after_edit(&state, &id);
    Ok(())
}

// ================================================================== queue

#[tauri::command]
pub fn queue_get(state: S<'_>) -> AppResult<queue::QueueSnapshot> {
    queue::snapshot(&state.conn())
}

#[tauri::command]
pub fn queue_replace(state: S<'_>, track_ids: Vec<String>, start_index: usize) -> AppResult<queue::QueueSnapshot> {
    queue::replace(&state.conn(), &track_ids, start_index)
}

#[tauri::command]
pub fn queue_add(state: S<'_>, track_ids: Vec<String>, play_next: bool) -> AppResult<queue::QueueSnapshot> {
    queue::add(&state.conn(), &track_ids, play_next)
}

#[tauri::command]
pub fn queue_remove(state: S<'_>, position: usize) -> AppResult<queue::QueueSnapshot> {
    queue::remove_at(&state.conn(), position)
}

#[tauri::command]
pub fn queue_move(state: S<'_>, from: usize, to: usize) -> AppResult<queue::QueueSnapshot> {
    queue::move_item(&state.conn(), from, to)
}

#[tauri::command]
pub fn queue_clear(state: S<'_>) -> AppResult<queue::QueueSnapshot> {
    queue::clear(&state.conn())
}

/// Move the current index. Returns the new snapshot; `state.current_index`
/// is `None` when playback should stop (end of queue, repeat off).
#[tauri::command]
pub fn queue_advance(state: S<'_>, direction: String, user_initiated: bool) -> AppResult<queue::QueueSnapshot> {
    let conn = state.conn();
    let snap = queue::snapshot(&conn)?;
    let len = snap.items.len();
    let cur = snap.state.current_index;
    let next = match direction.as_str() {
        "next" => queue::next_index(cur, len, snap.state.repeat_mode, user_initiated),
        "previous" => queue::previous_index(cur, len, snap.state.repeat_mode),
        other => return Err(AppError::InvalidInput(format!("unknown direction {other}"))),
    };
    let mut st = snap.state.clone();
    st.position_ms = 0;
    match next {
        Some(i) => st.current_index = Some(i),
        // End of queue: keep pointing at the last track, report stop.
        None => {
            queue::save_state(&conn, &st)?;
            let mut s = queue::snapshot(&conn)?;
            s.state.current_index = None;
            return Ok(s);
        }
    }
    queue::save_state(&conn, &st)?;
    queue::snapshot(&conn)
}

#[tauri::command]
pub fn queue_set_current(state: S<'_>, index: usize) -> AppResult<queue::QueueSnapshot> {
    let conn = state.conn();
    let mut st = queue::get_state(&conn)?;
    if index >= queue::track_ids(&conn)?.len() {
        return Err(AppError::InvalidInput("index out of range".into()));
    }
    st.current_index = Some(index);
    st.position_ms = 0;
    queue::save_state(&conn, &st)?;
    queue::snapshot(&conn)
}

/// Shuffle the items after the current one (the current track keeps playing).
#[tauri::command]
pub fn queue_shuffle_upcoming(state: S<'_>) -> AppResult<queue::QueueSnapshot> {
    let conn = state.conn();
    let snap = queue::snapshot(&conn)?;
    let mut ids: Vec<String> = snap.items.iter().map(|t| t.id.clone()).collect();
    let start = snap.state.current_index.map(|c| c + 1).unwrap_or(0);
    if start < ids.len() {
        let tail = &mut ids[start..];
        for i in (1..tail.len()).rev() {
            let r = uuid::Uuid::new_v4().as_u128();
            let j = (r % (i as u128 + 1)) as usize;
            tail.swap(i, j);
        }
    }
    let cur = snap.state.current_index.unwrap_or(0);
    let s = queue::replace(&conn, &ids, cur)?;
    let mut st = snap.state;
    st.shuffle = true;
    queue::save_state(&conn, &st)?;
    let _ = s;
    queue::snapshot(&conn)
}

#[tauri::command]
pub fn playback_save_state(state: S<'_>, value: queue::PlaybackState) -> AppResult<()> {
    queue::save_state(&state.conn(), &value)
}

// ============================================================== providers

#[tauri::command]
pub fn providers_status(state: S<'_>) -> AppResult<Vec<ProviderStatus>> {
    let conn = state.conn();
    let sp_configured = settings::get_nonempty(&conn, settings::SPOTIFY_CLIENT_ID)?.is_some();
    let sc_configured = settings::get_nonempty(&conn, settings::SOUNDCLOUD_CLIENT_ID)?.is_some() && state.secrets.has(secrets::SOUNDCLOUD_CLIENT_SECRET);
    let lf_configured = settings::get_nonempty(&conn, settings::LASTFM_API_KEY)?.is_some() && state.secrets.has(secrets::LASTFM_SHARED_SECRET);
    let premium_denied = settings::get_bool(&conn, settings::SPOTIFY_PREMIUM_DENIED, false)?;
    Ok(vec![
        LocalMusicAdapter::status(),
        state.spotify.status(sp_configured, settings::get(&conn, settings::SPOTIFY_ACCOUNT)?, premium_denied),
        state.soundcloud.status(
            sc_configured,
            settings::get(&conn, settings::SOUNDCLOUD_ACCOUNT)?,
            settings::get_bool(&conn, settings::SOUNDCLOUD_WEB_MODE, false)?,
        ),
        LastFmAdapter::status(
            lf_configured,
            state.secrets.has(secrets::LASTFM_SESSION_KEY),
            settings::get(&conn, settings::LASTFM_ACCOUNT)?,
            settings::get_bool(&conn, settings::LASTFM_ENABLED, true)?,
            settings::get_bool(&conn, settings::LASTFM_SCROBBLE_SPOTIFY, true)?,
        ),
    ])
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub spotify_client_id: String,
    pub soundcloud_client_id: String,
    pub soundcloud_secret_set: bool,
    pub soundcloud_web_mode: bool,
    pub lastfm_api_key: String,
    pub lastfm_secret_set: bool,
    pub lastfm_enabled: bool,
    pub lastfm_scrobble_spotify: bool,
    pub oauth_port: u16,
    /// Automatic Spotify playlist sync interval (minutes, default 5).
    pub spotify_sync_interval_minutes: u64,
    /// Scale in-app playback towards a common loudness (default on).
    pub normalize_volume: bool,
    pub spotify_redirect_uri: String,
    pub soundcloud_redirect_uri: String,
    pub data_dir: String,
}

#[tauri::command]
pub fn settings_get(state: S<'_>) -> AppResult<AppSettings> {
    let port = oauth_port(&state)?;
    let conn = state.conn();
    Ok(AppSettings {
        spotify_client_id: settings::get(&conn, settings::SPOTIFY_CLIENT_ID)?.unwrap_or_default(),
        soundcloud_client_id: settings::get(&conn, settings::SOUNDCLOUD_CLIENT_ID)?.unwrap_or_default(),
        soundcloud_secret_set: state.secrets.has(secrets::SOUNDCLOUD_CLIENT_SECRET),
        soundcloud_web_mode: settings::get_bool(&conn, settings::SOUNDCLOUD_WEB_MODE, false)?,
        lastfm_api_key: settings::get(&conn, settings::LASTFM_API_KEY)?.unwrap_or_default(),
        lastfm_secret_set: state.secrets.has(secrets::LASTFM_SHARED_SECRET),
        lastfm_enabled: settings::get_bool(&conn, settings::LASTFM_ENABLED, true)?,
        lastfm_scrobble_spotify: settings::get_bool(&conn, settings::LASTFM_SCROBBLE_SPOTIFY, true)?,
        oauth_port: port,
        spotify_sync_interval_minutes: sync::interval(&conn).as_secs() / 60,
        normalize_volume: settings::get_bool(&conn, settings::NORMALIZE_VOLUME, true)?,
        spotify_redirect_uri: oauth::redirect_uri(port, "spotify"),
        soundcloud_redirect_uri: oauth::redirect_uri(port, "soundcloud"),
        data_dir: state.paths.data_dir.to_string_lossy().into_owned(),
    })
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub spotify_client_id: Option<String>,
    pub soundcloud_client_id: Option<String>,
    /// Empty string deletes the stored secret.
    pub soundcloud_client_secret: Option<String>,
    pub soundcloud_web_mode: Option<bool>,
    pub lastfm_api_key: Option<String>,
    pub lastfm_shared_secret: Option<String>,
    pub lastfm_enabled: Option<bool>,
    pub lastfm_scrobble_spotify: Option<bool>,
    pub oauth_port: Option<u16>,
    pub spotify_sync_interval_minutes: Option<u64>,
    pub normalize_volume: Option<bool>,
}

#[tauri::command]
pub fn settings_update(state: S<'_>, patch: SettingsPatch) -> AppResult<AppSettings> {
    {
        let conn = state.conn();
        let set_text = |k: &str, v: &Option<String>| -> AppResult<()> {
            if let Some(v) = v {
                settings::set(&conn, k, v.trim())?;
            }
            Ok(())
        };
        set_text(settings::SPOTIFY_CLIENT_ID, &patch.spotify_client_id)?;
        set_text(settings::SOUNDCLOUD_CLIENT_ID, &patch.soundcloud_client_id)?;
        set_text(settings::LASTFM_API_KEY, &patch.lastfm_api_key)?;
        if let Some(b) = patch.lastfm_enabled {
            settings::set(&conn, settings::LASTFM_ENABLED, if b { "true" } else { "false" })?;
        }
        if let Some(b) = patch.lastfm_scrobble_spotify {
            settings::set(&conn, settings::LASTFM_SCROBBLE_SPOTIFY, if b { "true" } else { "false" })?;
        }
        if let Some(b) = patch.normalize_volume {
            settings::set(&conn, settings::NORMALIZE_VOLUME, if b { "true" } else { "false" })?;
        }
        if let Some(b) = patch.soundcloud_web_mode {
            settings::set(&conn, settings::SOUNDCLOUD_WEB_MODE, if b { "true" } else { "false" })?;
        }
        if let Some(m) = patch.spotify_sync_interval_minutes {
            if !(1..=1440).contains(&m) {
                return Err(AppError::InvalidInput("sync interval must be between 1 and 1440 minutes".into()));
            }
            settings::set(&conn, settings::SPOTIFY_SYNC_INTERVAL, &(m * 60).to_string())?;
            state.sync.wake.notify_one();
        }
        if let Some(p) = patch.oauth_port {
            if p < 1024 {
                return Err(AppError::InvalidInput("OAuth port must be 1024 or higher".into()));
            }
            settings::set(&conn, settings::OAUTH_PORT, &p.to_string())?;
        }
    }
    for (key, v) in [
        (secrets::SOUNDCLOUD_CLIENT_SECRET, &patch.soundcloud_client_secret),
        (secrets::LASTFM_SHARED_SECRET, &patch.lastfm_shared_secret),
    ] {
        match v.as_deref().map(str::trim) {
            None => {}
            Some("") => state.secrets.delete(key)?,
            Some(s) => state.secrets.set(key, s)?,
        }
    }
    settings_get(state)
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotSynced {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub tracks: u32,
    pub playlists: u32,
    pub skipped_playlists: Vec<String>,
    /// Imported playlists that are now Spotify Synced.
    pub synced_playlists: u32,
    /// Imported as ordinary playlists, with the reason sync isn't available.
    pub not_synced: Vec<NotSynced>,
    pub errors: Vec<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportProgress {
    provider: String,
    message: String,
}

fn upsert_many(state: &AppState, source: Source, items: &[crate::model::TrackUpsert], in_library: bool) -> AppResult<Vec<String>> {
    let conn = state.conn();
    let tx = conn.unchecked_transaction()?;
    let mut ids = Vec::with_capacity(items.len());
    for t in items {
        ids.push(library::upsert_track(&tx, source, t, in_library)?);
    }
    tx.commit()?;
    Ok(ids)
}

fn save_imported_playlist(state: &AppState, origin: &str, name: &str, ids: &[String]) -> AppResult<()> {
    let pid = {
        let conn = state.conn();
        let pid = match playlists::find_by_origin(&conn, origin)? {
            Some(id) => id,
            None => playlists::create(&conn, name, Some(origin))?.id,
        };
        if spotify_links::get_enabled(&conn, &pid)?.is_none() {
            return playlists::set_items(&conn, &pid, ids);
        }
        pid
    };
    // Spotify Synced: never overwrite (that could drop unsynced Branflakes
    // edits); reconcile through the sync engine instead.
    state.sync.request(&pid, Duration::ZERO);
    Ok(())
}

// ================================================================ spotify

async fn spotify_player<T, F, Fut>(state: &AppState, f: F) -> AppResult<T>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = AppResult<T>>,
{
    let client_id = spotify_client_id(state)?;
    match f(client_id).await {
        Err(AppError::PremiumRequired { provider }) => {
            settings::set(&state.conn(), settings::SPOTIFY_PREMIUM_DENIED, "true")?;
            Err(AppError::PremiumRequired { provider })
        }
        Ok(v) => {
            settings::delete(&state.conn(), settings::SPOTIFY_PREMIUM_DENIED)?;
            Ok(v)
        }
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub async fn spotify_connect(app: AppHandle, state: S<'_>) -> AppResult<()> {
    let client_id = spotify_client_id(&state)?;
    let _guard = state.oauth_lock.try_lock().map_err(|_| AppError::Other("another sign-in is already in progress".into()))?;
    let port = oauth_port(&state)?;
    let listener = oauth::bind(port).await?;
    let pkce = oauth::Pkce::generate();
    let csrf = oauth::random_token(1);
    let redirect = oauth::redirect_uri(port, "spotify");
    open_browser(&app, &state.spotify.authorize_url(&client_id, &redirect, &pkce.challenge, &csrf))?;
    let cb = oauth::wait_for_callback(listener, "spotify", &csrf, OAUTH_TIMEOUT).await?;
    let code = cb.0.get("code").ok_or_else(|| AppError::Auth { provider: "spotify".into(), message: "no authorization code returned".into() })?;
    state.spotify.exchange_code(&client_id, code, &pkce.verifier, &redirect).await?;
    let (id, name) = state.spotify.me(&client_id).await?;
    {
        let conn = state.conn();
        settings::set(&conn, settings::SPOTIFY_ACCOUNT, name.as_deref().unwrap_or(&id))?;
        settings::set(&conn, settings::SPOTIFY_USER_ID, &id)?;
        settings::delete(&conn, settings::SPOTIFY_PREMIUM_DENIED)?;
    }
    // A (re)connect can fix auth or scope errors: sync again soon.
    state.sync.wake.notify_one();
    Ok(())
}

#[tauri::command]
pub async fn spotify_disconnect(state: S<'_>) -> AppResult<()> {
    state.spotify.disconnect().await?;
    let conn = state.conn();
    settings::delete(&conn, settings::SPOTIFY_ACCOUNT)?;
    settings::delete(&conn, settings::SPOTIFY_USER_ID)?;
    settings::delete(&conn, settings::SPOTIFY_PREMIUM_DENIED)
}

#[tauri::command]
pub async fn spotify_import_library(app: AppHandle, state: S<'_>) -> AppResult<ImportReport> {
    let cid = spotify_client_id(&state)?;
    let mut report = ImportReport::default();
    let progress = |m: String| {
        let _ = app.emit("import-progress", ImportProgress { provider: "spotify".into(), message: m });
    };
    progress("Importing saved tracks…".into());
    let saved = state.spotify.saved_tracks(&cid, 20_000).await?;
    report.tracks += upsert_many(&state, Source::Spotify, &saved, true)?.len() as u32;
    let lists = state.spotify.my_playlists(&cid).await?;
    let ctx = sync_ctx(&state)?;
    let me = sync::my_user_id(&ctx).await?;
    // One sync operation at a time (imports enable sync on playlists).
    let _run = state.sync.run_lock.lock().await;
    for p in lists {
        progress(format!("Importing playlist “{}”…", p.name));
        match sync::import_playlist(&ctx, &p, &me).await {
            Ok(sync::ImportResult::Synced { .. }) => {
                report.playlists += 1;
                report.synced_playlists += 1;
            }
            Ok(sync::ImportResult::AlreadySynced { playlist_id }) => {
                // Reconciled by the sync engine; never overwritten here.
                state.sync.request(&playlist_id, Duration::ZERO);
                report.playlists += 1;
                report.synced_playlists += 1;
            }
            Ok(sync::ImportResult::Plain { reason, .. }) => {
                report.playlists += 1;
                report.not_synced.push(NotSynced { name: p.name, reason });
            }
            Ok(sync::ImportResult::Skipped { .. }) => report.skipped_playlists.push(p.name),
            Err(e @ AppError::RateLimited { .. }) => return Err(e),
            Err(e) => report.errors.push(format!("{}: {e}", p.name)),
        }
    }
    Ok(report)
}

#[tauri::command]
pub async fn spotify_devices(state: S<'_>) -> AppResult<Vec<SpotifyDevice>> {
    let cid = spotify_client_id(&state)?;
    state.spotify.devices(&cid).await
}

#[tauri::command]
pub async fn spotify_playback_state(state: S<'_>) -> AppResult<Option<SpotifyPlaybackState>> {
    let cid = spotify_client_id(&state)?;
    state.spotify.playback_state(&cid).await
}

#[tauri::command]
pub async fn spotify_play_track(state: S<'_>, track_id: String, device_id: Option<String>, position_ms: Option<u64>) -> AppResult<()> {
    let t = library::get_track(&state.conn(), &track_id)?;
    if t.source != Source::Spotify {
        return Err(AppError::InvalidInput("not a Spotify track".into()));
    }
    let uri = t.provider_uri.unwrap_or_else(|| format!("spotify:track:{}", t.provider_id));
    let sp = &state.spotify;
    spotify_player(&state, |cid| async move { sp.play_uri(&cid, device_id.as_deref(), &uri, position_ms.unwrap_or(0)).await }).await
}

#[tauri::command]
pub async fn spotify_control(state: S<'_>, action: String, device_id: Option<String>, value: Option<u64>) -> AppResult<()> {
    let sp = &state.spotify;
    let d = device_id.clone();
    spotify_player(&state, |cid| async move {
        let d = d.as_deref();
        match action.as_str() {
            "pause" => sp.pause(&cid, d).await,
            "resume" => sp.resume(&cid, d).await,
            "seek" => sp.seek(&cid, d, value.unwrap_or(0)).await,
            "volume" => sp.set_volume(&cid, d, value.unwrap_or(50) as u32).await,
            "transfer" => sp.transfer(&cid, d.ok_or_else(|| AppError::InvalidInput("device required".into()))?, value == Some(1)).await,
            other => Err(AppError::InvalidInput(format!("unknown Spotify action {other}"))),
        }
    })
    .await
}

#[tauri::command]
pub async fn spotify_add_to_queue(state: S<'_>, track_id: String, device_id: Option<String>) -> AppResult<()> {
    let t = library::get_track(&state.conn(), &track_id)?;
    let uri = t.provider_uri.unwrap_or_else(|| format!("spotify:track:{}", t.provider_id));
    let sp = &state.spotify;
    spotify_player(&state, |cid| async move { sp.add_to_queue(&cid, device_id.as_deref(), &uri).await }).await
}

/// Access token for the optional in-app Web Playback SDK device. The SDK
/// requires the page to supply a token; it is passed only to Spotify's SDK.
#[tauri::command]
pub async fn spotify_sdk_token(state: S<'_>) -> AppResult<String> {
    let cid = spotify_client_id(&state)?;
    Ok(state.spotify.sdk_access_token(&cid).await?.expose().to_string())
}

// ================================================== spotify playlist sync

fn sync_ctx(state: &AppState) -> AppResult<SyncCtx<'_>> {
    if !state.secrets.has(secrets::SPOTIFY_TOKENS) {
        return Err(AppError::NotConnected { provider: "spotify".into() });
    }
    Ok(SyncCtx { db: &state.db, spotify: &state.spotify, client_id: spotify_client_id(state)?, artwork_dir: state.paths.artwork_dir.clone() })
}

fn emit_sync(app: &AppHandle, ev: SyncEvent) {
    let _ = app.emit("playlist-sync", ev);
}

/// Run one sync (manual or scheduled), emitting `playlist-sync` events.
/// Refuses a second simultaneous sync of the same playlist.
pub async fn run_sync(app: &AppHandle, state: &AppState, playlist_id: &str, manual: bool) -> AppResult<SyncReport> {
    let Some(_guard) = state.sync.begin(playlist_id) else {
        return Err(AppError::Other("this playlist is already syncing".into()));
    };
    let ctx = sync_ctx(state)?;
    emit_sync(app, SyncEvent { playlist_id: playlist_id.into(), status: "syncing".into(), changed: false, message: None });
    let _run = state.sync.run_lock.lock().await;
    let result = sync::sync_playlist(&ctx, playlist_id, manual).await;
    match &result {
        Ok(r) => emit_sync(
            app,
            SyncEvent {
                playlist_id: playlist_id.into(),
                // "removed": the Spotify playlist was confirmed gone and the
                // Branflakes copy was removed (reported once).
                status: if r.removed.is_some() { "removed".into() } else { "ok".into() },
                changed: r.changed_locally(),
                message: r.removed.clone().or_else(|| if r.notices.is_empty() { None } else { Some(r.notices.join(" ")) }),
            },
        ),
        Err(e) => {
            log::info!("spotify sync of {playlist_id} failed: {e}");
            emit_sync(app, SyncEvent { playlist_id: playlist_id.into(), status: "error".into(), changed: false, message: Some(e.to_string()) })
        }
    }
    result
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpotifyPlaylistOption {
    pub id: String,
    pub name: String,
    pub total: Option<u32>,
    pub owned: bool,
    pub collaborative: bool,
    /// Can this playlist be synced (owned or collaborative)?
    pub can_sync: bool,
    pub reason: Option<String>,
    /// Branflakes playlist it is synced with, if any.
    pub synced_playlist_id: Option<String>,
}

/// The user's Spotify playlists with sync eligibility.
#[tauri::command]
pub async fn spotify_sync_candidates(state: S<'_>) -> AppResult<Vec<SpotifyPlaylistOption>> {
    let cid = spotify_client_id(&state)?;
    let cached = settings::get_nonempty(&state.conn(), settings::SPOTIFY_USER_ID)?;
    let me = match cached {
        Some(id) => id,
        None => {
            let (id, _) = state.spotify.me(&cid).await?;
            settings::set(&state.conn(), settings::SPOTIFY_USER_ID, &id)?;
            id
        }
    };
    let lists = state.spotify.my_playlists(&cid).await?;
    let conn = state.conn();
    let mut out = Vec::with_capacity(lists.len());
    for p in lists {
        let owned = p.owner_id.as_deref() == Some(me.as_str());
        let can_sync = owned || p.collaborative;
        out.push(SpotifyPlaylistOption {
            synced_playlist_id: spotify_links::find_by_spotify_id(&conn, &p.id)?.filter(|l| l.enabled).map(|l| l.playlist_id),
            reason: (!can_sync).then(|| "Owned by another Spotify user. Spotify only lets apps change playlists you own or collaborate on.".to_string()),
            id: p.id,
            name: p.name,
            total: p.total,
            owned,
            collaborative: p.collaborative,
            can_sync,
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn spotify_sync_enable(app: AppHandle, state: S<'_>, spotify_playlist_id: String) -> AppResult<playlists::Playlist> {
    let ctx = sync_ctx(&state)?;
    let _run = state.sync.run_lock.lock().await;
    let pid = sync::enable(&ctx, &spotify_playlist_id).await?;
    emit_sync(&app, SyncEvent { playlist_id: pid.clone(), status: "ok".into(), changed: true, message: None });
    playlists::get(&state.conn(), &pid)
}

/// Stop syncing. The Branflakes playlist and the Spotify playlist both stay.
#[tauri::command]
pub fn spotify_sync_disable(state: S<'_>, playlist_id: String) -> AppResult<()> {
    sync::disable(&state.conn(), &playlist_id)
}

#[tauri::command]
pub async fn spotify_sync_now(app: AppHandle, state: S<'_>, playlist_id: String) -> AppResult<SyncReport> {
    run_sync(&app, &state, &playlist_id, true).await
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncAllReport {
    pub synced: u32,
    pub failed: Vec<String>,
}

#[tauri::command]
pub async fn spotify_sync_all(app: AppHandle, state: S<'_>) -> AppResult<SyncAllReport> {
    sync_ctx(&state)?;
    let links = spotify_links::list_enabled(&state.conn())?;
    let mut report = SyncAllReport::default();
    for l in links {
        let name = playlists::get(&state.conn(), &l.playlist_id).map(|p| p.name).unwrap_or_default();
        match run_sync(&app, &state, &l.playlist_id, true).await {
            Ok(_) => report.synced += 1,
            Err(e) => report.failed.push(format!("{name}: {e}")),
        }
    }
    Ok(report)
}

/// Backend scheduler: syncs every linked playlist every interval (default
/// 5 min) and edited playlists shortly after each edit. Runs regardless of
/// the window state; stops on `SyncEngine::shutdown`.
pub async fn sync_scheduler(app: AppHandle) {
    use std::time::Instant;
    let state = app.state::<AppState>();
    let eng = &state.sync;
    // Syncs interrupted by a restart run first.
    if let Ok(ids) = spotify_links::recover_interrupted(&state.conn()) {
        for id in ids {
            eng.request(&id, Duration::from_secs(5));
        }
    }
    // Deletions interrupted by a restart: finish or undo them first.
    let pending = spotify_links::pending_deletes(&state.conn());
    if let (Ok(ids), Ok(ctx)) = (pending, sync_ctx(&state)) {
        for id in ids {
            match sync::resume_delete(&ctx, &id).await {
                Ok(true) => emit_sync(&app, SyncEvent { playlist_id: id, status: "deleted".into(), changed: true, message: None }),
                Ok(false) => {}
                Err(e) => log::info!("resuming deletion of {id} postponed: {e}"),
            }
        }
    }
    // Imported Spotify playlists that can't be synced (checked once per run).
    let mut not_linkable: std::collections::HashSet<String> = std::collections::HashSet::new();
    // First full pass shortly after startup, then every interval.
    let mut last_full = Instant::now() - sync::interval(&state.conn()) + Duration::from_secs(30);
    loop {
        if eng.is_shutdown() {
            break;
        }
        let interval = sync::interval(&state.conn());
        let next_full = last_full + interval;
        let mut wake = eng.next_due().map_or(next_full, |d| d.min(next_full));
        if let Some(b) = eng.backoff_until() {
            wake = wake.max(b);
        }
        tokio::select! {
            _ = tokio::time::sleep_until(wake.into()) => {}
            _ = eng.wake.notified() => {}
        }
        if eng.is_shutdown() {
            break;
        }
        if eng.backoff_until().is_some() {
            continue;
        }
        let now = Instant::now();
        let connected = state.secrets.has(secrets::SPOTIFY_TOKENS) && spotify_client_id(&state).is_ok();
        let full = now >= last_full + interval;
        let mut ids = eng.take_due(now);
        if full {
            last_full = now;
            if connected {
                // Earlier imports become Spotify Synced automatically.
                if let Ok(ctx) = sync_ctx(&state) {
                    let linked = {
                        let _run = eng.run_lock.lock().await;
                        sync::auto_link_imported(&ctx, &mut not_linkable).await
                    };
                    match linked {
                        Ok(ids) => {
                            for id in ids {
                                emit_sync(&app, SyncEvent { playlist_id: id, status: "ok".into(), changed: true, message: None });
                            }
                        }
                        Err(e) => log::info!("auto-sync of imported playlists postponed: {e}"),
                    }
                }
                for l in spotify_links::list_enabled(&state.conn()).unwrap_or_default() {
                    if !ids.contains(&l.playlist_id) {
                        ids.push(l.playlist_id);
                    }
                }
            }
        }
        if !connected || ids.is_empty() {
            continue;
        }
        for id in ids {
            if eng.is_shutdown() {
                break;
            }
            if let Err(e) = run_sync(&app, &state, &id, false).await {
                if let Some(d) = sync::backoff_for(&e, interval) {
                    log::info!("spotify sync paused for {d:?}: {e}");
                    eng.back_off(d);
                    // Retry this playlist after the pause.
                    eng.request(&id, d);
                    break;
                }
            }
        }
    }
    log::info!("spotify sync scheduler stopped");
}

// ====================================================== find alternative

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlternativesResult {
    /// Matching tracks, best first.
    pub candidates: Vec<crate::alternatives::Ranked>,
    /// The search failed (never reported as "no results").
    pub error: Option<AppError>,
    /// The provider isn't set up, with what to do about it.
    pub unavailable: Option<String>,
}

/// Search one provider for alternatives to an (unavailable) track. The UI
/// calls "local" first, then "soundcloud". Results are filtered to the same
/// song (title and artist) and ranked; nothing is invented on failure.
#[tauri::command]
pub async fn alternatives_search(state: S<'_>, track_id: String, provider: String) -> AppResult<AlternativesResult> {
    use crate::alternatives::{rank, search_query, title_fragment};
    let original = library::get_track(&state.conn(), &track_id)?;
    match provider.as_str() {
        "local" => {
            let conn = state.conn();
            let mut found = Vec::new();
            let artist = original.artist.split([',', '&']).next().unwrap_or("").trim().to_string();
            for q in [title_fragment(&original.title), artist] {
                if q.chars().count() < 2 {
                    continue;
                }
                found.extend(library::list_tracks(
                    &conn,
                    &library::TrackQuery {
                        search: Some(q),
                        source: Some(Source::Local),
                        include_unavailable: Some(false),
                        limit: Some(300),
                        ..Default::default()
                    },
                )?);
            }
            Ok(AlternativesResult { candidates: rank(&original, found), error: None, unavailable: None })
        }
        "soundcloud" => {
            let q = search_query(&original);
            let official = state.secrets.has(secrets::SOUNDCLOUD_TOKENS);
            let web = soundcloud_web_enabled(&state)?;
            let result = if official {
                match soundcloud_creds(&state) {
                    Ok(creds) => state.soundcloud.search(&creds, &q, 30).await,
                    Err(e) => Err(e),
                }
            } else if web {
                state.soundcloud_web.search(&q, 30, 0).await
            } else {
                return Ok(AlternativesResult {
                    candidates: Vec::new(),
                    error: None,
                    unavailable: Some("SoundCloud isn't connected. Connect it, or turn on web mode, in Settings > Accounts.".into()),
                });
            };
            match result {
                Ok(items) => {
                    // Stored as non-library tracks (like search results) so they can be added to playlists.
                    let ids = upsert_many(&state, Source::Soundcloud, &items, false)?;
                    let tracks = library::get_tracks(&state.conn(), &ids)?;
                    Ok(AlternativesResult { candidates: rank(&original, tracks), error: None, unavailable: None })
                }
                Err(e) => Ok(AlternativesResult { candidates: Vec::new(), error: Some(e), unavailable: None }),
            }
        }
        other => Err(AppError::InvalidInput(format!("unknown provider {other}"))),
    }
}

/// Add a chosen alternative right after the original in a playlist. The
/// original stays; nothing is sent to Spotify (for a Spotify Synced
/// playlist the alternative exists only in Branflakes).
#[tauri::command]
pub fn alternatives_add(
    state: S<'_>,
    playlist_id: String,
    original_track_id: String,
    alternative_track_id: String,
    position: Option<usize>,
) -> AppResult<crate::db::alternatives::AddAlternativeOutcome> {
    let conn = state.conn();
    let original = library::get_track(&conn, &original_track_id)?;
    let alternative = library::get_track(&conn, &alternative_track_id)?;
    crate::db::alternatives::add(&conn, &playlist_id, &original, &alternative, position)
}

#[tauri::command]
pub fn alternatives_list(state: S<'_>, playlist_id: String) -> AppResult<Vec<crate::db::alternatives::Relation>> {
    crate::db::alternatives::list(&state.conn(), &playlist_id)
}


// ============================================================= soundcloud

#[tauri::command]
pub async fn soundcloud_connect(app: AppHandle, state: S<'_>) -> AppResult<()> {
    let creds = soundcloud_creds(&state)?;
    let _guard = state.oauth_lock.try_lock().map_err(|_| AppError::Other("another sign-in is already in progress".into()))?;
    let port = oauth_port(&state)?;
    let listener = oauth::bind(port).await?;
    let pkce = oauth::Pkce::generate();
    let csrf = oauth::random_token(1);
    let redirect = oauth::redirect_uri(port, "soundcloud");
    open_browser(&app, &state.soundcloud.authorize_url(&creds.client_id, &redirect, &pkce.challenge, &csrf))?;
    let cb = oauth::wait_for_callback(listener, "soundcloud", &csrf, OAUTH_TIMEOUT).await?;
    let code = cb.0.get("code").ok_or_else(|| AppError::Auth { provider: "soundcloud".into(), message: "no authorization code returned".into() })?;
    state.soundcloud.exchange_code(&creds, code, &pkce.verifier, &redirect).await?;
    let name = state.soundcloud.me(&creds).await?;
    settings::set(&state.conn(), settings::SOUNDCLOUD_ACCOUNT, &name)?;
    Ok(())
}

#[tauri::command]
pub async fn soundcloud_disconnect(state: S<'_>) -> AppResult<()> {
    state.soundcloud.disconnect().await?;
    settings::delete(&state.conn(), settings::SOUNDCLOUD_ACCOUNT)
}

#[tauri::command]
pub async fn soundcloud_import_library(app: AppHandle, state: S<'_>) -> AppResult<ImportReport> {
    let creds = soundcloud_creds(&state)?;
    let mut report = ImportReport::default();
    let progress = |m: String| {
        let _ = app.emit("import-progress", ImportProgress { provider: "soundcloud".into(), message: m });
    };
    progress("Importing liked tracks…".into());
    let likes = state.soundcloud.liked_tracks(&creds, 20_000).await?;
    report.tracks += upsert_many(&state, Source::Soundcloud, &likes, true)?.len() as u32;
    for p in state.soundcloud.my_playlists(&creds).await? {
        progress(format!("Importing playlist “{}”…", p.title));
        match state.soundcloud.playlist_tracks(&creds, &p.urn).await {
            Ok(items) => {
                let ids = upsert_many(&state, Source::Soundcloud, &items, false)?;
                save_imported_playlist(&state, &p.urn, &p.title, &ids)?;
                report.playlists += 1;
            }
            Err(e @ AppError::RateLimited { .. }) => return Err(e),
            Err(e) => report.errors.push(format!("{}: {e}", p.title)),
        }
    }
    Ok(report)
}

#[tauri::command]
pub async fn soundcloud_resolve_stream(state: S<'_>, track_id: String) -> AppResult<StreamInfo> {
    let t = library::get_track(&state.conn(), &track_id)?;
    resolve_soundcloud_stream(&state, &t).await
}

async fn resolve_soundcloud_stream(state: &AppState, t: &Track) -> AppResult<StreamInfo> {
    if t.source != Source::Soundcloud {
        return Err(AppError::InvalidInput("not a SoundCloud track".into()));
    }
    if t.availability == crate::model::Availability::Unavailable {
        return Err(AppError::Unsupported(t.unavailable_reason.clone().unwrap_or_else(|| "track is unavailable".into())));
    }
    // Official API when connected; otherwise the opt-in unofficial web mode.
    if state.secrets.has(secrets::SOUNDCLOUD_TOKENS) {
        let creds = soundcloud_creds(state)?;
        return state.soundcloud.resolve_stream(&creds, &t.provider_id).await;
    }
    if soundcloud_web_enabled(state)? {
        return state.soundcloud_web.resolve_stream(&t.provider_id).await;
    }
    Err(AppError::NotConnected { provider: "soundcloud".into() })
}

/// Serialises loudness analyses: one decode at a time, and a second request for
/// the same track finds the first one's cached result.
static LOUDNESS_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Integrated loudness (LUFS) for volume normalization, analysed on first
/// request and cached. `None` when it can't be measured (yet).
///
/// Spotify audio can't be decoded, so for a Spotify track this uses, in order:
/// the same song in the local library (exact title + artist, same length),
/// then a measurement captured live from the in-app player
/// (`loudness_capture_*`). `None` means neither exists yet.
#[tauri::command]
pub async fn track_loudness(state: S<'_>, track_id: String) -> AppResult<Option<f64>> {
    let t = library::get_track(&state.conn(), &track_id)?;
    if t.source != Source::Spotify {
        return measured_loudness(&state, t).await;
    }
    if let Some(local) = local_copy(&state, &t)? {
        match measured_loudness(&state, local).await {
            Ok(Some(l)) => return Ok(Some(l)),
            Ok(None) => {}
            Err(e) => log::warn!("loudness of local copy failed: {e}"),
        }
    }
    Ok(crate::loudness::cached(&state.conn(), &track_id, SPOTIFY_CAPTURE_KEY)?.flatten())
}

const SPOTIFY_CAPTURE_KEY: &str = "spotify:capture";

/// The same recording in the local library, if there is one: exact title and
/// primary artist, and (when both are known) a length within 3 seconds so a
/// live version or remix isn't mistaken for it.
fn local_copy(state: &AppState, t: &Track) -> AppResult<Option<Track>> {
    use crate::alternatives::{rank, title_fragment};
    let q = title_fragment(&t.title);
    if q.chars().count() < 2 {
        return Ok(None);
    }
    let found = library::list_tracks(
        &state.conn(),
        &library::TrackQuery { search: Some(q), source: Some(Source::Local), include_unavailable: Some(false), limit: Some(300), ..Default::default() },
    )?;
    Ok(rank(t, found)
        .into_iter()
        .filter(|r| r.score >= 100)
        .find(|r| match (t.duration_ms, r.track.duration_ms) {
            (Some(a), Some(b)) => a.abs_diff(b) <= 3_000,
            _ => true,
        })
        .map(|r| r.track))
}

/// Decode-and-measure path for local files and SoundCloud streams.
async fn measured_loudness(state: &AppState, t: Track) -> AppResult<Option<f64>> {
    let track_id = t.id.clone();
    let key = match t.source {
        Source::Local => {
            let path = t.file_path.clone().ok_or_else(|| AppError::InvalidInput("local track has no file path".into()))?;
            let meta = std::fs::metadata(&path)?;
            let mtime = meta.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
            format!("file:{path}|{}|{mtime}", meta.len())
        }
        Source::Soundcloud => format!("soundcloud:{}", t.provider_id),
        Source::Spotify => return Ok(None),
    };
    if let Some(v) = crate::loudness::cached(&state.conn(), &track_id, &key)? {
        return Ok(v);
    }
    let _guard = LOUDNESS_LOCK.lock().await;
    if let Some(v) = crate::loudness::cached(&state.conn(), &track_id, &key)? {
        return Ok(v);
    }
    let measured = match t.source {
        Source::Local => {
            let path = PathBuf::from(t.file_path.unwrap_or_default());
            tokio::task::spawn_blocking(move || crate::loudness::measure_file(&path))
                .await
                .map_err(|e| AppError::Other(format!("loudness task failed: {e}")))?
        }
        _ => {
            // Transient failures (network, not connected) are not cached.
            let stream = resolve_soundcloud_stream(state, &t).await?;
            if stream.preview {
                return Ok(None);
            }
            let (bytes, ext) = crate::loudness::download_stream(&crate::http::client(), &stream.url, &stream.protocol).await?;
            tokio::task::spawn_blocking(move || crate::loudness::measure_bytes(bytes, ext.as_deref()))
                .await
                .map_err(|e| AppError::Other(format!("loudness task failed: {e}")))?
        }
    };
    let lufs = match measured {
        Ok(v) => v,
        Err(AppError::Io(e)) => return Err(AppError::Io(e)),
        Err(e) => {
            log::warn!("loudness analysis failed for {track_id}: {e}");
            None
        }
    };
    crate::loudness::store(&state.conn(), &track_id, &key, lufs)?;
    Ok(lufs)
}

/// Start measuring a Spotify track playing on the in-app device. `gain` is
/// the player volume applied right now (0..1).
#[tauri::command]
pub fn loudness_capture_start(track_id: String, gain: f64, settle_ms: u64) {
    crate::loudness_capture::start(&track_id, gain, Duration::from_millis(settle_ms));
}

/// The in-app player volume changed; audio is ignored for `settle_ms`.
#[tauri::command]
pub fn loudness_capture_gain(gain: f64, settle_ms: u64) {
    crate::loudness_capture::set_gain(gain, Duration::from_millis(settle_ms));
}

#[tauri::command]
pub fn loudness_capture_status() -> Option<crate::loudness_capture::CaptureStatus> {
    crate::loudness_capture::status()
}

/// Stop measuring. The result is saved for the track when enough of it was
/// heard (70 % of its length, or 90 s if the length is unknown); a short
/// listen isn't representative of the whole song.
#[tauri::command]
pub fn loudness_capture_stop(state: S<'_>) -> AppResult<Option<f64>> {
    let Some(st) = crate::loudness_capture::stop() else { return Ok(None) };
    let Some(lufs) = st.lufs else { return Ok(None) };
    let conn = state.conn();
    let Ok(t) = library::get_track(&conn, &st.track_id) else { return Ok(None) };
    let needed = t.duration_ms.map(|d| d as f64 / 1000.0 * 0.7).unwrap_or(90.0);
    if t.source != Source::Spotify || st.seconds < needed {
        return Ok(None);
    }
    crate::loudness::store(&conn, &st.track_id, SPOTIFY_CAPTURE_KEY, Some(lufs))?;
    Ok(Some(lufs))
}

fn soundcloud_web_enabled(state: &AppState) -> AppResult<bool> {
    settings::get_bool(&state.conn(), settings::SOUNDCLOUD_WEB_MODE, false)
}

/// Unofficial web mode: import a pasted track / playlist / profile (public likes) link.
#[tauri::command]
pub async fn soundcloud_web_import(state: S<'_>, url: String) -> AppResult<ImportReport> {
    use crate::providers::soundcloud_web::WebImport;
    if !soundcloud_web_enabled(&state)? {
        return Err(AppError::NotConfigured { provider: "soundcloud".into(), message: "enable SoundCloud web mode in Settings first".into() });
    }
    let mut report = ImportReport::default();
    match state.soundcloud_web.import_url(&url, 5000).await? {
        WebImport::Track(t) => {
            report.tracks = upsert_many(&state, Source::Soundcloud, &[t], true)?.len() as u32;
        }
        WebImport::Playlist { urn, title, tracks } => {
            let ids = upsert_many(&state, Source::Soundcloud, &tracks, true)?;
            report.tracks = ids.len() as u32;
            save_imported_playlist(&state, &urn, &title, &ids)?;
            report.playlists = 1;
        }
        WebImport::UserLikes { username, user_id, tracks } => {
            let ids = upsert_many(&state, Source::Soundcloud, &tracks, true)?;
            report.tracks = ids.len() as u32;
            save_imported_playlist(&state, &format!("soundcloud:users:{user_id}:likes"), &format!("{username} – SoundCloud likes"), &ids)?;
            report.playlists = 1;
        }
    }
    Ok(report)
}

// ================================================================= search

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSearchResults {
    pub spotify: Vec<Track>,
    pub soundcloud: Vec<Track>,
    pub errors: Vec<ProviderError>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderError {
    pub provider: String,
    pub error: AppError,
}

/// Search connected streaming providers. Results are stored as non-library
/// tracks so they get stable IDs and can be queued or added to playlists.
#[tauri::command]
pub async fn search_providers(state: S<'_>, query: String, spotify_offset: Option<u32>) -> AppResult<ProviderSearchResults> {
    let q = query.trim().to_string();
    if q.is_empty() {
        return Ok(ProviderSearchResults::default());
    }
    let mut out = ProviderSearchResults::default();
    if state.secrets.has(secrets::SPOTIFY_TOKENS) {
        match spotify_client_id(&state) {
            Ok(cid) => match state.spotify.search(&cid, &q, spotify_offset.unwrap_or(0)).await {
                Ok(items) => {
                    let ids = upsert_many(&state, Source::Spotify, &items, false)?;
                    out.spotify = library::get_tracks(&state.conn(), &ids)?;
                }
                Err(e) => out.errors.push(ProviderError { provider: "spotify".into(), error: e }),
            },
            Err(e) => out.errors.push(ProviderError { provider: "spotify".into(), error: e }),
        }
    }
    if !state.secrets.has(secrets::SOUNDCLOUD_TOKENS) && soundcloud_web_enabled(&state)? {
        match state.soundcloud_web.search(&q, 20, 0).await {
            Ok(items) => {
                let ids = upsert_many(&state, Source::Soundcloud, &items, false)?;
                out.soundcloud = library::get_tracks(&state.conn(), &ids)?;
            }
            Err(e) => out.errors.push(ProviderError { provider: "soundcloud (web mode)".into(), error: e }),
        }
    } else if state.secrets.has(secrets::SOUNDCLOUD_TOKENS) {
        match soundcloud_creds(&state) {
            Ok(creds) => match state.soundcloud.search(&creds, &q, 20).await {
                Ok(items) => {
                    let ids = upsert_many(&state, Source::Soundcloud, &items, false)?;
                    out.soundcloud = library::get_tracks(&state.conn(), &ids)?;
                }
                Err(e) => out.errors.push(ProviderError { provider: "soundcloud".into(), error: e }),
            },
            Err(e) => out.errors.push(ProviderError { provider: "soundcloud".into(), error: e }),
        }
    }
    Ok(out)
}

// ================================================================= lastfm

#[tauri::command]
pub async fn lastfm_connect(app: AppHandle, state: S<'_>) -> AppResult<String> {
    let creds = lastfm_creds(&state)?;
    let _guard = state.oauth_lock.try_lock().map_err(|_| AppError::Other("another sign-in is already in progress".into()))?;
    let token = state.lastfm.get_token(&creds).await?;
    open_browser(&app, &state.lastfm.auth_url(&creds.api_key, &token))?;
    // Desktop auth: poll auth.getSession until the user approves (error 14 = not yet).
    let deadline = std::time::Instant::now() + OAUTH_TIMEOUT;
    loop {
        tokio::time::sleep(Duration::from_secs(3)).await;
        match state.lastfm.get_session(&creds, &token).await {
            Ok((sk, name)) => {
                state.secrets.set(secrets::LASTFM_SESSION_KEY, sk.expose())?;
                settings::set(&state.conn(), settings::LASTFM_ACCOUNT, &name)?;
                return Ok(name);
            }
            Err(CallError::Api { code: 14, .. }) if std::time::Instant::now() < deadline => continue,
            Err(CallError::Api { code: 14, .. }) => {
                return Err(AppError::Auth { provider: "lastfm".into(), message: "timed out waiting for authorization".into() })
            }
            Err(e) => return Err(e.into()),
        }
    }
}

#[tauri::command]
pub fn lastfm_disconnect(state: S<'_>) -> AppResult<()> {
    state.secrets.delete(secrets::LASTFM_SESSION_KEY)?;
    settings::delete(&state.conn(), settings::LASTFM_ACCOUNT)
}

/// Send Now Playing for an eligible track. Failures are not retried (per Last.fm guidance).
#[tauri::command]
pub async fn lastfm_now_playing(state: S<'_>, track_id: String) -> AppResult<bool> {
    let (track, artist, allowed) = {
        let conn = state.conn();
        let track = library::get_track(&conn, &track_id)?;
        let allowed = scrobbler::source_allowed(&conn, track.source)?.is_ok()
            && settings::get_bool(&conn, settings::LASTFM_ENABLED, true)?;
        let artist = library::scrobble_artist(&conn, &track)?;
        (track, artist, allowed)
    };
    if !allowed || artist.trim().is_empty() {
        return Ok(false);
    }
    let (Ok(creds), Ok(sk)) = (lastfm_creds(&state), lastfm_session(&state)) else { return Ok(false) };
    let np = NowPlaying {
        artist,
        track: track.title,
        album: Some(track.album),
        album_artist: Some(track.album_artist),
        track_number: track.track_number,
        duration_s: track.duration_ms.map(|d| (d / 1000) as u32),
    };
    match state.lastfm.update_now_playing(&creds, &sk, &np).await {
        Ok(()) => Ok(true),
        Err(e) => {
            log::info!("lastfm now playing not sent: {e}");
            Ok(false)
        }
    }
}

#[tauri::command]
pub async fn lastfm_submit_play(app: AppHandle, state: S<'_>, track_id: String, started_at: i64, played_ms: u64) -> AppResult<SubmitOutcome> {
    let outcome = scrobbler::submit_play(&state.conn(), &track_id, started_at, played_ms)?;
    if outcome.queued {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let st = app.state::<AppState>();
            if let Err(e) = flush_inner(&st).await {
                log::info!("scrobble flush deferred: {e}");
            }
        });
    }
    Ok(outcome)
}

pub async fn flush_inner(state: &AppState) -> AppResult<FlushReport> {
    let creds = lastfm_creds(state)?;
    let sk = lastfm_session(state)?;
    let report = scrobbler::flush(&state.db, &state.lastfm, &creds, &sk).await?;
    if report.session_invalid {
        state.secrets.delete(secrets::LASTFM_SESSION_KEY)?;
        settings::delete(&state.conn(), settings::LASTFM_ACCOUNT)?;
    }
    Ok(report)
}

#[tauri::command]
pub async fn lastfm_flush(state: S<'_>) -> AppResult<FlushReport> {
    flush_inner(&state).await
}

#[tauri::command]
pub fn lastfm_queue_status(state: S<'_>) -> AppResult<scrobbles::ScrobbleQueueStatus> {
    scrobbles::status(&state.conn())
}

#[tauri::command]
pub fn lastfm_clear_failed(state: S<'_>) -> AppResult<()> {
    scrobbles::clear_failed(&state.conn())
}

// ================================================================== media

#[tauri::command]
pub fn media_update(state: S<'_>, update: MediaUpdate) -> AppResult<()> {
    if let Some(t) = state.taskbar.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        t.update(update.toolbar_state());
    }
    if let Some(m) = state.media.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        m.update(update);
    }
    Ok(())
}

#[tauri::command]
pub fn app_info(state: S<'_>) -> AppResult<serde_json::Value> {
    Ok(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "dataDir": state.paths.data_dir,
        "schemaVersion": db::schema_version(&state.conn())?,
        "mediaControls": state.media.lock().map(|m| m.is_some()).unwrap_or(false),
        "taskbarControls": state.taskbar.lock().map(|m| m.is_some()).unwrap_or(false),
    }))
}
