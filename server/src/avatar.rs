//! Port of `lib/avatar.ts`: account avatars as stored objects.
//!
//! `user.image` holds a storage KEY this app wrote (`avatars/<id>-<token>.webp`),
//! never a URL; [`avatar_url`] turns it into the public `/avatar/{tag}` link.

use crate::http::cookies::encode_uri_component;

/// Storage prefix of every avatar object.
pub const AVATAR_PREFIX: &str = "avatars";
/// Avatars are always re-encoded to WebP; this is what /avatar/{tag} serves.
pub const AVATAR_CONTENT_TYPE: &str = "image/webp";
/// Largest source accepted for re-encoding (the upload route and the picker).
pub const AVATAR_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// Edge of the square output.
pub const AVATAR_SIZE: u32 = 256;
/// More frames than this and the avatar comes out still.
pub const AVATAR_MAX_FRAMES: u64 = 120;
/// A single frame over this is refused (decompression-bomb guard).
pub const AVATAR_MAX_FRAME_PIXELS: u64 = 64_000_000;
/// width x frame height x frames over this and the avatar comes out still.
pub const AVATAR_MAX_TOTAL_PIXELS: u64 = 40_000_000;
/// An animated result above this is re-encoded as a still.
pub const AVATAR_MAX_OUTPUT_BYTES: usize = 512 * 1024;

/// Storage key for a fresh avatar. The random token makes the public URL
/// immutable-cacheable and the key unguessable.
pub fn avatar_key(user_id: &str, token: &str) -> String {
    format!("{AVATAR_PREFIX}/{user_id}-{token}.webp")
}

/// True for a value this app wrote, as opposed to anything else in `user.image`.
pub fn is_avatar_key(value: Option<&str>) -> bool {
    value.is_some_and(|v| v.starts_with(&format!("{AVATAR_PREFIX}/")))
}

/// The cache-busting fragment of a key: `<prefix>/<id>-<token>.webp` -> token,
/// "" for anything that does not look like one of our keys.
pub fn key_token(key: Option<&str>) -> String {
    let Some(key) = key.filter(|k| !k.is_empty()) else {
        return String::new();
    };
    let last = key.rfind('/').map(|i| &key[i + 1..]).unwrap_or(key);
    let base = last.strip_suffix(".webp").unwrap_or(last);
    match base.rfind('-') {
        Some(dash) => base[dash + 1..].to_string(),
        None => String::new(),
    }
}

/// `avatarToken(key)`
pub fn avatar_token(key: Option<&str>) -> String {
    key_token(key)
}

/// `/^https?:\/\//i`
fn is_http_url(s: &str) -> bool {
    let lower: String = s.chars().take(8).collect::<String>().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// Public URL of an account's avatar, or `None` when it has none. Keyed by tag
/// (already public), root-relative (the app answers on several domains); an
/// absolute or root-relative `image` is passed through untouched.
pub fn avatar_url(user_code: Option<&str>, image: Option<&str>) -> Option<String> {
    let image = image.filter(|i| !i.is_empty())?;
    if is_http_url(image) || image.starts_with('/') {
        return Some(image.to_string());
    }
    let user_code = user_code.filter(|u| !u.is_empty())?;
    if !is_avatar_key(Some(image)) {
        return None;
    }
    let token = avatar_token(Some(image));
    let v = if token.is_empty() { String::new() } else { format!("?v={}", encode_uri_component(&token)) };
    Some(format!("/avatar/{}{v}", encode_uri_component(user_code)))
}

/// Image types accepted for re-encoding (SVG deliberately excluded).
pub const ACCEPTED_TYPES: &[&str] = &["image/png", "image/jpeg", "image/webp", "image/avif", "image/gif"];

/// `type.split(";")[0].trim().toLowerCase()` is one of [`ACCEPTED_TYPES`].
pub fn is_accepted_image_type(kind: Option<&str>) -> bool {
    let Some(kind) = kind.filter(|k| !k.is_empty()) else {
        return false;
    };
    let base = crate::config::js_trim(kind.split(';').next().unwrap_or("")).to_lowercase();
    ACCEPTED_TYPES.contains(&base.as_str())
}

pub fn is_accepted_avatar_type(kind: Option<&str>) -> bool {
    is_accepted_image_type(kind)
}

/// For the file picker's `accept` attribute.
pub fn avatar_accept() -> String {
    ACCEPTED_TYPES.join(",")
}

/// Why an avatar came out still, or was refused outright.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvatarPlanReason {
    Still,
    Frames,
    Pixels,
    FrameTooLarge,
    Animate,
}

/// `AvatarSource`: width, height of ONE frame, frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AvatarSource {
    pub width: f64,
    pub height: f64,
    pub pages: f64,
}

/// `AvatarPlan`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvatarPlan {
    pub ok: bool,
    pub animate: bool,
    pub frames: u64,
    pub reason: AvatarPlanReason,
}

/// `Math.max(0, Math.floor(v) || 0)`
fn whole(v: f64) -> f64 {
    let f = v.floor();
    if f.is_nan() || f == 0.0 { 0.0 } else { f.max(0.0) }
}

