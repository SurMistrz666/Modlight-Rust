//! Cookie header parsing, three ways, because upstream reads cookies through
//! three different libraries and they disagree on the edges:
//!
//! - [`parse_cookies_next`]: Next's `cookies()` (`@edge-runtime/cookies`), used
//!   for `ml_locale`, `ml_mode`, `ml_skin`, `ml_sidebar` and every other cookie
//!   a page or action reads. Later duplicates win, a pair without `=` reads as
//!   `"true"`, a value that fails `decodeURIComponent` is dropped.
//! - [`parse_cookies_better_call`]: what Better Auth's endpoints read the
//!   signed session cookie with (`ctx.getSignedCookie`). The FIRST duplicate
//!   wins, values are trimmed and unquoted, decoding failures keep the raw text.
//! - [`parse_cookies_better_auth`]: `better-auth/cookies` `parseCookies`, used by
//!   `getSessionCookie` (the optimistic dashboard gate in proxy.ts). Names and
//!   values are validated against RFC 6265, later duplicates win.

use axum::http::{HeaderMap, header};
use indexmap::IndexMap;

/// Every `Cookie` header joined the way Node joins repeated cookie headers.
pub fn cookie_header(headers: &HeaderMap) -> Option<String> {
    let values: Vec<&str> = headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).collect();
    if values.is_empty() { None } else { Some(values.join("; ")) }
}

/// JavaScript `decodeURIComponent`: `None` where it would throw.
pub fn decode_uri_component(s: &str) -> Option<String> {
    if !s.contains('%') {
        return Some(s.to_string());
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let h = std::str::from_utf8(hex).ok()?;
            let b = u8::from_str_radix(h, 16).ok()?;
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    // A malformed UTF-8 sequence is a URIError in JavaScript.
    String::from_utf8(out).ok()
}

/// JavaScript `encodeURIComponent`.
pub fn encode_uri_component(s: &str) -> String {
    const SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'!')
        .remove(b'~')
        .remove(b'*')
        .remove(b'\'')
        .remove(b'(')
        .remove(b')');
    percent_encoding::utf8_percent_encode(s, SET).to_string()
}

/// Next's `cookies()` view of the request.
pub fn parse_cookies_next(headers: &HeaderMap) -> IndexMap<String, String> {
    let mut map = IndexMap::new();
    let Some(raw) = cookie_header(headers) else {
        return map;
    };
    // `cookie.split(/; */)`
    for pair in split_semicolon_spaces(&raw) {
        if pair.is_empty() {
            continue;
        }
        match pair.find('=') {
            None => {
                map.insert(pair.to_string(), "true".to_string());
            }
            Some(at) => {
                let (key, value) = (&pair[..at], &pair[at + 1..]);
                if let Some(decoded) = decode_uri_component(value) {
                    map.insert(key.to_string(), decoded);
                }
            }
        }
    }
    map
}

fn split_semicolon_spaces(raw: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut rest = raw;
    while let Some(idx) = rest.find(';') {
        parts.push(&rest[..idx]);
        rest = rest[idx + 1..].trim_start_matches(' ');
    }
    parts.push(rest);
    parts
}

/// better-call `parseCookies` (first occurrence wins).
pub fn parse_cookies_better_call(headers: &HeaderMap) -> IndexMap<String, String> {
    let mut cookies = IndexMap::new();
    let Some(s) = cookie_header(headers) else {
        return cookies;
    };
    let mut index = 0;
    while index < s.len() {
        let Some(eq_rel) = s[index..].find('=') else {
            break;
        };
        let eq_idx = index + eq_rel;
        let mut end_idx = match s[index..].find(';') {
            Some(rel) => index + rel,
            None => s.len(),
        };
        if end_idx < eq_idx {
            // `str.lastIndexOf(";", eqIdx - 1) + 1`
            index = s[..eq_idx].rfind(';').map(|p| p + 1).unwrap_or(0);
            continue;
        }
        if end_idx > s.len() {
            end_idx = s.len();
        }
        let key = js_trim_ascii(&s[index..eq_idx]);
        if !cookies.contains_key(key) {
            let mut val = js_trim_ascii(&s[eq_idx + 1..end_idx]);
            if let Some(rest) = val.strip_prefix('"') {
                // `val.slice(1, -1)`: drop the quote and the last character.
                val = match rest.char_indices().last() {
                    Some((last, _)) => &rest[..last],
                    None => "",
                };
            }
            let decoded = if val.contains('%') {
                decode_uri_component(val).unwrap_or_else(|| val.to_string())
            } else {
                val.to_string()
            };
            cookies.insert(key.to_string(), decoded);
        }
        index = end_idx + 1;
    }
    cookies
}

