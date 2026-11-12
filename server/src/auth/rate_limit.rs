//! Better Auth's rate limiter (`api/rate-limiter/index.mjs`) with lib/auth.ts's
//! options: enabled, memory storage, default window 60 s / max 120, and
//! `customRules` 60 s / 30 for `/sign-up/email` and `/sign-in/email`.
//!
//! Exactly upstream's mechanics:
//! - checked in `onRequest`, BEFORE routing (so unknown paths count too), and
//!   counted in `onResponse` for every answer the router produced (a 429 from
//!   the check itself is not counted);
//! - one bucket per `"<ip>|<path>"`, `path` = the pathname below `/api/auth`
//!   with trailing slashes removed;
//! - refuse while `now - lastRequest < window && count >= max`; a request
//!   after the window resets the count to 1; every counted request moves
//!   `lastRequest`, so a steady trickle keeps the window open;
//! - stored entries expire `window` seconds after their last write;
//! - no client IP (production, nothing usable): no rate limiting at all.
//!
//! Not upstream (the storage only, never an answer): upstream's `memory` Map
//! forgets an entry only when the SAME key is read again after it expired.
//! The path is the caller's choice (unknown paths count too), so one client
//! could add an entry per made-up path, each holding the whole path (hyper
//! reads a request line of ~400 KB), and none was ever removed. Here expired
//! entries are swept (when the map is large, at most every 30 s), and a path
//! longer than [`KEY_PATH_MAX`] is keyed by its SHA-256, so an entry stays
//! small. An expired entry and a missing one answer alike, and no real route
//! is that long, so every request is limited exactly as before.
//!
//! Rules, last match wins: the defaults (`/sign-in*`, `/sign-up*`,
//! `/change-password*`, `/change-email*` 10 s / 3; `/request-password-reset`,
//! `/send-verification-email`, `/forget-password*` ... 60 s / 3), then the
//! first plugin rule (twoFactor: `/two-factor/*` 10 s / 3), then
//! lib/auth.ts's custom rules.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::db::types::JsDate;

/// The window/max that applies to one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub window: i64,
    pub max: i64,
}

/// lib/auth.ts `rateLimit` (window 60, max 120).
pub const GLOBAL: Rule = Rule { window: 60, max: 120 };

/// The rule for `path` (see the module docs for the order).
pub fn rule_for(path: &str) -> Rule {
    let mut rule = GLOBAL;
    if path.starts_with("/sign-in")
        || path.starts_with("/sign-up")
        || path.starts_with("/change-password")
        || path.starts_with("/change-email")
    {
        rule = Rule { window: 10, max: 3 };
    } else if path == "/request-password-reset"
        || path == "/send-verification-email"
        || path.starts_with("/forget-password")
        || path == "/email-otp/send-verification-otp"
        || path == "/email-otp/request-password-reset"
    {
        rule = Rule { window: 60, max: 3 };
    }
    // Plugins in order: admin() has no rule; twoFactor() does.
    if path.starts_with("/two-factor/") {
        rule = Rule { window: 10, max: 3 };
    }
    if path == "/sign-up/email" || path == "/sign-in/email" {
        rule = Rule { window: 60, max: 30 };
    }
    rule
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    count: i64,
    last_request: i64,
    expires_at: i64,
}

/// Above this many entries, expired ones are swept (at most every
/// [`SWEEP_INTERVAL_MS`]).
const SWEEP_SOFT_LIMIT: usize = 10_000;
const SWEEP_INTERVAL_MS: i64 = 30_000;

/// The longest path kept as written in a key; a longer one is keyed by its
/// digest (`#<sha256 hex>`, which no pathname can spell: they start with `/`).
pub const KEY_PATH_MAX: usize = 256;

/// The `memory` storage: entries by key, plus when they were last swept.
#[derive(Default)]
pub struct Memory {
    inner: Mutex<MemoryInner>,
}

#[derive(Default)]
struct MemoryInner {
    map: HashMap<String, Entry>,
    last_sweep: i64,
}

impl Memory {
    fn lock(&self) -> std::sync::MutexGuard<'_, MemoryInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn get(&self, key: &str, now: i64) -> Option<Entry> {
        let mut inner = self.lock();
        let entry = *inner.map.get(key)?;
        if now >= entry.expires_at {
            inner.map.remove(key);
            return None;
        }
        Some(entry)
    }

    fn set(&self, key: &str, count: i64, last_request: i64, rule: Rule, now: i64) {
        let mut inner = self.lock();
        if inner.map.len() >= SWEEP_SOFT_LIMIT && now - inner.last_sweep >= SWEEP_INTERVAL_MS {
            inner.map.retain(|_, e| now < e.expires_at);
            inner.last_sweep = now;
        }
        inner.map.insert(key.to_string(), Entry { count, last_request, expires_at: now + rule.window * 1000 });
    }

    /// Entries held right now (expired ones included until swept or read).
    pub fn len(&self) -> usize {
        self.lock().map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The longest key held (for the tests of the key bound).
    pub fn longest_key(&self) -> usize {
        self.lock().map.keys().map(String::len).max().unwrap_or(0)
    }

    /// `onRequestRateLimit` at `now`: `Some(retry_after_seconds)` when the
    /// request must be refused.
    pub fn check_at(&self, ip: Option<&str>, path: &str, now: i64) -> Option<i64> {
        let ip = ip?;
        let rule = rule_for(path);
        let entry = self.get(&key(ip, path), now)?;
        let window_ms = rule.window * 1000;
        if now - entry.last_request < window_ms && entry.count >= rule.max {
            // Math.ceil((lastRequest + windowInMs - now) / 1000)
            let ms = entry.last_request + window_ms - now;
            return Some((ms + 999).div_euclid(1000));
        }
        None
    }

    /// `onResponseRateLimit` at `now`: count one answered request.
    pub fn record_at(&self, ip: Option<&str>, path: &str, now: i64) {
        let Some(ip) = ip else {
            return;
        };
        let rule = rule_for(path);
        let k = key(ip, path);
        match self.get(&k, now) {
            None => self.set(&k, 1, now, rule, now),
            Some(entry) if now - entry.last_request > rule.window * 1000 => self.set(&k, 1, now, rule, now),
            Some(entry) => self.set(&k, entry.count + 1, now, rule, now),
        }
    }
}

