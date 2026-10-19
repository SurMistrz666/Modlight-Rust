//! `JSON.parse` and `JSON.stringify` as V8 implements them (Node 24, the
//! runtime of the upstream image), for the wire contracts that depend on
//! JavaScript's own view of a JSON document:
//!
//! - `lib/client-settings.ts` stores, measures and NUL-checks
//!   `JSON.stringify(JSON.parse(body).settings)`, and the database row is
//!   handed back through `JSON.parse` (node-postgres) and `JSON.stringify`
//!   (the route's `json()`);
//! - `lib/theme-api.ts` refuses exactly what `JSON.parse` refuses.
//!
//! serde_json differs in ways a client can see, which is why it is not used
//! for those documents: it refuses lone surrogate escapes (`"\ud800"`, which
//! `JSON.parse` accepts), stops at 128 levels of nesting (`JSON.parse` is
//! iterative and has no limit), keeps integers above 2^53 exact and prints
//! `1.0` / `1e21` where JavaScript prints `1` / `1e+21`, and does not move
//! integer-like keys to the front of an object.
//!
//! Strings are kept as UTF-16 code units ([`JsString`]) so a lone surrogate
//! survives a round trip exactly as it does in JavaScript; [`stringify`]
//! escapes it (`\ud800`, ES2019 well-formed `JSON.stringify`). Both
//! directions are iterative, so no document can exhaust the Rust stack.

use std::collections::HashMap;
use std::fmt::Write as _;

/// A JavaScript string: UTF-16 code units, lone surrogates allowed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct JsString(pub Vec<u16>);

impl JsString {
    pub fn units(&self) -> &[u16] {
        &self.0
    }

    /// `this === s` for a Rust string.
    pub fn eq_str(&self, s: &str) -> bool {
        self.0.iter().copied().eq(s.encode_utf16())
    }

    /// `String.prototype.trim()`: strips the ECMAScript WhiteSpace and
    /// LineTerminator code units at both ends.
    pub fn trim(&self) -> JsString {
        let is_ws = |u: &u16| char::from_u32(u32::from(*u)).is_some_and(crate::config::is_js_whitespace);
        let start = self.0.iter().position(|u| !is_ws(u)).unwrap_or(self.0.len());
        let end = self.0.iter().rposition(|u| !is_ws(u)).map_or(start, |i| i + 1);
        JsString(self.0[start..end].to_vec())
    }

    /// The string as Rust text; a lone surrogate becomes U+FFFD (it can never
    /// equal an ASCII identifier either way).
    pub fn to_string_lossy(&self) -> String {
        String::from_utf16_lossy(&self.0)
    }

    /// True for an "array index" key (a canonical numeric string of an
    /// integer in 0..=2^32-2): `OrdinaryOwnPropertyKeys` lists those first,
    /// in ascending numeric order, before every other key in creation order.
    fn array_index(&self) -> Option<u32> {
        let u = &self.0;
        if u.is_empty() || u.len() > 10 || !u.iter().all(|c| (u16::from(b'0')..=u16::from(b'9')).contains(c)) {
            return None;
        }
        if u.len() > 1 && u[0] == u16::from(b'0') {
            return None;
        }
        let n = u.iter().fold(0u64, |acc, c| acc * 10 + u64::from(c - u16::from(b'0')));
        (n <= 4_294_967_294).then_some(n as u32)
    }
}

impl From<&str> for JsString {
    fn from(s: &str) -> Self {
        JsString(s.encode_utf16().collect())
    }
}

/// A JSON value with JavaScript semantics.
#[derive(Debug, Clone, PartialEq)]
pub enum JsValue {
    Null,
    Bool(bool),
    /// A JavaScript number (non-finite values stringify as `null`).
    Number(f64),
    String(JsString),
    Array(Vec<JsValue>),
    Object(JsObject),
}

/// Dropping a document is iterative too: the derived drop recurses once per
/// level and a 512 KiB body can nest a quarter of a million levels deep.
impl Drop for JsValue {
    fn drop(&mut self) {
        fn take_children(v: &mut JsValue, into: &mut Vec<JsValue>) {
            match v {
                JsValue::Array(items) => into.append(items),
                JsValue::Object(obj) => into.extend(obj.entries.drain(..).map(|(_, v)| v)),
                _ => {}
            }
        }
        let mut pending = Vec::new();
        take_children(self, &mut pending);
        while let Some(mut v) = pending.pop() {
            take_children(&mut v, &mut pending);
        }
    }
}

