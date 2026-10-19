//! `ShareX.HelpersLib/Helpers/URLHelpers.cs`, the parts the task flow uses, with the
//! `System.Web.HttpUtility` pieces they call: `HtmlEncode`, `UrlDecode`, and
//! `UrlPathEncode` behind `Helpers.GetValidURL`.
//!
//! Upstream's `StartsWith`/`EndsWith` with a string compare with the current culture;
//! here they are ordinal, which differs only for zero-width characters.

use std::fmt::Write as _;

/// `URLHelpers.URLCharacters`: the characters a URL carries unencoded.
pub const URL_CHARACTERS: &str =
    "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz-._~";

/// `URLHelpers.URLPathCharacters`: the same plus `/`.
pub const URL_PATH_CHARACTERS: &str =
    "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz-._~/";

/// `URLHelpers.URLPrefixes`, in the order they are tried.
pub const URL_PREFIXES: [&str; 6] = ["http://", "https://", "ftp://", "ftps://", "file://", "//"];

/// `URLHelpers.BidiControlCharacters`: the marks, embeddings and overrides that can
/// make a file name read differently from what it is (isolates are not in the list).
pub const BIDI_CONTROL_CHARACTERS: [char; 7] = [
    '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}',
];

/// The default prefix of `FixPrefix` and `ForcePrefix`.
pub const DEFAULT_PREFIX: &str = "https://";

/// `RemoveBidiControlCharacters`.
pub fn remove_bidi_control_characters(text: &str) -> String {
    text.chars()
        .filter(|c| !BIDI_CONTROL_CHARACTERS.contains(c))
        .collect()
}

/// `ReplaceReservedCharacters`: every run of characters outside [`URL_CHARACTERS`]
/// becomes one `replace`. Like upstream, a kept character equal to `replace` also
/// swallows the run after it ("a_ b" with "_" gives "a_b").
pub fn replace_reserved_characters(text: &str, replace: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last: Option<String> = None;
    for c in text.chars() {
        let piece = if URL_CHARACTERS.contains(c) {
            c.to_string()
        } else if last.as_deref() != Some(replace) {
            replace.to_string()
        } else {
            continue;
        };
        out.push_str(&piece);
        last = Some(piece);
    }
    out
}

/// `HttpUtility.HtmlEncode` (`WebUtility.HtmlEncode` in .NET): `< > " ' &` as entities,
/// U+00A0..U+00FF and characters beyond the BMP as decimal references, the rest as is.
/// Used for `TextCustomEncodeInput`.
pub fn html_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '&' => out.push_str("&amp;"),
            '\u{A0}'..='\u{FF}' | '\u{10000}'.. => {
                let _ = write!(out, "&#{};", c as u32);
            }
            _ => out.push(c),
        }
    }
    out
}

/// `URLHelpers.HtmlEncode`: [`html_encode`], then every character past ASCII as a
/// decimal reference (the directory indexer's encoding).
pub fn html_encode_numeric(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in html_encode(text).chars() {
        if c as u32 > 127 {
            let _ = write!(out, "&#{};", c as u32);
        } else {
            out.push(c);
        }
    }
    out
}

/// `URLDecode(url, count)`: `HttpUtility.UrlDecode` up to `count` times, stopping when
/// a pass changes nothing.
pub fn url_decode(url: &str, count: i32) -> String {
    let mut url = url.to_string();
    let mut previous: Option<String> = None;
    let mut done = 0;
    while done < count && previous.as_deref() != Some(url.as_str()) {
        let decoded = http_url_decode(&url);
        previous = Some(std::mem::replace(&mut url, decoded));
        done += 1;
    }
    url
}

