//! JavaScript value semantics for raw action arguments.
//!
//! Upstream actions receive whatever React decoded from the request and use
//! it with plain JavaScript operators (`if (id)`, `Number(x)`, `x ?? y`); there
//! is no runtime type check. These helpers apply the same operators to the
//! tagged JSON an RPC call carries, so a value of an unexpected type takes the
//! branch it took upstream.

use serde_json::Value;

use crate::rpc::tagged;

/// `v == null`: absent, `null` or `undefined`.
pub fn is_nullish(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(v) => tagged::is_undefined(v),
    }
}

/// `Boolean(v)`. NaN, `0n` and the empty string are falsy, every object and
/// array (a date included) is truthy.
pub fn truthy(v: Option<&Value>) -> bool {
    let Some(v) = v else {
        return false;
    };
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) => true,
        Value::Object(m) => {
            if tagged::is_undefined(v) {
                return false;
            }
            if let Some(n) = tagged::raw_num(v) {
                return !n.is_nan() && n != 0.0;
            }
            if m.len() == 1
                && let Some(Value::String(b)) = m.get("$bigint")
            {
                return !b.trim_start_matches('-').trim_start_matches('0').is_empty();
            }
            true
        }
    }
}

/// `Number(v)` (ToNumber). `undefined` is NaN, `null` 0, a date its time
/// value, a bigint its value, an array its single element's number (or 0 when
/// empty), any other object NaN.
pub fn to_number(v: Option<&Value>) -> f64 {
    let Some(v) = v else {
        return f64::NAN;
    };
    match v {
        Value::Null => 0.0,
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) => crate::config::js_number(s),
        Value::Array(items) => match items.as_slice() {
            [] => 0.0,
            // `[x].toString()` is `String(x)`; null and undefined become "".
            [one] => match one {
                Value::Null => 0.0,
                Value::String(s) => crate::config::js_number(s),
                Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
                Value::Array(_) => to_number(Some(one)),
                o if tagged::is_undefined(o) => 0.0,
                o if tagged::raw_num(o).is_some() => tagged::raw_num(o).unwrap_or(f64::NAN),
                // "true", "[object Object]", a date string: not numeric.
                _ => f64::NAN,
            },
            _ => f64::NAN,
        },
        Value::Object(m) => {
            if tagged::is_undefined(v) {
                return f64::NAN;
            }
            if let Some(n) = tagged::raw_num(v) {
                return n;
            }
            if m.len() == 1 {
                if let Some(Value::String(d)) = m.get("$date") {
                    return crate::js_date::parse_js_date(d).map(|d| d.millis() as f64).unwrap_or(f64::NAN);
                }
                if let Some(Value::String(b)) = m.get("$bigint") {
                    return b.parse::<f64>().unwrap_or(f64::NAN);
                }
            }
            f64::NAN
        }
    }
}

/// A plain (non-tagged) number as it would print, for audit details and
/// results: integers without a fraction, NaN and infinities as `null` (what
/// `JSON.stringify` writes for them).
pub fn number_value(n: f64) -> Value {
    if !n.is_finite() {
        return Value::Null;
    }
    if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
        return Value::from(n as i64);
    }
    serde_json::Number::from_f64(n).map(Value::Number).unwrap_or(Value::Null)
}

/// `String(v)` / a template literal's `${v}` for the values an argument can
/// hold: numbers as JavaScript prints them, arrays joined with ",", objects
/// as "[object Object]".
pub fn js_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => {
            let f = n.as_f64().unwrap_or(f64::NAN);
            match number_value(f) {
                Value::Number(n) => n.to_string(),
                _ => f.to_string(),
            }
        }
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        // `Array#toString` is `join(",")`, with null and undefined as "".
        Value::Array(items) => items
            .iter()
            .map(|x| if is_nullish(Some(x)) { String::new() } else { js_string(x) })
            .collect::<Vec<_>>()
            .join(","),
        o if tagged::is_undefined(o) => "undefined".to_string(),
        o if tagged::raw_num(o).is_some() => match tagged::raw_num(o).unwrap_or(f64::NAN) {
            n if n.is_nan() => "NaN".to_string(),
            n if n > 0.0 => "Infinity".to_string(),
            _ => "-Infinity".to_string(),
        },
        _ => "[object Object]".to_string(),
    }
}

/// `obj?.[key]` on a raw argument: the property of an object, `None` for
/// anything else (a primitive has no own data properties worth reading here).
pub fn prop<'a>(obj: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    match obj {
        Some(v @ Value::Object(m)) if !tagged::is_undefined(v) => m.get(key),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn truthiness() {
        for falsy in [json!(null), json!(false), json!(0), json!(""), json!({"$undefined": 1}), json!({"$num": "NaN"})]
        {
            assert!(!truthy(Some(&falsy)), "{falsy}");
        }
        assert!(!truthy(None));
        assert!(!truthy(Some(&json!({"$bigint": "0"}))));
        for t in [json!(true), json!(1), json!("0"), json!([]), json!({}), json!({"$date": "2026-01-01T00:00:00.000Z"})]
        {
            assert!(truthy(Some(&t)), "{t}");
        }
        assert!(truthy(Some(&json!({"$bigint": "-3"}))));
        assert!(truthy(Some(&json!({"$num": "Infinity"}))));
    }

    #[test]
    fn numbers() {
        assert_eq!(to_number(Some(&json!("  12 "))), 12.0);
        assert_eq!(to_number(Some(&json!(true))), 1.0);
        assert_eq!(to_number(Some(&json!(null))), 0.0);
        assert!(to_number(None).is_nan());
        assert!(to_number(Some(&json!({"$undefined": 1}))).is_nan());
        assert_eq!(to_number(Some(&json!([]))), 0.0);
        assert_eq!(to_number(Some(&json!(["7"]))), 7.0);
        assert!(to_number(Some(&json!([1, 2]))).is_nan());
        assert!(to_number(Some(&json!({}))).is_nan());
        assert_eq!(to_number(Some(&json!({"$date": "1970-01-01T00:00:01.000Z"}))), 1000.0);
        assert_eq!(to_number(Some(&json!({"$bigint": "42"}))), 42.0);
        assert!(to_number(Some(&json!("abc"))).is_nan());
        assert_eq!(number_value(3.0), json!(3));
        assert_eq!(number_value(2.5), json!(2.5));
        assert_eq!(number_value(f64::NAN), Value::Null);
        assert_eq!(js_string(&json!(5)), "5");
        assert_eq!(js_string(&json!(2.5)), "2.5");
        assert_eq!(js_string(&json!([1, null, "a"])), "1,,a");
        assert_eq!(js_string(&json!({"a": 1})), "[object Object]");
    }

    #[test]
    fn nullish_and_props() {
        assert!(is_nullish(None));
        assert!(is_nullish(Some(&json!(null))));
        assert!(is_nullish(Some(&json!({"$undefined": 1}))));
        assert!(!is_nullish(Some(&json!(0))));
        let o = json!({"a": 1});
        assert_eq!(prop(Some(&o), "a"), Some(&json!(1)));
        assert_eq!(prop(Some(&json!("str")), "length"), None);
    }
}
