//! Port of `lib/media/urls.ts`: how storage keys and public links are spelled.
//!
//! Upstream reads `config.appUrl` / `config.appDomains` from a module-level
//! singleton and defaults `base` to `config.appUrl`; here the configuration is
//! passed in, and callers that relied on the default pass `&config.app_url`.

use crate::config::{Config, js_trim};

pub use super::paths::{code_from_media_code, suffix_key, swap_filename_ext, swap_key_ext};

/// Build the storage key for a media file: "{userCode}/{code}.{ext}".
pub fn storage_key_for(user_code: &str, code: &str, ext: &str) -> String {
    if ext.is_empty() { format!("{user_code}/{code}") } else { format!("{user_code}/{code}.{ext}") }
}

/// `trim().replace(/\/+$/, "").toLowerCase()`
fn origin_key(value: &str) -> String {
    js_trim(value).trim_end_matches('/').to_lowercase()
}

/// Which origin a given user's links are generated with.
///
/// The deployment answers on several names and every one of them serves the
/// same files, so this only decides what a NEW link *reads* as. An unknown
/// value falls back to `config.appUrl` rather than being echoed back: the
/// result ends up in ShareX configs, Open Graph tags and copied links, so a
/// stored origin the operator never configured must never be trusted.
pub fn user_base_url(config: &Config, preferred: Option<&str>) -> String {
    let Some(preferred) = preferred.filter(|p| !p.is_empty()) else {
        return config.app_url.clone();
    };
    let wanted = origin_key(preferred);
    if wanted.is_empty() {
        return config.app_url.clone();
    }
    config.app_domains.iter().find(|d| d.to_lowercase() == wanted).cloned().unwrap_or_else(|| config.app_url.clone())
}

/// Is this origin one the operator configured? Used to validate user input.
pub fn is_known_domain(config: &Config, origin: &str) -> bool {
    let wanted = origin_key(origin);
    config.app_domains.iter().any(|d| d.to_lowercase() == wanted)
}

/// Public preview page URL: /{userCode}/{code}.{ext} (the shape ShareX configs expect).
pub fn public_url(user_code: &str, code: &str, ext: &str, base: &str) -> String {
    if ext.is_empty() { format!("{base}/{user_code}/{code}") } else { format!("{base}/{user_code}/{code}.{ext}") }
}

/// Preview URL without the tag segment: /{code}.{ext}
///
/// Admin-only (user.skipTagInUrl) because a tagless code sits in the root
/// namespace shared with every top-level route - see RESERVED_TAGS in
/// lib/naming.ts, which is what keeps a code from shadowing /login.
pub fn tagless_url(code: &str, ext: &str, base: &str) -> String {
    if ext.is_empty() { format!("{base}/{code}") } else { format!("{base}/{code}.{ext}") }
}

/// Raw file URL without the tag segment: /{code}/raw.{ext}
pub fn tagless_raw_url(code: &str, ext: &str, base: &str) -> String {
    if ext.is_empty() { format!("{base}/{code}/raw") } else { format!("{base}/{code}/raw.{ext}") }
}

/// Raw file URL: /{userCode}/{code}/raw.{ext} (the shape ShareX configs expect).
pub fn raw_url(user_code: &str, code: &str, ext: &str, base: &str) -> String {
    if ext.is_empty() {
        format!("{base}/{user_code}/{code}/raw")
    } else {
        format!("{base}/{user_code}/{code}/raw.{ext}")
    }
}

/// The link-shaping facts that belong to an ACCOUNT rather than to a file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkOwner {
    pub user_code: Option<String>,
    pub preferred_domain: Option<String>,
    pub skip_tag_in_url: Option<bool>,
}

impl LinkOwner {
    /// The three columns of a full user row.
    pub fn of_user(user: &crate::db::models::User) -> LinkOwner {
        LinkOwner {
            user_code: user.user_code.clone(),
            preferred_domain: user.preferred_domain.clone(),
            skip_tag_in_url: user.skip_tag_in_url,
        }
    }
}

/// `{ url, rawUrl }`
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaUrls {
    pub url: String,
    pub raw_url: String,
}

/// The pair of public links for one upload, on the hostname and in the shape
/// its owner chose. Both account settings (`preferredDomain`, `skipTagInUrl`)
/// are folded in here so the panel, ShareX and the desktop client cannot hand
/// the same user three different spellings of the same file.
pub fn media_urls(config: &Config, owner: &LinkOwner, code: &str, ext: &str) -> MediaUrls {
    let base = user_base_url(config, owner.preferred_domain.as_deref());
    let tag = owner.user_code.as_deref().unwrap_or("u");
    if owner.skip_tag_in_url == Some(true) {
        return MediaUrls { url: tagless_url(code, ext, &base), raw_url: tagless_raw_url(code, ext, &base) };
    }
    MediaUrls { url: public_url(tag, code, ext, &base), raw_url: raw_url(tag, code, ext, &base) }
}

/// Sandboxed-render URL for an uploaded page: /{userCode}/{code}/render?t=...
/// Deliberately RELATIVE (same-origin frame).
pub fn render_path(user_code: &str, code: &str, ticket: &str) -> String {
    format!("/{user_code}/{code}/render?t={}", crate::http::cookies::encode_uri_component(ticket))
}

