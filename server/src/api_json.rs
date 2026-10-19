//! Port of `lib/api-json.ts`: reading a JSON request body on the desktop
//! client's HTTP surface, plus the small JSON response helpers every upstream
//! route file defines for itself (`function json(body, status)`).
//!
//! Three things every write endpoint has to get right, and which are easy to
//! get wrong once per endpoint:
//!
//!  1. **Content-Type is a CSRF control, not decoration.** These routes also
//!     accept a plain session cookie, so a POST from any page the victim visits
//!     arrives authenticated. What stops it is the browser: `application/json`
//!     is not one of the three CORS-safelisted request content types, so a
//!     cross-origin POST carrying it is preflighted and nothing here answers a
//!     preflight.
//!  2. **A ceiling before the read.** Buffering whatever arrives into memory
//!     before there is anything to validate is the attack.
//!  3. **An object, not an array or a bare scalar.**

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::config::js_number;
use crate::error::AppError;

/// Default ceiling. Generous for a body whose fields are an id, a name bounded
/// at 120 characters and a couple of booleans; small enough that a flood costs
/// the sender more than us.
pub const JSON_BODY_MAX: usize = 16 * 1024;

/// A refusal from [`read_json_object`]: the status and the exact upstream text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonBodyError {
    pub status: StatusCode,
    pub message: String,
}

impl JsonBodyError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        JsonBodyError { status, message: message.into() }
    }
}

impl From<JsonBodyError> for AppError {
    fn from(e: JsonBodyError) -> Self {
        AppError::message(e.status, e.message)
    }
}

/// `/^application\/json\s*(;|$)/i`
pub fn is_json_content_type(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("application/json") else {
        return false;
    };
    let rest = rest.trim_start_matches(|c: char| c.is_whitespace());
    rest.is_empty() || rest.starts_with(';')
}

/// Read and validate a JSON object body. Same order of checks, statuses and
/// messages as upstream `readJsonObject`.
///
/// Upstream counts the size in UTF-16 units of the decoded text (`raw.length`).
/// A string longer than N units is always more than N bytes of UTF-8, and one
/// unit is at most 3 bytes, so reading stops after `3 * max_bytes` bytes: past
/// that point the decoded text is certainly over the limit, which keeps memory
/// bounded without refusing anything upstream would accept.
///
/// Where `JSON.parse` and serde_json still part (known deviations): a number
/// beyond the f64 range (`1e400`, which JSON.parse reads as Infinity) and
/// nesting deeper than serde_json's 128 levels are refused here as "Body must
/// be a JSON object." (400); upstream parsed them and judged the fields.
pub async fn read_json_object(
    headers: &HeaderMap,
    body: Body,
    max_bytes: usize,
) -> Result<Map<String, Value>, JsonBodyError> {
    let content_type = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");
    if !is_json_content_type(content_type) {
        return Err(JsonBodyError::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Content-Type must be application/json."));
    }

    // Declared length first: refusing by reading a header is free.
    let too_large =
        || JsonBodyError::new(StatusCode::PAYLOAD_TOO_LARGE, format!("Body must be at most {max_bytes} bytes."));
    let declared =
        headers.get(header::CONTENT_LENGTH).map(|v| js_number(&String::from_utf8_lossy(v.as_bytes()))).unwrap_or(0.0);
    if declared.is_finite() && declared > max_bytes as f64 {
        return Err(too_large());
    }

    let hard_cap = max_bytes.saturating_mul(3).saturating_add(1);
    let mut buf: Vec<u8> = Vec::new();
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| JsonBodyError::new(StatusCode::BAD_REQUEST, "Body could not be read."))?;
        buf.extend_from_slice(&chunk);
        if buf.len() > hard_cap {
            return Err(too_large());
        }
    }

    // `req.text()`: UTF-8 decode with replacement, leading BOM stripped.
    let decoded = String::from_utf8_lossy(&buf);
    let raw = decoded.strip_prefix('\u{FEFF}').unwrap_or(&decoded);
    if raw.encode_utf16().count() > max_bytes {
        return Err(too_large());
    }

    match serde_json::from_str::<Value>(&lone_surrogates_replaced(raw)) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(JsonBodyError::new(StatusCode::BAD_REQUEST, "Body must be a JSON object.")),
    }
}

/// `JSON.parse` accepts a `\uD800`-style escape that is not half of a pair
/// (serde_json refuses the whole document). The lone surrogate then becomes
/// U+FFFD wherever the string is written as UTF-8 (Postgres, a response), so
/// that is what it is rewritten to here, before parsing. Everything else is
/// left byte for byte; a body without such an escape is not copied.
pub fn lone_surrogates_replaced(raw: &str) -> std::borrow::Cow<'_, str> {
    fn surrogate_at(b: &[u8], i: usize) -> Option<u16> {
        // `\u` followed by four hex digits naming a surrogate.
        let hex = b.get(i + 2..i + 6)?;
        if b.get(i) != Some(&b'\\') || b.get(i + 1) != Some(&b'u') {
            return None;
        }
        let v = u16::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
        (0xD800..=0xDFFF).contains(&v).then_some(v)
    }
    let b = raw.as_bytes();
    if !raw.contains("\\u") {
        return std::borrow::Cow::Borrowed(raw);
    }
    let mut out = String::with_capacity(raw.len());
    let mut in_string = false;
    let mut last = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => in_string = !in_string,
            b'\\' if in_string => {
                match surrogate_at(b, i) {
                    Some(hi) if hi <= 0xDBFF && surrogate_at(b, i + 6).is_some_and(|lo| lo >= 0xDC00) => {
                        i += 12; // a well-formed pair stays as it is
                        continue;
                    }
                    Some(_) => {
                        out.push_str(&raw[last..i]);
                        out.push_str("\\uFFFD");
                        i += 6;
                        last = i;
                        continue;
                    }
                    None => i += 1, // skip the escaped character
                }
            }
            _ => {}
        }
        i += 1;
    }
    if last == 0 {
        return std::borrow::Cow::Borrowed(raw);
    }
    out.push_str(&raw[last..]);
    std::borrow::Cow::Owned(out)
}

