//! JavaScript's `new Date(string)`, for the places upstream hands a client
//! string straight to it (the dashboard's date filters, upload expiry).
//!
//! The ECMAScript Date Time String Format is implemented exactly: `YYYY`,
//! `YYYY-MM`, `YYYY-MM-DD` (UTC), optionally `THH:mm[:ss[.sss]]` with `Z` or
//! `±HH:mm` (a date-time without an offset is LOCAL time), and the expanded
//! `±YYYYYY` years. V8's legacy fallback parser accepts much more; the forms a
//! person or a client realistically sends are covered here (a space instead
//! of `T`, `/` separators, `Month D, YYYY`, RFC 2822 / `toString()` /
//! `toUTCString()` output with `GMT`/`UTC`/`Z` and numeric offsets). Anything
//! else is an Invalid Date (`None`), which is what V8 answers for most junk.
//!
//! "Local" is the process time zone (`TZ`, UTC in the container), as for Node.

use chrono::{DateTime, Datelike, FixedOffset, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};

use crate::db::types::JsDate;

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

/// `new Date(s)`; `None` for an Invalid Date.
pub fn parse_js_date(s: &str) -> Option<JsDate> {
    parse_iso_format(s).or_else(|| parse_legacy(s)).filter(|d| d.millis().abs() <= 8_640_000_000_000_000)
}

fn local_to_utc(naive: NaiveDateTime) -> Option<JsDate> {
    // An ambiguous local time (DST fold) takes the earlier instant, as V8 does;
    // a nonexistent one (DST gap) moves forward.
    match Local.from_local_datetime(&naive) {
        chrono::LocalResult::Single(d) => Some(JsDate(d.with_timezone(&Utc))),
        chrono::LocalResult::Ambiguous(a, _) => Some(JsDate(a.with_timezone(&Utc))),
        chrono::LocalResult::None => {
            let later = naive + chrono::Duration::hours(1);
            Local.from_local_datetime(&later).earliest().map(|d| JsDate(d.with_timezone(&Utc)))
        }
    }
}

/// `MakeDay`: V8 checks each field on its own (month 1-12, day 1-31), not
/// against the month, and lets the arithmetic carry: "2031-02-30" is March 2.
fn make_day(year: i32, month: u32, day: u32) -> Option<NaiveDate> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    NaiveDate::from_ymd_opt(year, month, 1)?.checked_add_days(chrono::Days::new(u64::from(day - 1)))
}

fn digits(s: &str, n: usize) -> Option<u32> {
    if s.len() == n && s.bytes().all(|b| b.is_ascii_digit()) { s.parse().ok() } else { None }
}

/// The Date Time String Format (ECMA-262 21.4.1.32).
fn parse_iso_format(s: &str) -> Option<JsDate> {
    let (date_part, time_part) = match s.find('T') {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    // Year: YYYY or ±YYYYYY (-000000 is not allowed).
    let (year, rest) = if let Some(r) = date_part.strip_prefix('+').or_else(|| date_part.strip_prefix('-')) {
        let y: i32 = digits(r.get(..6)?, 6)? as i32;
        let neg = date_part.starts_with('-');
        if neg && y == 0 {
            return None;
        }
        (if neg { -y } else { y }, &r[6..])
    } else {
        (digits(date_part.get(..4)?, 4)? as i32, &date_part[4..])
    };
    let (month, day) = match rest {
        "" => (1, 1),
        r if r.len() == 3 && r.starts_with('-') => (digits(&r[1..], 2)?, 1),
        r if r.len() == 6 && r.starts_with('-') && &r[3..4] == "-" => (digits(&r[1..3], 2)?, digits(&r[4..], 2)?),
        _ => return None,
    };
    let date = make_day(year, month, day)?;
    let Some(time) = time_part else {
        // Date-only forms are UTC.
        return Some(JsDate(date.and_hms_opt(0, 0, 0)?.and_utc()));
    };
    // HH:mm[:ss[.sss]] then Z | ±HH:mm | nothing (local).
    let (clock, offset) = split_offset(time)?;
    let parts: Vec<&str> = clock.split(':').collect();
    let (h, m) = (digits(parts.first()?, 2)?, digits(parts.get(1)?, 2)?);
    let (sec, ms) = match parts.get(2) {
        None => (0, 0),
        Some(sp) => {
            let (whole, frac) = match sp.find('.') {
                Some(i) => (&sp[..i], Some(&sp[i + 1..])),
                None => (*sp, None),
            };
            let sec = digits(whole, 2)?;
            let ms = match frac {
                None => 0,
                Some(f) if !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()) => {
                    let f3: String = f.chars().chain("000".chars()).take(3).collect();
                    f3.parse().ok()?
                }
                _ => return None,
            };
            (sec, ms)
        }
    };
    if parts.len() > 3 {
        return None;
    }
    // 24:00 is the end of the day.
    let (naive, plus_day) = if h == 24 && m == 0 && sec == 0 && ms == 0 {
        (date.and_hms_milli_opt(0, 0, 0, 0)?, true)
    } else {
        (date.and_hms_milli_opt(h, m, sec, ms)?, false)
    };
    let naive = if plus_day { naive + chrono::Duration::days(1) } else { naive };
    match offset {
        Offset::Local => local_to_utc(naive),
        Offset::Fixed(seconds) => {
            let fixed = FixedOffset::east_opt(seconds)?;
            fixed.from_local_datetime(&naive).single().map(|d| JsDate(d.with_timezone(&Utc)))
        }
    }
}

