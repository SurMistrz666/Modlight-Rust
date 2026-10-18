//! Tagged JSON (PORT.md section 7), the codec of RPC and page-data payloads.
//!
//! Plain JSON plus four tags for the JavaScript values JSON cannot carry:
//!
//! | JavaScript | Tagged JSON |
//! |---|---|
//! | `Date` | `{"$date": "2026-10-18T12:00:00.000Z"}` |
//! | `undefined` | `{"$undefined": 1}` |
//! | `bigint` | `{"$bigint": "123"}` |
//! | `NaN`, `Infinity`, `-Infinity` | `{"$num": "NaN"}` |
//!
//! Encoding: serialize with [`to_tagged_value`]. Every [`JsDate`] inside then
//! writes itself as `{"$date"}` (outside it writes the ISO string that
//! `JSON.stringify` would), so the same model structs serve the JSON API and
//! the payloads. Use [`Undefined`], [`BigInt`] and [`Num`] for the rest.
//!
//! Decoding (RPC arguments): [`normalize`] turns a tagged value into plain JSON
//! for serde (`$undefined` -> null, `$date` -> its ISO string, `$bigint` -> a
//! number when it fits, `$num` -> null); [`raw_num`] reads a number exactly,
//! NaN and infinities included.
//!
//! [`JsDate`]: crate::db::types::JsDate

use std::cell::Cell;

use serde::{Serialize, Serializer};
use serde_json::{Map, Value, json};

thread_local! {
    static TAGGED: Cell<bool> = const { Cell::new(false) };
}

/// Is a tagged serialization in progress on this thread?
pub fn is_tagged_mode() -> bool {
    TAGGED.with(Cell::get)
}

struct TaggedGuard(bool);

impl Drop for TaggedGuard {
    fn drop(&mut self) {
        TAGGED.with(|t| t.set(self.0));
    }
}

/// Serialize `value` as tagged JSON. Serialization is synchronous, so the
/// thread-local switch cannot leak into another task.
pub fn to_tagged_value<T: Serialize + ?Sized>(value: &T) -> serde_json::Result<Value> {
    let previous = TAGGED.with(|t| t.replace(true));
    let _guard = TaggedGuard(previous);
    serde_json::to_value(value)
}

/// Run `f` with tagged serialization switched on (what [`tagged_json!`]
/// wraps around `serde_json::json!`).
///
/// [`tagged_json!`]: crate::tagged_json
pub fn with_tagged<R>(f: impl FnOnce() -> R) -> R {
    let previous = TAGGED.with(|t| t.replace(true));
    let _guard = TaggedGuard(previous);
    f()
}

/// `serde_json::json!` in tagged mode. Plain `json!` serializes its
/// interpolated values on the spot, so a [`JsDate`](crate::db::types::JsDate)
/// inside it becomes an ISO string, not `{"$date"}`; use this (or a typed
/// struct with [`to_tagged_value`]) for RPC and page payloads.
#[macro_export]
macro_rules! tagged_json {
    ($($json:tt)+) => {
        $crate::rpc::tagged::with_tagged(|| ::serde_json::json!($($json)+))
    };
}

/// `undefined`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Undefined;

impl Serialize for Undefined {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("$undefined", &1)?;
        map.end()
    }
}

/// The tagged form of `undefined`, as a value.
pub fn undefined() -> Value {
    json!({ "$undefined": 1 })
}

/// A JavaScript `bigint`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BigInt(pub i128);

impl Serialize for BigInt {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("$bigint", &self.0.to_string())?;
        map.end()
    }
}

/// A JavaScript number that may be NaN or infinite.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Num(pub f64);

impl Serialize for Num {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let n = self.0;
        if n.is_finite() {
            if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
                return serializer.serialize_i64(n as i64);
            }
            return serializer.serialize_f64(n);
        }
        use serde::ser::SerializeMap;
        let tag = if n.is_nan() {
            "NaN"
        } else if n > 0.0 {
            "Infinity"
        } else {
            "-Infinity"
        };
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("$num", tag)?;
        map.end()
    }
}

fn single_tag<'a>(map: &'a Map<String, Value>, tag: &str) -> Option<&'a Value> {
    if map.len() == 1 { map.get(tag) } else { None }
}

