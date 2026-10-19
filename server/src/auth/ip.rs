//! The client IP Better Auth keys its rate limiter on and stores in
//! `session.ip_address` (`utils/get-request-ip.mjs`, `@better-auth/core`
//! `utils/ip.mjs`), with lib/auth.ts's `ipAddressHeaders: ["x-real-ip"]`.
//!
//! SECURITY FIX 7 (PORT.md 12): upstream took `X-Real-IP` from any client, so
//! a direct connection could pick a fresh rate-limit bucket per request. Here
//! the header counts only when TRUST_PROXY is on (the reverse proxy sets it);
//! otherwise the socket peer address is the client. With TRUST_PROXY on the
//! behaviour is upstream's exactly: the first comma-separated entry, trimmed,
//! must be a valid IPv4/IPv6 address (zod's regexes), and is normalised
//! (IPv4-mapped IPv6 to IPv4, IPv6 to its /64 prefix). No usable address in
//! production means "unknown": rate limiting is skipped and the session's
//! `ip_address` is "".

use std::net::{IpAddr, SocketAddr};
use std::sync::OnceLock;

use axum::http::HeaderMap;
use regex::Regex;

fn ipv4_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(?:(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9][0-9]|[0-9])\.){3}(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9][0-9]|[0-9])$")
            .expect("valid regex")
    })
}

