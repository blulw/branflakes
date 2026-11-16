//! Secure credential storage.
//!
//! Production uses Windows Credential Manager through the `keyring` crate
//! (generic credentials under the service name below). Tests use an in-memory
//! store. Values are never logged.

use crate::error::{AppError, AppResult};
use std::collections::HashMap;
use std::sync::Mutex;

/// Service name of the release app (its bundle identifier). Credentials are
/// stored under the running app's identifier, so a build with a different
/// identifier (e.g. a test build) never reads or overwrites these.
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

/// Windows Credential Manager (via `keyring`), scoped to one service name.
pub struct KeyringStore {
    service: String,
}

impl KeyringStore {
    /// `service` should be the app identifier (`SERVICE` for the release app).
    pub fn new(service: &str) -> Self {
        KeyringStore { service: service.to_string() }
    }

    pub fn service(&self) -> &str {
        &self.service
    }

    fn entry(&self, key: &str) -> AppResult<keyring::Entry> {
        keyring::Entry::new(&self.service, key).map_err(AppError::from)
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, key: &str) -> AppResult<Option<String>> {
        match self.entry(key)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        self.entry(key)?.set_password(value).map_err(AppError::from)
    }

    fn delete(&self, key: &str) -> AppResult<()> {
        match self.entry(key)?.delete_credential() {
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

#[cfg(test)]
mod keyring_scope_tests {
    use super::*;

    #[test]
    fn release_identifier_keeps_the_existing_service_name() {
        // The installed app's credentials must stay where they are.
        assert_eq!(KeyringStore::new(SERVICE).service(), "com.unifiedmusic.desktop");
        assert_ne!(KeyringStore::new("com.unifiedmusic.desktop.acceptance").service(), SERVICE);
    }

    /// Real Credential Manager: a store with another service name can't see
    /// or delete entries of the release service. Uses a throwaway key.
    #[cfg(windows)]
    #[test]
    #[ignore = "writes a throwaway entry to Windows Credential Manager"]
    fn separate_services_are_isolated_in_credential_manager() {
        let key = "isolation-check.throwaway";
        let a = KeyringStore::new("com.unifiedmusic.desktop.isolation-test-a");
        let b = KeyringStore::new("com.unifiedmusic.desktop.isolation-test-b");
        a.set(key, "A").unwrap();
        assert_eq!(b.get(key).unwrap(), None);
        b.delete(key).unwrap();
        assert_eq!(a.get(key).unwrap().as_deref(), Some("A"));
        a.delete(key).unwrap();
        assert_eq!(a.get(key).unwrap(), None);
    }
}
