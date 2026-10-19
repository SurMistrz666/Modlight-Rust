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

fn memory() -> &'static Mutex<HashMap<String, Entry>> {
    static MEMORY: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    MEMORY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get(key: &str, now: i64) -> Option<Entry> {
    let mut map = memory().lock().unwrap_or_else(|e| e.into_inner());
    let entry = *map.get(key)?;
    if now >= entry.expires_at {
        map.remove(key);
        return None;
    }
    Some(entry)
}

fn set(key: &str, count: i64, last_request: i64, rule: Rule, now: i64) {
    let mut map = memory().lock().unwrap_or_else(|e| e.into_inner());
    map.insert(key.to_string(), Entry { count, last_request, expires_at: now + rule.window * 1000 });
}

/// `createRateLimitKey(ip, path)`.
pub fn key(ip: &str, path: &str) -> String {
    format!("{ip}|{path}")
}

/// `onRequestRateLimit`: `Some(retry_after_seconds)` when the request must be
/// refused.
pub fn check(ip: Option<&str>, path: &str) -> Option<i64> {
    let ip = ip?;
    let rule = rule_for(path);
    let now = JsDate::now().millis();
    let entry = get(&key(ip, path), now)?;
    let window_ms = rule.window * 1000;
    if now - entry.last_request < window_ms && entry.count >= rule.max {
        // Math.ceil((lastRequest + windowInMs - now) / 1000)
        let ms = entry.last_request + window_ms - now;
        return Some((ms + 999).div_euclid(1000));
    }
    None
}

/// `onResponseRateLimit`: count one answered request.
pub fn record(ip: Option<&str>, path: &str) {
    let Some(ip) = ip else {
        return;
    };
    let rule = rule_for(path);
    let now = JsDate::now().millis();
    let k = key(ip, path);
    match get(&k, now) {
        None => set(&k, 1, now, rule, now),
        Some(entry) if now - entry.last_request > rule.window * 1000 => set(&k, 1, now, rule, now),
        Some(entry) => set(&k, entry.count + 1, now, rule, now),
    }
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
}
