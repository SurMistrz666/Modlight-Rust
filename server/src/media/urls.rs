//! Port of `lib/media/urls.ts`: the public URLs of a file and of an account.
//!
//! Pure string building over [`Config`]: every function takes the config (or a
//! `base`) explicitly where upstream read the `config` singleton. The
//! re-exports of `lib/media/paths.ts` at the end of the upstream file are not
//! part of this module.

use crate::config::{Config, js_trim};

/// Build the storage key for a media file: `{userCode}/{code}.{ext}`.
pub fn storage_key_for(user_code: &str, code: &str, ext: &str) -> String {
    if ext.is_empty() { format!("{user_code}/{code}") } else { format!("{user_code}/{code}.{ext}") }
}

/// `x.trim().replace(/\/+$/, "").toLowerCase()`
fn normalize_wanted(origin: &str) -> String {
    js_trim(origin).trim_end_matches('/').to_lowercase()
}

/// Which origin a given user's links are generated with.
///
/// The deployment answers on several names (see `config.appDomains`) and every
/// one of them serves the same files, so this only decides what a NEW link
/// *reads* as. Anything already shared keeps resolving on whichever host it
/// was created with, forever.
///
/// An unknown value falls back to `config.appUrl` rather than being echoed
/// back: the result ends up in ShareX configs, Open Graph tags and copied
/// links, so a stored origin the operator never configured must never be
/// trusted. That also makes the column self-healing: drop a domain from
/// APP_DOMAINS and the accounts pointing at it quietly return to the default.
pub fn user_base_url(config: &Config, preferred: Option<&str>) -> String {
    let Some(preferred) = preferred.filter(|p| !p.is_empty()) else {
        return config.app_url.clone();
    };
    let wanted = normalize_wanted(preferred);
    if wanted.is_empty() {
        return config.app_url.clone();
    }
    config.app_domains.iter().find(|d| d.to_lowercase() == wanted).cloned().unwrap_or_else(|| config.app_url.clone())
}

/// Is this origin one the operator configured? Used to validate user input.
pub fn is_known_domain(config: &Config, origin: &str) -> bool {
    let wanted = normalize_wanted(origin);
    config.app_domains.iter().any(|d| d.to_lowercase() == wanted)
}

/// Public preview page URL: `/{userCode}/{code}.{ext}` (the shape ShareX
/// configs expect).
pub fn public_url(user_code: &str, code: &str, ext: &str, base: &str) -> String {
    if ext.is_empty() { format!("{base}/{user_code}/{code}") } else { format!("{base}/{user_code}/{code}.{ext}") }
}

/// Preview URL without the tag segment: `/{code}.{ext}`.
///
/// Admin-only (user.skipTagInUrl) because a tagless code sits in the root
/// namespace shared with every top-level route; see RESERVED_TAGS in
/// lib/naming.ts, which is what keeps a code from shadowing /login.
pub fn tagless_url(code: &str, ext: &str, base: &str) -> String {
    if ext.is_empty() { format!("{base}/{code}") } else { format!("{base}/{code}.{ext}") }
}

/// Raw file URL without the tag segment: `/{code}/raw.{ext}`.
pub fn tagless_raw_url(code: &str, ext: &str, base: &str) -> String {
    if ext.is_empty() { format!("{base}/{code}/raw") } else { format!("{base}/{code}/raw.{ext}") }
}

/// Raw file URL: `/{userCode}/{code}/raw.{ext}` (the shape ShareX configs expect).
pub fn raw_url(user_code: &str, code: &str, ext: &str, base: &str) -> String {
    if ext.is_empty() {
        format!("{base}/{user_code}/{code}/raw")
    } else {
        format!("{base}/{user_code}/{code}/raw.{ext}")
    }
}

/// The link-shaping facts that belong to an ACCOUNT rather than to a file.
#[derive(Debug, Clone, Default)]
pub struct LinkOwner<'a> {
    pub user_code: Option<&'a str>,
    pub preferred_domain: Option<&'a str>,
    pub skip_tag_in_url: Option<bool>,
}