impl From<&str> for JsValue {
    fn from(s: &str) -> Self {
        JsValue::String(JsString::from(s))
    }
}

impl From<String> for JsValue {
    fn from(s: String) -> Self {
        JsValue::String(JsString::from(s.as_str()))
    }
}

impl From<bool> for JsValue {
    fn from(b: bool) -> Self {
        JsValue::Bool(b)
    }
}

impl From<f64> for JsValue {
    fn from(n: f64) -> Self {
        JsValue::Number(n)
    }
}

impl<T: Into<JsValue>> From<Option<T>> for JsValue {
    fn from(v: Option<T>) -> Self {
        v.map_or(JsValue::Null, Into::into)
    }
}

/// An object's own properties in enumeration order (array-index keys first,
/// ascending, then the other keys in creation order).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JsObject {
    entries: Vec<(JsString, JsValue)>,
}

impl JsObject {
    /// Build from properties in creation order; a repeated key keeps its
    /// first position and takes the last value (`CreateDataProperty`).
    pub fn from_entries(entries: impl IntoIterator<Item = (JsString, JsValue)>) -> JsObject {
        let mut out: Vec<(JsString, JsValue)> = Vec::new();
        let mut seen: HashMap<JsString, usize> = HashMap::new();
        for (key, value) in entries {
            match seen.get(&key) {
                Some(&i) => out[i].1 = value,
                None => {
                    seen.insert(key.clone(), out.len());
                    out.push((key, value));
                }
            }
        }
        JsObject::ordered(out)
    }

    /// `{ k1: v1, k2: v2, ... }` written in code.
    pub fn new(entries: Vec<(&str, JsValue)>) -> JsObject {
        JsObject::from_entries(entries.into_iter().map(|(k, v)| (JsString::from(k), v)))
    }

    fn ordered(entries: Vec<(JsString, JsValue)>) -> JsObject {
        if !entries.iter().any(|(k, _)| k.array_index().is_some()) {
            return JsObject { entries };
        }
        let (mut indices, named): (Vec<_>, Vec<_>) = entries.into_iter().partition(|(k, _)| k.array_index().is_some());
        indices.sort_by_key(|(k, _)| k.array_index());
        indices.extend(named);
        JsObject { entries: indices }
    }

    /// `obj[key]` for an own property.
    pub fn get(&self, key: &str) -> Option<&JsValue> {
        self.entries.iter().find(|(k, _)| k.eq_str(key)).map(|(_, v)| v)
    }

    pub fn entries(&self) -> &[(JsString, JsValue)] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// `JSON.parse` threw a `SyntaxError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxError;

impl std::fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SyntaxError: not valid JSON")
    }
}

impl std::error::Error for SyntaxError {}

/// `JSON.stringify` threw `RangeError: Maximum call stack size exceeded`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangeError;

impl std::fmt::Display for RangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RangeError: Maximum call stack size exceeded")
    }
}

impl std::error::Error for RangeError {}

/// The WHATWG "UTF-8 decode" that `Request.text()` and
/// `new TextDecoder().decode()` perform: one leading byte-order mark dropped,
/// each maximal ill-formed subsequence replaced by U+FFFD.
pub fn utf8_decode(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    match text.strip_prefix('\u{FEFF}') {
        Some(rest) => rest.to_string(),
        None => text.into_owned(),
    }
}

// ── JSON.parse ──────────────────────────────────────────────────────────────

