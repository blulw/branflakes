//! OAuth 2 helpers: PKCE, state, and a one-shot loopback redirect listener.
//!
//! Spotify requires loopback redirect URIs to use an explicit IP literal
//! (`http://127.0.0.1:PORT/...`) — `localhost` is not accepted for new apps.

use crate::error::{AppError, AppResult};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub const DEFAULT_LOOPBACK_PORT: u16 = 43821;

pub fn redirect_uri(port: u16, provider: &str) -> String {
    format!("http://127.0.0.1:{port}/callback/{provider}")
}

/// Cryptographically random URL-safe string (uuid v4 uses the OS CSPRNG).
pub fn random_token(len_uuids: usize) -> String {
    (0..len_uuids).map(|_| uuid::Uuid::new_v4().simple().to_string()).collect()
}

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn generate() -> Self {
        // 3 × 32 hex chars = 96 chars, inside RFC 7636's 43..=128 range.
        let verifier = random_token(3);
        let challenge = challenge_for(&verifier);
        Pkce { verifier, challenge }
    }
}

pub fn challenge_for(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

#[derive(Debug)]
pub struct CallbackParams(pub HashMap<String, String>);

/// Bind the loopback port. Do this *before* opening the browser so a fast
/// redirect cannot race the listener.
pub async fn bind(port: u16) -> AppResult<TcpListener> {
    TcpListener::bind(("127.0.0.1", port)).await.map_err(|e| {
        AppError::Other(format!(
            "could not listen on 127.0.0.1:{port} for the OAuth redirect ({e}). Is another sign-in in progress?"
        ))
    })
}

/// Wait for a redirect to `/callback/{provider}` carrying the expected `state`.
pub async fn wait_for_callback(
    listener: TcpListener,
    provider: &str,
    expected_state: &str,
    timeout: Duration,
) -> AppResult<CallbackParams> {
    let path = format!("/callback/{provider}");
    let fut = async {
        loop {
            let (mut sock, _) = listener.accept().await?;
            let mut buf = vec![0u8; 8192];
            let n = sock.read(&mut buf).await?;
            let req = String::from_utf8_lossy(&buf[..n]);
            let target = req.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("");
            let Some(params) = parse_callback_target(target, &path) else {
                let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                continue;
            };
            let state_ok = params.get("state").map(|s| s == expected_state).unwrap_or(false);
            let (title, body) = if !state_ok {
                ("Sign-in failed", "The sign-in response did not match this request. Please try again from the app.")
            } else if params.contains_key("error") {
                ("Sign-in cancelled", "Authorization was not granted. You can close this window.")
            } else {
                ("Signed in", "You can close this window and return to branflakes.")
            };
            let html = format!(
                "<!doctype html><html><head><meta charset=utf-8><title>{title}</title></head>\
                 <body style=\"font-family:Segoe UI,sans-serif;background:#111;color:#eee;display:grid;place-items:center;height:100vh\">\
                 <div><h2>{title}</h2><p>{body}</p></div></body></html>"
            );
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                html.len(),
                html
            );
            let _ = sock.write_all(resp.as_bytes()).await;
            if !state_ok {
                // Possible CSRF / stale tab — keep waiting for the genuine redirect.
                continue;
            }
            if let Some(err) = params.get("error") {
                let desc = params.get("error_description").cloned().unwrap_or_else(|| err.clone());
                return Err(AppError::Auth { provider: provider.into(), message: desc });
            }
            return Ok(CallbackParams(params));
        }
    };
    match tokio::time::timeout(timeout, fut).await {
        Ok(r) => r,
        Err(_) => Err(AppError::Auth {
            provider: provider.into(),
            message: "timed out waiting for the browser sign-in".into(),
        }),
    }
}

/// Parse `"/callback/x?code=..&state=.."`. Returns `None` for other paths.
pub fn parse_callback_target(target: &str, expected_path: &str) -> Option<HashMap<String, String>> {
    let url = url::Url::parse(&format!("http://127.0.0.1{target}")).ok()?;
    if url.path() != expected_path {
        return None;
    }
    Some(url.query_pairs().into_owned().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc7636_example() {
        // RFC 7636 Appendix B.
        assert_eq!(
            challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let p = Pkce::generate();
        assert!((43..=128).contains(&p.verifier.len()));
        assert_ne!(Pkce::generate().verifier, p.verifier);
    }

    #[test]
    fn parses_callback() {
        let p = parse_callback_target("/callback/spotify?code=abc&state=xyz", "/callback/spotify").unwrap();
        assert_eq!(p["code"], "abc");
        assert_eq!(p["state"], "xyz");
        assert!(parse_callback_target("/favicon.ico", "/callback/spotify").is_none());
    }

    #[tokio::test]
    async fn loopback_ignores_wrong_state_and_accepts_right_one() {
        let listener = bind(0).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            wait_for_callback(listener, "spotify", "good", Duration::from_secs(5)).await
        });
        let c = reqwest::Client::new();
        let _ = c.get(format!("http://127.0.0.1:{port}/callback/spotify?code=evil&state=bad")).send().await;
        let _ = c.get(format!("http://127.0.0.1:{port}/callback/spotify?code=real&state=good")).send().await;
        let params = task.await.unwrap().unwrap();
        assert_eq!(params.0["code"], "real");
    }
}
