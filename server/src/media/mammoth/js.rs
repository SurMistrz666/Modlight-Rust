//! The JavaScript semantics the mammoth port depends on: what `\s` and
//! `trim()` mean, `parseInt`, `ToNumber`, `ToInt32`, how a plain `{}` used as
//! a map behaves (`Object.prototype` members show through, `__proto__` is not
//! an ordinary key, integer-like keys enumerate first), and JavaScript's
//! UTF-16 strings.
//!
//! Strings. A JavaScript string can hold a lone UTF-16 surrogate (an XML
//! character reference such as `&#xD800;` produces one), a Rust `String`
//! cannot. Inside the port a lone surrogate `0xD800 + k` is carried as the
//! code point `U+10F800 + k` (the last 2048 code points of Supplementary
//! Private Use Area-B), so concatenating two strings can still form a pair the
//! way it does in JavaScript; [`resolve_surrogates`] turns the finished HTML
//! into what `TextEncoder`/`Response` would send (pairs joined, lone halves
//! U+FFFD) and [`utf16_len`] measures `string.length`. A document that holds
//! those private-use code points literally would see them treated as
//! surrogates; nothing real uses them.

use std::collections::HashMap;

/// `\s` / `String.prototype.trim` whitespace: WhiteSpace + LineTerminator.
/// (Rust's `char::is_whitespace` has U+0085 and lacks U+FEFF.)
pub fn js_is_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{9}' | '\u{a}' | '\u{b}' | '\u{c}' | '\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// The same set as a regex character class body (for the `regex` crate).
pub const JS_WHITESPACE_CLASS: &str =
    r"\t\n\x0B\x0C\r \u{A0}\u{1680}\u{2000}-\u{200A}\u{2028}\u{2029}\u{202F}\u{205F}\u{3000}\u{FEFF}";

/// `.` in a non-unicode JavaScript regex: anything but a line terminator.
pub const JS_DOT_CLASS: &str = r"[^\n\r\u{2028}\u{2029}]";

/// `String.prototype.trim`.
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(js_is_whitespace)
}

/// `/^\s*$/.test(s)`.
pub fn js_is_blank(s: &str) -> bool {
    s.chars().all(js_is_whitespace)
}

// ---------------------------------------------------------------------------
// Strings as UTF-16
// ---------------------------------------------------------------------------

const MAPPED_BASE: u32 = 0x10_F800;

fn map_lone(unit: u16) -> char {
    char::from_u32(MAPPED_BASE + (unit as u32 - 0xD800)).expect("mapped surrogate")
}

fn mapped_unit(c: char) -> Option<u16> {
    let v = c as u32;
    (v >= MAPPED_BASE).then(|| (v - MAPPED_BASE) as u16 + 0xD800)
}

/// Append UTF-16 code units to a string, carrying lone surrogates (see the
/// module docs).
pub fn push_utf16(out: &mut String, units: &[u16]) {
    for r in char::decode_utf16(units.iter().copied()) {
        match r {
            Ok(c) => out.push(c),
            Err(e) => out.push(map_lone(e.unpaired_surrogate())),
        }
    }
}

/// `String.fromCharCode(...)` of already-wrapped code units.
pub fn from_utf16(units: &[u16]) -> String {
    let mut s = String::new();
    push_utf16(&mut s, units);
    s
}

/// The UTF-16 code units of a port string.
pub fn to_utf16(s: &str) -> Vec<u16> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        if let Some(u) = mapped_unit(c) {
            out.push(u);
        } else {
            let mut buf = [0u16; 2];
            out.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    out
}

/// `string.length`.
pub fn utf16_len(s: &str) -> usize {
    s.chars().map(|c| if mapped_unit(c).is_some() || (c as u32) < 0x1_0000 { 1 } else { 2 }).sum()
}

/// What a JavaScript string becomes on the wire (`Response`, `Buffer.from`):
/// surrogate pairs joined, lone surrogates U+FFFD.
pub fn resolve_surrogates(s: &str) -> String {
    if !s.chars().any(|c| mapped_unit(c).is_some()) {
        return s.to_owned();
    }
    String::from_utf16_lossy(&to_utf16(s))
}

// ---------------------------------------------------------------------------
// Numbers
// ---------------------------------------------------------------------------

/// `ToInt32`.
pub fn to_int32(x: f64) -> i32 {
    if !x.is_finite() {
        return 0;
    }
    let m = x.trunc().rem_euclid(4_294_967_296.0);
    if m >= 2_147_483_648.0 { (m - 4_294_967_296.0) as i32 } else { m as i32 }
}

