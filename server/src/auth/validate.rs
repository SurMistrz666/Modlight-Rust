//! The request validation Better Auth's endpoints run before their handler:
//! better-call's `runValidation` over the zod 4.4.3 schemas the endpoints
//! declare, with zod's English messages, so a refusal reads exactly like
//! upstream's:
//!
//! ```text
//! 400 400 {"message":"[body.email] Invalid input: expected string, received undefined; [body.password] ...","code":"VALIDATION_ERROR"}
//! ```
//!
//! Only the zod features those schemas use are implemented: strings (with
//! `nonempty`/`min`), `z.email()`, booleans, numbers, `z.coerce.string()`,
//! `z.coerce.boolean()`, `any`, optional/default wrappers, objects (unknown
//! keys stripped), records, intersections (`.and`), unions, arrays and enums.
//!
//! zod semantics kept on purpose:
//! - every issue is collected (one per failing field, in schema order), not
//!   just the first;
//! - a length check runs even after a type error when the input HAS a length
//!   (`z.string().nonempty()` on `[]` reports the type AND "Too small:
//!   expected array to have >=1 items");
//! - a key that is absent stays absent in the output; `.default(v)` fills it.

use serde_json::{Map, Value};

use crate::config::is_js_whitespace;

/// One zod schema node.
#[derive(Debug, Clone)]
pub enum Schema {
    /// `z.string()`, optionally `.min(n)`/`.nonempty(message?)`.
    String {
        min: Option<(usize, Option<&'static str>)>,
    },
    /// `z.email()`: a string with the email format.
    Email,
    Boolean,
    Number,
    /// `z.coerce.string()` (`String(input)`), optionally `.nonempty(message)`.
    CoerceString {
        min: Option<(usize, Option<&'static str>)>,
    },
    /// `z.coerce.boolean()` (`Boolean(input)`).
    CoerceBoolean,
    Any,
    Optional(Box<Schema>),
    /// `.default(value)`: undefined becomes `value`.
    Default(Box<Schema>, Value),
    /// `z.object({...})`, unknown keys stripped.
    Object(Vec<(&'static str, Schema)>),
    /// `z.record(z.string(), value)`.
    Record(Box<Schema>),
    /// `a.and(b)`
    Intersection(Box<Schema>, Box<Schema>),
    Union(Vec<Schema>),
    /// `z.xor([...])`: exactly one option must match.
    Xor(Vec<Schema>),
    Array(Box<Schema>),
    Enum(&'static [&'static str]),
}

impl Schema {
    pub fn string() -> Schema {
        Schema::String { min: None }
    }
    pub fn nonempty_string() -> Schema {
        Schema::String { min: Some((1, None)) }
    }
    pub fn optional(self) -> Schema {
        Schema::Optional(Box::new(self))
    }
    pub fn object(fields: Vec<(&'static str, Schema)>) -> Schema {
        Schema::Object(fields)
    }
}

/// A zod issue: where (relative path) and what.
#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub path: Vec<String>,
    pub message: String,
    /// `continue: true` (a check failure) vs. an aborting type error.
    pub continues: bool,
}

/// `util.parsedType(input)` for JSON values (`None` = undefined).
pub fn parsed_type(v: Option<&Value>) -> &'static str {
    match v {
        None => "undefined",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}

fn invalid_type(expected: &str, input: Option<&Value>) -> Issue {
    Issue {
        path: Vec::new(),
        message: format!("Invalid input: expected {expected}, received {}", parsed_type(input)),
        continues: false,
    }
}

/// zod 4.4.3 `regexes.email`, without the look-aheads the regex crate lacks:
/// `^(?!\.)(?!.*\.\.)([A-Za-z0-9_'+\-\.]*)[A-Za-z0-9_+-]@([A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$`
pub fn is_zod_email(s: &str) -> bool {
    if s.starts_with('.') || s.contains("..") {
        return false;
    }
    let Some((local, domain)) = s.split_once('@') else {
        return false;
    };
    let local_ok = !local.is_empty()
        && local.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '\'' | '+' | '-' | '.'))
        && local.chars().last().is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-'));
    if !local_ok {
        return false;
    }
    let labels: Vec<&str> = domain.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    let (tld, rest) = labels.split_last().expect("at least two labels");
    let label_ok = |l: &&str| {
        let mut chars = l.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphanumeric()) && chars.all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    rest.iter().all(label_ok) && tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic())
}

