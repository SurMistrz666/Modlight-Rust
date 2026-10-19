//! Port of `lib/password.ts`: the request identity (`clientIp`), per-file
//! password hashing (`hashPassword` / `verifyPassword`) and the in-memory
//! attempt throttle (`checkRateLimit` / `resetRateLimit`).
//!
//! The unlock tokens and their cookies (`mint_unlock_token`,
//! `verify_unlock_token`, the owner-scoped "remember for 30 days" token,
//! `is_media_unlocked`, `unlock_cookie_name`, `unlock_cookie_options`) were
//! added by the uploads/library port.
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

// ── Unlock tokens ───────────────────────────────────────────────────────────

/// What a token opens: `media` exactly one file, `shortlink` exactly one short
/// link, `owner` every file of one owner that carries a given generation of
/// that owner's DEFAULT password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnlockScope {
    Media,
    Shortlink,
    Owner,
}

impl UnlockScope {
    pub fn as_str(self) -> &'static str {
        match self {
            UnlockScope::Media => "media",
            UnlockScope::Shortlink => "shortlink",
            UnlockScope::Owner => "owner",
        }
    }

    /// How long a successful unlock stays valid (seconds).
    pub fn ttl(self) -> i64 {
        match self {
            UnlockScope::Media => 60 * 60 * 24,
            UnlockScope::Shortlink => 60 * 60,
            UnlockScope::Owner => 60 * 60 * 24 * 30,
        }
    }
}

/// What "remember this" buys, whatever the scope: 30 days.
pub const REMEMBER_TTL: i64 = 60 * 60 * 24 * 30;

/// Proof of a successful unlock: `<expiry>.<HMAC-SHA256(secret, parts|expiry)>`.
///
/// The value MUST be unforgeable, because cookies are entirely
/// client-controlled. Binding the signature to the resource id stops a token
/// minted for one file from unlocking another, and to the scope so a
/// short-link token can't unlock a file.
pub fn mint_unlock_token(secret: &str, scope: UnlockScope, id: &str, ttl_seconds: Option<i64>) -> String {
    mint_at(secret, &[scope.as_str(), id], ttl_seconds.unwrap_or(scope.ttl()), now_ms())
}

pub fn verify_unlock_token(secret: &str, token: Option<&str>, scope: UnlockScope, id: &str) -> bool {
    verify_at(secret, token, &[scope.as_str(), id], now_ms())
}

/// A token that opens every file of `owner_id` carrying generation `version` of
/// that owner's default file password. The generation is INSIDE the signature,
/// which is what makes changing the default password revoke the remembered
/// consent.
pub fn mint_owner_unlock_token(
    secret: &str,
    owner_id: &str,
    version: i64,
    ttl_seconds: Option<i64>,
) -> anyhow::Result<String> {
    if !is_version(version as f64) {
        anyhow::bail!("mintOwnerUnlockToken: invalid version");
    }
    let v = format!("v{version}");
    Ok(mint_at(secret, &["owner", owner_id, &v], ttl_seconds.unwrap_or(UnlockScope::Owner.ttl()), now_ms()))
}

pub fn verify_owner_unlock_token(
    secret: &str,
    token: Option<&str>,
    owner_id: Option<&str>,
    version: Option<i64>,
) -> bool {
    let (Some(owner_id), Some(version)) = (owner_id.filter(|o| !o.is_empty()), version) else {
        return false;
    };
    if !is_version(version as f64) {
        return false;
    }
    let v = format!("v{version}");
    verify_at(secret, token, &["owner", owner_id, &v], now_ms())
}

fn is_version(v: f64) -> bool {
    v.fract() == 0.0 && v.abs() <= 9_007_199_254_740_991.0 && v >= 0.0
}

/// The bits of an upload the unlock decision depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaUnlockSubject {
    /// uploads.id
    pub media_id: String,
    /// uploads.ownerId (None for an orphaned file).
    pub owner_id: Option<String>,
    /// uploads.passwordVersion: the generation of the owner's DEFAULT password
    /// this file was created under, or None when its password was set by hand.
    pub password_version: Option<i64>,
}

