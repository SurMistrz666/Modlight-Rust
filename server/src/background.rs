//! Port of `lib/background.ts`: per-account wallpaper, the image a user puts
//! behind their panel and behind every public page they own.
//!
//! Framework-agnostic helpers only (no db), so the serving route, the upload
//! route, the profile form and the theme applier all agree on the same rules.

use crate::config::{js_number, js_round};
use crate::http::cookies::encode_uri_component;

/// Where a wallpaper lives in storage. The token is what makes the URL cacheable.
pub const BACKGROUND_PREFIX: &str = "backgrounds";

/// Re-encoded output, always.
pub const BACKGROUND_CONTENT_TYPE: &str = "image/webp";

/// Largest upload we will even read, before re-encoding.
pub const BACKGROUND_MAX_BYTES: usize = 12 * 1024 * 1024;

/// Longest edge kept after re-encoding.
pub const BACKGROUND_MAX_EDGE: u32 = 2560;

/// Veil strength bounds, in percent of the skin's own background colour.
pub const DIM_MIN: f64 = 0.0;
pub const DIM_MAX: f64 = 95.0;

/// Weakest veil at which body text still clears WCAG AA over the worst image
/// (not enforced; the slider warns below it).
pub const DIM_SAFE: i32 = 68;

/// The value a wallpaper gets without anyone touching the slider: legible over
/// any photo (5.31:1 worst case).
pub const DIM_DEFAULT: i32 = 70;

/// A value as `clampDim` may receive it (the action passes whatever the
/// client sent).
#[derive(Debug, Clone, PartialEq)]
pub enum DimInput<'a> {
    /// `null` or `undefined`
    Absent,
    Number(f64),
    Text(&'a str),
    /// Anything else (`Number(true)` is 1, `Number({})` NaN, ...): the already
    /// converted number.
    Other(f64),
}

/// `clampDim(value)`. null, undefined and "" all coerce to 0 through
/// `Number()`, and 0 is a meaningful setting here (no veil at all), so they
/// are rejected before any coercion happens.
pub fn clamp_dim(value: DimInput<'_>) -> i32 {
    let n = match value {
        DimInput::Absent => return DIM_DEFAULT,
        DimInput::Text("") => return DIM_DEFAULT,
        DimInput::Text(t) => js_number(t),
        DimInput::Number(n) | DimInput::Other(n) => n,
    };
    if !n.is_finite() {
        return DIM_DEFAULT;
    }
    js_round(n).clamp(DIM_MIN, DIM_MAX) as i32
}

/// `clampDim(n)` for a stored integer (`background_dim` column).
pub fn clamp_dim_i(value: Option<i32>) -> i32 {
    match value {
        None => DIM_DEFAULT,
        Some(v) => clamp_dim(DimInput::Number(f64::from(v))),
    }
}

/// Storage key for a fresh wallpaper: `backgrounds/<userId>-<token>.webp`.
pub fn background_key(user_id: &str, token: &str) -> String {
    format!("{BACKGROUND_PREFIX}/{user_id}-{token}.webp")
}

/// The cache-busting fragment of a key: `backgrounds/<id>-<token>.webp` ->
/// token; "" for anything that does not look like one of our keys.
pub fn background_token(key: Option<&str>) -> String {
    let Some(key) = key.filter(|k| !k.is_empty()) else {
        return String::new();
    };
    let base = match key.rfind('/') {
        Some(i) => &key[i + 1..],
        None => key,
    };
    let base = base.strip_suffix(".webp").unwrap_or(base);
    match base.rfind('-') {
        Some(dash) => base[dash + 1..].to_string(),
        None => String::new(),
    }
}

/// Public URL of an account's wallpaper (`/bg/<tag>[?v=<token>]`), or `None`
/// when it has none. Keyed by tag (already public), root-relative on purpose.
pub fn background_url(user_code: Option<&str>, key: Option<&str>) -> Option<String> {
    let user_code = user_code.filter(|c| !c.is_empty())?;
    let key = key.filter(|k| !k.is_empty())?;
    let token = background_token(Some(key));
    let mut url = format!("/bg/{}", encode_uri_component(user_code));
    if !token.is_empty() {
        url.push_str(&format!("?v={}", encode_uri_component(&token)));
    }
    Some(url)
}