/// `HttpUtility.UrlDecode(text)` (UTF-8): `+` is a space, `%xx` a byte, `%uxxxx` a
/// UTF-16 unit; bytes that are not UTF-8 and unpaired surrogates become U+FFFD, and
/// anything malformed stays as written.
pub fn http_url_decode(value: &str) -> String {
    fn hex(unit: u16) -> Option<u16> {
        char::from_u32(u32::from(unit))
            .and_then(|c| c.to_digit(16))
            .map(|digit| digit as u16)
    }
    fn flush(units: &mut Vec<u16>, bytes: &mut Vec<u8>) {
        if !bytes.is_empty() {
            units.extend(String::from_utf8_lossy(bytes).encode_utf16());
            bytes.clear();
        }
    }
    let input: Vec<u16> = value.encode_utf16().collect();
    let count = input.len();
    let mut units: Vec<u16> = Vec::with_capacity(count);
    let mut bytes: Vec<u8> = Vec::new();
    let mut pos = 0;
    while pos < count {
        let mut unit = input[pos];
        if unit == u16::from(b'+') {
            unit = u16::from(b' ');
        } else if unit == u16::from(b'%') && pos + 2 < count {
            if input[pos + 1] == u16::from(b'u') && pos + 5 < count {
                let digits = [
                    hex(input[pos + 2]),
                    hex(input[pos + 3]),
                    hex(input[pos + 4]),
                    hex(input[pos + 5]),
                ];
                if let [Some(a), Some(b), Some(c), Some(d)] = digits {
                    flush(&mut units, &mut bytes);
                    units.push((a << 12) | (b << 8) | (c << 4) | d);
                    pos += 6;
                    continue;
                }
            } else if let (Some(high), Some(low)) = (hex(input[pos + 1]), hex(input[pos + 2])) {
                bytes.push(((high << 4) | low) as u8);
                pos += 3;
                continue;
            }
        }
        if unit & 0xFF80 == 0 {
            // Seven-bit characters go in as bytes, so they join the UTF-8 around them.
            bytes.push(unit as u8);
        } else {
            flush(&mut units, &mut bytes);
            units.push(unit);
        }
        pos += 1;
    }
    flush(&mut units, &mut bytes);
    String::from_utf16_lossy(&units)
}

/// `Helpers.GetValidURL`: [`url_path_encode`], optionally with spaces made `_` first.
pub fn get_valid_url(url: &str, replace_space: bool) -> String {
    if replace_space {
        url_path_encode(&url.replace(' ', "_"))
    } else {
        url_path_encode(url)
    }
}

/// `HttpUtility.UrlPathEncode` (.NET Core): of an absolute URL with a host only the
/// path is encoded, the scheme, host, query and fragment stay; anything else is encoded
/// up to its first `?`. Encoding means bytes outside printable ASCII as `%xx` (small
/// letters) and spaces as `%20`.
///
/// .NET decides "absolute URL with a host" with `Uri.TryCreate`; this checks the
/// `scheme://host` shape, which only differs for malformed hosts and ports.
pub fn url_path_encode(value: &str) -> String {
    match split_for_path_encode(value) {
        Some((scheme_and_authority, path, query_and_fragment)) => format!(
            "{scheme_and_authority}{}{query_and_fragment}",
            url_path_encode_part(path)
        ),
        None => url_path_encode_part(value),
    }
}

/// `UriUtil.TrySplitUriForPathEncode`: query and fragment off, then the scheme and
/// authority of an absolute URL with a non-empty host.
fn split_for_path_encode(input: &str) -> Option<(&str, &str, &str)> {
    let split = input.find(['?', '#']).unwrap_or(input.len());
    let (without, query_and_fragment) = input.split_at(split);
    let colon = without.find(':')?;
    let scheme = &without[..colon];
    let mut scheme_chars = scheme.chars();
    if !scheme_chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        || !scheme_chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return None;
    }
    let rest = without[colon + 1..].strip_prefix("//")?;
    let authority = &rest[..rest.find('/').unwrap_or(rest.len())];
    let host_and_port = authority.rsplit('@').next().unwrap_or_default();
    let (host, port) = match host_and_port.rfind(':') {
        Some(at) if !host_and_port[at..].contains(']') => {
            (&host_and_port[..at], &host_and_port[at + 1..])
        }
        _ => (host_and_port, ""),
    };
    if host.is_empty()
        || host.chars().any(|c| c.is_whitespace() || c.is_control())
        || !port.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let end = colon + 3 + authority.len();
    Some((&without[..end], &without[end..], query_and_fragment))
}

/// `UrlPathEncodeImpl`: up to the first `?`, non-ASCII and control bytes and spaces.
fn url_path_encode_part(value: &str) -> String {
    let (path, query) = value.split_at(value.find('?').unwrap_or(value.len()));
    let mut out = String::with_capacity(value.len());
    for byte in path.bytes() {
        if !(0x20..0x7F).contains(&byte) {
            let _ = write!(out, "%{byte:02x}");
        } else if byte == b' ' {
            out.push_str("%20");
        } else {
            out.push(char::from(byte));
        }
    }
    out.push_str(query);
    out
}

