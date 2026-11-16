//! Shared HTTP behaviour: retries, rate-limit handling and error mapping.
//!
//! Rules:
//! * 429 → honour `Retry-After` (capped) and retry, unless the body says
//!   `QUOTA_EXCEEDED` (Spotify development-mode quota), which is surfaced
//!   immediately because retrying cannot help.
//! * 5xx and transport errors → exponential backoff, limited attempts.
//! * Other 4xx → mapped to an explicit `AppError`, never retried.
//! * Nothing here logs URLs with query strings, headers or bodies.

use crate::error::{AppError, AppResult};
use reqwest::{RequestBuilder, Response, StatusCode};
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_retry_after: Duration,
    /// Retry after a 5xx or a failure mid-request. Must be false for
    /// non-idempotent requests (e.g. "create playlist", "add items"): the
    /// server may already have applied them. 429 and connection failures
    /// (request never sent) are always retried.
    pub retry_ambiguous: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy {
            max_attempts: 4,
            base_delay: Duration::from_millis(500),
            max_retry_after: Duration::from_secs(30),
            retry_ambiguous: true,
        }
    }
}

impl RetryPolicy {
    /// Fast policy used by unit tests.
    pub fn immediate() -> Self {
        RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(1),
            max_retry_after: Duration::from_millis(5),
            retry_ambiguous: true,
        }
    }

    /// The same policy for a non-idempotent request.
    pub fn non_idempotent(self) -> Self {
        RetryPolicy { retry_ambiguous: false, ..self }
    }
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("branflakes/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .build()
        .expect("failed to build HTTP client")
}

/// Send a request (rebuilt for each attempt) with retry and rate-limit handling.
/// Returns successful (2xx) responses only.
pub async fn send<F>(provider: &str, policy: RetryPolicy, build: F) -> AppResult<Response>
where
    F: Fn() -> RequestBuilder,
{
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let last = attempt >= policy.max_attempts;
        match build().send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    return Ok(resp);
                }
                if status == StatusCode::TOO_MANY_REQUESTS {
                    let retry_after = resp
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.trim().parse::<u64>().ok());
                    let body = resp.text().await.unwrap_or_default();
                    let quota_exceeded = body.contains("QUOTA_EXCEEDED");
                    if quota_exceeded || last {
                        return Err(AppError::RateLimited {
                            provider: provider.into(),
                            retry_after_secs: retry_after,
                            quota_exceeded,
                        });
                    }
                    let wait = retry_after
                        .map(Duration::from_secs)
                        .unwrap_or(policy.base_delay * 2u32.pow(attempt - 1))
                        .min(policy.max_retry_after);
                    log::warn!("{provider}: rate limited, retrying in {wait:?}");
                    tokio::time::sleep(wait).await;
                    continue;
                }
                if status.is_server_error() && !last && policy.retry_ambiguous {
                    let wait = policy.base_delay * 2u32.pow(attempt - 1);
                    log::warn!("{provider}: server error {status}, retrying in {wait:?}");
                    tokio::time::sleep(wait).await;
                    continue;
                }
                return Err(error_from_response(provider, resp).await);
            }
            Err(e) => {
                // Not sent at all (connection refused etc.) is always safe to retry.
                if last || (!policy.retry_ambiguous && !e.is_connect()) {
                    return Err(AppError::Network {
                        provider: provider.into(),
                        message: sanitize_reqwest_error(&e),
                    });
                }
                let wait = policy.base_delay * 2u32.pow(attempt - 1);
                log::warn!("{provider}: network error, retrying in {wait:?}");
                tokio::time::sleep(wait).await;
            }
        }
    }
}

/// reqwest errors can include the request URL; strip query strings so that
/// any credentials in a query (we avoid this, but defensively) never leak.
pub fn sanitize_reqwest_error(e: &reqwest::Error) -> String {
    let e = match e.url() {
        Some(_) => {
            // Rebuild the message without the URL.
            if e.is_timeout() {
                return "request timed out".into();
            }
            if e.is_connect() {
                return "could not connect".into();
            }
            e.to_string()
        }
        None => e.to_string(),
    };
    match e.find('?') {
        Some(i) => format!("{}…", &e[..i]),
        None => e,
    }
}

async fn error_from_response(provider: &str, resp: Response) -> AppError {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    let message = extract_error_message(&body).unwrap_or_else(|| {
        status.canonical_reason().unwrap_or("request failed").to_string()
    });
    match status {
        StatusCode::UNAUTHORIZED => AppError::Auth { provider: provider.into(), message },
        StatusCode::FORBIDDEN if body.contains("PREMIUM_REQUIRED") => {
            AppError::PremiumRequired { provider: provider.into() }
        }
        _ => AppError::Http { provider: provider.into(), status: status.as_u16(), message },
    }
}