/// Everything the theme applier needs to paint one.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct BackgroundStyle {
    pub url: String,
    /// 0-1, ready for CSS. Serialised as JavaScript writes the number (`0`,
    /// not serde's `0.0`, for a veil of 0).
    #[serde(serialize_with = "serialize_js_number")]
    pub dim: f64,
}

fn serialize_js_number<S: serde::Serializer>(n: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    serde::Serialize::serialize(&crate::rpc::tagged::Num(*n), serializer)
}

/// `backgroundStyle(userCode, key, dim)`.
pub fn background_style(user_code: Option<&str>, key: Option<&str>, dim: Option<i32>) -> Option<BackgroundStyle> {
    let url = background_url(user_code, key)?;
    Some(BackgroundStyle { url, dim: f64::from(clamp_dim_i(dim)) / 100.0 })
}

const ACCEPTED: &[&str] = &["image/png", "image/jpeg", "image/webp", "image/avif", "image/gif"];

/// Image types we accept for re-encoding (SVG excluded deliberately).
pub fn is_accepted_background_type(t: Option<&str>) -> bool {
    t.filter(|t| !t.is_empty()).is_some_and(|t| {
        let base = t.split(';').next().unwrap_or("");
        ACCEPTED.contains(&crate::config::js_trim(base).to_lowercase().as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // tests/unit/background.test.ts (the pure helpers)
    #[test]
    fn clamp_dim_rejects_absent_values_before_coercion() {
        assert_eq!(clamp_dim(DimInput::Absent), 70);
        assert_eq!(clamp_dim(DimInput::Text("")), 70);
        assert_eq!(clamp_dim(DimInput::Number(0.0)), 0);
        assert_eq!(clamp_dim(DimInput::Number(-5.0)), 0);
        assert_eq!(clamp_dim(DimInput::Number(120.0)), 95);
        assert_eq!(clamp_dim(DimInput::Number(42.5)), 43);
        assert_eq!(clamp_dim(DimInput::Text("33")), 33);
        assert_eq!(clamp_dim(DimInput::Text("abc")), 70);
        assert_eq!(clamp_dim(DimInput::Number(f64::NAN)), 70);
        assert_eq!(clamp_dim(DimInput::Number(f64::INFINITY)), 70);
    }

    #[test]
    fn urls_and_tokens() {
        assert_eq!(background_token(Some("backgrounds/u1-abc123.webp")), "abc123");
        assert_eq!(background_token(Some("backgrounds/u1.webp")), "");
        assert_eq!(background_token(None), "");
        assert_eq!(background_url(Some("bob"), Some("backgrounds/u1-tok.webp")).as_deref(), Some("/bg/bob?v=tok"));
        assert_eq!(background_url(Some("a b"), Some("backgrounds/x.webp")).as_deref(), Some("/bg/a%20b"));
        assert_eq!(background_url(None, Some("k")), None);
        assert_eq!(background_url(Some("bob"), None), None);
        assert_eq!(
            background_style(Some("bob"), Some("backgrounds/u-t.webp"), None),
            Some(BackgroundStyle { url: "/bg/bob?v=t".into(), dim: 0.7 })
        );
        assert_eq!(
            serde_json::to_string(&background_style(Some("bob"), Some("backgrounds/u-t.webp"), Some(0))).unwrap(),
            r#"{"url":"/bg/bob?v=t","dim":0}"#
        );
        assert_eq!(
            serde_json::to_string(&background_style(Some("bob"), Some("backgrounds/u-t.webp"), Some(7))).unwrap(),
            r#"{"url":"/bg/bob?v=t","dim":0.07}"#
        );
        assert!(is_accepted_background_type(Some("image/PNG; q=1")));
        assert!(!is_accepted_background_type(Some("image/svg+xml")));
    }
}
