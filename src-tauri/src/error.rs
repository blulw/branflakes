//! Application-wide error type.
//!
//! Errors are serialised to the frontend as `{ kind, message }`. Messages must
//! never contain access tokens, refresh tokens, session keys or secrets — the
//! HTTP layer only ever forwards provider error *descriptions*, never request
//! headers or bodies we constructed.

use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("metadata error: {0}")]
    Metadata(String),

    #[error("unsupported: {0}")]
    Unsupported(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("{provider} is not configured: {message}")]
    NotConfigured { provider: String, message: String },

    #[error("{provider} is not connected")]
    NotConnected { provider: String },

    #[error("{provider} authorization failed: {message}")]
    Auth { provider: String, message: String },

    #[error("{provider} requires Spotify Premium for this action")]
    PremiumRequired { provider: String },

    #[error("{provider} rate limit reached{}", .retry_after_secs.map(|s| format!(" (retry after {s}s)")).unwrap_or_default())]
    RateLimited {
        provider: String,
        retry_after_secs: Option<u64>,
        quota_exceeded: bool,
    },

    #[error("{provider} request failed ({status}): {message}")]
    Http {
        provider: String,
        status: u16,
        message: String,
    },

    #[error("network error talking to {provider}: {message}")]
    Network { provider: String, message: String },

    #[error("secure credential storage error: {0}")]
    Credential(String),

    #[error("metadata write verification failed: {0}")]
    VerificationFailed(String),

    #[error("{0}")]
    Other(String),
}

impl AppError {
    pub fn kind(&self) -> &'static str {
        match self {
            AppError::Database(_) => "database",
            AppError::Io(_) => "io",
            AppError::Metadata(_) => "metadata",
            AppError::Unsupported(_) => "unsupported",
            AppError::NotFound(_) => "not_found",
            AppError::InvalidInput(_) => "invalid_input",
            AppError::NotConfigured { .. } => "not_configured",
            AppError::NotConnected { .. } => "not_connected",
            AppError::Auth { .. } => "auth",
            AppError::PremiumRequired { .. } => "premium_required",
            AppError::RateLimited { .. } => "rate_limited",
            AppError::Http { .. } => "http",
            AppError::Network { .. } => "network",
            AppError::Credential(_) => "credential",
            AppError::VerificationFailed(_) => "verification_failed",
            AppError::Other(_) => "other",
        }
    }
}

impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("AppError", 2)?;
        s.serialize_field("kind", self.kind())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

macro_rules! metadata_error {
    ($($t:ty),* $(,)?) => {$(
        impl From<$t> for AppError {
            fn from(e: $t) -> Self {
                AppError::Metadata(e.to_string())
            }
        }
    )*};
}

metadata_error!(
    lofty::error::FileParseError,
    lofty::error::FileEncodingError,
    lofty::error::TagParseError,
    lofty::error::TagEncodingError,
    lofty::picture::error::PictureParseError,
);

impl From<keyring::Error> for AppError {
    fn from(e: keyring::Error) -> Self {
        // keyring errors never include the secret value itself.
        AppError::Credential(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Other(format!("serialization error: {e}"))
    }
}

pub type AppResult<T> = Result<T, AppError>;
