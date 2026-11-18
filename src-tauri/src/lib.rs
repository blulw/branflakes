//! branflakes — Tauri backend.

pub mod commands;
pub mod db;
pub mod error;
pub mod http;
pub mod media;
pub mod model;
pub mod oauth;
pub mod providers;
pub mod scrobbler;
pub mod secrets;
pub mod state;
pub mod sync;
pub mod taskbar;
pub mod alternatives;
pub mod loudness;
pub mod loudness_capture;
pub mod lyrics;
pub mod share;

use providers::{lastfm::LastFmAdapter, soundcloud::SoundCloudAdapter, spotify::SpotifyAdapter};
use secrets::{KeyringStore, SecretStore};
use state::{AppPaths, AppState};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Never log at debug/trace for HTTP crates: they can print headers.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,reqwest=warn,hyper=warn,hyper_util=warn"))
        .init();

    // `.bran` files the app was opened with (double-click / "Open with").
    share::push_pending(share::bran_args(std::env::args()));

    tauri::Builder::default()
        // Must be first: a second launch (e.g. double-clicking a .bran while
        // the app runs) hands its arguments to this instance and exits.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let files = share::bran_args(argv);
            if !files.is_empty() {
                share::push_pending(files);
                let _ = app.emit("bran-open", ());
            }
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let paths = AppPaths::new(data_dir);
            let conn = db::open(&paths.db_path)?;

            // Allow the asset protocol to serve cached artwork and the user's
            // library folders (and nothing else).
            commands::allow_asset_dir(app.handle(), &paths.artwork_dir);
            std::fs::create_dir_all(&paths.artwork_dir)?;
            commands::allow_asset_dir(app.handle(), &paths.shared_dir);
            std::fs::create_dir_all(&paths.shared_dir)?;
            for f in db::library::list_folders(&conn)? {
                commands::allow_asset_dir(app.handle(), std::path::Path::new(&f.path));
            }

            let http = http::client();
            // Credentials are namespaced by the app identifier: the release
            // app keeps using `com.unifiedmusic.desktop`; a test build with
            // another identifier gets its own, separate credentials.
            let identifier = app.config().identifier.clone();
            if identifier != secrets::SERVICE {
                log::warn!("non-release identifier {identifier}: using separate data, WebView profile and credentials");
            }
            let secrets: Arc<dyn SecretStore> = Arc::new(KeyringStore::new(&identifier));
            let state = AppState {
                db: Mutex::new(conn),
                spotify: SpotifyAdapter::new(http.clone(), secrets.clone()),
                soundcloud: SoundCloudAdapter::new(http.clone(), secrets.clone()),
                soundcloud_web: providers::soundcloud_web::SoundCloudWebAdapter::new(),
                lyrics: lyrics::LyricsClient::new(http.clone()),
                lastfm: LastFmAdapter::new(http),
                secrets,
                paths,
                media: Mutex::new(None),
                taskbar: Mutex::new(None),
                oauth_lock: tokio::sync::Mutex::new(()),
                sync: sync::SyncEngine::default(),
            };
            app.manage(state);

            // Windows media keys / SMTC for playback owned by this app.
            if let Some(window) = app.get_webview_window("main") {
                #[cfg(windows)]
                let hwnd = window.hwnd().ok().map(|h| h.0 as *mut std::ffi::c_void);
                #[cfg(not(windows))]
                let hwnd = None;
                let _ = &window;
                let handle = media::start(app.handle().clone(), hwnd);
                if handle.is_none() {
                    log::warn!("system media controls are unavailable");
                }
                *app.state::<AppState>().media.lock().unwrap() = handle;

                // Taskbar thumbnail toolbar (Previous / Play-Pause / Next).
                // `setup` runs on the UI thread, which the toolbar requires.
                let toolbar = taskbar::start(app.handle(), &window);
                if toolbar.is_none() {
                    log::warn!("taskbar thumbnail toolbar is unavailable");
                }
                *app.state::<AppState>().taskbar.lock().unwrap() = toolbar;
            }

            // Spotify playlist sync runs in the backend, independent of the window.
            tauri::async_runtime::spawn(commands::sync_scheduler(app.handle().clone()));

            // Periodically retry cached scrobbles.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                    let st = handle.state::<AppState>();
                    let pending = db::scrobbles::status(&st.conn()).map(|s| s.pending).unwrap_or(0);
                    if pending > 0 {
                        if let Err(e) = commands::flush_inner(&st).await {
                            log::info!("scheduled scrobble flush skipped: {e}");
                        }
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::library_list_tracks,
            commands::library_get_track,
            commands::library_list_artists,
            commands::library_list_albums,
            commands::library_set_membership,
            commands::local_list_folders,
            commands::local_add_folder,
            commands::local_remove_folder,
            commands::local_scan,
            commands::local_read_metadata,
            commands::local_write_metadata,
            commands::local_list_backups,
            commands::local_restore_backup,
            commands::override_get,
            commands::override_set,
            commands::override_set_artwork,
            commands::override_clear,
            commands::playlist_list,
            commands::playlist_create,
            commands::playlist_rename,
            commands::playlist_delete,
            commands::playlist_tracks,
            commands::playlist_mark_played,
            commands::playlist_set_artwork,
            commands::playlist_add_tracks,
            commands::playlist_remove_item,
            commands::playlist_move_item,
            commands::playlist_export_bran,
            commands::playlist_inspect_bran,
            commands::playlist_import_bran,
            commands::bran_take_pending,
            commands::queue_get,
            commands::queue_replace,
            commands::queue_add,
            commands::queue_remove,
            commands::queue_move,
            commands::queue_clear,
            commands::queue_advance,
            commands::queue_set_current,
            commands::queue_shuffle_upcoming,
            commands::playback_save_state,
            commands::providers_status,
            commands::settings_get,
            commands::settings_update,
            commands::spotify_connect,
            commands::spotify_disconnect,
            commands::spotify_import_library,
            commands::spotify_devices,
            commands::spotify_playback_state,
            commands::spotify_play_track,
            commands::spotify_control,
            commands::spotify_add_to_queue,
            commands::spotify_sdk_token,
            commands::spotify_sync_candidates,
            commands::spotify_sync_enable,
            commands::spotify_sync_disable,
            commands::spotify_sync_now,
            commands::spotify_sync_all,
            commands::playlist_link_spotify,
            commands::alternatives_search,
            commands::alternatives_add,
            commands::alternatives_list,
            commands::soundcloud_connect,
            commands::soundcloud_disconnect,
            commands::soundcloud_import_library,
            commands::soundcloud_resolve_stream,
            commands::track_loudness,
            commands::loudness_capture_start,
            commands::loudness_capture_gain,
            commands::loudness_capture_status,
            commands::loudness_capture_stop,
            commands::lyrics_get,
            commands::soundcloud_web_import,
            commands::search_providers,
            commands::lastfm_connect,
            commands::lastfm_disconnect,
            commands::lastfm_now_playing,
            commands::lastfm_submit_play,
            commands::lastfm_flush,
            commands::lastfm_queue_status,
            commands::lastfm_clear_failed,
            commands::media_update,
            commands::app_info,
        ])
        .build(tauri::generate_context!())
        .expect("error while building branflakes")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                // Runs on the UI thread: release the toolbar's subclass,
                // icons and COM object.
                taskbar::shutdown();
                if let Some(st) = app.try_state::<AppState>() {
                    st.sync.shutdown();
                    *st.taskbar.lock().unwrap_or_else(|e| e.into_inner()) = None;
                }
            }
        });
}