/// `ToUint16` (what `String.fromCharCode` applies).
pub fn to_uint16(x: f64) -> u16 {
    if !x.is_finite() {
        return 0;
    }
    x.trunc().rem_euclid(65_536.0) as u16
}

fn digit_value(c: char, radix: u32) -> Option<u32> {
    c.to_digit(36).filter(|d| *d < radix)
}

/// A digit string in a power-of-two radix, rounded to the nearest double
/// (ties to even), as V8 does.
fn binary_radix_to_f64(digits: &[u32], bits_per_digit: u32) -> f64 {
    let mut bits: Vec<bool> = Vec::with_capacity(digits.len() * bits_per_digit as usize);
    for &d in digits {
        for i in (0..bits_per_digit).rev() {
            bits.push((d >> i) & 1 == 1);
        }
    }
    let first = match bits.iter().position(|b| *b) {
        Some(i) => i,
        None => return 0.0,
    };
    let bits = &bits[first..];
    if bits.len() <= 53 {
        return bits.iter().fold(0.0, |acc, b| acc * 2.0 + if *b { 1.0 } else { 0.0 });
    }
    let mut mantissa: u64 = bits[..53].iter().fold(0, |acc, b| (acc << 1) | *b as u64);
    let half = bits[53];
    let sticky = bits[54..].iter().any(|b| *b);
    if half && (sticky || mantissa & 1 == 1) {
        mantissa += 1;
    }
    let exponent = (bits.len() - 53) as i32;
    (mantissa as f64) * 2f64.powi(exponent)
}

fn digits_to_f64(digits: &str, radix: u32) -> f64 {
    match radix {
        10 => digits.parse::<f64>().unwrap_or(f64::NAN),
        2 | 4 | 8 | 16 | 32 => {
            let values: Vec<u32> = digits.chars().filter_map(|c| digit_value(c, radix)).collect();
            binary_radix_to_f64(&values, radix.trailing_zeros())
        }
        _ => digits.chars().filter_map(|c| digit_value(c, radix)).fold(0.0, |acc, d| acc * radix as f64 + d as f64),
    }
}

/// `parseInt(string, radix)` (`radix` `None` = undefined).
pub fn parse_int(input: &str, radix: Option<i32>) -> f64 {
    let mut s = input.trim_start_matches(js_is_whitespace);
    let mut sign = 1.0;
    if let Some(rest) = s.strip_prefix('-') {
        sign = -1.0;
        s = rest;
    } else if let Some(rest) = s.strip_prefix('+') {
        s = rest;
    }
    let mut r = radix.unwrap_or(0);
    let mut strip_prefix = true;
    if r != 0 {
        if !(2..=36).contains(&r) {
            return f64::NAN;
        }
        if r != 16 {
            strip_prefix = false;
        }
    } else {
        r = 10;
    }
    if strip_prefix && (s.starts_with("0x") || s.starts_with("0X")) {
        s = &s[2..];
        r = 16;
    }
    let end = s.char_indices().find(|(_, c)| digit_value(*c, r as u32).is_none()).map_or(s.len(), |(i, _)| i);
    let digits = &s[..end];
    if digits.is_empty() {
        return f64::NAN;
    }
    sign * digits_to_f64(digits, r as u32)
}

/// `ToNumber(string)` (what `"1" == 1` compares).
pub fn string_to_number(input: &str) -> f64 {
    let s = js_trim(input);
    if s.is_empty() {
        return 0.0;
    }
    for (prefix, radix) in [("0x", 16), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)] {
        if let Some(rest) = s.strip_prefix(prefix) {
            if rest.is_empty() || rest.chars().any(|c| digit_value(c, radix).is_none()) {
                return f64::NAN;
            }
            return digits_to_f64(rest, radix);
        }
    }
    let (sign, body) = match s.as_bytes()[0] {
        b'-' => (-1.0, &s[1..]),
        b'+' => (1.0, &s[1..]),
        _ => (1.0, s),
    };
    if body == "Infinity" {
        return sign * f64::INFINITY;
    }
    // StrUnsignedDecimalLiteral: digits [. digits] | . digits, then [eE][+-]digits.
    let bytes = body.as_bytes();
    let mut i = 0;
    let int_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    let int_digits = i - int_start;
    let mut frac_digits = 0;
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        let f = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        frac_digits = i - f;
    }
    if int_digits == 0 && frac_digits == 0 {
        return f64::NAN;
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        let e = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == e {
            return f64::NAN;
        }
    }
    if i != bytes.len() {
        return f64::NAN;
    }
    sign * body.parse::<f64>().unwrap_or(f64::NAN)
}

