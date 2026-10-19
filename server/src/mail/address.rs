//! How nodemailer 8.0.10 reads the From and To values lib/mail.ts hands it:
//! `addressparser` (lib/addressparser), then `MimeNode#_convertAddresses`
//! and `_normalizeAddress` (lib/mime-node), which give both the header text
//! and the SMTP envelope (`getEnvelope`).
//!
//! The test-mail recipient is whatever the administrator typed, unvalidated,
//! so this is where "Name <a@b.c>", "a@b.c, d@e.f" and "not-an-address" get
//! the meaning they had upstream: two names, two recipients, or no recipient
//! at all ("No recipients defined").

use std::sync::LazyLock;

use regex::Regex;

/// One parsed entry: an address, or a group of them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedAddress {
    pub name: String,
    pub address: String,
    pub group: Option<Vec<ParsedAddress>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    operator: bool,
    value: String,
    no_break: bool,
}

/// JavaScript's `\s`.
const JS_SPACE: &str =
    r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

fn operator_end(c: char) -> Option<Option<char>> {
    match c {
        '"' => Some(Some('"')),
        '(' => Some(Some(')')),
        '<' => Some(Some('>')),
        ':' => Some(Some(';')),
        // Semicolons are treated like commas outside a group.
        ',' | ';' => Some(None),
        _ => None,
    }
}

/// `Tokenizer#tokenize`
fn tokenize(input: &str) -> Vec<Token> {
    let chars: Vec<char> = input.chars().collect();
    let mut list: Vec<Token> = Vec::new();
    let mut node: Option<usize> = None;
    let mut expecting: Option<char> = None;
    let mut escaped = false;
    for (i, &chr) in chars.iter().enumerate() {
        let next = chars.get(i + 1).copied();
        if !escaped {
            if Some(chr) == expecting {
                let no_break = next.is_some_and(|n| !matches!(n, ' ' | '\t' | '\r' | '\n' | ',' | ';'));
                list.push(Token { operator: true, value: chr.to_string(), no_break });
                node = None;
                expecting = None;
                continue;
            } else if expecting.is_none()
                && let Some(end) = operator_end(chr)
            {
                list.push(Token { operator: true, value: chr.to_string(), no_break: false });
                node = None;
                expecting = end;
                continue;
            } else if matches!(expecting, Some('"' | '\'')) && chr == '\\' {
                escaped = true;
                continue;
            }
        }
        let index = match node {
            Some(n) => n,
            None => {
                list.push(Token { operator: false, value: String::new(), no_break: false });
                list.len() - 1
            }
        };
        node = Some(index);
        // Newlines become spaces; other control bytes are skipped.
        let chr = if chr == '\n' { ' ' } else { chr };
        if chr as u32 >= 0x21 || chr == ' ' || chr == '\t' {
            list[index].value.push(chr);
        }
        escaped = false;
    }
    list.into_iter()
        .map(|mut t| {
            t.value = crate::config::js_trim(&t.value).to_string();
            t
        })
        .filter(|t| !t.value.is_empty())
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Address,
    Comment,
    Group,
    Text,
}

static STRICT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^[^@{JS_SPACE}]+@[^@{JS_SPACE}]+$")).expect("valid regex"));
static LOOSE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"[{JS_SPACE}]*(?-u:\b)[^@{JS_SPACE}]+@[^{JS_SPACE}]+(?-u:\b)[{JS_SPACE}]*"))
        .expect("valid regex")
});
static UP_TO_ANGLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^[^<]*<[{JS_SPACE}]*")).expect("valid regex"));