/// Has this visitor cleared the password gate for `subject`? A token minted for
/// this exact file, or an owner-scoped token whose generation matches the one
/// the file records.
pub fn is_media_unlocked(secret: &str, subject: &MediaUnlockSubject, read: impl Fn(&str) -> Option<String>) -> bool {
    let own = read(&unlock_cookie_name(UnlockScope::Media, &subject.media_id));
    if verify_unlock_token(secret, own.as_deref(), UnlockScope::Media, &subject.media_id) {
        return true;
    }
    // Hand-picked password, or an orphan with no owner to scope a token to.
    let (Some(version), Some(owner)) =
        (subject.password_version, subject.owner_id.as_deref().filter(|o| !o.is_empty()))
    else {
        return false;
    };
    verify_owner_unlock_token(secret, read(&owner_unlock_cookie_name(owner)).as_deref(), Some(owner), Some(version))
}

/// Cookie carrying the unlock token for a given resource.
pub fn unlock_cookie_name(scope: UnlockScope, id: &str) -> String {
    match scope {
        UnlockScope::Media => format!("ml_unlock_{id}"),
        UnlockScope::Shortlink => format!("ml_sl_{id}"),
        UnlockScope::Owner => format!("ml_dpw_{id}"),
    }
}

/// Cookie carrying the owner-scoped ("remember for 30 days") token.
pub fn owner_unlock_cookie_name(owner_id: &str) -> String {
    unlock_cookie_name(UnlockScope::Owner, owner_id)
}

/// `unlockCookieOptions(scope, ttl)` applied to a cookie: `httpOnly`, path
/// `/`, `sameSite: lax`, `secure` in production, `maxAge` = the scope's TTL.
pub fn unlock_cookie(
    name: String,
    value: String,
    scope: UnlockScope,
    ttl_seconds: Option<i64>,
    is_prod: bool,
) -> crate::http::set_cookie::SetCookie {
    crate::http::set_cookie::SetCookie::new(name, value)
        .http_only(true)
        .path("/")
        .same_site(crate::http::set_cookie::SameSite::Lax)
        .secure(is_prod)
        .max_age(ttl_seconds.unwrap_or(scope.ttl()))
}

fn now_ms() -> i64 {
    crate::db::types::JsDate::now().millis()
}

fn mint_at(secret: &str, parts: &[&str], ttl_seconds: i64, now_ms: i64) -> String {
    let exp = now_ms.div_euclid(1000) + ttl_seconds;
    format!("{exp}.{}", sign(secret, parts, exp))
}

fn verify_at(secret: &str, token: Option<&str>, parts: &[&str], now_ms: i64) -> bool {
    let Some(token) = token.filter(|t| !t.is_empty()) else {
        return false;
    };
    let Some(dot) = token.find('.').filter(|d| *d > 0) else {
        return false;
    };
    let exp = crate::config::js_number(&token[..dot]);
    if !(exp.fract() == 0.0 && exp.abs() <= 9_007_199_254_740_991.0) || exp <= 0.0 {
        return false;
    }
    if exp * 1000.0 <= now_ms as f64 {
        return false;
    }
    let expected = sign(secret, parts, exp as i64);
    crate::auth::crypto::constant_time_eq(&token.as_bytes()[dot + 1..], expected.as_bytes())
}