enum Offset {
    Local,
    Fixed(i32),
}

fn split_offset(time: &str) -> Option<(&str, Offset)> {
    if let Some(clock) = time.strip_suffix('Z') {
        return Some((clock, Offset::Fixed(0)));
    }
    if let Some(i) = time.rfind(['+', '-']) {
        let off = &time[i + 1..];
        if off.len() == 5 && &off[2..3] == ":" {
            let sign = if &time[i..i + 1] == "-" { -1 } else { 1 };
            let secs = sign * (digits(&off[..2], 2)? as i32 * 3600 + digits(&off[3..], 2)? as i32 * 60);
            return Some((&time[..i], Offset::Fixed(secs)));
        }
        return None;
    }
    Some((time, Offset::Local))
}

/// The subset of V8's legacy date parser described in the module docs.
fn parse_legacy(s: &str) -> Option<JsDate> {
    // Text in parentheses is a comment to V8 (`(Central European Summer Time)`).
    let mut uncommented = String::new();
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => uncommented.push(c),
            _ => {}
        }
    }
    let lower = uncommented.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    // Tokenise: words, numbers, and the separators that matter.
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    for c in lower.chars() {
        if c.is_ascii_alphanumeric() || c == ':' || c == '.' {
            cur.push(c);
        } else {
            if !cur.is_empty() {
                tokens.push(std::mem::take(&mut cur));
            }
            if c == '+' || c == '-' || c == '/' {
                tokens.push(c.to_string());
            }
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    let mut year: Option<i32> = None;
    let mut month: Option<u32> = None;
    let day: Option<u32>;
    let mut time: Option<NaiveTime> = None;
    let mut offset: Option<i32> = None;
    let mut numbers: Vec<u32> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i].as_str();
        if t.contains(':') {
            let mut parts = t.split(':');
            let h: u32 = parts.next()?.parse().ok()?;
            let m: u32 = parts.next()?.parse().ok()?;
            let (sec, ms) = match parts.next() {
                Some(sp) => match sp.split_once('.') {
                    Some((w, f)) => {
                        (w.parse().ok()?, f.chars().chain("000".chars()).take(3).collect::<String>().parse().ok()?)
                    }
                    None => (sp.parse().ok()?, 0),
                },
                None => (0, 0),
            };
            time = Some(NaiveTime::from_hms_milli_opt(h, m, sec, ms)?);
        } else if t == "gmt" || t == "utc" || t == "z" || t == "ut" {
            offset = Some(offset.unwrap_or(0));
        } else if (t == "+" || t == "-") && time.is_some() && i + 1 < tokens.len() {
            let n = &tokens[i + 1];
            let v: i32 = n.replace(':', "").parse().ok()?;
            let (hh, mm) = if n.len() <= 2 { (v, 0) } else { (v / 100, v % 100) };
            offset = Some(if t == "-" { -1 } else { 1 } * (hh * 3600 + mm * 60));
            i += 1;
        } else if t == "am" || t == "pm" {
            let tm = time?;
            let mut h = tm.hour_val();
            if t == "pm" && h < 12 {
                h += 12;
            }
            if t == "am" && h == 12 {
                h = 0;
            }
            time = Some(NaiveTime::from_hms_milli_opt(h, tm.minute_val(), tm.second_val(), 0)?);
        } else if let Some(mi) = MONTHS.iter().position(|m| t.starts_with(m)) {
            month = Some(mi as u32 + 1);
        } else if let Ok(n) = t.parse::<u32>() {
            if t.len() >= 3 && year.is_none() {
                year = Some(n as i32);
            } else {
                numbers.push(n);
            }
        } else if t == "/" || t == "-" || t == "+" {
            // separators between date numbers
        } else if t.chars().all(|c| c.is_ascii_alphabetic()) {
            // weekday names and other words V8 skips
            if !["mon", "tue", "wed", "thu", "fri", "sat", "sun"].iter().any(|d| t.starts_with(d)) {
                return None;
            }
        } else {
            return None;
        }
        i += 1;
    }
    // Numbers: with a month name, the first is the day; otherwise M/D[/Y].
    let mut nums = numbers.into_iter();
    if month.is_some() {
        day = nums.next();
        if year.is_none() {
            year = nums.next().map(|y| {
                if y < 50 {
                    2000 + y as i32
                } else if y < 100 {
                    1900 + y as i32
                } else {
                    y as i32
                }
            });
        }
    } else {
        month = nums.next();
        day = nums.next();
        if year.is_none() {
            year = nums.next().map(|y| {
                if y < 50 {
                    2000 + y as i32
                } else if y < 100 {
                    1900 + y as i32
                } else {
                    y as i32
                }
            });
        }
    }
    let date = make_day(year?, month?, day.unwrap_or(1))?;
    let naive = date.and_time(time.unwrap_or(NaiveTime::MIN));
    match offset {
        Some(secs) => {
            FixedOffset::east_opt(secs)?.from_local_datetime(&naive).single().map(|d| JsDate(d.with_timezone(&Utc)))
        }
        None => local_to_utc(naive),
    }
}

