//! ICU message formatting, as next-intl (intl-messageformat) does it, for the
//! syntax the dictionaries use: `{arg}`, `{n, plural, =0 {..} one {..} ...}`
//! with `#`, `{v, select, ...}`, `{n, number}` and ICU apostrophe quoting.
//!
//! Rules reproduced from intl-messageformat:
//! - a simple `{arg}` is `String(value)` (no locale formatting);
//! - `#` and `{n, number}` use `Intl.NumberFormat(locale)`;
//! - a plural tries `=<value>` first, then the CLDR category of `value - offset`,
//!   then `other`;
//! - a missing argument is an error (next-intl then shows `namespace.key`).
//!
//! Rich-text tags (`<b>..</b>`) are only meaningful to `t.rich` in the browser;
//! here they are passed through as text.

use crate::i18n::Locale;

/// A value for a placeholder.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Str(String),
    Num(f64),
}

impl From<&str> for Arg {
    fn from(v: &str) -> Self {
        Arg::Str(v.to_string())
    }
}
impl From<String> for Arg {
    fn from(v: String) -> Self {
        Arg::Str(v)
    }
}
impl From<i64> for Arg {
    fn from(v: i64) -> Self {
        Arg::Num(v as f64)
    }
}
impl From<i32> for Arg {
    fn from(v: i32) -> Self {
        Arg::Num(f64::from(v))
    }
}
impl From<usize> for Arg {
    fn from(v: usize) -> Self {
        Arg::Num(v as f64)
    }
}
impl From<f64> for Arg {
    fn from(v: f64) -> Self {
        Arg::Num(v)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FormatError {
    #[error("malformed message: {0}")]
    Syntax(String),
    #[error("missing value for argument \"{0}\"")]
    MissingValue(String),
    #[error("no matching option for \"{0}\"")]
    NoOption(String),
}

/// Format, falling back to the raw text on any error (for logs and tests that
/// do not care). Production callers go through `Messages::t`, which falls back
/// to the key the way next-intl does.
pub fn format_message(text: &str, locale: Locale, args: &[(&str, Arg)]) -> String {
    match try_format_message(text, locale, args) {
        Ok(s) => s,
        Err(err) => {
            tracing::error!("[i18n] {err} in {text:?}");
            text.to_string()
        }
    }
}

pub fn try_format_message(text: &str, locale: Locale, args: &[(&str, Arg)]) -> Result<String, FormatError> {
    let chars: Vec<char> = text.chars().collect();
    let mut parser = Parser { chars: &chars, pos: 0 };
    let nodes = parser.parse_nodes(false, 0)?;
    if parser.pos != chars.len() {
        return Err(FormatError::Syntax("unexpected '}'".into()));
    }
    let mut out = String::new();
    render(&nodes, locale, args, None, &mut out)?;
    Ok(out)
}

#[derive(Debug)]
enum Node {
    Text(String),
    Pound,
    Arg(String),
    Number(String),
    Plural { name: String, offset: f64, options: Vec<(String, Vec<Node>)> },
    Select { name: String, options: Vec<(String, Vec<Node>)> },
}

struct Parser<'a> {
    chars: &'a [char],
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    /// Text and placeholders until a closing `}` (when nested) or the end.
    fn parse_nodes(&mut self, nested: bool, plural_depth: usize) -> Result<Vec<Node>, FormatError> {
        let mut nodes = Vec::new();
        let mut text = String::new();
        while let Some(c) = self.peek() {
            match c {
                '{' => {
                    if !text.is_empty() {
                        nodes.push(Node::Text(std::mem::take(&mut text)));
                    }
                    self.pos += 1;
                    nodes.push(self.parse_argument(plural_depth)?);
                }
                '}' => {
                    if nested {
                        break;
                    }
                    return Err(FormatError::Syntax("unbalanced '}'".into()));
                }
                '#' if plural_depth > 0 => {
                    if !text.is_empty() {
                        nodes.push(Node::Text(std::mem::take(&mut text)));
                    }
                    self.pos += 1;
                    nodes.push(Node::Pound);
                }
                '\'' => {
                    self.pos += 1;
                    match self.peek() {
                        Some('\'') => {
                            text.push('\'');
                            self.pos += 1;
                        }
                        Some(n) if n == '{' || n == '}' || (n == '#' && plural_depth > 0) => {
                            // Quoted literal until the next lone apostrophe.
                            while let Some(q) = self.peek() {
                                self.pos += 1;
                                if q == '\'' {
                                    if self.peek() == Some('\'') {
                                        text.push('\'');
                                        self.pos += 1;
                                        continue;
                                    }
                                    break;
                                }
                                text.push(q);
                            }
                        }
                        _ => text.push('\''),
                    }
                }
                _ => {
                    text.push(c);
                    self.pos += 1;
                }
            }
        }
        if nested && self.peek() != Some('}') {
            return Err(FormatError::Syntax("unclosed '{'".into()));
        }
        if !text.is_empty() {
            nodes.push(Node::Text(text));
        }
        Ok(nodes)
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }

    fn ident(&mut self) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_whitespace() || c == ',' || c == '}' || c == '{' {
                break;
            }
            s.push(c);
            self.pos += 1;
        }
        s
    }

    fn expect(&mut self, c: char) -> Result<(), FormatError> {
        if self.peek() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            Err(FormatError::Syntax(format!("expected '{c}'")))
        }
    }

    /// After `{`: `name}` or `name, type[, style]}`.
    fn parse_argument(&mut self, plural_depth: usize) -> Result<Node, FormatError> {
        self.skip_ws();
        let name = self.ident();
        if name.is_empty() {
            return Err(FormatError::Syntax("empty argument".into()));
        }
        self.skip_ws();
        if self.peek() == Some('}') {
            self.pos += 1;
            return Ok(Node::Arg(name));
        }
        self.expect(',')?;
        self.skip_ws();
        let kind = self.ident();
        self.skip_ws();
        match kind.as_str() {
            "number" => {
                // Styles (`, integer`, `::...`) are not used by the dictionaries.
                while let Some(c) = self.peek() {
                    self.pos += 1;
                    if c == '}' {
                        return Ok(Node::Number(name));
                    }
                }
                Err(FormatError::Syntax("unclosed number argument".into()))
            }
            "plural" | "select" => {
                self.expect(',')?;
                let mut offset = 0.0;
                let mut options = Vec::new();
                loop {
                    self.skip_ws();
                    match self.peek() {
                        Some('}') => {
                            self.pos += 1;
                            break;
                        }
                        None => return Err(FormatError::Syntax("unclosed plural".into())),
                        _ => {}
                    }
                    let key = self.ident();
                    if kind == "plural" && key.starts_with("offset:") {
                        offset = key["offset:".len()..].parse().unwrap_or(0.0);
                        continue;
                    }
                    if key.is_empty() {
                        return Err(FormatError::Syntax("empty option".into()));
                    }
                    self.skip_ws();
                    self.expect('{')?;
                    let depth = if kind == "plural" { plural_depth + 1 } else { plural_depth };
                    let body = self.parse_nodes(true, depth)?;
                    self.expect('}')?;
                    options.push((key, body));
                }
                if kind == "plural" {
                    Ok(Node::Plural { name, offset, options })
                } else {
                    Ok(Node::Select { name, options })
                }
            }
            other => Err(FormatError::Syntax(format!("unsupported argument type {other}"))),
        }
    }
}

