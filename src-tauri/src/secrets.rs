//! Secure credential storage.
//!
//! Production uses Windows Credential Manager through the `keyring` crate
//! (generic credentials under the service name below). Tests use an in-memory
//! store. Values are never logged.

use crate::error::{AppError, AppResult};
use std::collections::HashMap;
use std::sync::Mutex;

pub const SERVICE: &str = "com.unifiedmusic.desktop";

pub const SPOTIFY_TOKENS: &str = "spotify.tokens";
pub const SOUNDCLOUD_TOKENS: &str = "soundcloud.tokens";
pub const SOUNDCLOUD_CLIENT_SECRET: &str = "soundcloud.client_secret";
pub const LASTFM_SHARED_SECRET: &str = "lastfm.shared_secret";
pub const LASTFM_SESSION_KEY: &str = "lastfm.session_key";

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> AppResult<Option<String>>;
    fn set(&self, key: &str, value: &str) -> AppResult<()>;
    fn delete(&self, key: &str) -> AppResult<()>;

    fn has(&self, key: &str) -> bool {
        matches!(self.get(key), Ok(Some(v)) if !v.is_empty())
    }
}

/// Windows Credential Manager (via `keyring`).
pub struct KeyringStore;

impl KeyringStore {
    fn entry(key: &str) -> AppResult<keyring::Entry> {
        keyring::Entry::new(SERVICE, key).map_err(AppError::from)
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, key: &str) -> AppResult<Option<String>> {
        match Self::entry(key)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        Self::entry(key)?.set_password(value).map_err(AppError::from)
    }

    fn delete(&self, key: &str) -> AppResult<()> {
        match Self::entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// In-memory store for tests.
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> AppResult<Option<String>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> AppResult<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

/// Wrapper that prevents accidental `Debug`/`Display` printing of a secret.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Secret(s.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_roundtrip() {
        let s = MemoryStore::default();
        assert!(!s.has("k"));
        s.set("k", "v").unwrap();
        assert_eq!(s.get("k").unwrap().as_deref(), Some("v"));
        s.delete("k").unwrap();
        assert_eq!(s.get("k").unwrap(), None);
    }

    #[test]
    fn secret_debug_is_redacted() {
        let s = Secret::new("super-secret-token");
        assert_eq!(format!("{s:?}"), "Secret(<redacted>)");
    }
}