/// JavaScript `String(value)` for JSON values (`undefined` -> "undefined").
pub fn js_string(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => crate::i18n::format::js_number_to_string(n.as_f64().unwrap_or(f64::NAN)),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|x| match x {
                Value::Null => String::new(),
                other => js_string(Some(other)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}

/// JavaScript truthiness of a JSON value.
pub fn js_truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

fn js_len(v: &str) -> usize {
    v.encode_utf16().count()
}

/// The min-length check zod runs whenever the input has a `.length`.
fn length_check(input: Option<&Value>, min: usize, custom: Option<&'static str>) -> Option<Issue> {
    let (len, origin): (f64, &str) = match input {
        Some(Value::String(s)) => (js_len(s) as f64, "string"),
        Some(Value::Array(a)) => (a.len() as f64, "array"),
        // `{ length: x }`: val.length !== undefined, compared with `<`.
        Some(Value::Object(m)) => (js_to_number(m.get("length")?), "unknown"),
        _ => return None,
    };
    if len >= min as f64 {
        return None;
    }
    let message = match custom {
        Some(m) => m.to_string(),
        None => match origin {
            "string" => format!("Too small: expected string to have >={min} characters"),
            "array" => format!("Too small: expected array to have >={min} items"),
            other => format!("Too small: expected {other} to be >={min}"),
        },
    };
    Some(Issue { path: Vec::new(), message, continues: true })
}

/// `Number(v)` for the `<` comparison above.
fn js_to_number(v: &Value) -> f64 {
    match v {
        Value::Null => 0.0,
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) => crate::config::js_number(s),
        Value::Array(a) if a.is_empty() => 0.0,
        Value::Array(a) if a.len() == 1 => js_to_number(&a[0]),
        _ => f64::NAN,
    }
}

/// Parse `input` against `schema`: the (possibly transformed) output, or the
/// issues. `None` input/output means `undefined`.
pub fn parse(schema: &Schema, input: Option<&Value>) -> (Option<Value>, Vec<Issue>) {
    match schema {
        Schema::String { min } => {
            let mut issues = Vec::new();
            if !matches!(input, Some(Value::String(_))) {
                issues.push(invalid_type("string", input));
            }
            if let Some((n, msg)) = min
                && let Some(issue) = length_check(input, *n, *msg)
            {
                issues.push(issue);
            }
            (input.cloned(), issues)
        }
        Schema::Email => match input {
            Some(Value::String(s)) => {
                if is_zod_email(s) {
                    (input.cloned(), Vec::new())
                } else {
                    (
                        input.cloned(),
                        vec![Issue { path: Vec::new(), message: "Invalid email address".into(), continues: true }],
                    )
                }
            }
            _ => (input.cloned(), vec![invalid_type("string", input)]),
        },
        Schema::Boolean => match input {
            Some(Value::Bool(_)) => (input.cloned(), Vec::new()),
            _ => (input.cloned(), vec![invalid_type("boolean", input)]),
        },
        Schema::Number => match input {
            Some(Value::Number(_)) => (input.cloned(), Vec::new()),
            _ => (input.cloned(), vec![invalid_type("number", input)]),
        },
        Schema::CoerceString { min } => {
            let s = Value::String(js_string(input));
            let mut issues = Vec::new();
            if let Some((n, msg)) = min
                && let Some(issue) = length_check(Some(&s), *n, *msg)
            {
                issues.push(issue);
            }
            (Some(s), issues)
        }
        Schema::CoerceBoolean => (Some(Value::Bool(js_truthy(input))), Vec::new()),
        Schema::Any => (input.cloned(), Vec::new()),
        Schema::Optional(inner) => match input {
            None => {
                // `.default().optional()`: the default still applies.
                if let Schema::Default(_, v) = inner.as_ref() {
                    return (Some(v.clone()), Vec::new());
                }
                (None, Vec::new())
            }
            Some(_) => parse(inner, input),
        },
        Schema::Default(inner, value) => match input {
            None => (Some(value.clone()), Vec::new()),
            Some(_) => parse(inner, input),
        },
        Schema::Object(fields) => {
            let Some(Value::Object(map)) = input else {
                return (input.cloned(), vec![invalid_type("object", input)]);
            };
            let mut out = Map::new();
            let mut issues = Vec::new();
            for (key, field_schema) in fields {
                let (value, field_issues) = parse(field_schema, map.get(*key));
                for mut issue in field_issues {
                    issue.path.insert(0, (*key).to_string());
                    issues.push(issue);
                }
                if let Some(v) = value {
                    out.insert((*key).to_string(), v);
                }
            }
            (Some(Value::Object(out)), issues)
        }
        Schema::Record(value_schema) => {
            let Some(Value::Object(map)) = input else {
                return (input.cloned(), vec![invalid_type("record", input)]);
            };
            let mut out = Map::new();
            let mut issues = Vec::new();
            for (key, v) in map {
                let (value, value_issues) = parse(value_schema, Some(v));
                for mut issue in value_issues {
                    issue.path.insert(0, key.clone());
                    issues.push(issue);
                }
                if let Some(v) = value {
                    out.insert(key.clone(), v);
                }
            }
            (Some(Value::Object(out)), issues)
        }
        Schema::Intersection(left, right) => {
            let (l, mut issues) = parse(left, input);
            let (r, right_issues) = parse(right, input);
            issues.extend(right_issues);
            if !issues.is_empty() {
                return (input.cloned(), issues);
            }
            // mergeValues: objects merge key-wise (left keys first).
            match (l, r) {
                (Some(Value::Object(mut a)), Some(Value::Object(b))) => {
                    for (k, v) in b {
                        a.insert(k, v);
                    }
                    (Some(Value::Object(a)), Vec::new())
                }
                (l, _) => (l, Vec::new()),
            }
        }
        Schema::Union(options) => {
            let results: Vec<(Option<Value>, Vec<Issue>)> = options.iter().map(|o| parse(o, input)).collect();
            if let Some((v, _)) = results.iter().find(|(_, issues)| issues.is_empty()) {
                return (v.clone(), Vec::new());
            }
            let non_aborted: Vec<&(Option<Value>, Vec<Issue>)> =
                results.iter().filter(|(_, issues)| issues.iter().all(|i| i.continues)).collect();
            if non_aborted.len() == 1 {
                return non_aborted[0].clone();
            }
            (input.cloned(), vec![Issue { path: Vec::new(), message: "Invalid input".into(), continues: false }])
        }
        Schema::Xor(options) => {
            let successes: Vec<Option<Value>> = options
                .iter()
                .map(|o| parse(o, input))
                .filter(|(_, issues)| issues.is_empty())
                .map(|(v, _)| v)
                .collect();
            if successes.len() == 1 {
                return (successes.into_iter().next().flatten(), Vec::new());
            }
            (input.cloned(), vec![Issue { path: Vec::new(), message: "Invalid input".into(), continues: false }])
        }
        Schema::Array(element) => {
            let Some(Value::Array(items)) = input else {
                return (input.cloned(), vec![invalid_type("array", input)]);
            };
            let mut out = Vec::new();
            let mut issues = Vec::new();
            for (i, item) in items.iter().enumerate() {
                let (value, item_issues) = parse(element, Some(item));
                for mut issue in item_issues {
                    issue.path.insert(0, i.to_string());
                    issues.push(issue);
                }
                out.push(value.unwrap_or(Value::Null));
            }
            (Some(Value::Array(out)), issues)
        }
        Schema::Enum(values) => match input {
            Some(Value::String(s)) if values.contains(&s.as_str()) => (input.cloned(), Vec::new()),
            _ => {
                let message = if values.len() == 1 {
                    format!("Invalid input: expected \"{}\"", values[0])
                } else {
                    format!(
                        "Invalid option: expected one of {}",
                        values.iter().map(|v| format!("\"{v}\"")).collect::<Vec<_>>().join("|")
                    )
                };
                (input.cloned(), vec![Issue { path: Vec::new(), message, continues: false }])
            }
        },
    }
}

/// better-call `fromError(issues, "body" | "query")`: the message of the 400.
pub fn format_issues(issues: &[Issue], validating: &str) -> String {
    issues
        .iter()
        .map(|i| {
            let at =
                if i.path.is_empty() { validating.to_string() } else { format!("{validating}.{}", i.path.join(".")) };
            format!("[{at}] {}", i.message)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// `z.string().trim()`-free helper: is this a JavaScript-blank string?
pub fn is_blank(s: &str) -> bool {
    s.chars().all(is_js_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sign_in() -> Schema {
        Schema::object(vec![
            ("email", Schema::string()),
            ("password", Schema::string()),
            ("callbackURL", Schema::string().optional()),
            ("rememberMe", Schema::Default(Box::new(Schema::Boolean), json!(true)).optional()),
        ])
    }

    #[test]
    fn missing_fields_are_all_reported_in_schema_order() {
        let (_, issues) = parse(&sign_in(), Some(&json!({})));
        assert_eq!(
            format_issues(&issues, "body"),
            "[body.email] Invalid input: expected string, received undefined; [body.password] Invalid input: expected string, received undefined"
        );
        let (_, issues) = parse(&sign_in(), Some(&json!([])));
        assert_eq!(format_issues(&issues, "body"), "[body] Invalid input: expected object, received array");
    }

    #[test]
    fn unknown_keys_are_stripped_and_defaults_applied() {
        let (out, issues) = parse(&sign_in(), Some(&json!({"email": "a", "password": "b", "x": 1})));
        assert!(issues.is_empty());
        assert_eq!(out.unwrap(), json!({"email": "a", "password": "b", "rememberMe": true}));
    }

    #[test]
    fn sign_up_intersection_reports_both_sides() {
        let schema = Schema::Intersection(
            Box::new(Schema::object(vec![
                ("name", Schema::string()),
                ("email", Schema::Email),
                ("password", Schema::nonempty_string()),
            ])),
            Box::new(Schema::Record(Box::new(Schema::Any))),
        );
        let (_, issues) = parse(&schema, Some(&json!([])));
        assert_eq!(
            format_issues(&issues, "body"),
            "[body] Invalid input: expected object, received array; [body] Invalid input: expected record, received array"
        );
        let (_, issues) = parse(&schema, Some(&json!({"name": "n", "email": "bad", "password": []})));
        assert_eq!(
            format_issues(&issues, "body"),
            "[body.email] Invalid email address; [body.password] Invalid input: expected string, received array; [body.password] Too small: expected array to have >=1 items"
        );
        let (out, issues) =
            parse(&schema, Some(&json!({"name": "n", "email": "a@b.co", "password": "x", "theme": "t"})));
        assert!(issues.is_empty());
        assert_eq!(out.unwrap()["theme"], "t");
    }

    #[test]
    fn zod_email_regex() {
        for ok in ["a@b.co", "a.b+c@x-y.example.com", "a'b@b.co", "_@b.co"] {
            assert!(is_zod_email(ok), "{ok}");
        }
        for bad in [".a@b.co", "a..b@b.co", "a.@b.co", "a@b", "a@b.c", "a@-b.co", "a@b.c0", "a b@b.co", "a@b..co", ""] {
            assert!(!is_zod_email(bad), "{bad}");
        }
    }

    #[test]
    fn coercions_like_javascript() {
        assert_eq!(parse(&Schema::CoerceString { min: None }, None).0, Some(json!("undefined")));
        assert_eq!(parse(&Schema::CoerceString { min: None }, Some(&json!([1, null, "a"]))).0, Some(json!("1,,a")));
        assert_eq!(parse(&Schema::CoerceBoolean, Some(&json!("false"))).0, Some(json!(true)));
        assert_eq!(parse(&Schema::CoerceBoolean, Some(&json!(""))).0, Some(json!(false)));
        let (_, issues) =
            parse(&Schema::CoerceString { min: Some((1, Some("userId cannot be empty"))) }, Some(&json!("")));
        assert_eq!(issues[0].message, "userId cannot be empty");
    }

    #[test]
    fn unions_and_enums() {
        let role = Schema::Union(vec![Schema::string(), Schema::Array(Box::new(Schema::string()))]);
        assert!(parse(&role, Some(&json!(["a"]))).1.is_empty());
        assert_eq!(parse(&role, Some(&json!(5))).1[0].message, "Invalid input");
        let e = Schema::Enum(&["email", "name"]);
        assert_eq!(parse(&e, Some(&json!("x"))).1[0].message, "Invalid option: expected one of \"email\"|\"name\"");
    }
}
