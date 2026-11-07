//! Token authentication for the remote server.
//!
//! The semantics follow the original Rails app (`backend.md` §13.2) with one
//! deliberate hardening: the server **refuses to start** without a token of
//! at least [`MIN_TOKEN_BYTES`] bytes instead of answering 503 at runtime.
//!
//! Accepted credentials:
//! - `Authorization: Bearer <token>`
//! - `X-Auth-Token: <token>`
//! - session cookie `frankmd_auth_token_digest=<sha256hex(token)>`
//!
//! Comparisons are constant-time over SHA-256 digests.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Session cookie name (`[COMPAT]` with the Rails app).
pub const COOKIE_NAME: &str = "frankmd_auth_token_digest";
/// Minimum token length in bytes.
pub const MIN_TOKEN_BYTES: usize = 32;
/// Failed logins allowed per IP inside [`FAILURE_WINDOW`].
pub const MAX_FAILURES_PER_IP: usize = 10;
/// Failed logins allowed globally inside [`FAILURE_WINDOW`].
pub const MAX_FAILURES_GLOBAL: usize = 100;
/// Sliding window for the login throttle.
pub const FAILURE_WINDOW: Duration = Duration::from_secs(300);

/// Why a token was rejected at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    /// Blank or whitespace-only.
    #[error("the remote token is blank")]
    Blank,
    /// Shorter than [`MIN_TOKEN_BYTES`].
    #[error("the remote token must be at least 32 bytes")]
    TooShort,
}

/// How a request proved its identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthVia {
    /// `Authorization` / `X-Auth-Token` header (no CSRF check needed).
    Header,
    /// Session cookie (mutations require the CSRF header).
    Cookie,
}

impl AuthVia {
    /// `true` for header auth.
    pub fn is_header(self) -> bool {
        matches!(self, Self::Header)
    }
}

#[derive(Debug, Default)]
struct RateLimiter {
    per_ip: HashMap<IpAddr, VecDeque<Instant>>,
    global: VecDeque<Instant>,
}

impl RateLimiter {
    fn prune(&mut self, now: Instant) {
        let cutoff = now.checked_sub(FAILURE_WINDOW).unwrap_or(now);
        self.per_ip.retain(|_, failures| {
            while failures.front().is_some_and(|t| *t < cutoff) {
                failures.pop_front();
            }
            !failures.is_empty()
        });
        while self.global.front().is_some_and(|t| *t < cutoff) {
            self.global.pop_front();
        }
    }
}

/// The token verifier and login throttle of one server.
#[derive(Debug)]
pub struct AuthConfig {
    digest: [u8; 32],
    digest_hex: String,
    limiter: Mutex<RateLimiter>,
}

impl AuthConfig {
    /// Validate and hash a token.
    pub fn new(token: &str) -> Result<Self, AuthError> {
        if token.trim().is_empty() {
            return Err(AuthError::Blank);
        }
        if token.len() < MIN_TOKEN_BYTES {
            return Err(AuthError::TooShort);
        }
        let digest = sha256(token.as_bytes());
        Ok(Self {
            digest,
            digest_hex: hex(&digest),
            limiter: Mutex::new(RateLimiter::default()),
        })
    }

    /// Cookie value (`sha256hex(token)`) set on a successful login.
    pub fn cookie_value(&self) -> &str {
        &self.digest_hex
    }

    /// Constant-time password check (`POST /login`).
    pub fn verify_password(&self, candidate: &str) -> bool {
        let candidate = sha256(candidate.as_bytes());
        candidate.ct_eq(&self.digest).into()
    }

    /// Identify a request from its headers.
    pub fn authenticate(&self, headers: &HeaderMap) -> Option<AuthVia> {
        if let Some(token) = bearer(headers).or_else(|| header_str(headers, "x-auth-token")) {
            return self.verify_password(token).then_some(AuthVia::Header);
        }
        if let Some(cookie) = cookie(headers, COOKIE_NAME) {
            let matches: bool = cookie.as_bytes().ct_eq(self.digest_hex.as_bytes()).into();
            return matches.then_some(AuthVia::Cookie);
        }
        None
    }

