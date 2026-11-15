//! Provider adapters. Each adapter declares its own capabilities explicitly;
//! the UI never assumes that all providers support the same features.

pub mod lastfm;
pub mod local;
pub mod soundcloud;
pub mod soundcloud_web;
pub mod spotify;

use crate::model::{PlaybackCapabilities, Source};

/// Capabilities for a playback source given the current connection state.
/// This is the single place the backend answers "what can this source do?".
pub fn playback_capabilities(source: Source, connected: bool, spotify_premium_denied: bool) -> PlaybackCapabilities {
    match source {
        Source::Local => local::LocalMusicAdapter::capabilities(),
        Source::Spotify => spotify::SpotifyAdapter::capabilities(connected, spotify_premium_denied),
        Source::Soundcloud => soundcloud::SoundCloudAdapter::capabilities(connected),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_do_not_share_identical_capabilities() {
        let local = playback_capabilities(Source::Local, true, false);
        let spotify = playback_capabilities(Source::Spotify, true, false);
        let sc = playback_capabilities(Source::Soundcloud, true, false);
        assert!(!local.device_selection);
        assert!(spotify.device_selection);
        assert!(!sc.device_selection);
        // Disconnected streaming providers can do nothing.
        assert_eq!(playback_capabilities(Source::Spotify, false, false), PlaybackCapabilities::NONE);
        assert_eq!(playback_capabilities(Source::Soundcloud, false, false), PlaybackCapabilities::NONE);
    }
}