fn lookup<'a>(args: &'a [(&str, Arg)], name: &str) -> Result<&'a Arg, FormatError> {
    args.iter().find(|(k, _)| *k == name).map(|(_, v)| v).ok_or_else(|| FormatError::MissingValue(name.to_string()))
}

fn render(
    nodes: &[Node],
    locale: Locale,
    args: &[(&str, Arg)],
    pound: Option<f64>,
    out: &mut String,
) -> Result<(), FormatError> {
    for node in nodes {
        match node {
            Node::Text(t) => out.push_str(t),
            Node::Pound => match pound {
                Some(n) => out.push_str(&format_number(n, locale)),
                None => out.push('#'),
            },
            Node::Arg(name) => match lookup(args, name)? {
                Arg::Str(s) => out.push_str(s),
                Arg::Num(n) => out.push_str(&js_number_to_string(*n)),
            },
            Node::Number(name) => match lookup(args, name)? {
                Arg::Num(n) => out.push_str(&format_number(*n, locale)),
                Arg::Str(s) => out.push_str(&format_number(crate::config::js_number(s), locale)),
            },
            Node::Plural { name, offset, options } => {
                let value = match lookup(args, name)? {
                    Arg::Num(n) => *n,
                    Arg::Str(s) => crate::config::js_number(s),
                };
                let exact = format!("={}", js_number_to_string(value));
                let category = plural_category(locale, value - offset);
                let chosen = options
                    .iter()
                    .find(|(k, _)| *k == exact)
                    .or_else(|| options.iter().find(|(k, _)| k == category))
                    .or_else(|| options.iter().find(|(k, _)| k == "other"))
                    .ok_or_else(|| FormatError::NoOption(name.clone()))?;
                render(&chosen.1, locale, args, Some(value - offset), out)?;
            }
            Node::Select { name, options } => {
                let value = match lookup(args, name)? {
                    Arg::Str(s) => s.clone(),
                    Arg::Num(n) => js_number_to_string(*n),
                };
                let chosen = options
                    .iter()
                    .find(|(k, _)| *k == value)
                    .or_else(|| options.iter().find(|(k, _)| k == "other"))
                    .ok_or_else(|| FormatError::NoOption(name.clone()))?;
                render(&chosen.1, locale, args, pound, out)?;
            }
        }
    }
    Ok(())
}

