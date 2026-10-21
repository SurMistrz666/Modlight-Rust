//! Port of the `headers()` rules in `next.config.ts`.
//!
//! Upstream computed the CSP at BUILD time from APP_URL / SHORTLINK_URL /
//! APP_DOMAINS (the Docker image received APP_URL as a build argument). Here it
//! is computed once at startup from the same variables, read raw exactly as
//! next.config.ts read them (not the normalised values of lib/config.ts).
//!
//! Semantics reproduced from Next:
//! - rules are matched against the RAW request path, case-insensitively, with
//!   path-to-regexp's `strict` matching;
//! - every matching rule applies, a later rule's value wins for the same key
//!   (so `/{tag}/{code}/raw` ends up with the raw CSP);
//! - config headers win over the same header set by a route handler (Next sets
//!   them first and only fills in keys the handler response adds that are not
//!   present yet).

use std::collections::HashMap;

use axum::http::{HeaderMap, HeaderName, HeaderValue};
use url::Url;

/// The policies, built once.
#[derive(Debug, Clone)]
pub struct SecurityHeaders {
    pub app_csp: String,
    pub authorize_csp: String,
    pub raw_csp: String,
    pub render_csp: String,
    /// Not upstream: `app_csp` and `authorize_csp` with Cloudflare Turnstile's
    /// origin added to `script-src` and `frame-src`, sent instead of them
    /// while the optional check is on (src/turnstile.rs). Every page gets it,
    /// not only the four forms: the SPA reaches /login from any page without
    /// loading a new document.
    pub app_csp_turnstile: String,
    pub authorize_csp_turnstile: String,
}

/// Every origin this deployment serves on: APP_URL plus APP_DOMAINS plus the
/// shortener. A user's links are generated on the hostname they chose, so the
/// panel on one domain routinely references media on another - with `'self'`
/// alone the browser blocks those silently and thumbnails just never appear.
pub fn self_origins(app_url: Option<&str>, shortlink_url: Option<&str>, app_domains: Option<&str>) -> Vec<String> {
    let mut raw: Vec<&str> = vec![app_url.unwrap_or(""), shortlink_url.unwrap_or("")];
    raw.extend(app_domains.unwrap_or("").split(','));
    let mut origins: Vec<String> = Vec::new();
    for value in raw {
        let trimmed = crate::config::js_trim(value);
        if trimmed.is_empty() {
            continue;
        }
        // Malformed entries are ignored rather than breaking the build.
        if let Ok(url) = Url::parse(trimmed) {
            let origin = url.origin().ascii_serialization();
            if !origins.contains(&origin) {
                origins.push(origin);
            }
        }
    }
    origins
}

impl SecurityHeaders {
    /// `is_dev` adds what Turbopack HMR needed upstream ('unsafe-eval',
    /// websockets); kept so a dev build behaves like the upstream dev server.
    pub fn new(self_origins: &[String], is_dev: bool) -> SecurityHeaders {
        let joined = self_origins.join(" ");
        let own = if joined.is_empty() { "'self'".to_string() } else { format!("'self' {joined}") };
        let connect = if is_dev { format!("connect-src {own} ws: wss:") } else { format!("connect-src {own}") };
        // `extra`: an origin added to script-src and frame-src (Turnstile).
        let app_policy = |extra: Option<&str>| {
            let plus = extra.map(|o| format!(" {o}")).unwrap_or_default();
            [
                "default-src 'self'".to_string(),
                "base-uri 'self'".to_string(),
                "object-src 'none'".to_string(),
                "frame-ancestors 'self'".to_string(),
                "form-action 'self'".to_string(),
                format!("img-src {own} data: blob:"),
                format!("media-src {own} blob:"),
                format!("frame-src {own}{plus}"),
                "font-src 'self' data:".to_string(),
                "worker-src 'self' blob:".to_string(),
                "style-src 'self' 'unsafe-inline'".to_string(),
                if is_dev {
                    format!("script-src 'self' 'unsafe-inline' 'unsafe-eval'{plus}")
                } else {
                    format!("script-src 'self' 'unsafe-inline'{plus}")
                },
                connect.clone(),
            ]
            .join("; ")
        };
        let app_csp = app_policy(None);
        let app_csp_turnstile = app_policy(Some(crate::turnstile::CHALLENGES_ORIGIN));
        // `/authorize` needs one thing the rest of the app must never have:
        // permission to talk to a port on this machine (RFC 8252 loopback
        // handoff). This policy REPLACES app_csp for that path.
        let authorize_connect = if is_dev {
            format!("connect-src {own} http://127.0.0.1:* ws: wss:")
        } else {
            format!("connect-src {own} http://127.0.0.1:*")
        };
        let authorize_csp = app_csp.replacen(&connect, &authorize_connect, 1);
        let authorize_csp_turnstile = app_csp_turnstile.replacen(&connect, &authorize_connect, 1);
        // User-uploaded bytes get a policy of their own.
        let raw_csp = [
            "default-src 'none'",
            "img-src 'self' data:",
            "media-src 'self'",
            "style-src 'unsafe-inline'",
            "frame-ancestors 'self'",
            "sandbox",
        ]
        .join("; ");
        // The policy that actually sandboxes the HTML/SVG preview.
        let render_csp = [
            "sandbox allow-scripts",
            "default-src 'none'",
            "img-src data: blob:",
            "media-src data: blob:",
            "font-src data:",
            "style-src 'unsafe-inline'",
            "script-src 'unsafe-inline'",
            "form-action 'none'",
            "base-uri 'none'",
            "object-src 'none'",
            "frame-ancestors 'self'",
        ]
        .join("; ");
        SecurityHeaders { app_csp, authorize_csp, raw_csp, render_csp, app_csp_turnstile, authorize_csp_turnstile }
    }