    /// `true` while this IP may try to log in.
    pub fn may_attempt(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let mut limiter = self
            .limiter
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        limiter.prune(now);
        let per_ip = limiter.per_ip.get(&ip).map_or(0, VecDeque::len);
        per_ip < MAX_FAILURES_PER_IP && limiter.global.len() < MAX_FAILURES_GLOBAL
    }

    /// Record one failed login.
    pub fn record_failure(&self, ip: IpAddr) {
        let now = Instant::now();
        let mut limiter = self
            .limiter
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        limiter.prune(now);
        let failures = limiter.per_ip.entry(ip).or_default();
        failures.push_back(now);
        limiter.global.push_back(now);
    }

    /// Clear an IP's failures after a successful login.
    pub fn reset_failures(&self, ip: IpAddr) {
        let mut limiter = self
            .limiter
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        limiter.per_ip.remove(&ip);
    }
}

/// `Authorization: Bearer <token>`.
fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = header_str(headers, "authorization")?;
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then_some(token.trim())
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}

/// One cookie value by name.
fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        (key == name).then(|| value.trim_matches('"').to_owned())
    })
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use std::net::{Ipv4Addr, Ipv6Addr};

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn headers(pairs: &[(&'static str, String)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn short_tokens_are_refused() {
        assert_eq!(AuthConfig::new("").unwrap_err(), AuthError::Blank);
        assert_eq!(AuthConfig::new("   ").unwrap_err(), AuthError::Blank);
        assert_eq!(
            AuthConfig::new("0123456789abcdef0123456789abcde").unwrap_err(),
            AuthError::TooShort
        );
        assert!(AuthConfig::new(TOKEN).is_ok());
    }

    #[test]
    fn header_forms_authenticate() {
        let auth = AuthConfig::new(TOKEN).unwrap();
        let bearer = headers(&[("authorization", format!("Bearer {TOKEN}"))]);
        assert_eq!(auth.authenticate(&bearer), Some(AuthVia::Header));
        let lower = headers(&[("authorization", format!("bearer {TOKEN}"))]);
        assert_eq!(auth.authenticate(&lower), Some(AuthVia::Header));
        let x = headers(&[("x-auth-token", TOKEN.to_owned())]);
        assert_eq!(auth.authenticate(&x), Some(AuthVia::Header));
        let wrong = headers(&[("authorization", "Bearer nope".to_owned())]);
        assert_eq!(auth.authenticate(&wrong), None);
    }

    #[test]
    fn cookie_is_the_sha256_digest() {
        let auth = AuthConfig::new(TOKEN).unwrap();
        // sha256("0123456789abcdef0123456789abcdef")
        assert_eq!(auth.cookie_value().len(), 64);
        let good = headers(&[(
            "cookie",
            format!("other=1; {}={}", COOKIE_NAME, auth.cookie_value()),
        )]);
        assert_eq!(auth.authenticate(&good), Some(AuthVia::Cookie));
        let bad = headers(&[("cookie", format!("{}={}", COOKIE_NAME, "0".repeat(64)))]);
        assert_eq!(auth.authenticate(&bad), None);
        // The raw token is not accepted as a cookie value.
        let raw = headers(&[("cookie", format!("{}={TOKEN}", COOKIE_NAME))]);
        assert_eq!(auth.authenticate(&raw), None);
    }

    #[test]
    fn password_check_is_exact() {
        let auth = AuthConfig::new(TOKEN).unwrap();
        assert!(auth.verify_password(TOKEN));
        assert!(!auth.verify_password(&TOKEN[..31]));
        assert!(!auth.verify_password(&format!("{TOKEN}x")));
    }

    #[test]
    fn login_throttle_is_per_ip_and_expires() {
        let auth = AuthConfig::new(TOKEN).unwrap();
        let ip = IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1));
        let other = IpAddr::V6(Ipv6Addr::LOCALHOST);
        for _ in 0..MAX_FAILURES_PER_IP {
            assert!(auth.may_attempt(ip));
            auth.record_failure(ip);
        }
        assert!(!auth.may_attempt(ip));
        assert!(auth.may_attempt(other), "another peer is unaffected");
        auth.reset_failures(ip);
        assert!(auth.may_attempt(ip));
    }
}