/// `CombineURL(url1, url2)`: one `/` between them; an empty side gives the other.
pub fn combine_url(url1: &str, url2: &str) -> String {
    if url1.is_empty() {
        return url2.to_string();
    }
    if url2.is_empty() {
        return url1.to_string();
    }
    let url1 = url1.strip_suffix('/').unwrap_or(url1);
    let url2 = url2.strip_prefix('/').unwrap_or(url2);
    format!("{url1}/{url2}")
}

/// `CombineURL(params string[])`: the pieces combined from the left ("" for none,
/// where upstream throws).
pub fn combine_urls(urls: &[&str]) -> String {
    let mut pieces = urls.iter();
    let first = pieces
        .next()
        .map(|first| first.to_string())
        .unwrap_or_default();
    pieces.fold(first, |combined, piece| combine_url(&combined, piece))
}

/// `GetFileName` of a URL: after the last `/`, before any `?` or `#`.
pub fn get_file_name(path: &str) -> String {
    let mut path = path;
    if let Some(slash) = path.rfind('/') {
        path = &path[slash + 1..];
    }
    if let Some(query) = path.find('?') {
        path = &path[..query];
    }
    if let Some(fragment) = path.find('#') {
        path = &path[..fragment];
    }
    path.to_string()
}

/// `IsFileURL`: whether what follows the last `/` has a dot in it.
pub fn is_file_url(url: &str) -> bool {
    url.rfind('/')
        .map(|slash| &url[slash + 1..])
        .is_some_and(|name| name.contains('.'))
}

fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len()
        && text.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// `HasPrefix`: whether the URL starts with one of [`URL_PREFIXES`], case ignored.
pub fn has_prefix(url: &str) -> bool {
    get_prefix(url).is_some()
}

/// `GetPrefix`: the first of [`URL_PREFIXES`] the URL starts with, case ignored.
pub fn get_prefix(url: &str) -> Option<&'static str> {
    URL_PREFIXES
        .into_iter()
        .find(|prefix| starts_with_ignore_case(url, prefix))
}

/// `FixPrefix(url)`: [`DEFAULT_PREFIX`] in front of a URL without one.
pub fn fix_prefix(url: &str) -> String {
    fix_prefix_with(url, DEFAULT_PREFIX)
}

/// `FixPrefix(url, prefix)`.
pub fn fix_prefix_with(url: &str, prefix: &str) -> String {
    if !url.is_empty() && !has_prefix(url) {
        format!("{prefix}{url}")
    } else {
        url.to_string()
    }
}

/// `ForcePrefix(url)`: whatever prefix the URL has replaced by [`DEFAULT_PREFIX`].
pub fn force_prefix(url: &str) -> String {
    force_prefix_with(url, DEFAULT_PREFIX)
}

/// `ForcePrefix(url, prefix)`.
pub fn force_prefix_with(url: &str, prefix: &str) -> String {
    if url.is_empty() {
        return String::new();
    }
    format!("{prefix}{}", remove_prefixes(url))
}

/// `RemovePrefixes`: the first matching prefix of [`URL_PREFIXES`] taken off.
pub fn remove_prefixes(url: &str) -> String {
    match get_prefix(url) {
        Some(prefix) => url[prefix.len()..].to_string(),
        None => url.to_string(),
    }
}

/// `IsValidURL(url)` with its default `useRegex: true` (upstream never passes false):
/// http, https or ftp, an optional user, then a public IPv4 address or a host name whose
/// last label has two or more letters, an optional port of 2 to 5 digits, and a path,
/// query or fragment without white space; leading and trailing white space is ignored.
///
/// Upstream matches the regex of gist 729294 with `RegexOptions.IgnoreCase`. It is
/// evaluated here by hand over UTF-16 code units, with .NET's classes (`\d` is any BMP
/// decimal digit, `\S` anything but Unicode white space, `[¡-￿]` any unit
/// from there, surrogates included) and gives the same answers in linear time, where
/// .NET's backtracking can take exponential time on long hosts that do not match.
pub fn is_valid_url(url: &str) -> bool {
    if url.is_empty() {
        return false;
    }
    let units: Vec<u16> = url.trim().encode_utf16().collect();
    UrlPattern { units: &units }.is_match()
}

/// The positions the pattern can have reached, one flag per position 0..=len.
type Reach = Vec<bool>;

struct UrlPattern<'a> {
    units: &'a [u16],
}