/// CLDR cardinal plural category (`Intl.PluralRules(locale).select(n)`).
pub fn plural_category(locale: Locale, n: f64) -> &'static str {
    let integer = n.fract() == 0.0;
    let i = n.abs().trunc() as u64;
    match locale {
        Locale::En => {
            if integer && i == 1 {
                "one"
            } else {
                "other"
            }
        }
        Locale::Pl => {
            if !integer {
                return "other";
            }
            let (m10, m100) = (i % 10, i % 100);
            if i == 1 {
                "one"
            } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
                "few"
            } else {
                "many"
            }
        }
    }
}

/// `String(n)` for the numbers messages receive.
pub fn js_number_to_string(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n.is_infinite() {
        if n > 0.0 { "Infinity".into() } else { "-Infinity".into() }
    } else if n.fract() == 0.0 && n.abs() < 1e21 {
        format!("{}", n as i128)
    } else {
        format!("{n}")
    }
}

/// `new Intl.NumberFormat(locale).format(n)`: at most 3 fraction digits,
/// en groups with "," from 4 digits, pl groups with U+00A0 from 5 digits.
pub fn format_number(n: f64, locale: Locale) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 { "∞".into() } else { "-∞".into() };
    }
    let negative = n < 0.0;
    let rounded = format!("{:.3}", n.abs());
    let (int_part, frac_part) = rounded.split_once('.').unwrap_or((&rounded, ""));
    let frac = frac_part.trim_end_matches('0');
    let (sep, min_digits, decimal) = match locale {
        Locale::En => (",", 4, "."),
        Locale::Pl => ("\u{a0}", 5, ","),
    };
    let grouped = if int_part.len() >= min_digits {
        let mut s = String::new();
        let len = int_part.len();
        for (idx, c) in int_part.chars().enumerate() {
            if idx > 0 && (len - idx) % 3 == 0 {
                s.push_str(sep);
            }
            s.push(c);
        }
        s
    } else {
        int_part.to_string()
    };
    let mut out = String::new();
    if negative && (grouped != "0" || !frac.is_empty()) {
        out.push('-');
    }
    out.push_str(&grouped);
    if !frac.is_empty() {
        out.push_str(decimal);
        out.push_str(frac);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(text: &str, locale: Locale, args: &[(&str, Arg)]) -> String {
        try_format_message(text, locale, args).unwrap()
    }

    #[test]
    fn simple_arguments() {
        assert_eq!(f("Hello {name}!", Locale::En, &[("name", "Ala".into())]), "Hello Ala!");
        assert_eq!(f("{n} files", Locale::En, &[("n", 1234.into())]), "1234 files");
        assert!(matches!(try_format_message("{x}", Locale::En, &[]), Err(FormatError::MissingValue(_))));
    }

    #[test]
    fn english_plurals() {
        let msg = "{count, plural, =0 {No files} one {# file} other {# files}}";
        assert_eq!(f(msg, Locale::En, &[("count", 0.into())]), "No files");
        assert_eq!(f(msg, Locale::En, &[("count", 1.into())]), "1 file");
        assert_eq!(f(msg, Locale::En, &[("count", 1234.into())]), "1,234 files");
    }

    #[test]
    fn polish_plurals() {
        let msg = "{count, plural, one {# plik} few {# pliki} many {# plików} other {# pliku}}";
        let p = |n: f64| f(msg, Locale::Pl, &[("count", n.into())]);
        assert_eq!(p(1.0), "1 plik");
        assert_eq!(p(2.0), "2 pliki");
        assert_eq!(p(5.0), "5 plików");
        assert_eq!(p(12.0), "12 plików");
        assert_eq!(p(22.0), "22 pliki");
        assert_eq!(p(1.5), "1,5 pliku");
        assert_eq!(p(12345.0), "12\u{a0}345 plików");
        assert_eq!(p(1234.0), "1234 pliki");
    }

    #[test]
    fn quoting_and_select() {
        assert_eq!(f("It''s '{literal}'", Locale::En, &[]), "It's {literal}");
        assert_eq!(f("don't", Locale::En, &[]), "don't");
        let sel = "{kind, select, image {a picture} other {a file}}";
        assert_eq!(f(sel, Locale::En, &[("kind", "image".into())]), "a picture");
        assert_eq!(f(sel, Locale::En, &[("kind", "zip".into())]), "a file");
    }

    #[test]
    fn nested_arguments_inside_plural_branches() {
        let msg = "Showing {filtered} of {total, plural, one {# user} other {# users}}.";
        assert_eq!(f(msg, Locale::En, &[("filtered", 3.into()), ("total", 10.into())]), "Showing 3 of 10 users.");
    }

    #[test]
    fn every_shipped_message_parses() {
        let m = crate::i18n::Messages::load(&crate::paths::Paths::crate_dir().messages).unwrap();
        fn walk(v: &serde_json::Value, path: &str, out: &mut Vec<(String, String)>) {
            match v {
                serde_json::Value::String(s) => out.push((path.to_string(), s.clone())),
                serde_json::Value::Object(o) => {
                    for (k, v) in o {
                        walk(v, &format!("{path}.{k}"), out);
                    }
                }
                _ => {}
            }
        }
        for locale in crate::i18n::LOCALES {
            let mut all = Vec::new();
            for ns in ["admin", "albums", "auth", "authorize", "common", "dashboard", "download", "files"] {
                walk(m.namespace(locale, ns).unwrap(), ns, &mut all);
            }
            for ns in ["landing", "links", "mail", "nav", "paste", "profile", "public", "tokens", "upload", "viewer"] {
                walk(m.namespace(locale, ns).unwrap(), ns, &mut all);
            }
            for (path, text) in all {
                let chars: Vec<char> = text.chars().collect();
                let mut p = Parser { chars: &chars, pos: 0 };
                let parsed = p.parse_nodes(false, 0);
                assert!(parsed.is_ok() && p.pos == chars.len(), "{} {path}: {text}", locale.as_str());
            }
        }
    }
}
