//! Port of `proxy.ts` (Next 16's middleware).
//!
//! Two jobs, both cheap and neither authoritative:
//!
//!  1. An OPTIMISTIC dashboard gate (cookie presence only) - real authorization
//!     happens in each route/page via `require_user()`/`require_admin()`.
//!  2. Host-based rewriting for the dedicated shortener domain.
//!
//! The SPA adds three top-level paths of its own that must keep working on the
//! shortener host the way `/_next` did upstream: `/_page`, `/_rpc` and
//! `/assets/`. They are added to the pass-through list; none of them can be a
//! short code a user typed, except a code literally named "assets" (documented
//! in SKELETON.md).

use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::http::cookies::parse_cookies_better_auth;

#[derive(Debug, Clone, Default)]
pub struct Proxy {
    /// Hostname that serves short links, from SHORTLINK_URL (read raw, as
    /// proxy.ts did, so it never depends on the rest of the config).
    pub short_host: Option<String>,
}

/// Paths that must keep working on the shortener host: certificate renewal,
/// health checks, framework assets, and the app's own API.
/// `/^\/(?:_next|api|s\/|favicon|icon|apple-icon|robots|sitemap|\.well-known)/`
/// plus the SPA's `_page`, `_rpc` and `assets/`.
pub fn is_passthrough(pathname: &str) -> bool {
    let Some(rest) = pathname.strip_prefix('/') else {
        return false;
    };
    const PREFIXES: &[&str] = &[
        "_next",
        "api",
        "s/",
        "favicon",
        "icon",
        "apple-icon",
        "robots",
        "sitemap",
        ".well-known",
        // Rust port: the SPA's own surface.
        "_page",
        "_rpc",
        "assets/",
    ];
    PREFIXES.iter().any(|p| rest.starts_with(p))
}

impl Proxy {
    pub fn new(shortlink_url_raw: Option<&str>) -> Proxy {
        let short_host = shortlink_url_raw
            .map(crate::config::js_trim)
            .filter(|s| !s.is_empty())
            .and_then(|raw| url::Url::parse(raw).ok())
            .and_then(|u| u.host_str().map(|h| h.to_lowercase()))
            .filter(|h| !h.is_empty());
        Proxy { short_host }
    }

    /// On the shortener domain the short code IS the path: s.example.com/promo
    /// instead of s.example.com/s/promo. Returns the rewritten path, if any.
    pub fn rewrite(&self, host_header: Option<&str>, pathname: &str) -> Option<String> {
        let short = self.short_host.as_deref()?;
        let host = host_header?.split(':').next()?.to_lowercase();
        if host == short && pathname != "/" && !is_passthrough(pathname) { Some(format!("/s{pathname}")) } else { None }
    }

    /// The optimistic gate: `/dashboard*` without any session cookie goes to
    /// `/login?next=<path>`. Returns the redirect location.
    pub fn dashboard_gate(&self, pathname: &str, headers: &HeaderMap) -> Option<String> {
        if !pathname.starts_with("/dashboard") || has_session_cookie(headers) {
            return None;
        }
        let next: String = form_urlencoded::Serializer::new(String::new()).append_pair("next", pathname).finish();
        Some(format!("/login?{next}"))
    }
}

/// `getSessionCookie(request)` from `better-auth/cookies`: is there a non-empty
/// session token cookie under any of the names Better Auth uses?
pub fn has_session_cookie(headers: &HeaderMap) -> bool {
    let parsed = parse_cookies_better_auth(headers);
    let get = |name: &str| {
        parsed
            .get(name)
            .filter(|v| !v.is_empty())
            .or_else(|| parsed.get(&format!("__Secure-{name}")).filter(|v| !v.is_empty()))
            .is_some()
    };
    get("better-auth.session_token") || get("better-auth-session_token")
}

/// The redirect response `NextResponse.redirect()` produced in the proxy:
/// 307, a relative `location`, and the location as the body.
pub fn redirect_response(location: &str) -> Response {
    let mut res = (StatusCode::TEMPORARY_REDIRECT, location.to_string()).into_response();
    if let Ok(v) = HeaderValue::from_str(location) {
        res.headers_mut().insert(header::LOCATION, v);
    }
    res.headers_mut().remove(header::CONTENT_TYPE);
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookies(c: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, HeaderValue::from_str(c).unwrap());
        h
    }

    #[test]
    fn rewrites_only_on_the_short_host() {
        let p = Proxy::new(Some(" https://S.example.com "));
        assert_eq!(p.rewrite(Some("s.example.com"), "/promo"), Some("/s/promo".into()));
        assert_eq!(p.rewrite(Some("s.example.com:443"), "/tag/promo"), Some("/s/tag/promo".into()));
        assert_eq!(p.rewrite(Some("s.example.com"), "/"), None);
        assert_eq!(p.rewrite(Some("s.example.com"), "/s/promo"), None);
        assert_eq!(p.rewrite(Some("s.example.com"), "/api/health"), None);
        assert_eq!(p.rewrite(Some("s.example.com"), "/_page"), None);
        assert_eq!(p.rewrite(Some("s.example.com"), "/assets/app.js"), None);
        assert_eq!(p.rewrite(Some("s.example.com"), "/iconic"), None);
        assert_eq!(p.rewrite(Some("host.example.com"), "/promo"), None);
        assert_eq!(Proxy::new(None).rewrite(Some("s.example.com"), "/promo"), None);
        assert_eq!(Proxy::new(Some("not a url")).short_host, None);
    }

    #[test]
    fn dashboard_gate_needs_only_a_cookie() {
        let p = Proxy::default();
        assert_eq!(p.dashboard_gate("/dashboard", &HeaderMap::new()), Some("/login?next=%2Fdashboard".into()));
        assert_eq!(
            p.dashboard_gate("/dashboard%2Fadmin", &HeaderMap::new()),
            Some("/login?next=%2Fdashboard%252Fadmin".into())
        );
        assert_eq!(p.dashboard_gate("/login", &HeaderMap::new()), None);
        assert_eq!(p.dashboard_gate("/dashboard", &cookies("better-auth.session_token=x")), None);
        assert_eq!(p.dashboard_gate("/dashboard", &cookies("__Secure-better-auth.session_token=x")), None);
        assert_eq!(p.dashboard_gate("/dashboard", &cookies("better-auth-session_token=x")), None);
        assert!(p.dashboard_gate("/dashboard", &cookies("better-auth.session_token=")).is_some());
    }
}
