//! Port of `lib/client-settings.ts`: the desktop client's own configuration,
//! kept on the server.
//!
//! The owner's requirement (docs/decyzje-modlight-app.md, "Ustawienia w
//! chmurze"): the client's configuration - hotkeys above all - lives with the
//! ACCOUNT, so signing in on a new machine restores it. The local `.mlcfg`
//! file survives only as an import path from ShareX.
//!
//! The server deliberately knows NOTHING about what is inside. Hotkeys,
//! capture regions, ffmpeg presets and whatever the client grows next are the
//! client's business, and a server that validated their shape would have to
//! be redeployed every time the client added a checkbox. What the server DOES
//! own is the two things a store cannot delegate: how big the blob may get,
//! and which write won when two machines saved at once.
//!
//! Pure: no database, no framework. The JSON is handled with JavaScript's
//! semantics ([`crate::js_json`]) because the canonical form is what the
//! column stores and what the size cap and the NUL check measure, and a
//! client must get the same byte count and the same refusals from either
//! server.

use axum::http::StatusCode;

use crate::js_json::{self, JsObject, JsValue, RangeError, STRINGIFY_STACK};
use crate::quota::is_safe_integer;

/// The only client that exists today.
pub const DESKTOP_CLIENT: &str = "desktop";

/// Which client kinds may own a row, as an ALLOWLIST rather than a free-form
/// string.
///
/// This is a size guard, not tidiness. The table's footprint is bounded by
/// `accounts x client kinds x the byte cap below`, and that middle term only
/// stays small because the server picks it. A discriminator taken straight
/// from the query string turns one row per account into as many rows as a
/// buggy (or malicious) client cares to invent, and the cap on each one stops
/// meaning anything. Adding a second client is a code change here, which is
/// correct: the server has to know a client exists before it will store its
/// settings.
pub const CLIENT_KINDS: &[&str] = &[DESKTOP_CLIENT];

/// Largest settings document we will store, measured on the canonical JSON.
///
/// Sized against the thing being replaced: ShareX keeps its configuration in
/// three files that together come to well under 100 kB. 256 KiB leaves
/// several times that much headroom while keeping the whole table's worst
/// case at `accounts x 256 KiB`. That bound is the point: this is the one
/// jsonb column an unsupervised background process writes to on a timer.
pub const CLIENT_SETTINGS_MAX_BYTES: usize = 256 * 1024;

/// Largest request body we will read, as opposed to largest document we will
/// store.
///
/// Deliberately looser than the cap above, because the two measure different
/// things. What lands in the column is `JSON.stringify(settings)`; what
/// arrives on the wire is whatever the client's serialiser produced, which
/// may be indented, may escape non-ASCII as `\uXXXX`, and carries the
/// envelope. Twice the storage cap absorbs all of that and still refuses
/// anything that could not possibly fit once canonicalised.
pub const CLIENT_SETTINGS_BODY_MAX_BYTES: usize = CLIENT_SETTINGS_MAX_BYTES * 2;

/// Highest version a client may claim to hold.
///
/// `client_settings.version` is an `int4`, and an expected version travels
/// to Postgres as a parameter typed by that column - so a value outside the
/// range is not a comparison that matches nothing, it is "value out of range
/// for type integer" raised by the driver, i.e. a 500 for a body that
/// deserved a 400. One below the int4 ceiling, because a successful write
/// stores `expected + 1` and that has to fit too.
pub const MAX_EXPECTED_VERSION: i64 = 2_147_483_646;

/// `262144 bytes (256 KiB)`, so every refusal can quote the same limit.
fn limit_label() -> String {
    format!(
        "{CLIENT_SETTINGS_MAX_BYTES} bytes ({} KiB)",
        crate::config::js_round(CLIENT_SETTINGS_MAX_BYTES as f64 / 1024.0)
    )
}

/// Refusal for a body we stopped reading before it was parsed.
pub fn body_too_large_message() -> String {
    format!("Request body too large. Settings are limited to {} of JSON.", limit_label())
}