/// Is this the tagged `undefined`?
pub fn is_undefined(v: &Value) -> bool {
    matches!(v, Value::Object(m) if single_tag(m, "$undefined").is_some())
}

/// A tagged or plain number, exactly (NaN and infinities included).
pub fn raw_num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::Object(m) => match single_tag(m, "$num").and_then(Value::as_str) {
            Some("NaN") => Some(f64::NAN),
            Some("Infinity") => Some(f64::INFINITY),
            Some("-Infinity") => Some(f64::NEG_INFINITY),
            _ => None,
        },
        _ => None,
    }
}

/// Tagged JSON -> plain JSON for typed decoding (see the module docs).
pub fn normalize(v: Value) -> Value {
    match v {
        Value::Array(items) => Value::Array(items.into_iter().map(normalize).collect()),
        Value::Object(map) => {
            if map.len() == 1 {
                if map.contains_key("$undefined") {
                    return Value::Null;
                }
                if let Some(Value::String(d)) = map.get("$date") {
                    return Value::String(d.clone());
                }
                if let Some(Value::String(b)) = map.get("$bigint") {
                    return match b.parse::<i64>() {
                        Ok(n) => Value::from(n),
                        Err(_) => Value::String(b.clone()),
                    };
                }
                if map.contains_key("$num") {
                    return Value::Null;
                }
            }
            Value::Object(map.into_iter().map(|(k, v)| (k, normalize(v))).collect())
        }
        other => other,
    }
}

/// Serialize for inlining into HTML (`<script type="application/json">`):
/// nothing in the output can close the script element or start a comment.
pub fn json_for_html(value: &Value) -> String {
    let text = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());
    let mut out = String::with_capacity(text.len() + 16);
    for c in text.chars() {
        match c {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::types::JsDate;

    #[derive(Serialize)]
    struct Row {
        when: JsDate,
        missing: Undefined,
        big: BigInt,
        nan: Num,
        one: Num,
    }

    #[test]
    fn encodes_the_four_tags() {
        let row = Row {
            when: JsDate::from_millis(1_790_000_000_000).unwrap(),
            missing: Undefined,
            big: BigInt(123),
            nan: Num(f64::NAN),
            one: Num(1.0),
        };
        let v = to_tagged_value(&row).unwrap();
        assert_eq!(
            v,
            json!({
                "when": { "$date": "2026-09-21T14:13:20.000Z" },
                "missing": { "$undefined": 1 },
                "big": { "$bigint": "123" },
                "nan": { "$num": "NaN" },
                "one": 1
            })
        );
        // Outside tagged mode a date is what JSON.stringify writes.
        assert_eq!(serde_json::to_value(&row).unwrap()["when"], json!("2026-09-21T14:13:20.000Z"));
        assert!(!is_tagged_mode());
    }

    #[test]
    fn tagged_json_macro_tags_interpolated_dates() {
        let d = JsDate::from_millis(0).unwrap();
        assert_eq!(crate::tagged_json!({ "d": d }), json!({ "d": { "$date": "1970-01-01T00:00:00.000Z" } }));
        assert_eq!(json!({ "d": d }), json!({ "d": "1970-01-01T00:00:00.000Z" }));
        assert!(!is_tagged_mode());
    }

    #[test]
    fn normalizes_for_serde() {
        let v = json!([{"$undefined": 1}, {"$date": "2026-10-18T12:00:00.000Z"}, {"$bigint": "5"}, {"a": {"$num": "Infinity"}}]);
        assert_eq!(normalize(v), json!([null, "2026-10-18T12:00:00.000Z", 5, {"a": null}]));
        assert!(raw_num(&json!({"$num": "-Infinity"})).unwrap().is_infinite());
        assert!(is_undefined(&json!({"$undefined": 1})));
    }

    #[test]
    fn html_escaping_cannot_close_the_script() {
        let s = json_for_html(&json!({"x": "</script><!-- & \u{2028}"}));
        assert!(!s.contains('<') && !s.contains('>') && !s.contains('&'));
        let back: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(back["x"], "</script><!-- & \u{2028}");
    }
}
