//! The pieces of `ShareX.HelpersLib` the name parsers lean on: the character sets and
//! random picks of `Helpers.cs` and `RandomCrypto.cs`, `ToBase` (`NumberExtensions.cs`),
//! `Truncate`, `ReplaceAll`, `ForEachBetween` and `Lines` (`StringExtensions.cs`),
//! `WeekOfYear` (`Extensions.cs`), `TextInfo.ToTitleCase`, the emoji list (`Emoji.cs`)
//! and the random word lists (`Resources/adjectives.txt`, `Resources/animals.txt`).
//!
//! .NET strings are UTF-16. Where upstream counts characters (`Length`, `Substring`,
//! `PadLeft`) these functions count UTF-16 code units as well, so a name cut at 100 is
//! cut where the Windows app cuts it.

use std::sync::LazyLock;

use chrono::Datelike;
use rand::Rng;
use rand::seq::IndexedRandom;

use super::name_parser::NameParserError;

/// `Helpers.Numbers`.
pub const NUMBERS: &str = "0123456789";
/// `Helpers.AlphabetCapital`.
pub const ALPHABET_CAPITAL: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
/// `Helpers.Alphabet`.
pub const ALPHABET: &str = "abcdefghijklmnopqrstuvwxyz";
/// `Helpers.Alphanumeric`: digits, capitals, small letters.
pub const ALPHANUMERIC: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
/// `Helpers.AlphanumericInverse`: digits, small letters, capitals.
pub const ALPHANUMERIC_INVERSE: &str =
    "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
/// `Helpers.Hexadecimal` (capitals; `%rx` lowercases it).
pub const HEXADECIMAL: &str = "0123456789ABCDEF";
/// `Helpers.Base58`.
pub const BASE58: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
/// `Helpers.Base56`: Base58 without `1` and `o`, the characters people misread.
pub const BASE56: &str = "23456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz";

/// Upstream's `Resources.adjectives`, the list `%radjective` picks from.
pub const ADJECTIVES: &str = include_str!("../../../../assets/upstream/names/adjectives.txt");
/// Upstream's `Resources.animals`, the list `%ranimal` picks from.
pub const ANIMALS: &str = include_str!("../../../../assets/upstream/names/animals.txt");

/// `Emoji.Emojis`: smileys, animals and nature, food and drink, travel and places,
/// objects, in that order (the file is generated from `Emoji.cs`).
pub static EMOJIS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    include_str!("../../../../assets/upstream/names/emoji.txt")
        .lines()
        .filter(|line| !line.is_empty())
        .collect()
});

static ADJECTIVE_LINES: LazyLock<Vec<&'static str>> = LazyLock::new(|| lines(ADJECTIVES.trim()));
static ANIMAL_LINES: LazyLock<Vec<&'static str>> = LazyLock::new(|| lines(ANIMALS.trim()));

/// The longest piece a single code may generate (a repeat count, a zero padding).
/// Upstream builds whatever is asked and runs out of memory or time on absurd counts
/// (`RepeatGenerator` concatenates in a loop); a pattern asking for more is refused.
pub const MAX_GENERATED_LENGTH: i64 = 1_000_000;

/// How many replacements one `ReplaceAll` may make. Upstream searches again from the
/// start after every replacement and never stops when the replacement brings the code
/// back (a `%rf` line holding its own code); this cap turns that hang into an error.
pub const MAX_REPLACEMENTS: usize = 100_000;

/// The length of `text` the way .NET counts it.
pub fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// `Helpers.AddZeroes(string, digits)`: `PadLeft(digits, '0')`.
pub fn add_zeroes(input: &str, digits: usize) -> String {
    let length = utf16_len(input);
    if length >= digits {
        return input.to_string();
    }
    let mut out = "0".repeat(digits - length);
    out.push_str(input);
    out
}

/// `Helpers.AddZeroes(int, digits)`: the number's text padded with zeros.
pub fn add_zeroes_number(number: i64, digits: usize) -> String {
    add_zeroes(&number.to_string(), digits)
}

/// `Helpers.HourTo12`: 0 is "12", 13 is "01", 12 stays "12".
pub fn hour_to_12(hour: u32) -> String {
    if hour == 0 {
        return "12".to_string();
    }
    if hour > 12 {
        return add_zeroes_number(i64::from(hour) - 12, 2);
    }
    add_zeroes_number(i64::from(hour), 2)
}