/// Resolve the `client` discriminator a request named.
///
/// Absent means "desktop": the only client today should not have to say so,
/// and the parameter exists for the one after it. Anything not on the
/// allowlist is `None`, which the caller turns into a refusal - never a
/// silently created row under an unknown name.
pub fn normalize_client(raw: Option<&str>) -> Option<&'static str> {
    let raw = match raw {
        None | Some("") => return Some(DESKTOP_CLIENT),
        Some(raw) => raw,
    };
    let value = crate::config::js_trim(raw).to_lowercase();
    CLIENT_KINDS.iter().find(|k| **k == value).copied()
}

/// Refusal for an unknown client kind, listing what does exist.
pub fn unknown_client_message() -> String {
    format!("Unknown client. Known clients: {}.", CLIENT_KINDS.join(", "))
}

/// A settings document accepted for storage.
#[derive(Debug)]
pub struct AcceptedSettings {
    /// `JSON.stringify(settings)`: what the column will actually hold.
    pub canonical: String,
    /// Canonical size in UTF-8 bytes.
    pub bytes: usize,
    /// The version the client believes it is replacing, or `None` for "just
    /// take this". See `routes/api/client_settings.rs` for what each one does.
    pub expected_version: Option<i64>,
}

/// `SettingsBody`: accepted, or a refusal with its status.
#[derive(Debug)]
pub enum SettingsBody {
    Ok(AcceptedSettings),
    Refused { status: StatusCode, message: String },
}

fn refused(message: impl Into<String>) -> SettingsBody {
    SettingsBody::Refused { status: StatusCode::BAD_REQUEST, message: message.into() }
}

