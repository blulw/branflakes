//! Windows System Media Transport Controls (media keys, volume flyout,
//! lock screen) for playback owned by this application, via `souvlaki`.
//!
//! The SMTC object lives on a dedicated thread; the rest of the app talks to
//! it through a channel. Button presses are emitted to the frontend as the
//! `media-control` event, where the playback controller decides what to do
//! based on the active source's capabilities.

use serde::{Deserialize, Serialize};
use std::sync::mpsc::{channel, Sender};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaUpdate {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// `file://` URL or https URL for the thumbnail.
    pub cover_url: Option<String>,
    pub duration_ms: Option<u64>,
    /// "playing" | "paused" | "stopped"
    pub status: String,
    pub position_ms: Option<u64>,
    /// Whether the active source currently allows each action, as resolved by
    /// the frontend controller. Used by the taskbar thumbnail toolbar.
    #[serde(default)]
    pub can_previous: bool,
    #[serde(default)]
    pub can_play_pause: bool,
    #[serde(default)]
    pub can_next: bool,
}

impl MediaUpdate {
    pub fn toolbar_state(&self) -> crate::taskbar::ToolbarState {
        crate::taskbar::ToolbarState {
            playing: self.status == "playing",
            can_previous: self.can_previous,
            can_play_pause: self.can_play_pause,
            can_next: self.can_next,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaControlPayload {
    pub action: String,
    pub position_ms: Option<u64>,
}

enum Msg {
    Update(MediaUpdate),
}

pub struct MediaControlsHandle {
    tx: Sender<Msg>,
}

impl MediaControlsHandle {
    pub fn update(&self, u: MediaUpdate) {
        let _ = self.tx.send(Msg::Update(u));
    }
}

/// Start SMTC integration bound to the given window handle.
pub fn start(app: AppHandle, hwnd: Option<*mut std::ffi::c_void>) -> Option<MediaControlsHandle> {
    use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig, SeekDirection};
    let hwnd_addr = hwnd.map(|h| h as usize);
    let (tx, rx) = channel::<Msg>();
    let (ready_tx, ready_rx) = channel::<bool>();
    std::thread::Builder::new()
        .name("smtc".into())
        .spawn(move || {
            let config = PlatformConfig {
                display_name: "branflakes",
                dbus_name: "branflakes",
                hwnd: hwnd_addr.map(|a| a as *mut std::ffi::c_void),
            };
            let mut controls = match MediaControls::new(config) {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("media controls unavailable: {e:?}");
                    let _ = ready_tx.send(false);
                    return;
                }
            };
            let emitter = app.clone();
            let attach = controls.attach(move |event: MediaControlEvent| {
                let (action, position_ms) = match event {
                    MediaControlEvent::Play => ("play", None),
                    MediaControlEvent::Pause => ("pause", None),
                    MediaControlEvent::Toggle => ("toggle", None),
                    MediaControlEvent::Next => ("next", None),
                    MediaControlEvent::Previous => ("previous", None),
                    MediaControlEvent::Stop => ("stop", None),
                    MediaControlEvent::Seek(SeekDirection::Forward) => ("seekForward", None),
                    MediaControlEvent::Seek(SeekDirection::Backward) => ("seekBackward", None),
                    MediaControlEvent::SetPosition(MediaPosition(d)) => ("setPosition", Some(d.as_millis() as u64)),
                    _ => return,
                };
                let _ = emitter.emit("media-control", MediaControlPayload { action: action.into(), position_ms });
            });
            if let Err(e) = attach {
                log::warn!("could not attach media control handler: {e:?}");
                let _ = ready_tx.send(false);
                return;
            }
            let _ = ready_tx.send(true);
            while let Ok(Msg::Update(u)) = rx.recv() {
                let _ = controls.set_metadata(MediaMetadata {
                    title: u.title.as_deref(),
                    artist: u.artist.as_deref(),
                    album: u.album.as_deref(),
                    cover_url: u.cover_url.as_deref(),
                    duration: u.duration_ms.map(Duration::from_millis),
                });
                let progress = u.position_ms.map(|p| MediaPosition(Duration::from_millis(p)));
                let playback = match u.status.as_str() {
                    "playing" => MediaPlayback::Playing { progress },
                    "paused" => MediaPlayback::Paused { progress },
                    _ => MediaPlayback::Stopped,
                };
                let _ = controls.set_playback(playback);
            }
        })
        .ok()?;
    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(true) => Some(MediaControlsHandle { tx }),
        _ => None,
    }
}