thread_local! {
    /// A fixed generator for QA renders (`set_thread_seed`); the system one otherwise.
    static SEEDED: std::cell::RefCell<Option<rand::rngs::StdRng>> = const { std::cell::RefCell::new(None) };
}

/// Draw from this thread's QA generator when one is set (pictures that must not change
/// between runs), from the operating system's otherwise.
pub fn set_thread_seed(seed: Option<u64>) {
    use rand::SeedableRng;
    SEEDED.with(|cell| *cell.borrow_mut() = seed.map(rand::rngs::StdRng::seed_from_u64));
}

/// Run `f` with this thread's generator.
pub fn with_rng<R>(f: impl FnOnce(&mut dyn rand::RngCore) -> R) -> R {
    SEEDED.with(|cell| match cell.borrow_mut().as_mut() {
        Some(rng) => f(rng),
        None => f(&mut rand::rng()),
    })
}

/// `Helpers.GetRandomChar`: one character of `chars`, every one equally likely.
///
/// Upstream draws from `RandomNumberGenerator`; `rand`'s thread generator is a CSPRNG
/// (ChaCha) seeded from the operating system. `None` only for an empty set, where
/// upstream throws.
pub fn get_random_char(chars: &str) -> Option<char> {
    let count = chars.chars().count();
    if count == 0 {
        return None;
    }
    chars
        .chars()
        .nth(with_rng(|rng| rng.random_range(0..count)))
}

/// `Helpers.GetRandomString`: `length` characters of `chars`.
pub fn get_random_string(chars: &str, length: usize) -> String {
    (0..length).filter_map(|_| get_random_char(chars)).collect()
}

/// `RandomCrypto.Pick`: one element, every one equally likely; `None` when empty.
pub fn pick<T>(items: &[T]) -> Option<&T> {
    with_rng(|rng| items.choose(rng))
}

/// `Helpers.RepeatGenerator`: `count` results of `generator` one after another; nothing
/// for zero or a negative count.
pub fn repeat_generator(count: i32, mut generator: impl FnMut() -> String) -> String {
    let mut result = String::new();
    for _ in 0..count.max(0) {
        result.push_str(&generator());
    }
    result
}

/// `StringExtensions.Lines`: split on "\r\n" and "\n"; a lone "\r" stays in its line.
pub fn lines(text: &str) -> Vec<&str> {
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect()
}

/// `Helpers.GetRandomLine`: a random line of the trimmed text (an empty text has one
/// empty line, like `Split` gives).
pub fn get_random_line(text: &str) -> String {
    let lines = lines(text.trim());
    pick(&lines)
        .map(|line| line.to_string())
        .unwrap_or_default()
}

/// `Helpers.GetRandomLineFromFile`: the file read like `File.ReadAllText(path, UTF8)`
/// (a byte order mark picks UTF-16 or UTF-32, broken UTF-8 becomes U+FFFD), then a
/// random line of it.
pub fn get_random_line_from_file(path: &str) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(get_random_line(&decode_text(&bytes)))
}