/// `_handleAddress(tokens)`
fn handle_address(tokens: &[Token], depth: usize) -> Vec<ParsedAddress> {
    let mut is_group = false;
    let mut state = State::Text;
    let (mut address, mut comment, mut group, mut text) = (Vec::new(), Vec::new(), Vec::new(), Vec::<String>::new());
    let mut text_was_quoted: Vec<bool> = Vec::new();
    let mut inside_quotes = false;

    for (i, token) in tokens.iter().enumerate() {
        let prev = if i > 0 { tokens.get(i - 1) } else { None };
        if token.operator {
            match token.value.as_str() {
                "<" => {
                    state = State::Address;
                    inside_quotes = false;
                }
                "(" => {
                    state = State::Comment;
                    inside_quotes = false;
                }
                ":" => {
                    state = State::Group;
                    is_group = true;
                    inside_quotes = false;
                }
                "\"" => {
                    inside_quotes = !inside_quotes;
                    state = State::Text;
                }
                _ => {
                    state = State::Text;
                    inside_quotes = false;
                }
            }
            continue;
        }
        let mut value = token.value.clone();
        if state == State::Address {
            // An unquoted name that includes a "<": everything up to it goes.
            value = UP_TO_ANGLE.replace(&value, "").into_owned();
        }
        let list = match state {
            State::Address => &mut address,
            State::Comment => &mut comment,
            State::Group => &mut group,
            State::Text => &mut text,
        };
        if prev.is_some_and(|p| p.no_break) && !list.is_empty() {
            if let Some(last) = list.last_mut() {
                last.push_str(&value);
            }
            if state == State::Text
                && inside_quotes
                && let Some(last) = text_was_quoted.last_mut()
            {
                *last = true;
            }
        } else {
            list.push(value);
            if state == State::Text {
                text_was_quoted.push(inside_quotes);
            }
        }
    }

    // If there is no text but a comment, replace the two.
    if text.is_empty() && !comment.is_empty() {
        text = std::mem::take(&mut comment);
    }

    if is_group {
        let mut members = Vec::new();
        if !group.is_empty() {
            for member in parse_depth(&group.join(","), depth + 1) {
                match member.group {
                    Some(inner) => members.extend(inner),
                    None => members.push(member),
                }
            }
        }
        return vec![ParsedAddress { name: text.join(" "), address: String::new(), group: Some(members) }];
    }

    // If no address was found, try to detect one from regular text.
    if address.is_empty() && !text.is_empty() {
        for i in (0..text.len()).rev() {
            // Not from quoted strings: `"user@domain"@example.com` must not
            // be misrouted.
            if !text_was_quoted.get(i).copied().unwrap_or(false) && STRICT.is_match(&text[i]) {
                address.push(text.remove(i));
                text_was_quoted.remove(i);
                break;
            }
        }
        if address.is_empty() {
            for i in (0..text.len()).rev() {
                if text_was_quoted.get(i).copied().unwrap_or(false) {
                    continue;
                }
                let mut extracted = None;
                let replaced = LOOSE
                    .replacen(&text[i], 1, |m: &regex::Captures<'_>| {
                        extracted = Some(crate::config::js_trim(&m[0]).to_string());
                        " ".to_string()
                    })
                    .into_owned();
                text[i] = crate::config::js_trim(&replaced).to_string();
                if let Some(found) = extracted {
                    address.push(found);
                    break;
                }
            }
        }
    }

    if text.is_empty() && !comment.is_empty() {
        text = std::mem::take(&mut comment);
    }
    // Keep only the first address occurrence, push others to regular text.
    if address.len() > 1 {
        text.extend(address.split_off(1));
    }

    let text = text.join(" ");
    let address = address.join(" ");
    let mut parsed = ParsedAddress {
        address: if !address.is_empty() { address.clone() } else { text.clone() },
        name: if !text.is_empty() { text } else { address },
        group: None,
    };
    if parsed.address == parsed.name {
        if parsed.address.contains('@') {
            parsed.name.clear();
        } else {
            parsed.address.clear();
        }
    }
    vec![parsed]
}

/// RFC 5322 does not nest groups; deeper input is refused (DoS protection).
const MAX_NESTED_GROUP_DEPTH: usize = 50;