fn ipv6_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(([0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,7}:|([0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,5}(:[0-9a-fA-F]{1,4}){1,2}|([0-9a-fA-F]{1,4}:){1,4}(:[0-9a-fA-F]{1,4}){1,3}|([0-9a-fA-F]{1,4}:){1,3}(:[0-9a-fA-F]{1,4}){1,4}|([0-9a-fA-F]{1,4}:){1,2}(:[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:((:[0-9a-fA-F]{1,4}){1,6})|:((:[0-9a-fA-F]{1,4}){1,7}|:))$")
            .expect("valid regex")
    })
}

pub fn is_ipv4(ip: &str) -> bool {
    ipv4_re().is_match(ip)
}

pub fn is_ipv6(ip: &str) -> bool {
    ipv6_re().is_match(ip)
}

/// `isValidIP` (zod's ipv4 / ipv6 formats).
pub fn is_valid_ip(ip: &str) -> bool {
    is_ipv4(ip) || is_ipv6(ip)
}

/// `expandIPv6`: eight groups of four hex digits (case kept).
fn expand_ipv6(ip: &str) -> Vec<String> {
    if let Some((left, right)) = ip.split_once("::") {
        let left: Vec<&str> = if left.is_empty() { Vec::new() } else { left.split(':').collect() };
        let right: Vec<&str> = if right.is_empty() { Vec::new() } else { right.split(':').collect() };
        let missing = 8usize.saturating_sub(left.len() + right.len());
        let pad = |g: &&str| format!("{g:0>4}");
        left.iter()
            .map(pad)
            .chain(std::iter::repeat_n("0000".to_string(), missing))
            .chain(right.iter().map(pad))
            .collect()
    } else {
        ip.split(':').map(|g| format!("{g:0>4}")).collect()
    }
}

/// `extractIPv4FromMapped`.
fn extract_ipv4_from_mapped(ip: &str) -> Option<String> {
    let lower = ip.to_lowercase();
    if let Some(rest) = lower.strip_prefix("::ffff:")
        && is_ipv4(rest)
    {
        return Some(rest.to_string());
    }
    let parts: Vec<&str> = ip.split(':').collect();
    if parts.len() == 7 && parts[5].eq_ignore_ascii_case("ffff") && is_ipv4(parts[6]) {
        return Some(parts[6].to_string());
    }
    if lower.contains("::ffff:") || lower.contains(":ffff:") {
        let groups = expand_ipv6(ip);
        if groups.len() == 8
            && groups[..5].iter().all(|g| g == "0000")
            && groups[5].eq_ignore_ascii_case("ffff")
            && !groups[6].is_empty()
            && !groups[7].is_empty()
        {
            let byte = |g: &str, from: usize| {
                g.get(from..from + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).map(|b| b.to_string())
            };
            let parts = [byte(&groups[6], 0), byte(&groups[6], 2), byte(&groups[7], 0), byte(&groups[7], 2)];
            // `Number.parseInt` of a bad pair is NaN: keep that spelling.
            let parts: Vec<String> = parts.into_iter().map(|p| p.unwrap_or_else(|| "NaN".into())).collect();
            return Some(parts.join("."));
        }
    }
    None
}

/// `normalizeIP(ip, { ipv6Subnet: 64 })`.
pub fn normalize_ip(ip: &str) -> String {
    if is_ipv4(ip) {
        return ip.to_lowercase();
    }
    if !is_ipv6(ip) {
        return ip.to_lowercase();
    }
    if let Some(v4) = extract_ipv4_from_mapped(ip) {
        return v4.to_lowercase();
    }
    // normalizeIPv6(ip, 64): keep the first 64 bits, zero the rest.
    let groups = expand_ipv6(ip);
    let mut bits_remaining: u32 = 64;
    groups
        .iter()
        .map(|g| {
            if bits_remaining == 0 {
                return "0000".to_string();
            }
            if bits_remaining >= 16 {
                bits_remaining -= 16;
                return g.clone();
            }
            let n = u32::from_str_radix(g, 16).unwrap_or(0);
            let masked = n & ((0xffffu32 << (16 - bits_remaining)) & 0xffff);
            bits_remaining = 0;
            format!("{masked:04x}")
        })
        .collect::<Vec<_>>()
        .join(":")
        .to_lowercase()
}

/// `getIp(headers, options)` with PORT.md 12.7 applied (see the module docs).
/// `None` = "could not determine the client IP".
pub fn get_ip(headers: &HeaderMap, peer: Option<SocketAddr>, trust_proxy: bool, is_prod: bool) -> Option<String> {
    if trust_proxy {
        if let Some(value) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
            let ip = crate::config::js_trim(value.split(',').next().unwrap_or(""));
            if is_valid_ip(ip) {
                return Some(normalize_ip(ip));
            }
        }
    } else if let Some(peer) = peer {
        let ip = match peer.ip() {
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) => v4.to_string(),
                None => v6.to_string(),
            },
            IpAddr::V4(v4) => v4.to_string(),
        };
        if is_valid_ip(&ip) {
            return Some(normalize_ip(&ip));
        }
    }
    // isTest() || isDevelopment(): Better Auth falls back to localhost.
    if !is_prod { Some("127.0.0.1".to_string()) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zod_ip_formats() {
        assert!(is_valid_ip("203.0.113.9"));
        assert!(!is_valid_ip("203.0.113.09"));
        assert!(!is_valid_ip("256.0.0.1"));
        assert!(is_valid_ip("2001:db8::1"));
        assert!(is_valid_ip("::"));
        assert!(!is_valid_ip("::ffff:1.2.3.4"));
        assert!(!is_valid_ip("example.com"));
    }

    #[test]
    fn normalisation() {
        assert_eq!(normalize_ip("203.0.113.9"), "203.0.113.9");
        assert_eq!(normalize_ip("2001:DB8:aa:bb:cc:dd:ee:ff"), "2001:0db8:00aa:00bb:0000:0000:0000:0000");
        assert_eq!(normalize_ip("2001:db8::1"), "2001:0db8:0000:0000:0000:0000:0000:0000");
        assert_eq!(normalize_ip("::ffff:102:304"), "1.2.3.4");
    }

    #[test]
    fn real_ip_only_behind_a_trusted_proxy() {
        let mut h = HeaderMap::new();
        h.insert("x-real-ip", " 198.51.100.7, 10.0.0.1".parse().unwrap());
        let peer: SocketAddr = "127.0.0.1:5555".parse().unwrap();
        assert_eq!(get_ip(&h, Some(peer), true, true).as_deref(), Some("198.51.100.7"));
        assert_eq!(get_ip(&h, Some(peer), false, true).as_deref(), Some("127.0.0.1"));
        assert_eq!(get_ip(&HeaderMap::new(), Some(peer), true, true), None);
        assert_eq!(get_ip(&HeaderMap::new(), None, true, false).as_deref(), Some("127.0.0.1"));
    }
}