/// A container being filled while parsing.
enum Open {
    Array(Vec<JsValue>),
    Object { entries: Vec<(JsString, JsValue)>, key: JsString },
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    /// JSON whitespace only: space, tab, LF, CR (not the JavaScript set).
    fn skip_ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> Result<(), SyntaxError> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(SyntaxError)
        }
    }

    fn literal(&mut self, word: &[u8]) -> Result<(), SyntaxError> {
        if self.b[self.i..].starts_with(word) {
            self.i += word.len();
            Ok(())
        } else {
            Err(SyntaxError)
        }
    }

    fn hex4(&mut self) -> Result<u16, SyntaxError> {
        let s = self.b.get(self.i..self.i + 4).ok_or(SyntaxError)?;
        let mut n: u16 = 0;
        for &c in s {
            let d = (c as char).to_digit(16).ok_or(SyntaxError)?;
            n = n * 16 + d as u16;
        }
        self.i += 4;
        Ok(n)
    }

    /// A string token, the opening quote already current.
    fn string(&mut self) -> Result<JsString, SyntaxError> {
        self.eat(b'"')?;
        let mut out: Vec<u16> = Vec::new();
        loop {
            let c = self.peek().ok_or(SyntaxError)?;
            match c {
                b'"' => {
                    self.i += 1;
                    return Ok(JsString(out));
                }
                b'\\' => {
                    self.i += 1;
                    let e = self.peek().ok_or(SyntaxError)?;
                    self.i += 1;
                    let unit = match e {
                        b'"' => 0x22,
                        b'\\' => 0x5C,
                        b'/' => 0x2F,
                        b'b' => 0x08,
                        b'f' => 0x0C,
                        b'n' => 0x0A,
                        b'r' => 0x0D,
                        b't' => 0x09,
                        b'u' => self.hex4()?,
                        _ => return Err(SyntaxError),
                    };
                    out.push(unit);
                }
                0x00..=0x1F => return Err(SyntaxError),
                _ => {
                    // A run of ordinary characters; the input is valid UTF-8.
                    let start = self.i;
                    while self.i < self.b.len() && !matches!(self.b[self.i], b'"' | b'\\' | 0x00..=0x1F) {
                        self.i += 1;
                    }
                    let run = std::str::from_utf8(&self.b[start..self.i]).map_err(|_| SyntaxError)?;
                    out.extend(run.encode_utf16());
                }
            }
        }
    }

    /// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`, rounded to the
    /// nearest double (overflow is Infinity, as in V8).
    fn number(&mut self) -> Result<f64, SyntaxError> {
        let start = self.i;
        let digits = |p: &mut Self| {
            let s = p.i;
            while p.i < p.b.len() && p.b[p.i].is_ascii_digit() {
                p.i += 1;
            }
            p.i - s
        };
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        match self.peek() {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                digits(self);
            }
            _ => return Err(SyntaxError),
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            if digits(self) == 0 {
                return Err(SyntaxError);
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if digits(self) == 0 {
                return Err(SyntaxError);
            }
        }
        let text = std::str::from_utf8(&self.b[start..self.i]).map_err(|_| SyntaxError)?;
        text.parse::<f64>().map_err(|_| SyntaxError)
    }
}

/// `JSON.parse(text)` (no reviver).
pub fn parse(text: &str) -> Result<JsValue, SyntaxError> {
    let mut p = Parser { b: text.as_bytes(), i: 0 };
    let mut stack: Vec<Open> = Vec::new();
    loop {
        // Parse one value (or open a container and go round for its first member).
        p.skip_ws();
        let mut value = match p.peek().ok_or(SyntaxError)? {
            b'{' => {
                p.i += 1;
                p.skip_ws();
                if p.peek() == Some(b'}') {
                    p.i += 1;
                    JsValue::Object(JsObject::default())
                } else {
                    let key = p.string()?;
                    p.skip_ws();
                    p.eat(b':')?;
                    stack.push(Open::Object { entries: Vec::new(), key });
                    continue;
                }
            }
            b'[' => {
                p.i += 1;
                p.skip_ws();
                if p.peek() == Some(b']') {
                    p.i += 1;
                    JsValue::Array(Vec::new())
                } else {
                    stack.push(Open::Array(Vec::new()));
                    continue;
                }
            }
            b'"' => JsValue::String(p.string()?),
            b't' => {
                p.literal(b"true")?;
                JsValue::Bool(true)
            }
            b'f' => {
                p.literal(b"false")?;
                JsValue::Bool(false)
            }
            b'n' => {
                p.literal(b"null")?;
                JsValue::Null
            }
            _ => JsValue::Number(p.number()?),
        };

        // Hand the finished value to its container; close containers that end here.
        loop {
            let Some(top) = stack.last_mut() else {
                p.skip_ws();
                return if p.i == p.b.len() { Ok(value) } else { Err(SyntaxError) };
            };
            p.skip_ws();
            match top {
                Open::Array(items) => {
                    items.push(value);
                    match p.peek() {
                        Some(b',') => {
                            p.i += 1;
                            break;
                        }
                        Some(b']') => {
                            p.i += 1;
                            let Some(Open::Array(items)) = stack.pop() else { unreachable!() };
                            value = JsValue::Array(items);
                        }
                        _ => return Err(SyntaxError),
                    }
                }
                Open::Object { entries, key } => {
                    entries.push((std::mem::take(key), value));
                    match p.peek() {
                        Some(b',') => {
                            p.i += 1;
                            p.skip_ws();
                            *key = p.string()?;
                            p.skip_ws();
                            p.eat(b':')?;
                            break;
                        }
                        Some(b'}') => {
                            p.i += 1;
                            let Some(Open::Object { entries, .. }) = stack.pop() else { unreachable!() };
                            value = JsValue::Object(JsObject::from_entries(entries));
                        }
                        _ => return Err(SyntaxError),
                    }
                }
            }
        }
    }
}

