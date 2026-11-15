//! Shared domain model used by every provider adapter and the UI.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Where a track comes from. Every track in the unified library has exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Local,
    Spotify,
    Soundcloud,
}

impl Source {
    pub fn as_str(&self) -> &'static str {
        match self {
            Source::Local => "local",
            Source::Spotify => "spotify",
            Source::Soundcloud => "soundcloud",
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Source {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "local" => Ok(Source::Local),
            "spotify" => Ok(Source::Spotify),
            "soundcloud" => Ok(Source::Soundcloud),
            other => Err(format!("unknown source '{other}'")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Available,
    /// Only a short preview can be played (e.g. SoundCloud `access = preview`).
    PreviewOnly,
    Unavailable,
}

impl Availability {
    pub fn as_str(&self) -> &'static str {
        match self {
            Availability::Available => "available",
            Availability::PreviewOnly => "preview_only",
            Availability::Unavailable => "unavailable",
        }
    }
}

impl FromStr for Availability {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "available" => Ok(Availability::Available),
            "preview_only" => Ok(Availability::PreviewOnly),
            "unavailable" => Ok(Availability::Unavailable),
            other => Err(format!("unknown availability '{other}'")),
        }
    }
}

/// Provider-supplied metadata used when inserting/updating a track.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackUpsert {
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
    pub artwork_url: Option<String>,
    pub file_path: Option<String>,
    pub file_format: Option<String>,
    pub file_size: Option<u64>,
    pub file_mtime: Option<i64>,
    pub availability: Option<Availability>,
    pub unavailable_reason: Option<String>,
    pub permalink_url: Option<String>,
    pub uploader: Option<String>,
    pub uploader_url: Option<String>,
    /// First credited artist (Spotify), used for Last.fm scrobbles.
    pub primary_artist: Option<String>,
}

/// The unified track model sent to the UI. Overrides are already applied.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    /// Stable internal ID (UUID). Never changes for the lifetime of the row.
    pub id: String,
    pub source: Source,
    /// Provider-specific ID: normalised file path, Spotify track ID, or SoundCloud URN.
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
    /// Remote URL (Spotify/SoundCloud) or absolute local path to cached artwork.
    pub artwork: Option<String>,
    pub file_path: Option<String>,
    pub file_format: Option<String>,
    pub availability: Availability,
    pub unavailable_reason: Option<String>,
    pub permalink_url: Option<String>,
    pub uploader: Option<String>,
    pub uploader_url: Option<String>,
    pub in_library: bool,
    pub has_overrides: bool,
    pub added_at: String,
}

/// Explicit playback capability declaration. The UI enables a control only when
/// the corresponding flag is `true`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackCapabilities {
    pub play: bool,
    pub pause: bool,
    pub next: bool,
    pub previous: bool,
    pub seek: bool,
    pub volume: bool,
    pub queue: bool,
    pub device_selection: bool,
    pub playback_position: bool,
    pub duration: bool,
}

impl PlaybackCapabilities {
    pub const NONE: PlaybackCapabilities = PlaybackCapabilities {
        play: false,
        pause: false,
        next: false,
        previous: false,
        seek: false,
        volume: false,
        queue: false,
        device_selection: false,
        playback_position: false,
        duration: false,
    };
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub provider: String,
    pub display_name: String,
    /// Credentials (client ID etc.) have been entered.
    pub configured: bool,
    /// A user session/token exists.
    pub connected: bool,
    /// Connected, but the stored grant lacks permissions the app now needs.
    pub needs_reconnect: bool,
    pub account_name: Option<String>,
    pub capabilities: PlaybackCapabilities,
    /// Human-readable restrictions shown in the UI.
    pub limitations: Vec<String>,
}
