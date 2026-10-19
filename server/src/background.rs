//! Port of `lib/background.ts`: account wallpapers (`user.background_key`,
//! `user.background_dim`) and how the panel paints them.

use serde::Serialize;
use serde_json::Value;

use crate::http::cookies::encode_uri_component;

pub const BACKGROUND_PREFIX: &str = "backgrounds";
pub const BACKGROUND_CONTENT_TYPE: &str = "image/webp";
/// Largest source accepted for re-encoding.
pub const BACKGROUND_MAX_BYTES: u64 = 12 * 1024 * 1024;
/// Longest edge of the stored wallpaper.
pub const BACKGROUND_MAX_EDGE: u32 = 2560;
pub const DIM_MIN: i64 = 0;
pub const DIM_MAX: i64 = 95;
/// The lowest dim that keeps body text legible over any photo (measured).
pub const DIM_SAFE: i64 = 68;
/// Chosen for the same measurement with margin rather than for looks.
pub const DIM_DEFAULT: i64 = 70;

/// `clampDim(value)` for a number (`None` = null/undefined).
pub fn clamp_dim(value: Option<f64>) -> i64 {
    match value {
        None => DIM_DEFAULT,
        Some(n) if !n.is_finite() => DIM_DEFAULT,
        Some(n) => (crate::naming::js_round(n) as i64).clamp(DIM_MIN, DIM_MAX),
    }
}

/// `clampDim(value: unknown)`: null, undefined and "" are the default (they
/// would coerce to 0, which is a meaningful setting); anything else goes
/// through `Number()`.
pub fn clamp_dim_value(value: Option<&Value>) -> i64 {
    match value {
        None | Some(Value::Null) => DIM_DEFAULT,
        Some(Value::String(s)) if s.is_empty() => DIM_DEFAULT,
        Some(Value::String(s)) => clamp_dim(Some(crate::config::js_number(s))),
        Some(Value::Number(n)) => clamp_dim(n.as_f64()),
        Some(Value::Bool(b)) => clamp_dim(Some(if *b { 1.0 } else { 0.0 })),
        // Number([]) is 0, Number([5]) is 5, Number({}) is NaN.
        Some(Value::Array(a)) if a.is_empty() => clamp_dim(Some(0.0)),
        Some(Value::Array(a)) if a.len() == 1 => match &a[0] {
            Value::Number(n) => clamp_dim(n.as_f64()),
            Value::String(s) => clamp_dim(Some(crate::config::js_number(s))),
            Value::Null => clamp_dim(Some(0.0)),
            _ => DIM_DEFAULT,
        },
        Some(_) => DIM_DEFAULT,
    }
}

/// Storage key for a fresh wallpaper.
pub fn background_key(user_id: &str, token: &str) -> String {
    format!("{BACKGROUND_PREFIX}/{user_id}-{token}.webp")
}

/// The cache-busting fragment of a key.
pub fn background_token(key: Option<&str>) -> String {
    crate::avatar::key_token(key)
}

/// Public URL of an account's wallpaper, or `None` when it has none.
pub fn background_url(user_code: Option<&str>, key: Option<&str>) -> Option<String> {
    let user_code = user_code.filter(|u| !u.is_empty())?;
    let key = key.filter(|k| !k.is_empty())?;
    let token = background_token(Some(key));
    let v = if token.is_empty() { String::new() } else { format!("?v={}", encode_uri_component(&token)) };
    Some(format!("/bg/{}{v}", encode_uri_component(user_code)))
}

/// Everything the theme applier needs to paint one.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BackgroundStyle {
    pub url: String,
    /// 0-1, ready for CSS.
    pub dim: f64,
}

pub fn background_style(user_code: Option<&str>, key: Option<&str>, dim: Option<f64>) -> Option<BackgroundStyle> {
    let url = background_url(user_code, key)?;
    Some(BackgroundStyle { url, dim: clamp_dim(Some(dim.unwrap_or(DIM_DEFAULT as f64))) as f64 / 100.0 })
}

pub fn is_accepted_background_type(kind: Option<&str>) -> bool {
    crate::avatar::is_accepted_image_type(kind)
}

/// For the file picker's `accept` attribute.
pub fn background_accept() -> String {
    crate::avatar::ACCEPTED_TYPES.join(",")
}

#[cfg(test)]
mod tests {
    //! tests/unit/background.test.ts
    use serde_json::json;

    use super::*;

    #[test]
    fn clamp_dim_rules() {
        assert_eq!(clamp_dim(Some(0.0)), 0);
        assert_eq!(clamp_dim(Some(55.0)), 55);
        assert_eq!(clamp_dim(Some(95.0)), 95);
        assert_eq!(clamp_dim(Some(-40.0)), DIM_MIN);
        assert_eq!(clamp_dim(Some(1000.0)), DIM_MAX);
        assert_eq!(clamp_dim(None), DIM_DEFAULT);
        assert_eq!(clamp_dim_value(Some(&json!("abc"))), DIM_DEFAULT);
        assert_eq!(clamp_dim_value(Some(&json!(""))), DIM_DEFAULT);
        assert_eq!(clamp_dim_value(Some(&json!(null))), DIM_DEFAULT);
        assert_eq!(clamp_dim_value(Some(&json!("40"))), 40);
        assert_eq!(clamp_dim(Some(f64::NAN)), DIM_DEFAULT);
        assert_eq!(clamp_dim(Some(f64::INFINITY)), DIM_DEFAULT);
        assert_eq!(clamp_dim(Some(55.4)), 55);
        assert_eq!(clamp_dim(Some(55.6)), 56);
    }

    #[test]
    fn keys_urls_and_styles() {
        let key = background_key("user_123", "deadbeef");
        assert_eq!(key, "backgrounds/user_123-deadbeef.webp");
        assert_eq!(background_token(Some(&key)), "deadbeef");
        assert_eq!(background_token(Some(&background_key("a-b-c", "cafe"))), "cafe");
        assert_eq!(background_token(None), "");
        assert_eq!(background_token(Some("nodash")), "");
        assert_eq!(background_url(None, Some("backgrounds/u-1.webp")), None);
        assert_eq!(background_url(Some("tag"), None), None);
        assert_eq!(
            background_url(Some("mytag"), Some(&background_key("u", "abc123"))).as_deref(),
            Some("/bg/mytag?v=abc123")
        );
        assert_eq!(background_url(Some("a\"b"), Some(&background_key("u", "t"))).as_deref(), Some("/bg/a%22b?v=t"));
        assert_eq!(background_url(Some("tag"), Some("legacy.webp")).as_deref(), Some("/bg/tag"));
        assert_eq!(
            background_style(Some("tag"), Some(&background_key("u", "t")), Some(60.0)),
            Some(BackgroundStyle { url: "/bg/tag?v=t".into(), dim: 0.6 })
        );
        assert_eq!(background_style(Some("tag"), None, Some(60.0)), None);
        assert_eq!(background_style(Some("tag"), Some(&background_key("u", "t")), None).unwrap().dim, 0.7);
        for t in crate::avatar::ACCEPTED_TYPES {
            assert!(is_accepted_background_type(Some(t)));
        }
        assert!(is_accepted_background_type(Some("IMAGE/PNG")));
        assert!(is_accepted_background_type(Some("image/jpeg; charset=binary")));
        for t in [Some("image/svg+xml"), Some("text/html"), Some("application/octet-stream"), None, Some("")] {
            assert!(!is_accepted_background_type(t));
        }
    }
}