// ── JSON.stringify ──────────────────────────────────────────────────────────

/// How deep `JSON.stringify` may nest before V8 runs out of stack, modelled
/// as a budget each open object or array spends.
///
/// V8's serializer recurses once per container and throws
/// `RangeError: Maximum call stack size exceeded` when the native stack runs
/// out, so the limit is a property of the runtime rather than of any code
/// upstream wrote. It is still observable (a request the reference answers
/// with a bare 500 must not be stored here), so it is modelled from
/// measurements of the reference: see [`STRINGIFY_STACK`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackBudget {
    pub object: u64,
    pub array: u64,
    pub total: u64,
}

/// `JSON.stringify(settings)` inside `parseClientSettingsBody`, measured on
/// the reference image (Node 24.22, V8 13.6): a chain of 4061 nested objects
/// serializes and 4062 throw; one object holding 4346 nested arrays
/// serializes and 4347 throw; alternating object/array pairs stop at 2099
/// pairs; 2000 objects then arrays stop at 2205 arrays, 2191 objects then
/// 2000 arrays stop there. An object frame costs about 1.0705 array frames;
/// these integers reproduce the pure cases exactly and the mixed ones to
/// within one level. Repeated runs of the reference moved by up to four
/// levels (JIT state), so the edge is approximate by nature.
pub const STRINGIFY_STACK: StackBudget = StackBudget { object: 2141, array: 2000, total: 8_695_000 };

/// `JSON.stringify(value)` (no replacer, no indentation).
pub fn stringify(value: &JsValue) -> String {
    let mut out = String::new();
    // Without a budget nothing can fail.
    let _ = write_value(&mut out, value, None);
    out
}

/// `JSON.stringify(value)`, throwing like V8 when `budget` runs out.
pub fn stringify_within(value: &JsValue, budget: StackBudget) -> Result<String, RangeError> {
    let mut out = String::new();
    write_value(&mut out, value, Some(budget))?;
    Ok(out)
}