    /// Build from the process environment the way next.config.ts read it.
    pub fn from_env_map(vars: &HashMap<String, String>, is_dev: bool) -> SecurityHeaders {
        let get = |k: &str| vars.get(k).map(String::as_str);
        SecurityHeaders::new(&self_origins(get("APP_URL"), get("SHORTLINK_URL"), get("APP_DOMAINS")), is_dev)
    }

    /// The headers every matching rule sets for `raw_path`, later rules
    /// overriding earlier ones. Empty for paths no rule matches
    /// (`/authorize/anything`). `turnstile`: the optional Cloudflare
    /// Turnstile check is on (the two app policies let its widget in).
    pub fn for_path(&self, raw_path: &str, turnstile: bool) -> Vec<(HeaderName, HeaderValue)> {
        let lower = raw_path.to_ascii_lowercase();
        let mut csp: Option<&str> = None;
        let rest = lower.strip_prefix('/').unwrap_or(&lower);
        let (app, authorize) = if turnstile {
            (&self.app_csp_turnstile, &self.authorize_csp_turnstile)
        } else {
            (&self.app_csp, &self.authorize_csp)
        };
        // "/:path((?!authorize$|authorize/).*)"
        if lower.starts_with('/') && !(rest == "authorize" || rest.starts_with("authorize/")) {
            csp = Some(app);
        }
        // "/authorize"
        if lower == "/authorize" {
            csp = Some(authorize);
        }
        let segments: Vec<&str> = match lower.strip_prefix('/') {
            Some(rest) => rest.split('/').collect(),
            None => Vec::new(),
        };
        if segments.len() == 3 && !segments[0].is_empty() && !segments[1].is_empty() {
            let last = segments[2];
            let is_raw = last == "raw" || (last.starts_with("raw.") && last.len() > 4) || last == "download";
            if is_raw {
                csp = Some(&self.raw_csp);
            } else if last == "render" {
                csp = Some(&self.render_csp);
            }
        }
        let Some(csp) = csp else {
            return Vec::new();
        };
        let mut out = base_security_headers();
        if let Ok(v) = HeaderValue::from_str(csp) {
            out.push((HeaderName::from_static("content-security-policy"), v));
        }
        out
    }

    /// Apply to a response: config headers win over handler headers.
    pub fn apply(&self, raw_path: &str, headers: &mut HeaderMap, turnstile: bool) {
        for (name, value) in self.for_path(raw_path, turnstile) {
            headers.insert(name, value);
        }
    }
}

