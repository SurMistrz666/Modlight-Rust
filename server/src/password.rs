//! Port of `lib/password.ts`, the request-identity part (`clientIp`).
//!
//! The rest of that file (per-file password hashing, unlock tokens and
//! cookies, the in-memory `checkRateLimit` buckets) is ported here by the
//! agents that need it, keeping upstream function names:
//! `hash_password`, `verify_password`, `mint_unlock_token`,
//! `verify_unlock_token`, `is_media_unlocked`, `unlock_cookie_name`,
//! `check_rate_limit`, `reset_rate_limit`.

use std::net::SocketAddr;

use axum::http::HeaderMap;

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
}