/// Was a key SUPPLIED, as opposed to absent?
///
/// JSON `null` is a value here, not a gap: `{"parentId": null}` means "move to
/// the root" and `{}` means "leave the parent alone".
pub fn supplied(body: &Map<String, Value>, key: &str) -> bool {
    body.contains_key(key)
}

// ── Response helpers ────────────────────────────────────────────────────────
// Upstream builds these with `new Response(JSON.stringify(body), ...)`. The
// content type is exactly `application/json` (no charset), as there.

/// `Cache-Control` most per-account API answers carry.
pub const PRIVATE_NO_STORE: &str = "private, no-store";

/// JSON with a status and, optionally, a `Cache-Control` value.
pub fn json_response<T: Serialize + ?Sized>(status: StatusCode, body: &T, cache_control: Option<&str>) -> Response {
    let bytes = match serde_json::to_vec(body) {
        Ok(b) => b,
        Err(err) => {
            tracing::error!(error = %err, "could not serialize a JSON response");
            return crate::error::internal_error_response();
        }
    };
    let mut response = (status, bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(cache) = cache_control
        && let Ok(value) = HeaderValue::from_str(cache)
    {
        headers.insert(header::CACHE_CONTROL, value);
    }
    response
}

/// JSON that no shared cache may keep (`private, no-store`).
pub fn json_private<T: Serialize + ?Sized>(status: StatusCode, body: &T) -> Response {
    json_response(status, body, Some(PRIVATE_NO_STORE))
}

/// `{"message": ...}` with a status: the common refusal body.
pub fn json_message(status: StatusCode, message: &str, cache_control: Option<&str>) -> Response {
    json_response(status, &serde_json::json!({ "message": message }), cache_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
        }
        h
    }

    async fn read(pairs: &[(&str, &str)], body: &str, max: usize) -> Result<Map<String, Value>, JsonBodyError> {
        read_json_object(&headers(pairs), Body::from(body.to_string()), max).await
    }

    #[tokio::test]
    async fn requires_the_json_content_type() {
        let e = read(&[("content-type", "text/plain")], "{}", JSON_BODY_MAX).await.unwrap_err();
        assert_eq!(e.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert_eq!(e.message, "Content-Type must be application/json.");
        let e = read(&[], "{}", JSON_BODY_MAX).await.unwrap_err();
        assert_eq!(e.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert!(read(&[("content-type", "Application/JSON; charset=utf-8")], "{}", JSON_BODY_MAX).await.is_ok());
        assert!(read(&[("content-type", "application/json ;x")], "{}", JSON_BODY_MAX).await.is_ok());
        assert!(read(&[("content-type", "application/jsonx")], "{}", JSON_BODY_MAX).await.is_err());
    }

    #[tokio::test]
    async fn refuses_a_declared_length_over_the_ceiling() {
        let e = read(&[("content-type", "application/json"), ("content-length", "20")], "{}", 10).await.unwrap_err();
        assert_eq!(e.status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(e.message, "Body must be at most 10 bytes.");
    }

    #[tokio::test]
    async fn counts_utf16_units_like_upstream() {
        // 8 two-byte characters: the body is 24 bytes of UTF-8 but 16 UTF-16
        // units, so a 16 "byte" ceiling accepts it exactly as upstream does.
        let body = "{\"a\":\"ąąąąąąąą\"}";
        assert_eq!(body.encode_utf16().count(), 16);
        assert!(read(&[("content-type", "application/json")], body, 16).await.is_ok());
        let e = read(&[("content-type", "application/json")], body, 15).await.unwrap_err();
        assert_eq!(e.status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn lone_surrogates_become_replacement_characters() {
        assert_eq!(lone_surrogates_replaced(r#"{"a":"x"}"#), r#"{"a":"x"}"#);
        assert_eq!(lone_surrogates_replaced(r#"{"a":"\ud800"}"#), r#"{"a":"\uFFFD"}"#);
        assert_eq!(lone_surrogates_replaced(r#"{"a":"\udc00x\ud83d"}"#), r#"{"a":"\uFFFDx\uFFFD"}"#);
        // A pair, an escaped backslash before a `u`, and a surrogate outside a
        // string (invalid JSON either way) are left alone.
        assert_eq!(lone_surrogates_replaced(r#"{"a":"\ud83d\ude00"}"#), r#"{"a":"\ud83d\ude00"}"#);
        assert_eq!(lone_surrogates_replaced(r#"{"a":"\\ud800"}"#), r#"{"a":"\\ud800"}"#);
        let parsed: Value = serde_json::from_str(&lone_surrogates_replaced(r#"{"a":"ab\ud800cd"}"#)).unwrap();
        assert_eq!(parsed["a"], "ab\u{FFFD}cd");
    }

    #[tokio::test]
    async fn only_objects_are_accepted() {
        for bad in ["[]", "null", "1", "\"x\"", "{", ""] {
            let e = read(&[("content-type", "application/json")], bad, JSON_BODY_MAX).await.unwrap_err();
            assert_eq!(e.status, StatusCode::BAD_REQUEST);
            assert_eq!(e.message, "Body must be a JSON object.");
        }
        let ok =
            read(&[("content-type", "application/json")], "\u{FEFF}{\"parentId\":null}", JSON_BODY_MAX).await.unwrap();
        assert!(supplied(&ok, "parentId"));
        assert!(!supplied(&ok, "name"));
    }
}