/// Pull a human-readable message out of common provider error shapes:
/// `{"error":{"message":".."}}` (Spotify), `{"error_description":".."}` (OAuth),
/// `{"errors":[{"error_message":".."}]}` (SoundCloud), `{"message":".."}` (Last.fm).
pub fn extract_error_message(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let candidates = [
        v.pointer("/error/message"),
        v.get("error_description"),
        v.pointer("/errors/0/error_message"),
        v.get("message"),
        v.get("error"),
    ];
    candidates
        .into_iter()
        .flatten()
        .find_map(|x| x.as_str().map(|s| s.chars().take(300).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn retries_429_then_succeeds() {
        let mut server = mockito::Server::new_async().await;
        let limited = server
            .mock("GET", "/x")
            .with_status(429)
            .with_header("retry-after", "0")
            .expect(1)
            .create_async()
            .await;
        let ok = server.mock("GET", "/x").with_status(200).with_body("ok").create_async().await;
        let c = client();
        let url = format!("{}/x", server.url());
        // mockito matches the most recently created mock first; make the 429 win once.
        // mockito serves a matching mock until its `expect` count is reached,
        // then moves on to the next matching mock.
        let resp = send("test", RetryPolicy::immediate(), || c.get(&url)).await.unwrap();
        assert_eq!(resp.text().await.unwrap(), "ok");
        limited.assert_async().await;
        ok.assert_async().await;
    }

    #[tokio::test]
    async fn quota_exceeded_is_not_retried() {
        let mut server = mockito::Server::new_async().await;
        let m = server
            .mock("GET", "/q")
            .with_status(429)
            .with_body(r#"{"status":429,"message":"Too many requests","reason":"QUOTA_EXCEEDED"}"#)
            .expect(1)
            .create_async()
            .await;
        let c = client();
        let url = format!("{}/q", server.url());
        let err = send("spotify", RetryPolicy::immediate(), || c.get(&url)).await.unwrap_err();
        assert!(matches!(err, AppError::RateLimited { quota_exceeded: true, .. }));
        m.assert_async().await;
    }

    #[tokio::test]
    async fn server_errors_retry_up_to_limit() {
        let mut server = mockito::Server::new_async().await;
        let m = server.mock("GET", "/e").with_status(503).expect(3).create_async().await;
        let c = client();
        let url = format!("{}/e", server.url());
        let err = send("x", RetryPolicy::immediate(), || c.get(&url)).await.unwrap_err();
        assert!(matches!(err, AppError::Http { status: 503, .. }));
        m.assert_async().await;
    }

    #[tokio::test]
    async fn maps_401_and_premium_403() {
        let mut server = mockito::Server::new_async().await;
        server.mock("GET", "/a").with_status(401).with_body(r#"{"error":{"status":401,"message":"The access token expired"}}"#).create_async().await;
        server.mock("GET", "/p").with_status(403).with_body(r#"{"error":{"status":403,"message":"Player command failed: Premium required","reason":"PREMIUM_REQUIRED"}}"#).create_async().await;
        let c = client();
        let e = send("spotify", RetryPolicy::immediate(), || c.get(format!("{}/a", server.url()))).await.unwrap_err();
        match e {
            AppError::Auth { message, .. } => assert_eq!(message, "The access token expired"),
            other => panic!("unexpected {other:?}"),
        }
        let e = send("spotify", RetryPolicy::immediate(), || c.get(format!("{}/p", server.url()))).await.unwrap_err();
        assert!(matches!(e, AppError::PremiumRequired { .. }));
    }

    #[tokio::test]
    async fn non_idempotent_requests_are_not_retried_after_5xx() {
        let mut server = mockito::Server::new_async().await;
        let m = server.mock("POST", "/c").with_status(503).expect(1).create_async().await;
        let c = client();
        let url = format!("{}/c", server.url());
        let err = send("x", RetryPolicy::immediate().non_idempotent(), || c.post(&url)).await.unwrap_err();
        assert!(matches!(err, AppError::Http { status: 503, .. }));
        m.assert_async().await;
        // 429 is still retried (the request was not processed).
        let limited = server.mock("POST", "/r").with_status(429).with_header("retry-after", "0").expect(1).create_async().await;
        let ok = server.mock("POST", "/r").with_status(201).create_async().await;
        let url = format!("{}/r", server.url());
        send("x", RetryPolicy::immediate().non_idempotent(), || c.post(&url)).await.unwrap();
        limited.assert_async().await;
        ok.assert_async().await;
    }

    #[test]
    fn extracts_messages() {
        assert_eq!(extract_error_message(r#"{"error_description":"bad code"}"#).as_deref(), Some("bad code"));
        assert_eq!(extract_error_message(r#"{"errors":[{"error_message":"nope"}]}"#).as_deref(), Some("nope"));
        assert_eq!(extract_error_message("not json"), None);
    }
}