impl UrlPattern<'_> {
    fn len(&self) -> usize {
        self.units.len()
    }

    fn at(&self, i: usize) -> Option<u16> {
        self.units.get(i).copied()
    }

    fn is(&self, i: usize, c: char) -> bool {
        self.at(i) == Some(c as u16)
    }

    fn in_range(&self, i: usize, low: char, high: char) -> bool {
        self.at(i)
            .is_some_and(|unit| (low as u16..=high as u16).contains(&unit))
    }

    /// `\d`: a Unicode decimal digit; .NET tests one UTF-16 unit, so only the BMP ones.
    fn digit(&self, i: usize) -> bool {
        self.at(i).is_some_and(is_bmp_decimal_digit)
    }

    /// `\S`.
    fn non_space(&self, i: usize) -> bool {
        self.at(i)
            .is_some_and(|unit| !char::from_u32(u32::from(unit)).is_some_and(char::is_whitespace))
    }

    /// `[a-z¡-￿0-9]` ignoring case.
    fn host_char(&self, i: usize) -> bool {
        self.at(i)
            .is_some_and(|unit| matches!(unit, 0x30..=0x39 | 0x41..=0x5A | 0x61..=0x7A | 0xA1..))
    }

    /// `[a-z¡-￿]` ignoring case.
    fn tld_char(&self, i: usize) -> bool {
        self.at(i)
            .is_some_and(|unit| matches!(unit, 0x41..=0x5A | 0x61..=0x7A | 0xA1..))
    }

    /// A literal ignoring case: ASCII letters in either case, and `s` also as the long s,
    /// which .NET's case-insensitive regex treats as the same letter.
    fn literal(&self, at: usize, text: &str) -> bool {
        text.chars().enumerate().all(|(i, wanted)| {
            self.at(at + i).is_some_and(|unit| {
                char::from_u32(u32::from(unit)).is_some_and(|c| {
                    c.eq_ignore_ascii_case(&wanted) || (wanted == 's' && c == '\u{17F}')
                })
            })
        })
    }

    fn is_match(&self) -> bool {
        let n = self.len();
        let mut start = vec![false; n + 1];
        for scheme in ["http://", "https://", "ftp://"] {
            if scheme.len() <= n && self.literal(0, scheme) {
                start[scheme.len()] = true;
            }
        }
        let after_user = self.user_info(&start);
        let ip = self.ip_addresses(&after_user);
        let names = self.host_names(&after_user);
        let host: Reach = ip.iter().zip(&names).map(|(a, b)| *a || *b).collect();
        let port = self.port(&host);
        self.resource_path(&port)[n]
    }

    /// `(?:\S+(?::\S*)?@)?`, which is "one or more non-space units, then @".
    fn user_info(&self, from: &Reach) -> Reach {
        let n = self.len();
        let mut out = from.clone();
        // run[q]: some start p <= q has only non-space units between p and q.
        let mut run = vec![false; n + 1];
        for q in 0..=n {
            run[q] = from[q] || (q > 0 && run[q - 1] && self.non_space(q - 1));
        }
        for q in 2..=n {
            if self.is(q - 1, '@') && self.non_space(q - 2) && run[q - 2] {
                out[q] = true;
            }
        }
        out
    }

    /// The dotted IPv4 alternative, behind its three private-network lookaheads.
    fn ip_addresses(&self, from: &Reach) -> Reach {
        let mut out = vec![false; self.len() + 1];
        for (p, reached) in from.iter().enumerate() {
            if !reached || self.private_network(p) {
                continue;
            }
            let first = self.first_octet(p);
            let second = self.after_dot(&first, Self::middle_octet);
            let third = self.after_dot(&second, Self::middle_octet);
            for end in self.after_dot(&third, Self::last_octet) {
                out[end] = true;
            }
        }
        out
    }

    /// `(?!(?:10|127)(?:\.\d{1,3}){3})(?!(?:169\.254|192\.168)(?:\.\d{1,3}){2})
    /// (?!172\.(?:1[6-9]|2\d|3[0-1])(?:\.\d{1,3}){2})`: whether one of them matches.
    fn private_network(&self, p: usize) -> bool {
        let after = |text: &str| self.literal(p, text).then_some(p + text.len());
        let loopback = ["10", "127"]
            .into_iter()
            .filter_map(after)
            .any(|end| self.short_groups(end, 3));
        let local = ["169.254", "192.168"]
            .into_iter()
            .filter_map(after)
            .any(|end| self.short_groups(end, 2));
        let private = after("172.").is_some_and(|q| {
            let second = (self.is(q, '1') && self.in_range(q + 1, '6', '9'))
                || (self.is(q, '2') && self.digit(q + 1))
                || (self.is(q, '3') && self.in_range(q + 1, '0', '1'));
            second && self.short_groups(q + 2, 2)
        });
        loopback || local || private
    }

    /// Whether `(?:\.\d{1,3}){count}` matches at `p` (any length will do: lookahead).
    fn short_groups(&self, p: usize, count: usize) -> bool {
        let mut ends = vec![p];
        for _ in 0..count {
            let mut next = Vec::new();
            for end in ends {
                if !self.is(end, '.') {
                    continue;
                }
                let mut q = end + 1;
                while q - end <= 3 && self.digit(q) {
                    q += 1;
                    next.push(q);
                }
            }
            ends = next;
        }
        !ends.is_empty()
    }

    /// `(?:[1-9]\d?|1\d\d|2[01]\d|22[0-3])`: the ends of the first octet.
    fn first_octet(&self, p: usize) -> Vec<usize> {
        let mut ends = Vec::new();
        if self.in_range(p, '1', '9') {
            ends.push(p + 1);
            if self.digit(p + 1) {
                ends.push(p + 2);
            }
        }
        let three = (self.is(p, '1') && self.digit(p + 1) && self.digit(p + 2))
            || (self.is(p, '2') && self.in_range(p + 1, '0', '1') && self.digit(p + 2))
            || (self.is(p, '2') && self.is(p + 1, '2') && self.in_range(p + 2, '0', '3'));
        if three {
            ends.push(p + 3);
        }
        ends
    }

    /// `(?:1?\d{1,2}|2[0-4]\d|25[0-5])`.
    fn middle_octet(&self, p: usize) -> Vec<usize> {
        let mut ends = Vec::new();
        for q in [p, p + 1] {
            if q == p + 1 && !self.is(p, '1') {
                continue;
            }
            if self.digit(q) {
                ends.push(q + 1);
                if self.digit(q + 1) {
                    ends.push(q + 2);
                }
            }
        }
        let three = (self.is(p, '2') && self.in_range(p + 1, '0', '4') && self.digit(p + 2))
            || (self.is(p, '2') && self.is(p + 1, '5') && self.in_range(p + 2, '0', '5'));
        if three {
            ends.push(p + 3);
        }
        ends
    }

    /// `(?:[1-9]\d?|1\d\d|2[0-4]\d|25[0-4])`.
    fn last_octet(&self, p: usize) -> Vec<usize> {
        let mut ends = Vec::new();
        if self.in_range(p, '1', '9') {
            ends.push(p + 1);
            if self.digit(p + 1) {
                ends.push(p + 2);
            }
        }
        let three = (self.is(p, '1') && self.digit(p + 1) && self.digit(p + 2))
            || (self.is(p, '2') && self.in_range(p + 1, '0', '4') && self.digit(p + 2))
            || (self.is(p, '2') && self.is(p + 1, '5') && self.in_range(p + 2, '0', '4'));
        if three {
            ends.push(p + 3);
        }
        ends
    }

    /// `\.` and then an octet, from each of `ends`.
    fn after_dot(&self, ends: &[usize], octet: fn(&Self, usize) -> Vec<usize>) -> Vec<usize> {
        let mut next: Vec<usize> = ends
            .iter()
            .filter(|end| self.is(**end, '.'))
            .flat_map(|end| octet(self, end + 1))
            .collect();
        next.sort_unstable();
        next.dedup();
        next
    }

    /// The host name alternative: a label, more `.label`s, a `.tld` of two or more
    /// letters, and an optional final dot. A label (`(?:H-*)*H+`) starts and ends with
    /// a host character and may hold hyphens between.
    fn host_names(&self, from: &Reach) -> Reach {
        let n = self.len();
        // The first label: `label_end` after a host character, `label_dash` after '-'.
        let mut label_end = vec![false; n + 1];
        let mut label_dash = vec![false; n + 1];
        for q in 1..=n {
            label_end[q] =
                self.host_char(q - 1) && (from[q - 1] || label_end[q - 1] || label_dash[q - 1]);
            label_dash[q] = self.is(q - 1, '-') && (label_end[q - 1] || label_dash[q - 1]);
        }
        // `(?:\.label)*`: `domain` is where a complete label ends.
        let mut domain = vec![false; n + 1];
        let mut dot = vec![false; n + 1];
        let mut inner_end = vec![false; n + 1];
        let mut inner_dash = vec![false; n + 1];
        for q in 0..=n {
            if q > 0 {
                dot[q] = self.is(q - 1, '.') && domain[q - 1];
                inner_end[q] =
                    self.host_char(q - 1) && (dot[q - 1] || inner_end[q - 1] || inner_dash[q - 1]);
                inner_dash[q] = self.is(q - 1, '-') && (inner_end[q - 1] || inner_dash[q - 1]);
            }
            domain[q] = label_end[q] || inner_end[q];
        }
        // `(?:\.(?:T{2,}))` and `\.?`.
        let mut tld_dot = vec![false; n + 1];
        let mut tld_one = vec![false; n + 1];
        let mut tld = vec![false; n + 1];
        for q in 1..=n {
            tld_dot[q] = self.is(q - 1, '.') && domain[q - 1];
            tld_one[q] = self.tld_char(q - 1) && tld_dot[q - 1];
            tld[q] = self.tld_char(q - 1) && (tld_one[q - 1] || tld[q - 1]);
        }
        let mut out = tld.clone();
        for q in 1..=n {
            if self.is(q - 1, '.') && tld[q - 1] {
                out[q] = true;
            }
        }
        out
    }

    /// `(?::\d{2,5})?`.
    fn port(&self, from: &Reach) -> Reach {
        let mut out = from.clone();
        for (p, reached) in from.iter().enumerate() {
            if !reached || !self.is(p, ':') {
                continue;
            }
            let mut q = p + 1;
            while q - p <= 5 && self.digit(q) {
                q += 1;
                if q - p > 2 {
                    out[q] = true;
                }
            }
        }
        out
    }

    /// `(?:[/?#]\S*)?`.
    fn resource_path(&self, from: &Reach) -> Reach {
        let n = self.len();
        let mut out = from.clone();
        let mut open = vec![false; n + 1];
        for q in 1..=n {
            let starts =
                from[q - 1] && (self.is(q - 1, '/') || self.is(q - 1, '?') || self.is(q - 1, '#'));
            open[q] = starts || (open[q - 1] && self.non_space(q - 1));
            out[q] |= open[q];
        }
        out
    }
}