/// One action on a file as a PATH: /{userCode}/{code}/{action}. Never carries
/// an origin (same-origin fetches, unlock cookies).
pub fn media_action_path(user_code: &str, code: &str, action: &str) -> String {
    format!("/{user_code}/{code}/{action}")
}

/// Force-download URL.
pub fn download_url(user_code: &str, code: &str, base: &str) -> String {
    format!("{base}/{user_code}/{code}/download")
}

/// Deletion URL used by ShareX: /{userCode}/{code}/delete/{token}.
pub fn delete_url(user_code: &str, code: &str, token: &str, base: &str) -> String {
    format!("{base}/{user_code}/{code}/delete/{token}")
}

/// Extract a lowercase extension (no dot) from a filename.
pub fn ext_of(filename: &str) -> String {
    super::sniff::ext_of_name(Some(filename))
}

#[cfg(test)]
mod tests {
    //! tests/unit/urls.test.ts, tests/unit/media-urls-owner.test.ts
    use std::collections::HashMap;

    use super::*;

    fn config() -> Config {
        Config::from_env_map(&HashMap::new()).unwrap()
    }

    #[test]
    fn url_helpers() {
        assert_eq!(storage_key_for("u", "c", "png"), "u/c.png");
        assert_eq!(storage_key_for("u", "c", ""), "u/c");
        assert_eq!(ext_of("photo.PNG"), "png");
        assert_eq!(ext_of("archive.tar.gz"), "gz");
        assert_eq!(ext_of("noext"), "");
        let base = &config().app_url;
        assert!(public_url("ToqOse", "abc12", "png", base).ends_with("/ToqOse/abc12.png"));
        assert!(raw_url("ToqOse", "abc12", "png", base).ends_with("/ToqOse/abc12/raw.png"));
        assert!(download_url("ToqOse", "abc12", base).ends_with("/ToqOse/abc12/download"));
        assert!(delete_url("ToqOse", "abc12", "tok", base).ends_with("/ToqOse/abc12/delete/tok"));
    }

    fn owner(code: Option<&str>, skip: Option<bool>, domain: Option<&str>) -> LinkOwner {
        LinkOwner {
            user_code: code.map(str::to_string),
            skip_tag_in_url: skip,
            preferred_domain: domain.map(str::to_string),
        }
    }

    #[test]
    fn media_urls_for_an_owner() {
        let c = config();
        let base = c.app_url.clone();
        assert_eq!(
            media_urls(&c, &owner(Some("ToqOse"), None, None), "abc12", "png"),
            MediaUrls { url: format!("{base}/ToqOse/abc12.png"), raw_url: format!("{base}/ToqOse/abc12/raw.png") }
        );
        assert_eq!(
            media_urls(&c, &owner(Some("ToqOse"), Some(true), None), "abc12", "png"),
            MediaUrls { url: format!("{base}/abc12.png"), raw_url: format!("{base}/abc12/raw.png") }
        );
        for skip in [None, Some(false)] {
            assert_eq!(
                media_urls(&c, &owner(Some("ToqOse"), skip, None), "abc12", "png").url,
                format!("{base}/ToqOse/abc12.png")
            );
        }
        assert_eq!(media_urls(&c, &owner(None, None, None), "abc12", "png").url, format!("{base}/u/abc12.png"));
        assert_eq!(
            media_urls(&c, &owner(Some("ToqOse"), None, None), "abc12", ""),
            MediaUrls { url: format!("{base}/ToqOse/abc12"), raw_url: format!("{base}/ToqOse/abc12/raw") }
        );
        let links = media_urls(&c, &owner(Some("ToqOse"), None, Some("https://evil.example")), "abc12", "png");
        assert_eq!(links.url, format!("{base}/ToqOse/abc12.png"));
    }

    #[test]
    fn preferred_domains_must_be_configured() {
        let mut vars = HashMap::new();
        vars.insert("APP_URL".to_string(), "https://host.example.com".to_string());
        vars.insert("APP_DOMAINS".to_string(), "https://h.example.com".to_string());
        let c = Config::from_env_map(&vars).unwrap();
        assert_eq!(user_base_url(&c, Some("HTTPS://H.example.com/ ")), "https://h.example.com");
        assert_eq!(user_base_url(&c, Some("https://evil.example")), "https://host.example.com");
        assert_eq!(user_base_url(&c, Some("")), "https://host.example.com");
        assert_eq!(user_base_url(&c, None), "https://host.example.com");
        assert!(is_known_domain(&c, "https://h.example.com/"));
        assert!(!is_known_domain(&c, "https://x.example.com"));
    }
}

#[cfg(test)]
mod srv_auth_tests {
    //! The srv-auth port's tests of its narrow version of this module, kept
    //! against the reconciled implementation.
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
        let urls = media_urls(&c, &owner, "abc", "png");
        assert_eq!(
            (urls.url, urls.raw_url),
            ("https://host.example.com/abc.png".to_string(), "https://host.example.com/abc/raw.png".to_string())
        );
    }
}