/// Headers every rule carries, in upstream order.
pub fn base_security_headers() -> Vec<(HeaderName, HeaderValue)> {
    vec![
        (HeaderName::from_static("x-content-type-options"), HeaderValue::from_static("nosniff")),
        (HeaderName::from_static("referrer-policy"), HeaderValue::from_static("strict-origin-when-cross-origin")),
        // Pin the browser to HTTPS for a year; `preload` deliberately omitted.
        (
            HeaderName::from_static("strict-transport-security"),
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        ),
        // Legacy companion to frame-ancestors; SAMEORIGIN keeps the PDF iframe alive.
        (HeaderName::from_static("x-frame-options"), HeaderValue::from_static("SAMEORIGIN")),
        (
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static(
                "camera=(), microphone=(), geolocation=(), payment=(), usb=(), interest-cohort=()",
            ),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prod() -> SecurityHeaders {
        SecurityHeaders::new(&self_origins(Some("http://localhost:8420"), None, None), false)
    }

    fn csp(h: &SecurityHeaders, path: &str) -> Option<String> {
        csp_with(h, path, false)
    }

    fn csp_with(h: &SecurityHeaders, path: &str, turnstile: bool) -> Option<String> {
        h.for_path(path, turnstile)
            .into_iter()
            .find(|(k, _)| k == "content-security-policy")
            .map(|(_, v)| v.to_str().unwrap().to_string())
    }

    /// Byte-for-byte what the reference server (NODE_ENV=production,
    /// APP_URL=http://localhost:8420) sends.
    #[test]
    fn app_csp_matches_the_reference_server() {
        assert_eq!(
            csp(&prod(), "/login").unwrap(),
            "default-src 'self'; base-uri 'self'; object-src 'none'; frame-ancestors 'self'; form-action 'self'; img-src 'self' http://localhost:8420 data: blob:; media-src 'self' http://localhost:8420 blob:; frame-src 'self' http://localhost:8420; font-src 'self' data:; worker-src 'self' blob:; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline'; connect-src 'self' http://localhost:8420"
        );
    }

    #[test]
    fn authorize_gets_the_loopback_policy_and_nothing_below_it_matches() {
        let h = prod();
        assert!(
            csp(&h, "/authorize").unwrap().ends_with("connect-src 'self' http://localhost:8420 http://127.0.0.1:*")
        );
        assert!(csp(&h, "/AUTHORIZE").unwrap().contains("http://127.0.0.1:*"));
        assert!(h.for_path("/authorize/x", false).is_empty());
        assert!(csp(&h, "/authorized").unwrap().ends_with("connect-src 'self' http://localhost:8420"));
    }

    #[test]
    fn raw_and_render_paths_get_their_own_policy() {
        let h = prod();
        assert!(csp(&h, "/tag/code/raw").unwrap().starts_with("default-src 'none'"));
        assert!(csp(&h, "/tag/code/raw.png").unwrap().ends_with("sandbox"));
        assert!(csp(&h, "/tag/code/download").unwrap().ends_with("sandbox"));
        assert!(csp(&h, "/tag/code/render").unwrap().starts_with("sandbox allow-scripts"));
        // Matched on the raw path: an encoded segment is not "render".
        assert!(csp(&h, "/tag/code/%72ender").unwrap().starts_with("default-src 'self'"));
        assert!(csp(&h, "/tag/code/raw.").unwrap().starts_with("default-src 'self'"));
    }

    /// Not upstream: with Turnstile on, Cloudflare's origin joins script-src
    /// and frame-src of the two app policies, and nothing else changes.
    #[test]
    fn turnstile_adds_its_origin_to_script_and_frame_src_only() {
        let h = prod();
        assert_eq!(
            csp_with(&h, "/login", true).unwrap(),
            "default-src 'self'; base-uri 'self'; object-src 'none'; frame-ancestors 'self'; form-action 'self'; img-src 'self' http://localhost:8420 data: blob:; media-src 'self' http://localhost:8420 blob:; frame-src 'self' http://localhost:8420 https://challenges.cloudflare.com; font-src 'self' data:; worker-src 'self' blob:; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline' https://challenges.cloudflare.com; connect-src 'self' http://localhost:8420"
        );
        let authorize = csp_with(&h, "/authorize", true).unwrap();
        assert!(authorize.contains("script-src 'self' 'unsafe-inline' https://challenges.cloudflare.com;"));
        assert!(authorize.ends_with("connect-src 'self' http://localhost:8420 http://127.0.0.1:*"));
        // User content keeps its own policies.
        assert_eq!(csp_with(&h, "/tag/code/raw", true), csp(&h, "/tag/code/raw"));
        assert_eq!(csp_with(&h, "/tag/code/render", true), csp(&h, "/tag/code/render"));
        // Off: byte for byte the policy without it.
        assert!(!csp(&h, "/login").unwrap().contains("cloudflare"));
        let dev = SecurityHeaders::new(&[], true);
        assert!(dev.app_csp_turnstile.contains("'unsafe-eval' https://challenges.cloudflare.com;"));
    }

    #[test]
    fn origins_are_deduplicated_and_malformed_ones_skipped() {
        let o = self_origins(
            Some("https://host.example.com/"),
            Some("https://s.example.com"),
            Some("https://h.example.com, not a url ,https://host.example.com:443"),
        );
        assert_eq!(o, vec!["https://host.example.com", "https://s.example.com", "https://h.example.com"]);
        let dev = SecurityHeaders::new(&o, true);
        assert!(dev.app_csp.contains("'unsafe-eval'"));
        assert!(dev.authorize_csp.ends_with("http://127.0.0.1:* ws: wss:"));
        let none = SecurityHeaders::new(&[], false);
        assert!(none.app_csp.ends_with("connect-src 'self'"));
    }
}