fn js_trim_ascii(s: &str) -> &str {
    crate::config::js_trim(s)
}

/// `better-auth/cookies` `parseCookies` (RFC-validated, later duplicates win).
pub fn parse_cookies_better_auth(headers: &HeaderMap) -> IndexMap<String, String> {
    let mut map = IndexMap::new();
    let Some(cookie) = cookie_header(headers) else {
        return map;
    };
    if cookie.len() < 2 {
        return map;
    }
    for chunk in cookie.split(';') {
        let Some(eq) = chunk.find('=') else {
            continue;
        };
        let key = trim_ows(&chunk[..eq]);
        let mut val = trim_ows(&chunk[eq + 1..]);
        if val.len() >= 2 && val.starts_with('"') && val.ends_with('"') {
            val = &val[1..val.len() - 1];
        }
        if is_cookie_name(key) && is_cookie_value(val) {
            let decoded = if val.contains('%') {
                decode_uri_component(val).unwrap_or_else(|| val.to_string())
            } else {
                val.to_string()
            };
            map.insert(key.to_string(), decoded);
        }
    }
    map
}

fn trim_ows(s: &str) -> &str {
    s.trim_matches(|c| c == ' ' || c == '\t')
}

/// `/^[\w!#$%&'*.^`|~+-]+$/`
fn is_cookie_name(s: &str) -> bool {
    !s.is_empty()
        && s.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'_' | b'!'
                        | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'.'
                        | b'^'
                        | b'`'
                        | b'|'
                        | b'~'
                        | b'+'
                        | b'-'
                )
        })
}

/// `/^[ !#-:<-[\]-~]*$/`
fn is_cookie_value(s: &str) -> bool {
    s.bytes().all(|b| {
        b == b' ' || b == b'!' || (b'#'..=b':').contains(&b) || (b'<'..=b'[').contains(&b) || (b']'..=b'~').contains(&b)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn h(cookie: &str) -> HeaderMap {
        let mut m = HeaderMap::new();
        m.insert(header::COOKIE, HeaderValue::from_str(cookie).unwrap());
        m
    }

    #[test]
    fn next_parser() {
        let c = parse_cookies_next(&h("a=1; b=x%20y;c; a=2;  d=%E0%A4%A"));
        assert_eq!(c.get("a").map(String::as_str), Some("2"));
        assert_eq!(c.get("b").map(String::as_str), Some("x y"));
        assert_eq!(c.get("c").map(String::as_str), Some("true"));
        assert!(!c.contains_key("d"));
    }

    #[test]
    fn better_call_parser_keeps_the_first_duplicate() {
        let c = parse_cookies_better_call(&h("t=one.sig%3D; t=two; q=\"quoted\""));
        assert_eq!(c.get("t").map(String::as_str), Some("one.sig="));
        assert_eq!(c.get("q").map(String::as_str), Some("quoted"));
    }

    #[test]
    fn better_auth_parser_validates() {
        let c = parse_cookies_better_auth(&h("ok=1; bad name=2; v=a\"b; z=3; z=4"));
        assert_eq!(c.get("ok").map(String::as_str), Some("1"));
        assert!(!c.contains_key("bad name"));
        assert!(!c.contains_key("v"));
        assert_eq!(c.get("z").map(String::as_str), Some("4"));
    }

    #[test]
    fn uri_component_round_trip() {
        assert_eq!(encode_uri_component("a b/c+d=é"), "a%20b%2Fc%2Bd%3D%C3%A9");
        assert_eq!(decode_uri_component("a%20b%2Fc%2Bd%3D%C3%A9").unwrap(), "a b/c+d=é");
        assert!(decode_uri_component("%ZZ").is_none());
        assert!(decode_uri_component("%C3").is_none());
    }
}