/// Validate a PUT body.
///
/// Error-as-data rather than thrown strings: the caller needs the status code
/// as much as the sentence, and this way both travel together.
///
/// The envelope is required (`{"settings": {...}}`) rather than accepting a
/// bare object as the settings themselves. It has to carry `version` anyway,
/// and guessing between "this is the envelope" and "this is a settings
/// document that happens to have a `settings` key" is a coin flip that would
/// eventually be called wrong on somebody's real configuration.
///
/// `Err` is upstream's one uncaught path: `JSON.stringify` of a document
/// nested deeper than V8's stack allows throws a `RangeError`, which the
/// route answers as Next answers any thrown error (a bare 500, nothing
/// stored). See [`js_json::STRINGIFY_STACK`].
pub fn parse_client_settings_body(text: &str) -> Result<SettingsBody, RangeError> {
    if crate::config::js_trim(text).is_empty() {
        return Ok(refused(r#"Body must be JSON: {"settings": { ... }}."#));
    }

    let Ok(body) = js_json::parse(text) else {
        // Also the landing spot for absurdly nested input upstream, where V8
        // could raise a RangeError out of JSON.parse: a 400 describing their
        // body rather than a 500 describing our stack.
        return Ok(refused("Body is not valid JSON."));
    };

    let JsValue::Object(body) = &body else {
        return Ok(refused(r#"Body must be a JSON object: {"settings": { ... }}."#));
    };

    let Some(settings @ JsValue::Object(_)) = body.get("settings") else {
        return Ok(refused("`settings` must be a JSON object."));
    };

    let Ok(expected_version) = read_expected_version(body) else {
        return Ok(refused(format!("`version` must be an integer between 0 and {MAX_EXPECTED_VERSION}, or omitted.")));
    };

    // Measure the canonical form, not the request: that is the string the
    // column stores, and it is what the cap is a promise about.
    let canonical = js_json::stringify_within(settings, STRINGIFY_STACK)?;
    let bytes = canonical.len();
    if bytes > CLIENT_SETTINGS_MAX_BYTES {
        return Ok(SettingsBody::Refused {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            message: format!("Settings too large: {bytes} bytes. The limit is {}.", limit_label()),
        });
    }

    // Postgres `jsonb` cannot represent a NUL inside a string: it rejects the
    // escape outright, because a jsonb value has to convert to `text` and
    // text has no such character. Left unchecked this is a raised driver
    // error on INSERT, i.e. a 500 quoting Postgres at a client that is owed a
    // 400 quoting the reason. Checked on the canonical form because that is
    // the string the driver sends, however the client chose to spell it (and,
    // as upstream, a literal backslash followed by `u0000` trips it too).
    //
    // An UNPAIRED SURROGATE is the same class of problem and is deliberately
    // NOT guarded here: Postgres's JSON lexer refuses those too, and the
    // obvious guard (matching `\ud[89ab]..` in the canonical text) also
    // matches a canonicalised Windows path such as `C:\\ud800\\shots`. This
    // column stores a Windows client's capture paths, so a guard that refuses
    // a valid save is worse than the 500 it prevents on a body no real client
    // sends.
    if canonical.contains("\\u0000") {
        return Ok(refused("Settings must not contain NUL characters."));
    }

    Ok(SettingsBody::Ok(AcceptedSettings { canonical, bytes, expected_version }))
}

/// `number | null` for a usable value, `Err` for a refusal.
fn read_expected_version(body: &JsObject) -> Result<Option<i64>, ()> {
    // An explicit null reads as "do not check", the same as leaving the key
    // out. JSON has no other way to spell a missing number: a client whose
    // field is unset serialises it as null, and NaN/Infinity become null on
    // the way out of JSON.stringify too.
    match body.get("version") {
        None | Some(JsValue::Null) => Ok(None),
        Some(JsValue::Number(n)) if is_safe_integer(*n) && *n >= 0.0 && *n <= MAX_EXPECTED_VERSION as f64 => {
            // `-0` passes as 0, exactly as `-0 < 0` is false upstream.
            Ok(Some(*n as i64))
        }
        Some(_) => Err(()),
    }
}

#[cfg(test)]
mod tests {
    //! tests/unit/client-settings.test.ts
    use serde_json::{Value, json};

    use super::*;

    fn envelope(settings: Value, version: Option<Value>) -> String {
        match version {
            None => json!({ "settings": settings }).to_string(),
            Some(v) => json!({ "settings": settings, "version": v }).to_string(),
        }
    }

    fn parse(text: &str) -> SettingsBody {
        parse_client_settings_body(text).expect("no RangeError")
    }

    fn accepted(text: &str) -> AcceptedSettings {
        match parse(text) {
            SettingsBody::Ok(a) => a,
            SettingsBody::Refused { status, message } => panic!("refused {status}: {message}"),
        }
    }

    fn refusal(text: &str) -> (StatusCode, String) {
        match parse(text) {
            SettingsBody::Ok(a) => panic!("accepted: {}", a.canonical),
            SettingsBody::Refused { status, message } => (status, message),
        }
    }

    // ── client kind ──

    #[test]
    fn defaults_to_desktop_when_the_caller_says_nothing() {
        assert_eq!(normalize_client(None), Some(DESKTOP_CLIENT));
        assert_eq!(normalize_client(Some("")), Some(DESKTOP_CLIENT));
    }

    #[test]
    fn refuses_a_kind_that_is_not_on_the_allowlist() {
        assert_eq!(normalize_client(Some("mobile")), None);
        assert_eq!(normalize_client(Some("desktop2")), None);
        assert_eq!(normalize_client(Some("../../etc/passwd")), None);
        assert_eq!(normalize_client(Some(&"x".repeat(500))), None);
        // Whitespace-only is not "absent": it trims to "" and is refused.
        assert_eq!(normalize_client(Some("  ")), None);
    }

    #[test]
    fn accepts_the_known_kind_however_it_is_capitalised_or_padded() {
        assert_eq!(normalize_client(Some(" Desktop ")), Some(DESKTOP_CLIENT));
        assert_eq!(normalize_client(Some("DESKTOP")), Some(DESKTOP_CLIENT));
        // toLowerCase() is Unicode-aware: the Kelvin sign lowers to "k".
        assert_eq!(normalize_client(Some("DES\u{212A}TOP")), Some(DESKTOP_CLIENT));
        assert_eq!(unknown_client_message(), "Unknown client. Known clients: desktop.");
    }

    // ── PUT body ──

    #[test]
    fn accepts_an_envelope_and_reports_the_canonical_size() {
        let a = accepted(&envelope(json!({ "hotkeys": { "capture": "PrintScreen" } }), None));
        assert_eq!(a.expected_version, None);
        assert_eq!(a.canonical, r#"{"hotkeys":{"capture":"PrintScreen"}}"#);
        assert_eq!(a.bytes, r#"{"hotkeys":{"capture":"PrintScreen"}}"#.len());
    }

    #[test]
    fn keeps_no_expected_version_distinct_from_expected_version_0() {
        assert_eq!(accepted(&envelope(json!({}), None)).expected_version, None);
        assert_eq!(accepted(&envelope(json!({}), Some(json!(0)))).expected_version, Some(0));
    }

    #[test]
    fn refuses_a_version_that_is_not_a_non_negative_integer() {
        for bad in [json!(-1), json!(1.5), json!("3"), json!(true), json!([]), json!({})] {
            let (status, message) = refusal(&envelope(json!({}), Some(bad.clone())));
            assert_eq!(status, StatusCode::BAD_REQUEST, "version {bad}");
            assert_eq!(message, "`version` must be an integer between 0 and 2147483646, or omitted.");
        }
    }

    #[test]
    fn refuses_a_version_too_large_for_the_int4_column() {
        assert!(matches!(parse(&envelope(json!({}), Some(json!(MAX_EXPECTED_VERSION)))), SettingsBody::Ok(_)));
        for bad in [json!(MAX_EXPECTED_VERSION + 1), json!(2_147_483_648i64), json!(9_007_199_254_740_991i64)] {
            assert_eq!(refusal(&envelope(json!({}), Some(bad.clone()))).0, StatusCode::BAD_REQUEST, "version {bad}");
        }
    }

    #[test]
    fn reads_an_explicit_null_version_as_no_check() {
        assert_eq!(accepted(&envelope(json!({}), Some(Value::Null))).expected_version, None);
    }

    #[test]
    fn reads_versions_the_way_javascript_numbers_do() {
        // 1.0 and 1e2 are integers to JavaScript; -0 is 0.
        assert_eq!(accepted(r#"{"settings":{},"version":1.0}"#).expected_version, Some(1));
        assert_eq!(accepted(r#"{"settings":{},"version":1e2}"#).expected_version, Some(100));
        assert_eq!(accepted(r#"{"settings":{},"version":-0}"#).expected_version, Some(0));
    }

    #[test]
    fn refuses_a_payload_that_is_not_a_json_object() {
        for bad in ["[]", r#""a string""#, "42", "null", "true"] {
            assert_eq!(refusal(&format!(r#"{{"settings": {bad}}}"#)).1, "`settings` must be a JSON object.");
        }
        assert_eq!(refusal("[]").1, r#"Body must be a JSON object: {"settings": { ... }}."#);
        assert_eq!(refusal("null").1, r#"Body must be a JSON object: {"settings": { ... }}."#);
    }

    #[test]
    fn requires_the_envelope_rather_than_guessing() {
        let (status, message) = refusal(&json!({ "hotkeys": {} }).to_string());
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(message, "`settings` must be a JSON object.");
    }

    #[test]
    fn answers_400_not_500_for_a_body_that_is_not_json_at_all() {
        for bad in ["", "   "] {
            assert_eq!(refusal(bad), (StatusCode::BAD_REQUEST, r#"Body must be JSON: {"settings": { ... }}."#.into()));
        }
        for bad in ["{", "not json", "{'single': 'quotes'}"] {
            assert_eq!(refusal(bad), (StatusCode::BAD_REQUEST, "Body is not valid JSON.".into()));
        }
    }

    #[test]
    fn rejects_an_oversized_document_with_413_and_states_the_limit() {
        let huge = json!({ "blob": "x".repeat(CLIENT_SETTINGS_MAX_BYTES) });
        let (status, message) = refusal(&envelope(huge, None));
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(message.contains(&CLIENT_SETTINGS_MAX_BYTES.to_string()));
        assert!(message.contains("256 KiB"));
        assert_eq!(message, "Settings too large: 262155 bytes. The limit is 262144 bytes (256 KiB).");
    }

    #[test]
    fn accepts_a_document_sitting_exactly_on_the_limit() {
        let overhead = json!({ "blob": "" }).to_string().len();
        let exact = json!({ "blob": "x".repeat(CLIENT_SETTINGS_MAX_BYTES - overhead) });
        assert_eq!(accepted(&envelope(exact, None)).bytes, CLIENT_SETTINGS_MAX_BYTES);
    }

    #[test]
    fn measures_bytes_not_characters() {
        let a = accepted(&envelope(json!({ "k": "\u{1F600}" }), None));
        let canonical = json!({ "k": "\u{1F600}" }).to_string();
        assert!(a.bytes > canonical.encode_utf16().count());
        assert_eq!(a.bytes, canonical.len());
    }

    #[test]
    fn refuses_a_nul_character_instead_of_letting_postgres_raise() {
        let (status, message) = refusal(&envelope(json!({ "k": "a\u{0000}b" }), None));
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(message, "Settings must not contain NUL characters.");
        // The canonical text is what is checked, so a literal "\u0000" (six
        // characters) is refused as well, exactly as upstream.
        assert_eq!(refusal(r#"{"settings":{"k":"\\u0000"}}"#).1, "Settings must not contain NUL characters.");
    }

    #[test]
    fn measures_the_settings_not_the_request_so_indentation_is_not_penalised() {
        let settings = json!({ "a": "x".repeat(1000), "b": { "c": [1, 2, 3] } });
        let pretty = serde_json::to_string_pretty(&json!({ "settings": settings })).unwrap();
        let a = accepted(&pretty);
        assert!(a.bytes < pretty.len());
    }

    #[test]
    fn canonicalises_like_json_stringify() {
        let a = accepted(r#"{"settings":{"b":1.0,"10":1e21,"2":"\ud800","b":[-0,1E-7]}}"#);
        assert_eq!(a.canonical, r#"{"2":"\ud800","10":1e+21,"b":[0,1e-7]}"#);
    }

    #[test]
    fn a_document_deeper_than_v8_can_stringify_is_the_thrown_range_error() {
        let ok = format!(r#"{{"settings":{{"a":{}{}}}}}"#, "[".repeat(4351), "]".repeat(4351));
        assert!(parse_client_settings_body(&ok).is_ok());
        let deep = format!(r#"{{"settings":{{"a":{}{}}}}}"#, "[".repeat(4352), "]".repeat(4352));
        assert_eq!(parse_client_settings_body(&deep).err(), Some(RangeError));
        // After the version check and before the size check, as upstream.
        let bad_version = format!(r#"{{"settings":{{"a":{}{}}},"version":"x"}}"#, "[".repeat(4352), "]".repeat(4352));
        assert!(matches!(parse_client_settings_body(&bad_version), Ok(SettingsBody::Refused { .. })));
    }

    // ── transport cap ──

    #[test]
    fn is_looser_than_the_storage_cap_but_still_bounded() {
        const _: () = assert!(CLIENT_SETTINGS_BODY_MAX_BYTES > CLIENT_SETTINGS_MAX_BYTES);
        const _: () = assert!(CLIENT_SETTINGS_BODY_MAX_BYTES <= CLIENT_SETTINGS_MAX_BYTES * 4);
    }

    #[test]
    fn still_quotes_the_storage_limit_which_is_the_number_a_client_can_act_on() {
        assert!(body_too_large_message().contains(&CLIENT_SETTINGS_MAX_BYTES.to_string()));
        assert_eq!(
            body_too_large_message(),
            "Request body too large. Settings are limited to 262144 bytes (256 KiB) of JSON."
        );
    }
}