trait TimeParts {
    fn hour_val(&self) -> u32;
    fn minute_val(&self) -> u32;
    fn second_val(&self) -> u32;
}

impl TimeParts for NaiveTime {
    fn hour_val(&self) -> u32 {
        chrono::Timelike::hour(self)
    }
    fn minute_val(&self) -> u32 {
        chrono::Timelike::minute(self)
    }
    fn second_val(&self) -> u32 {
        chrono::Timelike::second(self)
    }
}

/// Local midnight of a calendar day (`new Date("YYYY-MM-DDT00:00:00")`).
pub fn local_midnight(date: NaiveDate) -> Option<JsDate> {
    local_to_utc(date.and_hms_opt(0, 0, 0)?)
}

/// The local calendar fields of an instant (`getFullYear()` and friends).
pub fn to_local(d: JsDate) -> DateTime<Local> {
    d.0.with_timezone(&Local)
}

/// `year()` of a local date, for callers that only need the calendar.
pub fn local_year(d: JsDate) -> i32 {
    to_local(d).year()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iso(s: &str) -> Option<String> {
        parse_js_date(s).map(|d| d.to_iso_string())
    }

    #[test]
    fn iso_forms() {
        assert_eq!(iso("2026-10-20").as_deref(), Some("2026-10-20T00:00:00.000Z"));
        assert_eq!(iso("2026-10").as_deref(), Some("2026-10-01T00:00:00.000Z"));
        assert_eq!(iso("2026").as_deref(), Some("2026-01-01T00:00:00.000Z"));
        assert_eq!(iso("2026-10-20T10:00:00.000Z").as_deref(), Some("2026-10-20T10:00:00.000Z"));
        assert_eq!(iso("2026-10-20T10:00:00.5Z").as_deref(), Some("2026-10-20T10:00:00.500Z"));
        assert_eq!(iso("2026-10-20T12:00:00+02:00").as_deref(), Some("2026-10-20T10:00:00.000Z"));
        assert_eq!(iso("2026-10-20T24:00:00Z").as_deref(), Some("2026-10-21T00:00:00.000Z"));
        assert_eq!(iso("+002026-10-20").as_deref(), Some("2026-10-20T00:00:00.000Z"));
        assert!(iso("2026-13-01").is_none());
        // V8 checks the day against 1-31, not against the month, and carries
        // (verified on the reference: "2031-02-30" is stored as March 2).
        assert_eq!(iso("2026-02-30").as_deref(), Some("2026-03-02T00:00:00.000Z"));
        assert_eq!(iso("2026-04-31T10:00:00Z").as_deref(), Some("2026-05-01T10:00:00.000Z"));
        assert!(iso("2026-02-32").is_none());
        assert!(iso("2026-00-10").is_none());
        assert!(iso("garbage").is_none());
        assert!(iso("").is_none());
    }

    #[test]
    fn legacy_forms_with_offsets() {
        assert_eq!(iso("Tue, 20 Oct 2026 10:00:00 GMT").as_deref(), Some("2026-10-20T10:00:00.000Z"));
        assert_eq!(iso("Oct 20, 2026 10:00 UTC").as_deref(), Some("2026-10-20T10:00:00.000Z"));
        assert_eq!(
            iso("Tue Oct 20 2026 12:00:00 GMT+0200 (Central European Summer Time)").as_deref(),
            Some("2026-10-20T10:00:00.000Z")
        );
    }
}