/// `String(number)`.
pub fn number_to_string(x: f64) -> String {
    crate::js_json::number_to_string(x)
}

// ---------------------------------------------------------------------------
// Plain objects used as maps
// ---------------------------------------------------------------------------

/// The properties every `{}` inherits from `Object.prototype` (node 24).
const OBJECT_PROTOTYPE: [&str; 12] = [
    "constructor",
    "__defineGetter__",
    "__defineSetter__",
    "hasOwnProperty",
    "__lookupGetter__",
    "__lookupSetter__",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toString",
    "valueOf",
    "__proto__",
    "toLocaleString",
];

/// `key` names something every plain object inherits (a function, or for
/// `__proto__` the prototype object itself): always truthy.
pub fn builtin(key: &str) -> Option<&'static str> {
    OBJECT_PROTOTYPE.iter().copied().find(|k| *k == key)
}

/// `String(value)` of an inherited member.
pub fn builtin_to_string(name: &str) -> String {
    match name {
        "__proto__" => "[object Object]".into(),
        "constructor" => "function Object() { [native code] }".into(),
        other => format!("function {other}() {{ [native code] }}"),
    }
}

/// `value.name` of an inherited member (`undefined` for the prototype object).
pub fn builtin_name(name: &str) -> Option<String> {
    match name {
        "__proto__" => None,
        "constructor" => Some("Object".into()),
        other => Some(other.into()),
    }
}

/// Is `key` a canonical array index (enumerated first, in numeric order)?
fn array_index(key: &str) -> Option<u32> {
    if key == "0" {
        return Some(0);
    }
    if key.is_empty() || key.starts_with('0') || !key.bytes().all(|b| b.is_ascii_digit()) || key.len() > 10 {
        return None;
    }
    key.parse::<u64>().ok().filter(|n| *n < 4_294_967_295).map(|n| n as u32)
}

/// What `obj[key]` finds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookup<'a, V> {
    Own(&'a V),
    /// An `Object.prototype` member: truthy, not a value of the map's type.
    Builtin(&'static str),
    Missing,
}

impl<'a, V> Lookup<'a, V> {
    pub fn own(self) -> Option<&'a V> {
        match self {
            Lookup::Own(v) => Some(v),
            _ => None,
        }
    }

    pub fn is_missing(&self) -> bool {
        matches!(self, Lookup::Missing)
    }
}

/// A plain JavaScript object used as a string-keyed map.
///
/// Assigning to `__proto__` is not a property write in JavaScript: a primitive
/// is ignored, an object becomes the prototype. Here an object assigned to
/// `__proto__` is remembered as that key's value but, like the real thing, is
/// not enumerated; lookups of OTHER keys do not see its properties (in
/// JavaScript they would, through the prototype chain).
#[derive(Debug, Clone)]
pub struct JsObject<V> {
    keys: Vec<String>,
    map: HashMap<String, V>,
    proto: Option<V>,
}

impl<V> Default for JsObject<V> {
    fn default() -> Self {
        JsObject { keys: Vec::new(), map: HashMap::new(), proto: None }
    }
}

impl<V> JsObject<V> {
    pub fn new() -> Self {
        Self::default()
    }

    /// `obj[key] = value` where `value` is an object.
    pub fn set(&mut self, key: &str, value: V) {
        if key == "__proto__" {
            self.proto = Some(value);
            return;
        }
        if !self.map.contains_key(key) {
            self.keys.push(key.to_owned());
        }
        self.map.insert(key.to_owned(), value);
    }

    /// `obj[key] = value` where `value` is a primitive (a string, a number,
    /// `undefined`): ignored for `__proto__`.
    pub fn set_primitive(&mut self, key: &str, value: V) {
        if key != "__proto__" {
            self.set(key, value);
        }
    }

