//! Port of `i18n/request.ts`: locale resolution (no URL routing).
//!
//!   1) explicit cookie `ml_locale` (set when a user picks a language)
//!   2) Accept-Language - Polish browsers get 'pl', everyone else 'en'
//!      (only pl/en supported; e.g. German falls back to English).

use axum::http::{HeaderMap, header};
use serde::{Deserialize, Serialize};

use crate::http::cookies::parse_cookies_next;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Locale {
    En,
    Pl,
}

impl Locale {
    pub fn as_str(self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::Pl => "pl",
        }
    }

    pub fn parse(value: &str) -> Option<Locale> {
        match value {
            "en" => Some(Locale::En),
            "pl" => Some(Locale::Pl),
            _ => None,
        }
    }
}

pub const LOCALES: [Locale; 2] = [Locale::En, Locale::Pl];

/// The cookie a user's explicit language choice lives in.
pub const LOCALE_COOKIE: &str = "ml_locale";

/// One file per UI area under `messages/<locale>/`. Each feature owns its own
/// namespace file so translations never collide. (`mail` is server-only.)
pub const NAMESPACES: &[&str] = &[
    "common",
    "nav",
    "auth",
    "dashboard",
    "profile",
    "paste",
    "links",
    "albums",
    "tokens",
    "admin",
    "public",
    "landing",
    "viewer",
    "files",
    "upload",
    "download",
    "authorize",
    // Not upstream: the optional Cloudflare Turnstile widget and its admin card
    // (src/turnstile.rs). A file of its own, so upstream's stay verbatim.
    "turnstile",
];

/// The resolution rule itself, on plain values.
///
/// A cookie that is present but holds something else (`ml_locale=de`) means
/// English WITHOUT consulting Accept-Language: upstream only looks at the
/// header when the cookie is absent or empty.
pub fn resolve_locale(cookie_value: Option<&str>, accept_language: Option<&str>) -> Locale {
    match cookie_value {
        Some(v) if !v.is_empty() => Locale::parse(v).unwrap_or(Locale::En),
        _ => {
            if accepts_polish(accept_language.unwrap_or("")) {
                Locale::Pl
            } else {
                Locale::En
            }
        }
    }
}

/// `/(?:^|[,\s])pl\b/i`
fn accepts_polish(al: &str) -> bool {
    let lower = al.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    for i in 0..chars.len() {
        if chars[i] != 'p' || chars.get(i + 1) != Some(&'l') {
            continue;
        }
        let before_ok = i == 0 || chars[i - 1] == ',' || chars[i - 1].is_whitespace();
        let after = chars.get(i + 2);
        let after_ok = after.is_none_or(|c| !(c.is_ascii_alphanumeric() || *c == '_'));
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

/// Resolve the locale of a request (cookies as Next's `cookies()` reads them).
pub fn request_locale(headers: &HeaderMap) -> Locale {
    let cookies = parse_cookies_next(headers);
    let al = headers.get(header::ACCEPT_LANGUAGE).and_then(|v| v.to_str().ok());
    resolve_locale(cookies.get(LOCALE_COOKIE).map(String::as_str), al)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_wins_and_an_unknown_cookie_means_english() {
        assert_eq!(resolve_locale(Some("pl"), Some("en-US")), Locale::Pl);
        assert_eq!(resolve_locale(Some("en"), Some("pl-PL")), Locale::En);
        assert_eq!(resolve_locale(Some("de"), Some("pl-PL")), Locale::En);
        assert_eq!(resolve_locale(Some(""), Some("pl-PL")), Locale::Pl);
    }

    #[test]
    fn accept_language_matches_upstream_regex() {
        assert_eq!(resolve_locale(None, Some("pl-PL,pl;q=0.9")), Locale::Pl);
        assert_eq!(resolve_locale(None, Some("en-US,pl;q=0.9")), Locale::Pl);
        assert_eq!(resolve_locale(None, Some("en, PL")), Locale::Pl);
        assert_eq!(resolve_locale(None, Some("de-DE")), Locale::En);
        assert_eq!(resolve_locale(None, Some("xpl")), Locale::En);
        assert_eq!(resolve_locale(None, Some("plx")), Locale::En);
        assert_eq!(resolve_locale(None, None), Locale::En);
    }
}
