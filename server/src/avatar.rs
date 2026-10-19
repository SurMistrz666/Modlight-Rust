//! Port of `lib/avatar.ts` (the URL and key helpers): per-account avatar, the
//! little round picture that replaces the initials.
//!
//! An avatar is ALWAYS a small square this app re-encoded itself; the URL
//! leaks the tag and a random token and nothing else (no id, no address, no
//! file name). The encoder plan (`planAvatar`) belongs to the upload port.

use crate::http::cookies::encode_uri_component;

/// Where a derived avatar lives in storage. The token is what makes it cacheable.
pub const AVATAR_PREFIX: &str = "avatars";

/// Re-encoded output, always. Animated avatars are animated WebP, same type.
pub const AVATAR_CONTENT_TYPE: &str = "image/webp";

/// Storage key for a fresh avatar: `avatars/<userId>-<token>.webp`.
pub fn avatar_key(user_id: &str, token: &str) -> String {
    format!("{AVATAR_PREFIX}/{user_id}-{token}.webp")
}

/// True for a value this app wrote, as opposed to anything else in `user.image`.
pub fn is_avatar_key(value: Option<&str>) -> bool {
    value.is_some_and(|v| v.starts_with(&format!("{AVATAR_PREFIX}/")))
}

/// The cache-busting fragment of a key: `avatars/<id>-<token>.webp` -> token;
/// "" for anything that does not look like one of our keys.
pub fn avatar_token(key: Option<&str>) -> String {
    crate::background::background_token(key)
}

/// Public URL of an account's avatar, or `None` when it has none.
///
/// `image` is Better Auth's own column. Nothing writes an http(s) URL into it
/// in this deployment (there are no social providers), but a future one would,
/// so an absolute value is passed through untouched rather than mangled.
pub fn avatar_url(user_code: Option<&str>, image: Option<&str>) -> Option<String> {
    let image = image.filter(|i| !i.is_empty())?;
    let b = image.as_bytes();
    let is_http = (b.len() >= 7 && b[..7].eq_ignore_ascii_case(b"http://"))
        || (b.len() >= 8 && b[..8].eq_ignore_ascii_case(b"https://"));
    if is_http || image.starts_with('/') {
        return Some(image.to_string());
    }
    let user_code = user_code.filter(|c| !c.is_empty())?;
    if !is_avatar_key(Some(image)) {
        return None;
    }
    let token = avatar_token(Some(image));
    let mut url = format!("/avatar/{}", encode_uri_component(user_code));
    if !token.is_empty() {
        url.push_str(&format!("?v={}", encode_uri_component(&token)));
    }
    Some(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    // tests/unit/avatar.test.ts (avatarUrl)
    #[test]
    fn avatar_urls() {
        assert_eq!(avatar_url(Some("bob"), None), None);
        assert_eq!(avatar_url(Some("bob"), Some("")), None);
        assert_eq!(avatar_url(Some("bob"), Some("avatars/u1-tok.webp")).as_deref(), Some("/avatar/bob?v=tok"));
        assert_eq!(avatar_url(Some("bob"), Some("avatars/u1.webp")).as_deref(), Some("/avatar/bob"));
        assert_eq!(avatar_url(None, Some("avatars/u1-tok.webp")), None);
        assert_eq!(avatar_url(Some("bob"), Some("uploads/x.png")), None);
        assert_eq!(avatar_url(None, Some("HTTPS://cdn/x.png")).as_deref(), Some("HTTPS://cdn/x.png"));
        assert_eq!(avatar_url(None, Some("/static/a.png")).as_deref(), Some("/static/a.png"));
    }
}