fn parse_depth(input: &str, depth: usize) -> Vec<ParsedAddress> {
    if depth > MAX_NESTED_GROUP_DEPTH {
        return Vec::new();
    }
    let tokens = tokenize(input);
    let mut groups: Vec<Vec<Token>> = Vec::new();
    let mut current: Vec<Token> = Vec::new();
    for token in tokens {
        if token.operator && (token.value == "," || token.value == ";") {
            if !current.is_empty() {
                groups.push(std::mem::take(&mut current));
            }
        } else {
            current.push(token);
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    let mut parsed: Vec<ParsedAddress> = groups.iter().flat_map(|g| handle_address(g, depth)).collect();

    // "Joe Foo, PhD <joe@example.com>" was split on the comma: recombine a
    // name-only entry with the named address that follows it.
    let mut i = parsed.len().saturating_sub(1);
    while i > 0 {
        i -= 1;
        let merge = {
            let (current, next) = (&parsed[i], &parsed[i + 1]);
            current.address.is_empty()
                && !current.name.is_empty()
                && current.group.is_none()
                && !next.address.is_empty()
                && !next.name.is_empty()
        };
        if merge {
            let name = format!("{}, {}", parsed[i].name, parsed[i + 1].name);
            parsed[i + 1].name = name;
            parsed.remove(i);
        }
    }
    parsed
}

/// `addressparser(str)`
pub fn addressparser(input: &str) -> Vec<ParsedAddress> {
    parse_depth(input, 0)
}

/// `_normalizeAddress(address)`: no control characters or angle brackets, a
/// lower-cased (punycoded) domain, a local part with spaces quoted.
pub fn normalize_address(address: &str) -> String {
    let mut cleaned = String::with_capacity(address.len());
    let mut in_run = false;
    for c in address.chars() {
        if (c as u32) < 0x20 || c == '<' || c == '>' {
            if !in_run {
                cleaned.push(' ');
            }
            in_run = true;
        } else {
            cleaned.push(c);
            in_run = false;
        }
    }
    let address = crate::config::js_trim(&cleaned);
    let Some(at) = address.rfind('@') else {
        return address.to_string();
    };
    let mut user = address[..at].to_string();
    let domain = &address[at + 1..];
    let lower = domain.to_lowercase();
    // ASCII local parts get a punycoded domain; a non-ASCII local part needs
    // SMTPUTF8 anyway and keeps the domain in Unicode.
    let encoded = if user.is_ascii() { to_ascii_domain(&lower) } else { lower };
    if user.contains(' ') {
        if !user.starts_with('"') {
            user.insert(0, '"');
        }
        if !user.ends_with('"') {
            user.push('"');
        }
    }
    format!("{user}@{encoded}")
}

/// `_encodeAddressName(name)`: as is when only word characters and spaces,
/// quoted when printable ASCII, an RFC 2047 word otherwise.
pub fn encode_address_name(name: &str) -> String {
    if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ' ') {
        return name.to_string();
    }
    if name.chars().all(|c| (' '..='~').contains(&c)) {
        let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
        return format!("\"{escaped}\"");
    }
    encode_word(name)
}

/// An RFC 2047 encoded word (base64, UTF-8), split so no word exceeds 75
/// characters.
pub fn encode_word(text: &str) -> String {
    use base64::Engine;
    let mut words = Vec::new();
    let mut chunk = String::new();
    for c in text.chars() {
        if chunk.len() + c.len_utf8() > 39 {
            words.push(std::mem::take(&mut chunk));
        }
        chunk.push(c);
    }
    if !chunk.is_empty() {
        words.push(chunk);
    }
    words
        .iter()
        .map(|w| format!("=?UTF-8?B?{}?=", base64::engine::general_purpose::STANDARD.encode(w.as_bytes())))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `_convertAddresses(addresses, uniqueList)`: the header text, with every
/// address (group members included) added once to `unique`.
pub fn convert_addresses(addresses: &[ParsedAddress], unique: &mut Vec<String>) -> String {
    let mut values = Vec::new();
    for entry in addresses {
        if !entry.address.is_empty() {
            let address = normalize_address(&entry.address);
            if entry.name.is_empty() {
                values.push(if address.contains(' ') { format!("<{address}>") } else { address.clone() });
            } else {
                values.push(format!("{} <{address}>", encode_address_name(&entry.name)));
            }
            if !unique.contains(&address) {
                unique.push(address);
            }
        } else if let Some(group) = &entry.group {
            let members = if group.is_empty() { String::new() } else { convert_addresses(group, unique) };
            values.push(format!("{}:{};", encode_address_name(&entry.name), crate::config::js_trim(&members)));
        }
    }
    values.join(", ")
}

// ── punycode.toASCII (RFC 3492), as nodemailer's bundled punycode.js ─────────

fn to_ascii_domain(domain: &str) -> String {
    // Labels are separated by dots, ideographic full stops and their kin.
    domain
        .split(['.', '\u{3002}', '\u{FF0E}', '\u{FF61}'])
        .map(|label| if label.is_ascii() { label.to_string() } else { format!("xn--{}", punycode_encode(label)) })
        .collect::<Vec<_>>()
        .join(".")
}

fn punycode_encode(input: &str) -> String {
    const BASE: u32 = 36;
    const T_MIN: u32 = 1;
    const T_MAX: u32 = 26;
    const SKEW: u32 = 38;
    const DAMP: u32 = 700;
    fn adapt(mut delta: u32, num_points: u32, first: bool) -> u32 {
        delta = if first { delta / DAMP } else { delta / 2 };
        delta += delta / num_points;
        let mut k = 0;
        while delta > ((BASE - T_MIN) * T_MAX) / 2 {
            delta /= BASE - T_MIN;
            k += BASE;
        }
        k + (BASE - T_MIN + 1) * delta / (delta + SKEW)
    }
    fn digit(d: u32) -> char {
        char::from_u32(if d < 26 { d + 97 } else { d + 22 }).unwrap_or('a')
    }
    let code_points: Vec<u32> = input.chars().map(|c| c as u32).collect();
    let mut output: String = input.chars().filter(char::is_ascii).collect();
    let basic = output.len() as u32;
    let mut handled = basic;
    if basic > 0 {
        output.push('-');
    }
    let (mut n, mut delta, mut bias) = (128u32, 0u32, 72u32);
    while (handled as usize) < code_points.len() {
        let m = code_points.iter().copied().filter(|&c| c >= n).min().unwrap_or(n);
        delta = delta.saturating_add((m - n).saturating_mul(handled + 1));
        n = m;
        for &c in &code_points {
            if c < n {
                delta = delta.saturating_add(1);
            }
            if c == n {
                let mut q = delta;
                let mut k = BASE;
                loop {
                    let t = if k <= bias {
                        T_MIN
                    } else if k >= bias + T_MAX {
                        T_MAX
                    } else {
                        k - bias
                    };
                    if q < t {
                        break;
                    }
                    output.push(digit(t + (q - t) % (BASE - t)));
                    q = (q - t) / (BASE - t);
                    k += BASE;
                }
                output.push(digit(q));
                bias = adapt(delta, handled + 1, handled == basic);
                delta = 0;
                handled += 1;
            }
        }
        delta += 1;
        n += 1;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(name: &str, address: &str) -> ParsedAddress {
        ParsedAddress { name: name.into(), address: address.into(), group: None }
    }

    #[test]
    fn parses_the_shapes_an_administrator_types() {
        assert_eq!(addressparser("ada@example.test"), vec![one("", "ada@example.test")]);
        assert_eq!(addressparser("Ada Lovelace <ada@example.test>"), vec![one("Ada Lovelace", "ada@example.test")]);
        assert_eq!(addressparser("\"Modlight\" <no-reply@localhost>"), vec![one("Modlight", "no-reply@localhost")]);
        assert_eq!(addressparser("a@b.test, c@d.test"), vec![one("", "a@b.test"), one("", "c@d.test")]);
        assert_eq!(addressparser("a@b.test; c@d.test").len(), 2);
        // No @ anywhere: a name with no address, i.e. no recipient at all.
        assert_eq!(addressparser("not-an-address"), vec![one("not-an-address", "")]);
        assert_eq!(addressparser(""), vec![]);
        assert_eq!(addressparser("Joe Foo, PhD <joe@example.com>"), vec![one("Joe Foo, PhD", "joe@example.com")]);
        assert_eq!(addressparser("ada@example.test (Ada)"), vec![one("Ada", "ada@example.test")]);
        assert_eq!(addressparser("write to ada@example.test now"), vec![one("write to now", "ada@example.test")]);
    }

    #[test]
    fn groups_are_flattened_into_the_envelope() {
        let parsed = addressparser("Team: a@b.test, c@d.test;");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "Team");
        let mut unique = Vec::new();
        assert_eq!(convert_addresses(&parsed, &mut unique), "Team:a@b.test, c@d.test;");
        assert_eq!(unique, vec!["a@b.test", "c@d.test"]);
    }

    #[test]
    fn addresses_are_normalized_like_mime_node() {
        assert_eq!(normalize_address("Ada@Example.TEST"), "Ada@example.test");
        assert_eq!(normalize_address("  a<b>@c.test "), "\"a b \"@c.test");
        assert_eq!(normalize_address("user@b\u{FC}cher.example"), "user@xn--bcher-kva.example");
        assert_eq!(normalize_address("bare"), "bare");
        let mut unique = Vec::new();
        assert_eq!(
            convert_addresses(&addressparser("Ada <ada@x.test>, ada@x.test"), &mut unique),
            "Ada <ada@x.test>, ada@x.test"
        );
        assert_eq!(unique, vec!["ada@x.test"]);
    }

    #[test]
    fn names_are_quoted_or_encoded_when_they_must_be() {
        assert_eq!(encode_address_name("Modlight"), "Modlight");
        assert_eq!(encode_address_name("Foo, Bar"), "\"Foo, Bar\"");
        assert_eq!(encode_address_name("a\"b"), "\"a\\\"b\"");
        assert!(encode_address_name("Zażółć").starts_with("=?UTF-8?B?"));
    }

    #[test]
    fn punycode_matches_rfc_3492() {
        assert_eq!(punycode_encode("b\u{FC}cher"), "bcher-kva");
        assert_eq!(
            punycode_encode("\u{4ED6}\u{4EEC}\u{4E3A}\u{4EC0}\u{4E48}\u{4E0D}\u{8BF4}\u{4E2D}\u{6587}"),
            "ihqwcrb4cv8a8dqg056pqjye"
        );
    }
}