/// The first code points of the BMP's ten-digit runs of general category Nd (the
/// set has not changed since Unicode 7; later digits are all outside the BMP).
const BMP_DIGIT_ZEROS: [u16; 37] = [
    0x0030, 0x0660, 0x06F0, 0x07C0, 0x0966, 0x09E6, 0x0A66, 0x0AE6, 0x0B66, 0x0BE6, 0x0C66, 0x0CE6,
    0x0D66, 0x0DE6, 0x0E50, 0x0ED0, 0x0F20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946, 0x19D0, 0x1A80,
    0x1A90, 0x1B50, 0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xA9F0, 0xAA50, 0xABF0,
    0xFF10,
];

fn is_bmp_decimal_digit(unit: u16) -> bool {
    BMP_DIGIT_ZEROS
        .iter()
        .any(|zero| (*zero..*zero + 10).contains(&unit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::names::helpers::ALPHANUMERIC;

    #[test]
    fn url_characters_are_alphanumerics_and_four_marks() {
        assert_eq!(URL_CHARACTERS, format!("{ALPHANUMERIC}-._~"));
        assert_eq!(URL_PATH_CHARACTERS, format!("{URL_CHARACTERS}/"));
    }

    #[test]
    fn bidi_and_reserved_characters() {
        assert_eq!(
            remove_bidi_control_characters("a\u{202E}gpj.exe\u{200F}"),
            "agpj.exe"
        );
        assert_eq!(remove_bidi_control_characters("a\u{2066}b"), "a\u{2066}b");
        assert_eq!(
            replace_reserved_characters("my file (1).png", "_"),
            "my_file_1_.png"
        );
        assert_eq!(
            replace_reserved_characters("zdjęcie łąka.jpg", "_"),
            "zdj_cie_ka.jpg"
        );
        assert_eq!(replace_reserved_characters("a😀😀b", "_"), "a_b");
        assert_eq!(replace_reserved_characters("a_ b", "_"), "a_b");
        assert_eq!(replace_reserved_characters("a b", ""), "ab");
        assert_eq!(replace_reserved_characters("a~b-c.d", "_"), "a~b-c.d");
        assert_eq!(replace_reserved_characters("", "_"), "");
    }

    #[test]
    fn html_encoding() {
        assert_eq!(
            html_encode("<a href=\"x\">'Tom' & Jerry</a>"),
            "&lt;a href=&quot;x&quot;&gt;&#39;Tom&#39; &amp; Jerry&lt;/a&gt;"
        );
        assert_eq!(html_encode("café ©"), "caf&#233; &#169;");
        assert_eq!(html_encode("żółw"), "ż&#243;łw");
        assert_eq!(html_encode("a😀b"), "a&#128512;b");
        assert_eq!(html_encode("\u{80}\u{9F}"), "\u{80}\u{9F}");
        assert_eq!(html_encode(""), "");
        assert_eq!(html_encode_numeric("żółw <"), "&#380;&#243;&#322;w &lt;");
        assert_eq!(html_encode_numeric("a😀"), "a&#128512;");
    }

    #[test]
    fn url_decoding() {
        assert_eq!(http_url_decode("a%20b+c"), "a b c");
        assert_eq!(http_url_decode("%E2%82%AC"), "€");
        assert_eq!(http_url_decode("%e2%82%ac"), "€");
        assert_eq!(http_url_decode("%u20AC"), "€");
        assert_eq!(http_url_decode("%uD83D%uDE00"), "😀");
        assert_eq!(http_url_decode("%uD83D"), "\u{FFFD}");
        assert_eq!(http_url_decode("%FF"), "\u{FFFD}");
        assert_eq!(http_url_decode("%C3"), "\u{FFFD}");
        assert_eq!(http_url_decode("%C3%u0041"), "\u{FFFD}A");
        assert_eq!(http_url_decode("%C3%A9"), "é");
        assert_eq!(http_url_decode("caf%C3%A9 żółw"), "café żółw");
        assert_eq!(http_url_decode("%zz%"), "%zz%");
        assert_eq!(http_url_decode("%2"), "%2");
        assert_eq!(http_url_decode("%u12"), "%u12");
        assert_eq!(http_url_decode("%u12zz"), "%u12zz");
        assert_eq!(http_url_decode("%U0041"), "%U0041");
        assert_eq!(http_url_decode("100%"), "100%");
        assert_eq!(http_url_decode(""), "");
        assert_eq!(url_decode("a%252520b", 1), "a%2520b");
        assert_eq!(url_decode("a%252520b", 2), "a%20b");
        assert_eq!(url_decode("a%252520b", 10), "a b");
        assert_eq!(url_decode("a%20b", 0), "a%20b");
        assert_eq!(url_decode("a%20b", -1), "a%20b");
    }

    #[test]
    fn path_encoding() {
        assert_eq!(url_path_encode("a b"), "a%20b");
        assert_eq!(url_path_encode("ą"), "%c4%85");
        assert_eq!(url_path_encode("a b?c d"), "a%20b?c d");
        assert_eq!(url_path_encode("a\tb\u{7F}"), "a%09b%7f");
        assert_eq!(
            url_path_encode("http://example.com/a b#c d"),
            "http://example.com/a%20b#c d"
        );
        // Not an absolute URL: the fragment is encoded too.
        assert_eq!(url_path_encode("a b#c d"), "a%20b#c%20d");
        // The host of an absolute URL is left alone, even in Unicode.
        assert_eq!(
            url_path_encode("http://żółw.pl/ż ó"),
            "http://żółw.pl/%c5%bc%20%c3%b3"
        );
        assert_eq!(
            url_path_encode("https://user:pass@host:8080/a b"),
            "https://user:pass@host:8080/a%20b"
        );
        // A space in the host is no URL: everything is encoded.
        assert_eq!(url_path_encode("http://a b/c"), "http://a%20b/c");
        assert_eq!(url_path_encode("file:///a b"), "file:///a%20b");
        assert_eq!(url_path_encode(""), "");
        assert_eq!(get_valid_url("a b", true), "a_b");
        assert_eq!(get_valid_url("a b", false), "a%20b");
    }

    #[test]
    fn combining_urls() {
        assert_eq!(
            combine_url("https://a.com/", "/upload"),
            "https://a.com/upload"
        );
        assert_eq!(
            combine_url("https://a.com", "upload"),
            "https://a.com/upload"
        );
        assert_eq!(combine_url("https://a.com//", "//x"), "https://a.com///x");
        assert_eq!(combine_url("", "x"), "x");
        assert_eq!(combine_url("x", ""), "x");
        assert_eq!(combine_url("", ""), "");
        assert_eq!(
            combine_urls(&["https://a.com/", "api", "/shortlinks"]),
            "https://a.com/api/shortlinks"
        );
        assert_eq!(combine_urls(&["x"]), "x");
        assert_eq!(combine_urls(&[]), "");
    }

    #[test]
    fn file_names_of_urls() {
        assert_eq!(
            get_file_name("https://a.com/x/photo.png?size=2#top"),
            "photo.png"
        );
        assert_eq!(get_file_name("https://a.com/x/"), "");
        assert_eq!(get_file_name("photo.png#a?b"), "photo.png");
        assert_eq!(get_file_name("noslash"), "noslash");
        assert!(is_file_url("https://a.com/x/photo.png"));
        assert!(!is_file_url("https://a.com/x/"));
        assert!(!is_file_url("photo.png"));
        assert!(!is_file_url("https://a.com/x/v1.2/readme"));
    }

    #[test]
    fn prefixes() {
        assert!(has_prefix("HTTPS://a.com"));
        assert!(has_prefix("//cdn.a.com"));
        assert!(!has_prefix("a.com"));
        assert_eq!(get_prefix("Ftp://x"), Some("ftp://"));
        assert_eq!(get_prefix("ftps://x"), Some("ftps://"));
        assert_eq!(get_prefix("x"), None);
        assert_eq!(fix_prefix("a.com"), "https://a.com");
        assert_eq!(fix_prefix("http://a.com"), "http://a.com");
        assert_eq!(fix_prefix(""), "");
        assert_eq!(fix_prefix_with("a.com", "ftp://"), "ftp://a.com");
        assert_eq!(force_prefix("http://a.com"), "https://a.com");
        assert_eq!(force_prefix("HTTP://a.com"), "https://a.com");
        assert_eq!(force_prefix("a.com"), "https://a.com");
        assert_eq!(force_prefix("//a.com"), "https://a.com");
        assert_eq!(force_prefix(""), "");
        assert_eq!(
            force_prefix_with("https://a.com", "http://"),
            "http://a.com"
        );
        assert_eq!(remove_prefixes("file://x"), "x");
        assert_eq!(remove_prefixes("żółw"), "żółw");
    }

    #[test]
    fn valid_urls() {
        for url in [
            "https://example.com",
            "http://example.com/path?x=1#y",
            "HTTPS://EXAMPLE.COM/",
            "ftp://user:pass@ftp.example.org:2121/file.txt",
            "http://user@example.com",
            "http://a@b@example.com",
            "  https://example.com/a  ",
            "http://example.com.",
            "http://ex--ample.co.uk",
            "http://1.2.3.4",
            "http://8.8.8.8:53/x",
            "http://223.255.255.254",
            "http://172.32.0.1",
            "http://11.0.0.1",
            "http://żółw.pl/ścieżka",
            "http://例子.测试",
            "http://example.😀",
            "http://example.com:65535",
            "http://example.com:00",
            "http://example.com/a\u{a0}",
            "https://modlight.example/f/AbC123",
            "http://192.168.1.1.com",
            "httpſ://example.com",
            // [\u00a1-\uffff] takes the ideographic space as a letter of the TLD.
            "https://example.com\u{3000}x",
        ] {
            assert!(is_valid_url(url), "{url}");
        }
        for url in [
            "",
            "   ",
            "example.com",
            "https://",
            "http://localhost",
            "http://localhost:8420/x",
            "http://a.b",
            "http://10.0.0.1",
            "http://127.0.0.1",
            "http://192.168.1.1",
            "http://169.254.1.1",
            "http://172.16.0.1",
            "http://172.31.255.255",
            "http://224.0.0.1",
            "http://1.2.3.255",
            "http://0.1.2.3",
            "http://example.com:1",
            "http://example.com:123456",
            "http://example.com/a b",
            "http://exa mple.com",
            "http://ex_ample.com",
            "http://-example.com",
            "http://example-.com",
            "http://@example.com",
            "http://example.c0m",
            "http://xn--d1abbgf6aiiy.xn--p1ai",
            "mailto:a@example.com",
            "file:///etc/hosts",
            "https://example.com x",
            "httpx://example.com",
        ] {
            assert!(!is_valid_url(url), "{url}");
        }
        // Linear time where backtracking would explode.
        let long = format!("http://{}!", "a-".repeat(20_000));
        assert!(!is_valid_url(&long));
        // .NET's \d takes any BMP decimal digit.
        assert!(is_valid_url("http://example.com:٨٠"));
        assert!(is_bmp_decimal_digit('٣' as u16));
        assert!(!is_bmp_decimal_digit('²' as u16));
    }
}