/// What to encode, decided from the source's shape alone: the per-frame
/// ceiling REFUSES, the total one DEGRADES to a still.
pub fn plan_avatar(src: AvatarSource) -> AvatarPlan {
    let width = whole(src.width);
    let height = whole(src.height);
    let pages = if src.pages.is_finite() && src.pages > 0.0 { src.pages.floor() } else { 1.0 };
    let frame_pixels = width * height;
    if !frame_pixels.is_finite() || frame_pixels > AVATAR_MAX_FRAME_PIXELS as f64 {
        return AvatarPlan { ok: false, animate: false, frames: 0, reason: AvatarPlanReason::FrameTooLarge };
    }
    if pages <= 1.0 {
        return AvatarPlan { ok: true, animate: false, frames: 1, reason: AvatarPlanReason::Still };
    }
    if pages > AVATAR_MAX_FRAMES as f64 {
        return AvatarPlan { ok: true, animate: false, frames: 1, reason: AvatarPlanReason::Frames };
    }
    if frame_pixels * pages > AVATAR_MAX_TOTAL_PIXELS as f64 {
        return AvatarPlan { ok: true, animate: false, frames: 1, reason: AvatarPlanReason::Pixels };
    }
    AvatarPlan { ok: true, animate: true, frames: pages as u64, reason: AvatarPlanReason::Animate }
}

#[cfg(test)]
mod tests {
    //! tests/unit/avatar.test.ts
    use super::*;

    #[test]
    fn keys_tokens_and_urls() {
        let key = avatar_key("user_123", "deadbeef");
        assert_eq!(key, "avatars/user_123-deadbeef.webp");
        assert_eq!(avatar_token(Some(&key)), "deadbeef");
        assert_eq!(avatar_token(Some(&avatar_key("a-b-c", "cafe"))), "cafe");
        assert_eq!(avatar_token(None), "");
        assert_eq!(avatar_token(Some("nodash")), "");
        assert!(is_avatar_key(Some(&avatar_key("u1", "t"))));
        assert!(!is_avatar_key(Some("backgrounds/u1-t.webp")));
        assert!(!is_avatar_key(Some("https://cdn.example.com/face.png")));
        assert!(!is_avatar_key(None));
        assert!(!is_avatar_key(Some("")));
        assert_eq!(
            avatar_url(Some("mytag"), Some(&avatar_key("u1", "abc123"))).as_deref(),
            Some("/avatar/mytag?v=abc123")
        );
        assert_eq!(avatar_url(Some("a\"b"), Some(&avatar_key("u1", "t"))).as_deref(), Some("/avatar/a%22b?v=t"));
        assert_eq!(
            avatar_url(Some("tag"), Some("https://cdn.example.com/face.png")).as_deref(),
            Some("https://cdn.example.com/face.png")
        );
        assert_eq!(avatar_url(Some("tag"), Some("/static/face.png")).as_deref(), Some("/static/face.png"));
        assert_eq!(avatar_url(Some("tag"), None), None);
        assert_eq!(avatar_url(Some("tag"), Some("")), None);
        assert_eq!(avatar_url(None, Some(&avatar_key("u1", "t"))), None);
        assert_eq!(avatar_url(Some("tag"), Some("some/other/object.bin")), None);
        assert_eq!(avatar_url(Some("tag"), Some("avatars/legacy.webp")).as_deref(), Some("/avatar/tag"));
    }

    #[test]
    fn accepted_types() {
        for t in ACCEPTED_TYPES {
            assert!(is_accepted_avatar_type(Some(t)));
        }
        assert!(is_accepted_avatar_type(Some("IMAGE/GIF")));
        assert!(is_accepted_avatar_type(Some("image/jpeg; charset=binary")));
        for t in [Some("image/svg+xml"), Some("text/html"), Some("application/octet-stream"), None] {
            assert!(!is_accepted_avatar_type(t));
        }
    }

    fn plan(w: f64, h: f64, p: f64) -> AvatarPlan {
        plan_avatar(AvatarSource { width: w, height: h, pages: p })
    }

    #[test]
    fn plans() {
        assert_eq!(
            plan(480.0, 270.0, 30.0),
            AvatarPlan { ok: true, animate: true, frames: 30, reason: AvatarPlanReason::Animate }
        );
        assert_eq!(
            plan(1920.0, 1080.0, 1.0),
            AvatarPlan { ok: true, animate: false, frames: 1, reason: AvatarPlanReason::Still }
        );
        // The 1654x762x256 GIF that a per-frame guard let through.
        let p = plan(1654.0, 762.0, 256.0);
        assert!(p.ok && !p.animate && p.frames == 1);
        assert_eq!(plan(2000.0, 1500.0, 20.0).reason, AvatarPlanReason::Pixels);
        assert_eq!(plan(64.0, 64.0, (AVATAR_MAX_FRAMES + 1) as f64).reason, AvatarPlanReason::Frames);
        assert!(plan(64.0, 64.0, AVATAR_MAX_FRAMES as f64).animate);
        assert!(!plan(64.0, 64.0, (AVATAR_MAX_FRAMES + 1) as f64).animate);
        let big = plan(100_000.0, 100_000.0, 1.0);
        assert!(!big.ok);
        assert_eq!(big.reason, AvatarPlanReason::FrameTooLarge);
        assert_eq!(plan(100.0, 100.0, 0.0).frames, 1);
        assert!(!plan(100.0, 100.0, f64::NAN).animate);
        assert!(plan(f64::NAN, 100.0, 4.0).ok);
    }
}

#[cfg(test)]
mod srv_auth_tests {
    //! The srv-auth port's tests of its narrow version of this module, kept
    //! against the reconciled implementation.
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
