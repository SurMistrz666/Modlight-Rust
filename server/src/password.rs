//! Port of `lib/password.ts`: the request identity (`clientIp`), per-file
//! password hashing (`hashPassword` / `verifyPassword`) and the in-memory
//! attempt throttle (`checkRateLimit` / `resetRateLimit`).
//!
//! The unlock tokens and cookies of the same file are ported by the
//! uploads/library agent, keeping upstream names: `mint_unlock_token`,
//! `verify_unlock_token`, `is_media_unlocked`, `unlock_cookie_name`.
//!
//! The auth agent added `hash_password`, `verify_password`,
//! `check_rate_limit` and `reset_rate_limit` (account actions, the sign-in,
//! invitation, authorize and mail-test throttles); faithful ports, so a merge
//! with another copy keeps either.

use std::net::SocketAddr;
use std::sync::Mutex;

use axum::http::HeaderMap;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use indexmap::IndexMap;
use rand::RngCore;
use rand::rngs::OsRng;

/// Caller identity for throttling and the audit log.
///
/// Upstream trusts only `x-real-ip` (X-Forwarded-For is client-appendable, so
/// honoring it would hand every request a fresh bucket), and answers
/// "unknown" when the proxy did not set it.
///
/// Security fix (PORT.md 12.7): the header is honoured only when TRUST_PROXY
/// is on. Without a trusted proxy in front, anybody could choose their own
/// bucket by sending the header, so the socket peer address is used instead.
pub fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>, trust_proxy: bool) -> String {
    if trust_proxy {
        let header = headers.get("x-real-ip").and_then(|v| v.to_str().ok()).map(crate::config::js_trim).unwrap_or("");
        if header.is_empty() { "unknown".to_string() } else { header.to_string() }
    } else {
        match peer {
            Some(addr) => addr.ip().to_canonical().to_string(),
            None => "unknown".to_string(),
        }
    }
}

// ── Hashing: `scrypt$<saltB64url>$<dkB64url>` ───────────────────────────────

/// Node's `crypto.scrypt` defaults: N=16384, r=8, p=1.
fn scrypt_node(password: &[u8], salt: &[u8], len: usize) -> Option<Vec<u8>> {
    // `Params::len` only matters to the PHC string API; `scrypt()` derives
    // exactly `out.len()` bytes.
    let params = scrypt::Params::new(14, 8, 1, 32).ok()?;
    let mut out = vec![0u8; len];
    scrypt::scrypt(password, salt, &params, &mut out).ok()?;
    Some(out)
}

/// `hashPassword(password)`: scrypt with a random 16-byte salt and a 32-byte
/// key, both base64url. Async upstream because scrypt is deliberately
/// expensive; here it runs on the blocking pool.
pub async fn hash_password(password: &str) -> anyhow::Result<String> {
    let password = password.to_string();
    tokio::task::spawn_blocking(move || {
        let mut salt = [0u8; 16];
        OsRng.fill_bytes(&mut salt);
        let dk = scrypt_node(password.as_bytes(), &salt, 32).ok_or_else(|| anyhow::anyhow!("scrypt failed"))?;
        Ok(format!("scrypt${}${}", URL_SAFE_NO_PAD.encode(salt), URL_SAFE_NO_PAD.encode(dk)))
    })
    .await?
}

/// `verifyPassword(password, stored)`: false for anything that is not a
/// well-formed scrypt hash of `password` (constant-time comparison).
pub async fn verify_password(password: &str, stored: Option<&str>) -> bool {
    let Some(stored) = stored.filter(|s| !s.is_empty()) else {
        return false;
    };
    let mut parts = stored.split('$');
    if parts.next() != Some("scrypt") {
        return false;
    }
    let salt = crate::secret_box::node_base64_decode(parts.next().unwrap_or(""));
    let expected = crate::secret_box::node_base64_decode(parts.next().unwrap_or(""));
    if expected.is_empty() {
        return false;
    }
    let password = password.to_string();
    let len = expected.len();
    let Ok(Some(dk)) = tokio::task::spawn_blocking(move || scrypt_node(password.as_bytes(), &salt, len)).await else {
        return false;
    };
    crate::auth::crypto::constant_time_eq(&expected, &dk)
}

// ── Attempt throttling ──────────────────────────────────────────────────────

const MAX_ATTEMPTS: u32 = 5;
const WINDOW_MS: i64 = 15 * 60 * 1000;

#[derive(Debug, Clone, Copy)]
struct Bucket {
    count: u32,
    reset_at: i64,
}

/// A per-process map in insertion order, like the upstream `Map`: eviction
/// takes the oldest entries first.
struct Buckets {
    map: IndexMap<String, Bucket>,
    last_sweep: i64,
}

static BUCKETS: Mutex<Option<Buckets>> = Mutex::new(None);

/// Above this many buckets, expired ones are swept (at most every 30 s).
const BUCKET_SOFT_LIMIT: usize = 5_000;
/// Above this, the oldest buckets are evicted.
const BUCKET_HARD_LIMIT: usize = 20_000;
const SWEEP_INTERVAL_MS: i64 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitResult {
    pub allowed: bool,
    /// Milliseconds until the bucket refills (0 when allowed).
    pub retry_after_ms: i64,
}