    /// `obj[key]`.
    pub fn get(&self, key: &str) -> Lookup<'_, V> {
        if let Some(v) = self.map.get(key) {
            return Lookup::Own(v);
        }
        if key == "__proto__"
            && let Some(p) = &self.proto
        {
            return Lookup::Own(p);
        }
        match builtin(key) {
            Some(b) => Lookup::Builtin(b),
            None => Lookup::Missing,
        }
    }

    /// `Object.prototype.hasOwnProperty.call(obj, key)`.
    pub fn has_own(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    /// Own enumerable keys in JavaScript order: array indices ascending, then
    /// the rest in insertion order.
    pub fn keys(&self) -> Vec<&str> {
        let mut indexed: Vec<(u32, &str)> =
            self.keys.iter().filter_map(|k| array_index(k).map(|i| (i, k.as_str()))).collect();
        indexed.sort_by_key(|(i, _)| *i);
        let mut out: Vec<&str> = indexed.into_iter().map(|(_, k)| k).collect();
        out.extend(self.keys.iter().filter(|k| array_index(k).is_none()).map(|k| k.as_str()));
        out
    }

    /// `_.values(obj)`.
    pub fn values(&self) -> Vec<&V> {
        self.keys().into_iter().map(|k| &self.map[k]).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// Order `(key, value)` pairs the way `Object.keys` would list them.
pub fn js_key_order<T>(pairs: Vec<(String, T)>) -> Vec<(String, T)> {
    let (mut indexed, rest): (Vec<_>, Vec<_>) = pairs.into_iter().partition(|(k, _)| array_index(k).is_some());
    indexed.sort_by_key(|(k, _)| array_index(k).unwrap_or(0));
    indexed.extend(rest);
    indexed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_int_matches_v8() {
        assert_eq!(parse_int("61505", Some(16)), 398_597.0);
        assert_eq!(parse_int("F041", Some(16)), 61_505.0);
        assert_eq!(parse_int("0xF041", Some(16)), 61_505.0);
        assert_eq!(parse_int(" 41", Some(16)), 65.0);
        assert!(parse_int("zz", Some(16)).is_nan());
        assert_eq!(parse_int("2abc", Some(10)), 2.0);
        assert_eq!(parse_int("0x41", None), 65.0);
        assert_eq!(parse_int("00x41", None), 0.0);
        assert_eq!(parse_int("10x", None), 10.0);
        assert!(parse_int("X41", None).is_nan());
        assert_eq!(parse_int("-0", Some(10)).to_bits(), (-0.0f64).to_bits());
        assert_eq!(parse_int(&"9".repeat(400), Some(10)), f64::INFINITY);
        assert_eq!(parse_int("123456789012345678901234", Some(10)), 1.2345678901234568e23);
        assert_eq!(parse_int("1fffffffffffff1", Some(16)), 144115188075855860.0);
    }

    #[test]
    fn number_semantics() {
        assert_eq!(to_int32(4_294_967_296.0 + 5.0), 5);
        assert_eq!(to_int32(2_147_483_648.0), -2_147_483_648);
        assert_eq!(to_int32(f64::NAN), 0);
        assert_eq!(to_uint16(-1.0), 65535);
        assert_eq!(string_to_number(" 1 "), 1.0);
        assert_eq!(string_to_number(""), 0.0);
        assert_eq!(string_to_number("0x10"), 16.0);
        assert_eq!(string_to_number("1e0"), 1.0);
        assert!(string_to_number("1_0").is_nan());
        assert!(string_to_number("-0x1").is_nan());
        assert_eq!(string_to_number(".5"), 0.5);
        assert!(string_to_number(".").is_nan());
    }

    #[test]
    fn surrogates_travel_like_javascript() {
        let lead = from_utf16(&[0xD83D]);
        let trail = from_utf16(&[0xDE00]);
        assert_eq!(utf16_len(&lead), 1);
        let joined = format!("{lead}{trail}");
        assert_eq!(resolve_surrogates(&joined), "\u{1F600}");
        assert_eq!(resolve_surrogates(&lead), "\u{FFFD}");
        assert_eq!(utf16_len("a\u{1F600}"), 3);
    }

    #[test]
    fn objects_behave_like_plain_objects() {
        let mut o: JsObject<i32> = JsObject::new();
        o.set("b", 1);
        o.set("10", 2);
        o.set("2", 3);
        o.set_primitive("__proto__", 4);
        assert_eq!(o.keys(), vec!["2", "10", "b"]);
        assert_eq!(o.get("__proto__"), Lookup::Builtin("__proto__"));
        assert_eq!(o.get("toString"), Lookup::Builtin("toString"));
        assert_eq!(o.get("x"), Lookup::Missing);
        o.set("toString", 5);
        assert_eq!(o.get("toString"), Lookup::Own(&5));
    }
}