/// The pair of public links for one upload, on the hostname and in the shape
/// its owner chose (`preferredDomain` and `skipTagInUrl` folded in, so no
/// caller can forget either).
pub fn media_urls(config: &Config, owner: &LinkOwner<'_>, code: &str, ext: &str) -> (String, String) {
    let base = user_base_url(config, owner.preferred_domain);
    let tag = owner.user_code.unwrap_or("u");
    if owner.skip_tag_in_url == Some(true) {
        return (tagless_url(code, ext, &base), tagless_raw_url(code, ext, &base));
    }
    (public_url(tag, code, ext, &base), raw_url(tag, code, ext, &base))
}

/// Sandboxed-render URL for an uploaded page: `/{userCode}/{code}/render?t=...`
/// (deliberately RELATIVE, see upstream).
pub fn render_path(user_code: &str, code: &str, ticket: &str) -> String {
    format!("/{user_code}/{code}/render?t={}", crate::http::cookies::encode_uri_component(ticket))
}

/// One action on a file as a PATH: `/{userCode}/{code}/{action}`.
pub fn media_action_path(user_code: &str, code: &str, action: &str) -> String {
    format!("/{user_code}/{code}/{action}")
}

/// Force-download URL.
pub fn download_url(user_code: &str, code: &str, base: &str) -> String {
    format!("{base}/{user_code}/{code}/download")
}

/// Deletion URL used by ShareX: `/{userCode}/{code}/delete/{token}`.
pub fn delete_url(user_code: &str, code: &str, token: &str, base: &str) -> String {
    format!("{base}/{user_code}/{code}/delete/{token}")
}

/// Extract a lowercase extension (no dot) from a filename
/// (`/\.([a-z0-9]+)$/i`).
pub fn ext_of(filename: &str) -> String {
    let Some(dot) = filename.rfind('.') else {
        return String::new();
    };
    let tail = &filename[dot + 1..];
    if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_alphanumeric()) {
        tail.to_ascii_lowercase()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config(domains: &str) -> Config {
        let mut vars = HashMap::new();
        vars.insert("APP_URL".to_string(), "https://host.example.com".to_string());
        vars.insert("APP_DOMAINS".to_string(), domains.to_string());
        Config::from_env_map(&vars).unwrap()
    }

    #[test]
    fn base_url_only_trusts_configured_domains() {
        let c = config("https://H.example.com, https://s.example.com/");
        assert_eq!(user_base_url(&c, None), "https://host.example.com");
        assert_eq!(user_base_url(&c, Some("")), "https://host.example.com");
        assert_eq!(user_base_url(&c, Some("  ")), "https://host.example.com");
        assert_eq!(user_base_url(&c, Some("https://h.example.com//")), "https://H.example.com");
        assert_eq!(user_base_url(&c, Some("https://evil.example.com")), "https://host.example.com");
        assert!(is_known_domain(&c, " https://S.example.com/ "));
        assert!(!is_known_domain(&c, "https://evil.example.com"));
    }

    #[test]
    fn url_shapes() {
        assert_eq!(raw_url("tag", "abc", "png", "https://x"), "https://x/tag/abc/raw.png");
        assert_eq!(raw_url("tag", "abc", "", "https://x"), "https://x/tag/abc/raw");
        assert_eq!(tagless_raw_url("abc", "png", "https://x"), "https://x/abc/raw.png");
        assert_eq!(public_url("tag", "abc", "png", "https://x"), "https://x/tag/abc.png");
        assert_eq!(tagless_url("abc", "", "https://x"), "https://x/abc");
        assert_eq!(ext_of("Photo.JPG"), "jpg");
        assert_eq!(ext_of("archive.tar.gz"), "gz");
        assert_eq!(ext_of("noext"), "");
        assert_eq!(ext_of("weird.ex-t"), "");
        let c = config("");
        let owner = LinkOwner { user_code: None, preferred_domain: None, skip_tag_in_url: Some(true) };
        assert_eq!(
            media_urls(&c, &owner, "abc", "png"),
            ("https://host.example.com/abc.png".to_string(), "https://host.example.com/abc/raw.png".to_string())
        );
    }
}