fn maintain(b: &mut Buckets, now: i64) {
    if b.map.len() <= BUCKET_SOFT_LIMIT {
        return;
    }
    if now - b.last_sweep >= SWEEP_INTERVAL_MS {
        b.map.retain(|_, bucket| bucket.reset_at > now);
        b.last_sweep = now;
    }
    if b.map.len() > BUCKET_HARD_LIMIT {
        let over = b.map.len() - BUCKET_HARD_LIMIT;
        b.map.drain(..over);
    }
}

/// `checkRateLimit(key, max = 5, windowMs = 15 min)`: a fixed window per key.
/// Counts the attempt it is asked about (refused ones included), so a miss on
/// a non-existent resource costs the same as a wrong password.
pub fn check_rate_limit(key: &str, max: u32, window_ms: i64) -> RateLimitResult {
    check_rate_limit_at(key, max, window_ms, crate::db::types::JsDate::now().millis())
}

/// [`check_rate_limit`] with the upstream defaults (5 per 15 minutes).
pub fn check_rate_limit_default(key: &str) -> RateLimitResult {
    check_rate_limit(key, MAX_ATTEMPTS, WINDOW_MS)
}

pub fn check_rate_limit_at(key: &str, max: u32, window_ms: i64, now: i64) -> RateLimitResult {
    let mut guard = BUCKETS.lock().unwrap_or_else(|e| e.into_inner());
    let b = guard.get_or_insert_with(|| Buckets { map: IndexMap::new(), last_sweep: 0 });
    maintain(b, now);
    match b.map.get_mut(key) {
        Some(bucket) if bucket.reset_at > now => {
            bucket.count += 1;
            if bucket.count > max {
                RateLimitResult { allowed: false, retry_after_ms: bucket.reset_at - now }
            } else {
                RateLimitResult { allowed: true, retry_after_ms: 0 }
            }
        }
        // `Map#set` on an existing key keeps its position.
        Some(bucket) => {
            *bucket = Bucket { count: 1, reset_at: now + window_ms };
            RateLimitResult { allowed: true, retry_after_ms: 0 }
        }
        None => {
            b.map.insert(key.to_string(), Bucket { count: 1, reset_at: now + window_ms });
            RateLimitResult { allowed: true, retry_after_ms: 0 }
        }
    }
}

/// `resetRateLimit(key)`: clear the bucket after a success.
pub fn reset_rate_limit(key: &str) {
    let mut guard = BUCKETS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(b) = guard.as_mut() {
        b.map.shift_remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn honours_x_real_ip_only_behind_a_trusted_proxy() {
        let mut h = HeaderMap::new();
        h.insert("x-real-ip", HeaderValue::from_static(" 203.0.113.9 "));
        let peer: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        assert_eq!(client_ip(&h, Some(peer), true), "203.0.113.9");
        assert_eq!(client_ip(&h, Some(peer), false), "127.0.0.1");
        assert_eq!(client_ip(&HeaderMap::new(), Some(peer), true), "unknown");
        assert_eq!(client_ip(&HeaderMap::new(), None, false), "unknown");
        let mapped: SocketAddr = "[::ffff:10.0.0.1]:1".parse().unwrap();
        assert_eq!(client_ip(&HeaderMap::new(), Some(mapped), false), "10.0.0.1");
    }

    // tests/unit/password.test.ts
    #[tokio::test]
    async fn produces_a_versioned_scrypt_hash_and_verifies_the_right_password() {
        let hash = hash_password("correct horse battery staple").await.unwrap();
        assert!(hash.starts_with("scrypt$"));
        assert!(verify_password("correct horse battery staple", Some(&hash)).await);
    }

    #[tokio::test]
    async fn rejects_the_wrong_password() {
        let hash = hash_password("hunter2").await.unwrap();
        assert!(!verify_password("hunter3", Some(&hash)).await);
    }

    #[tokio::test]
    async fn uses_a_random_salt() {
        assert_ne!(hash_password("same").await.unwrap(), hash_password("same").await.unwrap());
    }

    #[tokio::test]
    async fn is_safe_against_null_and_garbage_stored_values() {
        assert!(!verify_password("x", None).await);
        assert!(!verify_password("x", Some("garbage")).await);
        assert!(!verify_password("x", Some("")).await);
    }

    #[test]
    fn fixed_window_counts_every_attempt() {
        let key = "test|window";
        let t0 = 1_000_000;
        for _ in 0..3 {
            assert!(check_rate_limit_at(key, 3, 1000, t0).allowed);
        }
        let refused = check_rate_limit_at(key, 3, 1000, t0 + 10);
        assert!(!refused.allowed);
        assert_eq!(refused.retry_after_ms, 990);
        // The window rolls over.
        assert!(check_rate_limit_at(key, 3, 1000, t0 + 1000).allowed);
        reset_rate_limit(key);
        assert!(check_rate_limit_at(key, 1, 1000, t0 + 1001).allowed);
        assert!(!check_rate_limit_at(key, 1, 1000, t0 + 1002).allowed);
    }
}
