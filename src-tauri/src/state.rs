//! Shared application state managed by Tauri.

use crate::error::{AppError, AppResult};
use crate::media::MediaControlsHandle;
use crate::providers::{lastfm::LastFmAdapter, soundcloud::SoundCloudAdapter, soundcloud_web::SoundCloudWebAdapter, spotify::SpotifyAdapter};
use crate::secrets::SecretStore;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub artwork_dir: PathBuf,
    pub backup_dir: PathBuf,
    /// Local audio that arrived in shared `.bran` playlists (content-addressed).
    pub shared_dir: PathBuf,
}

impl AppPaths {
    pub fn new(data_dir: PathBuf) -> Self {
        AppPaths {
            db_path: data_dir.join("library.sqlite3"),
            artwork_dir: data_dir.join("artwork"),
            backup_dir: data_dir.join("backups"),
            shared_dir: data_dir.join("shared"),
            data_dir,
        }
    }
}

pub struct AppState {
    pub db: Mutex<Connection>,
    pub paths: AppPaths,
    pub secrets: Arc<dyn SecretStore>,
    pub spotify: SpotifyAdapter,
    pub soundcloud: SoundCloudAdapter,
    /// Unofficial, opt-in (Settings). See providers/soundcloud_web.rs.
    pub soundcloud_web: SoundCloudWebAdapter,
    pub lastfm: LastFmAdapter,
    /// Genius text + LRCLIB time-synced lyrics.
    pub lyrics: crate::lyrics::LyricsClient,
    pub media: Mutex<Option<MediaControlsHandle>>,
    /// Taskbar thumbnail toolbar (Windows), separate from SMTC.
    pub taskbar: Mutex<Option<crate::taskbar::TaskbarHandle>>,
    /// Spotify playlist sync scheduler state.
    pub sync: crate::sync::SyncEngine,
    /// Only one interactive sign-in may run at a time (shared loopback port).
    pub oauth_lock: tokio::sync::Mutex<()>,
}

impl AppState {
    pub fn conn(&self) -> MutexGuard<'_, Connection> {
        // A poisoned lock only means another command panicked mid-query;
        // SQLite itself is still consistent, so recover the guard.
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A separate connection for long-running work (scans, file writes) so
    /// the UI is not blocked on the shared connection.
    pub fn open_worker_connection(&self) -> AppResult<Connection> {
        crate::db::open(&self.paths.db_path)
    }
}

pub fn require<T>(v: Option<T>, provider: &str, what: &str) -> AppResult<T> {
    v.ok_or_else(|| AppError::NotConfigured { provider: provider.into(), message: format!("{what} is not set (Settings → Accounts)") })
}