/// What `File.ReadAllText` with `Encoding.UTF8` makes of the bytes: the byte order mark
/// decides (UTF-32 before UTF-16, since both start with FF FE), UTF-8 otherwise.
fn decode_text(bytes: &[u8]) -> String {
    fn units16(bytes: &[u8], big_endian: bool) -> String {
        let units: Vec<u16> = bytes
            .chunks(2)
            .map(|pair| match (pair, big_endian) {
                ([a, b], false) => u16::from_le_bytes([*a, *b]),
                ([a, b], true) => u16::from_be_bytes([*a, *b]),
                // A dangling last byte cannot make a code unit.
                _ => 0xFFFD,
            })
            .collect();
        String::from_utf16_lossy(&units)
    }
    fn units32(bytes: &[u8], big_endian: bool) -> String {
        bytes
            .chunks(4)
            .map(|quad| {
                let value = match (quad, big_endian) {
                    ([a, b, c, d], false) => u32::from_le_bytes([*a, *b, *c, *d]),
                    ([a, b, c, d], true) => u32::from_be_bytes([*a, *b, *c, *d]),
                    _ => 0xFFFD,
                };
                char::from_u32(value).unwrap_or('\u{FFFD}')
            })
            .collect()
    }
    match bytes {
        [0xFF, 0xFE, 0x00, 0x00, rest @ ..] => units32(rest, false),
        [0x00, 0x00, 0xFE, 0xFF, rest @ ..] => units32(rest, true),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        [0xFF, 0xFE, rest @ ..] => units16(rest, false),
        [0xFE, 0xFF, rest @ ..] => units16(rest, true),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// `NumberExtensions.ToBase`: `value` written in `radix` with the given digit set.
///
/// Like upstream it writes the magnitude only (no sign) and gives an empty text for 0.
/// The failures are upstream's exceptions: a radix outside 2..=digits, or a value (or
/// radix) of `int.MinValue`, whose magnitude does not fit.
pub fn to_base(value: i32, radix: i32, digits: &str) -> Result<String, NameParserError> {
    if digits.is_empty() {
        return Err(NameParserError::InvalidNumberFormat(
            "Digits must contain character value representations (Parameter 'digits')".to_string(),
        ));
    }
    let radix = radix.checked_abs().ok_or_else(negation_overflow)?;
    let digit_count = utf16_len(digits);
    if radix < 2 || radix as usize > digit_count {
        return Err(NameParserError::InvalidNumberFormat(format!(
            "Radix has to be > 2 and < {digit_count} (Parameter 'radix')\nActual value was {radix}."
        )));
    }
    let digits: Vec<char> = digits.chars().collect();
    let mut quotient = value.checked_abs().ok_or_else(negation_overflow)?;
    let mut result = Vec::new();
    while quotient > 0 {
        let index = (quotient % radix) as usize;
        let Some(digit) = digits.get(index) else {
            // Only a digit set with characters outside the BMP gets here: upstream
            // indexes UTF-16 units and would pick half a character.
            return Err(NameParserError::InvalidNumberFormat(
                "Index was outside the bounds of the array.".to_string(),
            ));
        };
        result.push(*digit);
        quotient /= radix;
    }
    Ok(result.iter().rev().collect())
}

/// .NET's `OverflowException` from `Math.Abs(int.MinValue)`.
pub(crate) fn negation_overflow() -> NameParserError {
    NameParserError::InvalidNumberFormat(
        "Negating the minimum value of a twos complement number is invalid.".to_string(),
    )
}

/// `StringExtensions.Truncate(maxLength)`: at most `max_length` UTF-16 code units.
///
/// When the cut falls inside a surrogate pair, .NET keeps half of the pair; a Rust
/// string cannot hold half a character, so the whole pair goes and the text is one unit
/// shorter. A negative length (upstream throws) is treated as zero.
pub fn truncate(text: &str, max_length: i32) -> String {
    let max_length = max_length.max(0) as usize;
    if utf16_len(text) <= max_length {
        return text.to_string();
    }
    let mut used = 0;
    let mut out = String::new();
    for c in text.chars() {
        used += c.len_utf16();
        if used > max_length {
            break;
        }
        out.push(c);
    }
    out
}

/// `StringExtensions.ReplaceAll`: replace every occurrence of `search`, calling
/// `replace` for each one, and look again from the start after every replacement (so
/// a replacement that completes a code with the text before it is replaced too).
///
/// The search restarts at the first place a new occurrence could begin, which finds the
/// same occurrence as starting over. A replacement that keeps bringing the code back
/// stops after [`MAX_REPLACEMENTS`] instead of hanging like upstream.
pub fn replace_all(
    text: &str,
    search: &str,
    mut replace: impl FnMut() -> Result<String, NameParserError>,
) -> Result<String, NameParserError> {
    if search.is_empty() {
        return Ok(text.to_string());
    }
    let mut text = text.to_string();
    let mut from = 0;
    let mut replacements = 0;
    while let Some(found) = text[from..].find(search) {
        let location = from + found;
        replacements += 1;
        if replacements > MAX_REPLACEMENTS {
            return Err(NameParserError::Endless(search.to_string()));
        }
        let replacement = replace()?;
        text.replace_range(location..location + search.len(), &replacement);
        // Nothing before `location` can start a match that ends before it: the earlier
        // text did not hold one. A match may still start a few bytes back and run into
        // the replacement.
        from = location.saturating_sub(search.len() - 1);
        while !text.is_char_boundary(from) {
            from += 1;
        }
    }
    Ok(text)
}

/// `StringExtensions.ForEachBetween`: every `front ... back` span, as (the whole span,
/// what is between), searching on from just after each `front`. A span may sit inside
/// an earlier one (`%ib{%ib{2}}` yields both).
pub fn for_each_between(text: &str, front: &str, back: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    if front.is_empty() {
        return found;
    }
    let mut f = 0;
    while text.len() > f {
        let Some(start) = text[f..].find(front).map(|i| f + i) else {
            break;
        };
        let inner_start = start + front.len();
        let Some(end) = text[inner_start..].find(back).map(|i| inner_start + i) else {
            break;
        };
        found.push((
            text[start..end + back.len()].to_string(),
            text[inner_start..end].to_string(),
        ));
        f = inner_start;
    }
    found
}

/// `int.TryParse(text)` with `NumberStyles.Integer`: optional white space (tab to
/// carriage return, space) around an optional sign and ASCII digits, trailing NULs
/// tolerated, `None` when it does not fit in an `int`.
pub fn try_parse_int(text: &str) -> Option<i32> {
    let is_white = |c: char| c == ' ' || ('\t'..='\r').contains(&c);
    let text = text.trim_start_matches(is_white);
    let (negative, text) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let digits_end = text
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map_or(text.len(), |(i, _)| i);
    let (digits, rest) = text.split_at(digits_end);
    if digits.is_empty() || !rest.trim_start_matches(is_white).chars().all(|c| c == '\0') {
        return None;
    }
    let mut value: i64 = 0;
    for digit in digits.bytes() {
        value = value * 10 + i64::from(digit - b'0');
        if value > i64::from(i32::MAX) + 1 {
            return None;
        }
    }
    let value = if negative { -value } else { value };
    i32::try_from(value).ok()
}

/// `DateTime.WeekOfYear()`: `Calendar.GetWeekOfYear` with `CalendarWeekRule.FirstDay`
/// and Monday as the first day: week 1 is the one holding January 1st, and every
/// Monday starts the next (both cultures the app models use the Gregorian calendar).
pub fn week_of_year(date: &impl Datelike) -> i32 {
    let day_of_year = date.ordinal0() as i32;
    let day_of_week = date.weekday().num_days_from_sunday() as i32;
    let day_for_jan1 = day_of_week - (day_of_year % 7);
    let offset = (day_for_jan1 - 1 + 14) % 7;
    (day_of_year + offset) / 7 + 1
}

/// A random `%radjective` word, title-cased like upstream.
pub fn random_adjective() -> String {
    title_case(pick(&ADJECTIVE_LINES).copied().unwrap_or_default())
}

/// A random `%ranimal` word, title-cased like upstream.
pub fn random_animal() -> String {
    title_case(pick(&ANIMAL_LINES).copied().unwrap_or_default())
}

/// A random `%remoji`.
pub fn random_emoji() -> String {
    pick(&EMOJIS).copied().unwrap_or_default().to_string()
}

/// `CultureInfo.InvariantCulture.TextInfo.ToTitleCase`: the first letter of every word
/// in capitals, the rest small, except words with no small letter (acronyms), which stay.
///
/// Exact for ASCII, which is all the word lists hold ("mouse/mice" gives "Mouse/Mice":
/// punctuation separates words, digits and apostrophes do not). Other characters count
/// as letters when Unicode calls them alphabetic.
pub fn title_case(text: &str) -> String {
    let is_letter = |c: char| c.is_alphabetic();
    let is_separator = |c: char| {
        if c.is_ascii() {
            !c.is_ascii_alphanumeric()
        } else {
            c.is_whitespace() || c.is_control()
        }
    };
    let lower = |c: char| {
        let mut lower = c.to_lowercase();
        match (lower.next(), lower.next()) {
            (Some(single), None) => single,
            _ => c,
        }
    };
    let upper = |c: char| {
        let mut upper = c.to_uppercase();
        match (upper.next(), upper.next()) {
            (Some(single), None) => single,
            _ => c,
        }
    };
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let flush = |out: &mut String, part: &[char], has_lower: bool| {
        for &c in part {
            out.push(if has_lower { lower(c) } else { c });
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if !is_letter(c) {
            out.push(c);
            i += 1;
            continue;
        }
        out.push(upper(c));
        i += 1;
        let mut lowercase_start = i;
        let mut has_lower = c.is_lowercase();
        while i < chars.len() {
            let next = chars[i];
            if is_letter(next) {
                has_lower |= next.is_lowercase();
                i += 1;
            } else if next == '\'' {
                i += 1;
                flush(&mut out, &chars[lowercase_start..i], has_lower);
                lowercase_start = i;
                has_lower = true;
            } else if !is_separator(next) {
                i += 1;
            } else {
                break;
            }
        }
        flush(&mut out, &chars[lowercase_start..i], has_lower);
        if i < chars.len() {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn character_sets_are_built_like_upstream() {
        assert_eq!(
            ALPHANUMERIC,
            format!("{NUMBERS}{ALPHABET_CAPITAL}{ALPHABET}")
        );
        assert_eq!(
            ALPHANUMERIC_INVERSE,
            format!("{NUMBERS}{ALPHABET}{ALPHABET_CAPITAL}")
        );
        assert_eq!(BASE58.len(), 58);
        assert_eq!(BASE56.len(), 56);
        for c in ['0', '1', 'I', 'O', 'l', 'o'] {
            assert!(!BASE56.contains(c), "{c}");
        }
    }

    #[test]
    fn zeroes_and_hours() {
        assert_eq!(add_zeroes("7", 2), "07");
        assert_eq!(add_zeroes("123", 2), "123");
        assert_eq!(add_zeroes("", 3), "000");
        assert_eq!(add_zeroes("5", 0), "5");
        assert_eq!(add_zeroes_number(-5, 3), "0-5");
        assert_eq!(hour_to_12(0), "12");
        assert_eq!(hour_to_12(1), "01");
        assert_eq!(hour_to_12(11), "11");
        assert_eq!(hour_to_12(12), "12");
        assert_eq!(hour_to_12(13), "01");
        assert_eq!(hour_to_12(23), "11");
    }

    #[test]
    fn random_picks_stay_in_their_sets() {
        for _ in 0..200 {
            let c = get_random_char(BASE56).unwrap();
            assert!(BASE56.contains(c));
        }
        assert_eq!(get_random_char(""), None);
        let text = get_random_string(NUMBERS, 40);
        assert_eq!(text.len(), 40);
        assert!(text.chars().all(|c| c.is_ascii_digit()));
        // Every character can come up (upstream's inclusive bound picks the last too).
        let seen: std::collections::HashSet<char> =
            (0..2000).filter_map(|_| get_random_char("ab")).collect();
        assert_eq!(seen.len(), 2);
        assert_eq!(repeat_generator(3, || "ab".to_string()), "ababab");
        assert_eq!(repeat_generator(0, || "ab".to_string()), "");
        assert_eq!(repeat_generator(-4, || "ab".to_string()), "");
    }

    #[test]
    fn lines_split_like_upstream() {
        assert_eq!(lines("a\r\nb\nc"), vec!["a", "b", "c"]);
        assert_eq!(lines("a\rb"), vec!["a\rb"]);
        assert_eq!(lines("a\r\r\nb"), vec!["a\r", "b"]);
        assert_eq!(lines(""), vec![""]);
        assert_eq!(get_random_line("  \n "), "");
        assert_eq!(get_random_line(" only \r\n"), "only");
        for _ in 0..50 {
            let line = get_random_line("x\ny\r\nz\n");
            assert!(["x", "y", "z"].contains(&line.as_str()), "{line}");
        }
    }

    #[test]
    fn files_are_decoded_like_read_all_text() {
        assert_eq!(decode_text(b"\xEF\xBB\xBFzebra"), "zebra");
        assert_eq!(decode_text(b"caf\xC3\xA9"), "café");
        assert_eq!(decode_text(b"bad\xFF"), "bad\u{FFFD}");
        assert_eq!(decode_text(b"\xFF\xFEh\x00i\x00"), "hi");
        assert_eq!(decode_text(b"\xFE\xFF\x00h\x00i"), "hi");
        assert_eq!(decode_text(b"\xFF\xFE\x00\x00h\x00\x00\x00"), "h");
        assert_eq!(decode_text(b"\x00\x00\xFE\xFF\x00\x00\x00h"), "h");
    }

    #[test]
    fn to_base_writes_magnitudes() {
        assert_eq!(to_base(255, 16, HEXADECIMAL).unwrap(), "FF");
        assert_eq!(to_base(35, 36, ALPHANUMERIC).unwrap(), "Z");
        assert_eq!(to_base(61, 62, ALPHANUMERIC).unwrap(), "z");
        assert_eq!(to_base(61, 62, ALPHANUMERIC_INVERSE).unwrap(), "Z");
        assert_eq!(to_base(62, 62, ALPHANUMERIC).unwrap(), "10");
        assert_eq!(to_base(5, 2, NUMBERS).unwrap(), "101");
        assert_eq!(to_base(-5, -2, NUMBERS).unwrap(), "101");
        assert_eq!(to_base(0, 10, NUMBERS).unwrap(), "");
        assert!(to_base(5, 1, NUMBERS).is_err());
        assert!(to_base(5, 11, NUMBERS).is_err());
        assert!(to_base(5, 10, "").is_err());
        assert_eq!(to_base(i32::MIN, 10, NUMBERS), Err(negation_overflow()));
        assert_eq!(to_base(5, i32::MIN, NUMBERS), Err(negation_overflow()));
    }

    #[test]
    fn truncate_counts_utf16_units() {
        assert_eq!(truncate("abcdef", 3), "abc");
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("abc", 10), "abc");
        assert_eq!(truncate("", 2), "");
        assert_eq!(truncate("żółw", 2), "żó");
        // The emoji is two units: cutting at 2 keeps "a" and drops the split pair.
        assert_eq!(truncate("a😀b", 2), "a");
        assert_eq!(truncate("a😀b", 3), "a😀");
        assert_eq!(truncate("abc", 0), "");
        assert_eq!(truncate("abc", -1), "");
    }

    #[test]
    fn replace_all_calls_the_generator_per_occurrence() {
        let mut n = 0;
        let out = replace_all("%ra-%ra-%ra", "%ra", || {
            n += 1;
            Ok(n.to_string())
        })
        .unwrap();
        assert_eq!(out, "1-2-3");
        // The replacement completes a code with the text before it: upstream finds it
        // again because it searches from the start every time.
        let mut first = true;
        let out = replace_all("%r%ra", "%ra", || {
            let piece = if first { "a" } else { "X" };
            first = false;
            Ok(piece.to_string())
        })
        .unwrap();
        assert_eq!(out, "X");
        assert_eq!(
            replace_all("abc", "", || Ok("x".to_string())).unwrap(),
            "abc"
        );
        assert_eq!(
            replace_all("ż%raż", "%ra", || Ok("ą".to_string())).unwrap(),
            "żąż"
        );
        // A replacement that brings the code back would loop forever upstream.
        assert_eq!(
            replace_all("%rf{x}", "%rf{x}", || Ok("%rf{x}".to_string())),
            Err(NameParserError::Endless("%rf{x}".to_string()))
        );
    }

    #[test]
    fn for_each_between_finds_nested_spans() {
        assert_eq!(
            for_each_between("a%ra{5}b%ra{x}", "%ra{", "}"),
            vec![
                ("%ra{5}".to_string(), "5".to_string()),
                ("%ra{x}".to_string(), "x".to_string())
            ]
        );
        assert_eq!(
            for_each_between("%ib{%ib{2}}", "%ib{", "}"),
            vec![
                ("%ib{%ib{2}".to_string(), "%ib{2".to_string()),
                ("%ib{2}".to_string(), "2".to_string())
            ]
        );
        assert!(for_each_between("%ra{5", "%ra{", "}").is_empty());
        assert!(for_each_between("", "%ra{", "}").is_empty());
        assert_eq!(
            for_each_between("%ra{}", "%ra{", "}"),
            vec![("%ra{}".to_string(), String::new())]
        );
    }

    #[test]
    fn int_parsing_follows_number_styles_integer() {
        assert_eq!(try_parse_int("5"), Some(5));
        assert_eq!(try_parse_int(" 5 "), Some(5));
        assert_eq!(try_parse_int("\t-12\r\n"), Some(-12));
        assert_eq!(try_parse_int("+7"), Some(7));
        assert_eq!(try_parse_int("007"), Some(7));
        assert_eq!(try_parse_int("5\0\0"), Some(5));
        assert_eq!(try_parse_int("2147483647"), Some(i32::MAX));
        assert_eq!(try_parse_int("-2147483648"), Some(i32::MIN));
        assert_eq!(try_parse_int("2147483648"), None);
        assert_eq!(try_parse_int("99999999999999999999"), None);
        assert_eq!(try_parse_int(""), None);
        assert_eq!(try_parse_int("-"), None);
        assert_eq!(try_parse_int("- 5"), None);
        assert_eq!(try_parse_int("5x"), None);
        assert_eq!(try_parse_int("1,5"), None);
        assert_eq!(try_parse_int("\u{a0}5"), None);
        assert_eq!(try_parse_int("٣"), None);
    }

    #[test]
    fn weeks_start_on_monday_from_january_first() {
        let week = |y, m, d| week_of_year(&NaiveDate::from_ymd_opt(y, m, d).unwrap());
        // 2026-01-01 is a Thursday: week 1 runs to Sunday the 4th.
        assert_eq!(week(2026, 1, 1), 1);
        assert_eq!(week(2026, 1, 4), 1);
        assert_eq!(week(2026, 1, 5), 2);
        assert_eq!(week(2026, 10, 19), 43);
        assert_eq!(week(2026, 12, 31), 53);
        // 2024-01-01 is a Monday.
        assert_eq!(week(2024, 1, 1), 1);
        assert_eq!(week(2024, 1, 7), 1);
        assert_eq!(week(2024, 1, 8), 2);
        assert_eq!(week(2024, 12, 31), 53);
    }

    #[test]
    fn title_case_matches_text_info() {
        assert_eq!(title_case("zebra"), "Zebra");
        assert_eq!(title_case("mouse/mice"), "Mouse/Mice");
        assert_eq!(title_case("hello world"), "Hello World");
        assert_eq!(title_case("USA today"), "USA Today");
        assert_eq!(title_case("mIxEd"), "Mixed");
        assert_eq!(title_case("o'neil"), "O'neil");
        assert_eq!(title_case("abc1def"), "Abc1def");
        assert_eq!(title_case("1abc"), "1Abc");
        assert_eq!(title_case("x-ray"), "X-Ray");
        assert_eq!(title_case(""), "");
    }

    #[test]
    fn word_lists_are_upstream_s() {
        assert_eq!(ADJECTIVE_LINES.len(), 1501);
        assert_eq!(ANIMAL_LINES.len(), 1750);
        assert_eq!(ADJECTIVE_LINES[0], "abandoned");
        assert_eq!(ANIMAL_LINES[ANIMAL_LINES.len() - 1], "zorilla");
        // Every entry is a plain small word, so title casing it is exact.
        for word in ADJECTIVE_LINES.iter().chain(ANIMAL_LINES.iter()) {
            assert!(
                word.chars().all(|c| c.is_ascii_lowercase() || c == '/'),
                "{word}"
            );
        }
        assert_eq!(EMOJIS.len(), 705);
        assert_eq!(EMOJIS[0], "😀");
        assert_eq!(EMOJIS[EMOJIS.len() - 1], "🔎");
        assert!(EMOJIS.contains(&"☺️"));
        assert!(EMOJIS.contains(&"🛀🏿"));
        // The four locks upstream comments out are not in the list.
        assert!(!EMOJIS.contains(&"🔒"));
        let adjective = random_adjective();
        assert!(adjective.chars().next().unwrap().is_ascii_uppercase());
        assert!(
            ADJECTIVE_LINES.contains(&adjective.to_ascii_lowercase().as_str()),
            "{adjective}"
        );
        let animal = random_animal();
        assert!(ANIMAL_LINES.contains(&animal.to_ascii_lowercase().as_str()));
        assert!(EMOJIS.contains(&random_emoji().as_str()));
    }
}