enum Frame<'a> {
    Array(std::slice::Iter<'a, JsValue>, bool),
    Object(std::slice::Iter<'a, (JsString, JsValue)>, bool),
}

fn write_value(out: &mut String, root: &JsValue, budget: Option<StackBudget>) -> Result<(), RangeError> {
    let mut stack: Vec<Frame<'_>> = Vec::new();
    let mut spent: u64 = 0;
    let mut next: Option<&JsValue> = Some(root);
    loop {
        if let Some(value) = next.take() {
            match value {
                JsValue::Null => out.push_str("null"),
                JsValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
                JsValue::Number(n) => {
                    if n.is_finite() {
                        out.push_str(&number_to_string(*n));
                    } else {
                        out.push_str("null");
                    }
                }
                JsValue::String(s) => quote(out, s.units()),
                JsValue::Array(items) => {
                    if let Some(b) = budget {
                        spent += b.array;
                        if spent > b.total {
                            return Err(RangeError);
                        }
                    }
                    out.push('[');
                    stack.push(Frame::Array(items.iter(), true));
                }
                JsValue::Object(obj) => {
                    if let Some(b) = budget {
                        spent += b.object;
                        if spent > b.total {
                            return Err(RangeError);
                        }
                    }
                    out.push('{');
                    stack.push(Frame::Object(obj.entries.iter(), true));
                }
            }
        }
        // Continue the innermost open container.
        let Some(top) = stack.last_mut() else {
            return Ok(());
        };
        match top {
            Frame::Array(iter, first) => match iter.next() {
                Some(v) => {
                    if !*first {
                        out.push(',');
                    }
                    *first = false;
                    next = Some(v);
                }
                None => {
                    out.push(']');
                    stack.pop();
                    if let Some(b) = budget {
                        spent -= b.array;
                    }
                }
            },
            Frame::Object(iter, first) => match iter.next() {
                Some((k, v)) => {
                    if !*first {
                        out.push(',');
                    }
                    *first = false;
                    quote(out, k.units());
                    out.push(':');
                    next = Some(v);
                }
                None => {
                    out.push('}');
                    stack.pop();
                    if let Some(b) = budget {
                        spent -= b.object;
                    }
                }
            },
        }
    }
}

/// `QuoteJSONString` (ES2019 well-formed): the two-character escapes, other
/// controls and lone surrogates as lower-case `\uXXXX`, everything else as is.
fn quote(out: &mut String, units: &[u16]) {
    out.push('"');
    for item in char::decode_utf16(units.iter().copied()) {
        match item {
            Ok('"') => out.push_str("\\\""),
            Ok('\\') => out.push_str("\\\\"),
            Ok('\u{8}') => out.push_str("\\b"),
            Ok('\u{c}') => out.push_str("\\f"),
            Ok('\n') => out.push_str("\\n"),
            Ok('\r') => out.push_str("\\r"),
            Ok('\t') => out.push_str("\\t"),
            Ok(c) if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            Ok(c) => out.push(c),
            Err(lone) => {
                let _ = write!(out, "\\u{:04x}", lone.unpaired_surrogate());
            }
        }
    }
    out.push('"');
}

/// `Number::toString(x)` (radix 10): the shortest digits that round-trip,
/// laid out by the ECMAScript rules (`1e+21`, `1e-7`, `0.000001`, `100`).
pub fn number_to_string(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x == 0.0 {
        return "0".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if x < 0.0 {
        return format!("-{}", number_to_string(-x));
    }
    // Rust's `{:e}` prints the shortest round-trip digits: "1.2345e-7".
    let sci = format!("{x:e}");
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i64;
    let n = exp.parse::<i64>().unwrap_or(0) + 1;
    if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e = n - 1;
        let sign = if e < 0 { '-' } else { '+' };
        if k == 1 {
            format!("{digits}e{sign}{}", e.abs())
        } else {
            format!("{}.{}e{sign}{}", &digits[..1], &digits[1..], e.abs())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(text: &str) -> String {
        stringify(&parse(text).unwrap_or_else(|_| panic!("parse failed: {text}")))
    }

    /// `JSON.stringify(JSON.parse(input))` as printed by the reference
    /// container's node (v24.22.0, V8 13.6).
    #[test]
    fn round_trips_like_v8() {
        let cases: &[(&str, &str)] = &[
            (
                r#"{"b":1,"4294967295":2,"4294967294":3,"1":4,"01":5,"a":6,"b":7}"#,
                r#"{"1":4,"4294967294":3,"b":7,"4294967295":2,"01":5,"a":6}"#,
            ),
            (r#"{"10":1,"9":2,"-1":3,"1.5":4,"0":5}"#, r#"{"0":5,"9":2,"10":1,"-1":3,"1.5":4}"#),
            (
                r#"[1.0, -0, 1e21, 1E-7, 123e-20, 0.000001, 1e400, -1e400, 5e-324, 12345678901234567890]"#,
                "[1,0,1e+21,1e-7,1.23e-18,0.000001,null,null,5e-324,12345678901234567000]",
            ),
            (
                r#"[0.1, 0.30000000000000004, 1.7976931348623157e308, 9007199254740993, 100, 1e20, 2.5e-7]"#,
                "[0.1,0.30000000000000004,1.7976931348623157e+308,9007199254740992,100,100000000000000000000,2.5e-7]",
            ),
            (r#""\ud800""#, r#""\ud800""#),
            (
                r#""\uD83D\uDE00 \u0000 \u001f \u007f \u2028 \/ \b\f\n\r\t""#,
                "\"\u{1F600} \\u0000 \\u001f \u{7f} \u{2028} / \\b\\f\\n\\r\\t\"",
            ),
            (r#""\udc00\ud800x""#, r#""\udc00\ud800x""#),
            (" \t\n\r{ \"a\" : [ ] , \"b\" : { } } \r\n", r#"{"a":[],"b":{}}"#),
            (r#"{"__proto__":{"x":1},"toJSON":2}"#, r#"{"__proto__":{"x":1},"toJSON":2}"#),
            ("\"é\\\\u0000\"", r#""é\\u0000""#),
        ];
        for (input, expected) in cases {
            assert_eq!(round_trip(input), *expected, "{input}");
        }
    }

    #[test]
    fn refuses_what_json_parse_refuses() {
        for bad in [
            "",
            " ",
            "{",
            "[1,]",
            "{\"a\":1,}",
            "01",
            "1.",
            ".5",
            "+1",
            "-",
            "1e",
            "1e+",
            "NaN",
            "Infinity",
            "'a'",
            "{a:1}",
            "\"\\x41\"",
            "\"\\u12\"",
            "\"a\nb\"",
            "\u{a0}1",
            "\u{feff}1",
            "[1] x",
            "{\"a\" 1}",
            "tru",
            "nul",
            "[",
            "[1",
            "{\"a\":",
            "\"abc",
        ] {
            assert_eq!(parse(bad), Err(SyntaxError), "{bad:?}");
        }
    }

    #[test]
    fn nesting_is_not_limited_by_the_rust_stack() {
        let depth = 200_000;
        let text = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        let v = parse(&text).unwrap();
        assert_eq!(stringify(&v), text);
        let objects = format!("{}1{}", "{\"a\":".repeat(depth), "}".repeat(depth));
        assert_eq!(stringify(&parse(&objects).unwrap()), objects);
    }

    #[test]
    fn the_stack_budget_reproduces_the_reference_edges() {
        let objects = |n: usize| parse(&format!("{}1{}", "{\"a\":".repeat(n), "}".repeat(n))).unwrap();
        let arrays = |m: usize| parse(&format!("{{\"a\":{}{}}}", "[".repeat(m), "]".repeat(m))).unwrap();
        let pairs = |k: usize| parse(&format!("{}1{}", "{\"a\":[".repeat(k), "]}".repeat(k))).unwrap();
        assert!(stringify_within(&objects(4061), STRINGIFY_STACK).is_ok());
        assert_eq!(stringify_within(&objects(4062), STRINGIFY_STACK), Err(RangeError));
        assert!(stringify_within(&arrays(4346), STRINGIFY_STACK).is_ok());
        assert_eq!(stringify_within(&arrays(4347), STRINGIFY_STACK), Err(RangeError));
        assert!(stringify_within(&pairs(2099), STRINGIFY_STACK).is_ok());
        assert_eq!(stringify_within(&pairs(2100), STRINGIFY_STACK), Err(RangeError));
        // Width costs nothing: only the open containers count.
        let wide = parse(&format!("[{}]", vec!["[[1]]"; 10_000].join(","))).unwrap();
        assert!(stringify_within(&wide, STRINGIFY_STACK).is_ok());
    }

    #[test]
    fn numbers_print_like_javascript() {
        for (n, s) in [
            (0.0, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (-1.5, "-1.5"),
            (1e21, "1e+21"),
            (1e-7, "1e-7"),
            (1.5e-7, "1.5e-7"),
            (0.000001, "0.000001"),
            (123456789.0, "123456789"),
            (9007199254740992.0, "9007199254740992"),
            (9223372036854775807.0, "9223372036854776000"),
            (f64::MAX, "1.7976931348623157e+308"),
            (f64::NAN, "NaN"),
            (f64::NEG_INFINITY, "-Infinity"),
        ] {
            assert_eq!(number_to_string(n), s, "{n}");
        }
    }

    #[test]
    fn trims_and_compares_like_javascript() {
        let parsed = parse("\"\\u00a0 terminal\\ufeff\\n\"").unwrap();
        let JsValue::String(s) = &parsed else { panic!() };
        assert!(s.trim().eq_str("terminal"));
        let parsed = parse(r#"{"theme":"a","theme":"paper"}"#).unwrap();
        let JsValue::Object(o) = &parsed else { panic!() };
        assert_eq!(o.get("theme"), Some(&JsValue::from("paper")));
        assert_eq!(o.len(), 1);
    }
}