/// Each part is percent-encoded before being joined, so a value containing the
/// separator cannot be split differently than it was signed.
fn sign(secret: &str, parts: &[&str], exp: i64) -> String {
    use hmac::Mac;
    let exp = exp.to_string();
    let message: Vec<String> = parts
        .iter()
        .copied()
        .chain(std::iter::once(exp.as_str()))
        .map(crate::http::cookies::encode_uri_component)
        .collect();
    let mut mac = <hmac::Hmac<sha2::Sha256> as Mac>::new_from_slice(secret.as_bytes()).expect("any key length");
    mac.update(message.join("|").as_bytes());
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
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

    // tests/unit/unlock-token.test.ts
    const SECRET: &str = "test-secret-test-secret-test-secret";
    const MEDIA: &str = "9d2e1a54-0000-4000-8000-000000000001";
    const OTHER: &str = "9d2e1a54-0000-4000-8000-000000000002";

    fn ok(token: Option<&str>, scope: UnlockScope, id: &str) -> bool {
        verify_unlock_token(SECRET, token, scope, id)
    }

    #[test]
    fn unlock_tokens() {
        let token = mint_unlock_token(SECRET, UnlockScope::Media, MEDIA, None);
        assert!(ok(Some(&token), UnlockScope::Media, MEDIA));
        for flag in [Some("1"), Some(""), None] {
            assert!(!ok(flag, UnlockScope::Media, MEDIA));
        }
        let (exp, sig) = token.split_once('.').unwrap();
        let flipped = match sig.strip_prefix('A') {
            Some(rest) => format!("B{rest}"),
            None => format!("A{}", &sig[1..]),
        };
        assert!(!ok(Some(&format!("{exp}.{flipped}")), UnlockScope::Media, MEDIA));
        assert!(!ok(Some(&format!("{exp}.{}", &sig[..sig.len() - 2])), UnlockScope::Media, MEDIA));
        assert!(!ok(Some(&format!("{exp}.{sig}xx")), UnlockScope::Media, MEDIA));
        assert!(!ok(Some(&format!("{exp}.")), UnlockScope::Media, MEDIA));
        let far = now_ms() / 1000 + 60 * 60 * 24 * 365;
        assert!(!ok(Some(&format!("{far}.{sig}")), UnlockScope::Media, MEDIA));
        let expired = mint_unlock_token(SECRET, UnlockScope::Media, MEDIA, Some(-1));
        assert!(!ok(Some(&expired), UnlockScope::Media, MEDIA));
        let other = mint_unlock_token(SECRET, UnlockScope::Media, OTHER, None);
        assert!(!ok(Some(&other), UnlockScope::Media, MEDIA));
        let scoped = mint_unlock_token(SECRET, UnlockScope::Shortlink, MEDIA, None);
        assert!(!ok(Some(&scoped), UnlockScope::Media, MEDIA));
        for junk in ["garbage", ".", ".sig", "abc.def", "-1.sig", "NaN.sig", "1e9.sig"] {
            assert!(!ok(Some(junk), UnlockScope::Media, MEDIA), "{junk}");
        }
        // A different secret never verifies.
        assert!(!verify_unlock_token("other-secret", Some(&token), UnlockScope::Media, MEDIA));
    }

    #[test]
    fn owner_tokens_carry_the_generation() {
        let t = mint_owner_unlock_token(SECRET, "owner1", 3, None).unwrap();
        assert!(verify_owner_unlock_token(SECRET, Some(&t), Some("owner1"), Some(3)));
        assert!(!verify_owner_unlock_token(SECRET, Some(&t), Some("owner1"), Some(4)));
        assert!(!verify_owner_unlock_token(SECRET, Some(&t), None, Some(3)));
        assert!(mint_owner_unlock_token(SECRET, "owner1", -1, None).is_err());
        let subject =
            MediaUnlockSubject { media_id: MEDIA.into(), owner_id: Some("owner1".into()), password_version: Some(3) };
        let cookie = |name: &str| (name == "ml_dpw_owner1").then(|| t.clone());
        assert!(is_media_unlocked(SECRET, &subject, cookie));
        let hand_picked = MediaUnlockSubject { password_version: None, ..subject.clone() };
        assert!(!is_media_unlocked(SECRET, &hand_picked, |name| (name == "ml_dpw_owner1").then(|| t.clone())));
        let own = mint_unlock_token(SECRET, UnlockScope::Media, MEDIA, None);
        assert!(
            is_media_unlocked(SECRET, &hand_picked, |name| (name == format!("ml_unlock_{MEDIA}")).then(|| own.clone()))
        );
    }

    #[test]
    fn unlock_cookies() {
        assert_eq!(unlock_cookie_name(UnlockScope::Media, MEDIA), format!("ml_unlock_{MEDIA}"));
        assert_eq!(unlock_cookie_name(UnlockScope::Shortlink, MEDIA), format!("ml_sl_{MEDIA}"));
        assert_eq!(UnlockScope::Media.ttl(), 60 * 60 * 24);
        assert_eq!(UnlockScope::Shortlink.ttl(), 60 * 60);
        let c = unlock_cookie("ml_sl_x".into(), "v".into(), UnlockScope::Shortlink, None, false).to_next_string();
        assert!(c.contains("Max-Age=3600"), "{c}");
        assert!(c.contains("HttpOnly"), "{c}");
        assert!(c.to_lowercase().contains("samesite=lax"), "{c}");
        assert!(!c.contains("Secure"), "{c}");
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

#[cfg(test)]
mod default_password_tests {
    //! tests/unit/default-password.test.ts
    use super::*;

    const SECRET: &str = "test-secret-test-secret-test-secret";
    const OWNER: &str = "usr_2f8a1c94";
    const OTHER_OWNER: &str = "usr_9b3e77d1";
    const FILE_A: &str = "9d2e1a54-0000-4000-8000-00000000000a";
    const FILE_B: &str = "9d2e1a54-0000-4000-8000-00000000000b";

    fn on_default(media: &str, v: i64, owner: &str) -> MediaUnlockSubject {
        MediaUnlockSubject { media_id: media.into(), owner_id: Some(owner.into()), password_version: Some(v) }
    }
    fn hand_picked(media: &str) -> MediaUnlockSubject {
        MediaUnlockSubject { media_id: media.into(), owner_id: Some(OWNER.into()), password_version: None }
    }
    fn owner_token(owner: &str, v: i64, ttl: Option<i64>) -> String {
        mint_owner_unlock_token(SECRET, owner, v, ttl).unwrap()
    }
    fn verify(token: Option<&str>, owner: Option<&str>, v: Option<i64>) -> bool {
        verify_owner_unlock_token(SECRET, token, owner, v)
    }
    fn unlocked(subject: &MediaUnlockSubject, jar: &[(String, String)]) -> bool {
        is_media_unlocked(SECRET, subject, |name| jar.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()))
    }

    #[test]
    fn owner_scoped_tokens() {
        assert!(verify(Some(&owner_token(OWNER, 3, None)), Some(OWNER), Some(3)));
        for junk in ["1", "", "garbage", "abc.def", ".sig"] {
            assert!(!verify(Some(junk), Some(OWNER), Some(0)), "{junk}");
        }
        assert!(!verify(None, Some(OWNER), Some(0)));
        let token = owner_token(OWNER, 1, None);
        let (exp, sig) = token.split_once('.').unwrap();
        let flipped =
            if let Some(rest) = sig.strip_prefix('A') { format!("B{rest}") } else { format!("A{}", &sig[1..]) };
        assert!(!verify(Some(&format!("{exp}.{flipped}")), Some(OWNER), Some(1)));
        assert!(!verify(Some(&format!("{exp}.{}", &sig[..sig.len() - 2])), Some(OWNER), Some(1)));
        assert!(!verify(Some(&format!("{exp}.{sig}xx")), Some(OWNER), Some(1)));
        let far = now_ms() / 1000 + 60 * 60 * 24 * 365;
        assert!(!verify(Some(&format!("{far}.{sig}")), Some(OWNER), Some(1)));
        assert!(!verify(Some(&owner_token(OWNER, 1, Some(-1))), Some(OWNER), Some(1)));
        assert!(!verify(Some(&token), Some(OWNER), Some(2)));
        assert!(!verify(Some(&token), Some(OWNER), Some(0)));
        assert!(!verify(Some(&owner_token(OTHER_OWNER, 1, None)), Some(OWNER), Some(1)));
        let zero = owner_token(OWNER, 0, None);
        assert!(!verify(Some(&zero), Some(OWNER), None));
        assert!(!verify(Some(&zero), Some(OWNER), Some(-1)));
        assert!(!verify(Some(&zero), None, Some(0)));
        assert!(mint_owner_unlock_token(SECRET, OWNER, -1, None).is_err());
        // Same string in the id slot, different scope: a different signature.
        let jar = vec![(unlock_cookie_name(UnlockScope::Media, FILE_A), owner_token(FILE_A, 0, None))];
        assert!(!unlocked(&hand_picked(FILE_A), &jar));
    }

    #[test]
    fn who_gets_in() {
        let file_a = vec![(
            unlock_cookie_name(UnlockScope::Media, FILE_A),
            mint_unlock_token(SECRET, UnlockScope::Media, FILE_A, None),
        )];
        assert!(unlocked(&hand_picked(FILE_A), &file_a));
        assert!(!unlocked(&hand_picked(FILE_B), &file_a));
        assert!(!unlocked(&on_default(FILE_B, 0, OWNER), &file_a));
        let remembered = vec![(owner_unlock_cookie_name(OWNER), owner_token(OWNER, 0, None))];
        assert!(unlocked(&on_default(FILE_A, 0, OWNER), &remembered));
        assert!(unlocked(&on_default(FILE_B, 0, OWNER), &remembered));
        assert!(!unlocked(&hand_picked(FILE_A), &remembered));
        assert!(!unlocked(&on_default(FILE_B, 1, OWNER), &remembered));
        let other = vec![(owner_unlock_cookie_name(OTHER_OWNER), owner_token(OTHER_OWNER, 0, None))];
        assert!(!unlocked(&on_default(FILE_A, 0, OWNER), &other));
        assert!(!unlocked(&on_default(FILE_A, 0, OWNER), &[(owner_unlock_cookie_name(OWNER), "1".into())]));
        let expired = vec![(owner_unlock_cookie_name(OWNER), owner_token(OWNER, 0, Some(-1)))];
        assert!(!unlocked(&on_default(FILE_A, 0, OWNER), &expired));
        let orphan = MediaUnlockSubject { media_id: FILE_A.into(), owner_id: None, password_version: Some(0) };
        assert!(!unlocked(&orphan, &remembered));
        assert!(!unlocked(&on_default(FILE_A, 0, OWNER), &[]));
        assert!(!unlocked(&hand_picked(FILE_A), &[]));
    }

    #[test]
    fn cookies() {
        assert_eq!(owner_unlock_cookie_name(OWNER), format!("ml_dpw_{OWNER}"));
        assert_eq!(unlock_cookie_name(UnlockScope::Owner, OWNER), format!("ml_dpw_{OWNER}"));
        assert_eq!(unlock_cookie_name(UnlockScope::Media, FILE_A), format!("ml_unlock_{FILE_A}"));
        let owner = unlock_cookie("n".into(), "v".into(), UnlockScope::Owner, None, true).to_next_string();
        assert!(owner.contains(&format!("Max-Age={REMEMBER_TTL}")), "{owner}");
        assert!(
            owner.contains("HttpOnly") && owner.contains("Path=/") && owner.to_lowercase().contains("samesite=lax")
        );
        let media = unlock_cookie("n".into(), "v".into(), UnlockScope::Media, None, false).to_next_string();
        assert!(media.contains(&format!("Max-Age={}", 60 * 60 * 24)), "{media}");
        let remembered =
            unlock_cookie("n".into(), "v".into(), UnlockScope::Media, Some(REMEMBER_TTL), false).to_next_string();
        assert!(remembered.contains(&format!("Max-Age={REMEMBER_TTL}")), "{remembered}");
    }
}