fn memory() -> &'static Memory {
    static MEMORY: OnceLock<Memory> = OnceLock::new();
    MEMORY.get_or_init(Memory::default)
}

/// `createRateLimitKey(ip, path)`, with an over-long path replaced by its
/// digest (module docs).
pub fn key(ip: &str, path: &str) -> String {
    if path.len() <= KEY_PATH_MAX {
        return format!("{ip}|{path}");
    }
    use sha2::Digest;
    format!("{ip}|#{}", hex::encode(sha2::Sha256::digest(path.as_bytes())))
}

/// `onRequestRateLimit`: `Some(retry_after_seconds)` when the request must be
/// refused.
pub fn check(ip: Option<&str>, path: &str) -> Option<i64> {
    memory().check_at(ip, path, JsDate::now().millis())
}

/// `onResponseRateLimit`: count one answered request.
pub fn record(ip: Option<&str>, path: &str) {
    memory().record_at(ip, path, JsDate::now().millis());
}

/// The 429 better-auth answers (`rateLimitResponse`).
pub fn too_many_requests(retry_after: i64) -> Response {
    let mut res =
        (StatusCode::TOO_MANY_REQUESTS, r#"{"message":"Too many requests. Please try again later."}"#.to_string())
            .into_response();
    // `new Response(string)` gets text/plain from the platform.
    res.headers_mut().insert(axum::http::header::CONTENT_TYPE, HeaderValue::from_static("text/plain;charset=UTF-8"));
    if let Ok(v) = HeaderValue::from_str(&retry_after.to_string()) {
        res.headers_mut().insert("x-retry-after", v);
    }
    res.extensions_mut().insert(hyper::ext::ReasonPhrase::from_static(b"Too Many Requests"));
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_match_upstream_precedence() {
        assert_eq!(rule_for("/sign-in/email"), Rule { window: 60, max: 30 });
        assert_eq!(rule_for("/sign-up/email"), Rule { window: 60, max: 30 });
        assert_eq!(rule_for("/sign-in/social"), Rule { window: 10, max: 3 });
        assert_eq!(rule_for("/change-password"), Rule { window: 10, max: 3 });
        assert_eq!(rule_for("/request-password-reset"), Rule { window: 60, max: 3 });
        assert_eq!(rule_for("/two-factor/verify-totp"), Rule { window: 10, max: 3 });
        assert_eq!(rule_for("/get-session"), GLOBAL);
    }

    #[test]
    fn counts_and_refuses_per_bucket() {
        let ip = "rate-limit-test-ip";
        for _ in 0..3 {
            assert_eq!(check(Some(ip), "/change-password"), None);
            record(Some(ip), "/change-password");
        }
        let retry = check(Some(ip), "/change-password").expect("fourth call refused");
        assert!((1..=10).contains(&retry));
        assert_eq!(check(Some(ip), "/change-email"), None);
        assert_eq!(check(None, "/change-password"), None);
    }

    /// A client inventing a path per request (unknown paths are counted too)
    /// used to leave one entry per path for good, each holding the whole
    /// path. Expired entries are swept now, and a long path is keyed by its
    /// digest; the answers stay those of upstream's limiter.
    #[test]
    fn made_up_paths_do_not_pile_up() {
        let store = Memory::default();
        let t0 = 1_760_000_000_000;
        let long = format!("/{}", "x".repeat(100_000));
        for i in 0..SWEEP_SOFT_LIMIT + 50 {
            store.record_at(Some("198.51.100.7"), &format!("/made-up/{i}"), t0);
        }
        store.record_at(Some("198.51.100.7"), &long, t0);
        assert!(store.longest_key() < 2 * KEY_PATH_MAX, "{} bytes", store.longest_key());
        assert_eq!(store.len(), SWEEP_SOFT_LIMIT + 51);
        // A window later every one of them has expired: the next write sweeps.
        let later = t0 + GLOBAL.window * 1000 + 1;
        store.record_at(Some("198.51.100.8"), "/get-session", later);
        assert_eq!(store.len(), 1);
    }

    /// The digest key counts a long path exactly like a short one.
    #[test]
    fn a_long_path_is_limited_like_any_other() {
        let store = Memory::default();
        let t0 = 1_760_000_000_000;
        let long = format!("/{}", "y".repeat(5_000));
        for _ in 0..GLOBAL.max {
            assert_eq!(store.check_at(Some("ip"), &long, t0), None);
            store.record_at(Some("ip"), &long, t0);
        }
        assert_eq!(store.check_at(Some("ip"), &long, t0), Some(GLOBAL.window));
        // Another long path is another bucket.
        assert_eq!(store.check_at(Some("ip"), &format!("{long}z"), t0), None);
        assert_ne!(key("ip", &long), key("ip", &format!("{long}z")));
        assert_eq!(key("ip", "/sign-in/email"), "ip|/sign-in/email");
    }
}
