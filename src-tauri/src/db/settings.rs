//! Non-secret key/value settings. Secrets live in Windows Credential Manager
//! (see `secrets.rs`) and must never be written here.

use crate::error::AppResult;
use rusqlite::{params, Connection, OptionalExtension};

pub const SPOTIFY_CLIENT_ID: &str = "spotify.client_id";
pub const SOUNDCLOUD_CLIENT_ID: &str = "soundcloud.client_id";
pub const LASTFM_API_KEY: &str = "lastfm.api_key";
pub const LASTFM_ENABLED: &str = "lastfm.scrobbling_enabled";
pub const LASTFM_SCROBBLE_SPOTIFY: &str = "lastfm.scrobble_spotify";
pub const OAUTH_PORT: &str = "oauth.loopback_port";
pub const SPOTIFY_ACCOUNT: &str = "spotify.account_name";
pub const SOUNDCLOUD_ACCOUNT: &str = "soundcloud.account_name";
pub const LASTFM_ACCOUNT: &str = "lastfm.account_name";
pub const SOUNDCLOUD_WEB_MODE: &str = "soundcloud.web_mode_enabled";
pub const SPOTIFY_PREMIUM_DENIED: &str = "spotify.premium_denied";
/// Spotify user id of the connected account (for playlist ownership checks).
pub const SPOTIFY_USER_ID: &str = "spotify.user_id";
/// Automatic playlist sync interval in seconds (default 7200).
pub const SPOTIFY_SYNC_INTERVAL: &str = "spotify.sync_interval_secs";
/// Automatic followed-artist sync interval in seconds (default 86400).
pub const ARTIST_SYNC_INTERVAL: &str = "spotify.artist_sync_interval_secs";
/// Normalize in-app playback loudness across tracks (default on).
pub const NORMALIZE_VOLUME: &str = "playback.normalize_volume";
/// Also use Musixmatch and NetEase for lyrics timing (default on).
pub const LYRICS_EXTRA_SOURCES: &str = "lyrics.extra_sources";

pub fn get(conn: &Connection, key: &str) -> AppResult<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| r.get(0))
        .optional()?)
}

pub fn get_nonempty(conn: &Connection, key: &str) -> AppResult<Option<String>> {
    Ok(get(conn, key)?.map(|v| v.trim().to_string()).filter(|v| !v.is_empty()))
}

pub fn get_bool(conn: &Connection, key: &str, default: bool) -> AppResult<bool> {
    Ok(get(conn, key)?.map(|v| v == "true").unwrap_or(default))
}

pub fn set(conn: &Connection, key: &str, value: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = ?2",
        params![key, value],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, key: &str) -> AppResult<()> {
    conn.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    #[test]
    fn settings_roundtrip() {
        let conn = open_in_memory().unwrap();
        assert_eq!(get(&conn, SPOTIFY_CLIENT_ID).unwrap(), None);
        set(&conn, SPOTIFY_CLIENT_ID, " abc ").unwrap();
        assert_eq!(get_nonempty(&conn, SPOTIFY_CLIENT_ID).unwrap().as_deref(), Some("abc"));
        set(&conn, SPOTIFY_CLIENT_ID, "   ").unwrap();
        assert_eq!(get_nonempty(&conn, SPOTIFY_CLIENT_ID).unwrap(), None);
        assert!(get_bool(&conn, LASTFM_ENABLED, true).unwrap());
        set(&conn, LASTFM_ENABLED, "false").unwrap();
        assert!(!get_bool(&conn, LASTFM_ENABLED, true).unwrap());
        delete(&conn, LASTFM_ENABLED).unwrap();
        assert_eq!(get(&conn, LASTFM_ENABLED).unwrap(), None);
    }
}
