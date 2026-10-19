//! The few .NET serialization shapes the app's files share with the Windows client.
//!
//! `Queue.json` is written by Newtonsoft.Json with its defaults: a `DateTime` comes out
//! as ISO 8601 with the fraction of a second trimmed of trailing zeros (and left out when
//! it is zero), followed by `Z` for a UTC value, an offset for a local one and nothing
//! for an unspecified one. `default(DateTime)` is `0001-01-01T00:00:00`. A file written
//! here must read back on Windows and the other way round, so these are reproduced
//! exactly rather than approximated with RFC 3339.

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc};

/// `default(DateTime)`: the value an unset member holds, and how it reads back.
pub fn min_value() -> DateTime<Utc> {
    Utc.from_utc_datetime(
        &NaiveDate::from_ymd_opt(1, 1, 1)
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .expect("year 1 is representable"),
    )
}

/// Whether `value` is `default(DateTime)`.
pub fn is_min(value: &DateTime<Utc>) -> bool {
    *value == min_value()
}

/// A time truncated to .NET's resolution (ticks of 100 ns), so two readings of one file
/// time compare equal on both platforms.
pub fn to_ticks_precision(value: DateTime<Utc>) -> DateTime<Utc> {
    let nanos = value.nanosecond() % 1_000_000_000;
    let truncated = nanos - nanos % 100;
    value.with_nanosecond(truncated).unwrap_or(value)
}

fn fraction(nanos: u32) -> String {
    // Ticks are 100 ns; the fraction is up to seven digits, trailing zeros trimmed.
    let mut ticks = (nanos % 1_000_000_000) / 100;
    if ticks == 0 {
        return String::new();
    }
    let mut digits = 7;
    while ticks.is_multiple_of(10) {
        ticks /= 10;
        digits -= 1;
    }
    format!(".{ticks:0digits$}")
}

fn date_part(value: &NaiveDateTime) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}",
        value.year(),
        value.month(),
        value.day(),
        value.hour(),
        value.minute(),
        value.second(),
        fraction(value.nanosecond())
    )
}

/// Newtonsoft's default writing of a `DateTime` whose kind is UTC
/// (`2026-10-19T20:30:12.1234567Z`); `default(DateTime)` keeps its unspecified shape.
pub fn write_utc(value: &DateTime<Utc>) -> String {
    if is_min(value) {
        return "0001-01-01T00:00:00".to_string();
    }
    format!("{}Z", date_part(&value.naive_utc()))
}

/// Newtonsoft's default writing of a local `DateTime` (`...+02:00`).
pub fn write_local(value: &DateTime<FixedOffset>) -> String {
    let offset = value.offset().local_minus_utc();
    let sign = if offset < 0 { '-' } else { '+' };
    let offset = offset.abs();
    format!(
        "{}{sign}{:02}:{:02}",
        date_part(&value.naive_local()),
        offset / 3600,
        (offset % 3600) / 60
    )
}

/// Read what Newtonsoft (or `DateTime.ToString("o")`) wrote. A value with no zone is
/// taken as UTC when `unspecified_is_utc`, else as local time.
pub fn parse(text: &str, unspecified_is_utc: bool) -> Option<DateTime<Utc>> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(value) = DateTime::parse_from_rfc3339(text) {
        return Some(value.with_timezone(&Utc));
    }
    // RFC 3339 wants a zone; .NET happily writes none.
    for pattern in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
    ] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(text, pattern) {
            if unspecified_is_utc || naive.year() <= 1 {
                return Some(Utc.from_utc_datetime(&naive));
            }
            return chrono::Local
                .from_local_datetime(&naive)
                .earliest()
                .map(|local| local.with_timezone(&Utc));
        }
    }
    // An offset written without the colon (+0200), which some writers produce.
    DateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f%z")
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

/// `Guid.NewGuid().ToString("N")`: 32 lower-case hexadecimal digits.
pub fn new_guid_n() -> String {
    let bytes: [u8; 16] = rand::random();
    hex::encode(bytes)
}

/// serde helpers for a UTC `DateTime` member written by Newtonsoft.
pub mod utc {
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        value: &DateTime<Utc>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&super::write_utc(value))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<DateTime<Utc>, D::Error> {
        let text = Option::<String>::deserialize(deserializer)?;
        Ok(text
            .as_deref()
            .and_then(|t| super::parse(t, true))
            .unwrap_or_else(super::min_value))
    }
}

/// serde helper for a string member Newtonsoft may have written as `null`.
pub mod nullable_string {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
        Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_values_are_written_like_newtonsoft() {
        let value = Utc.with_ymd_and_hms(2026, 10, 19, 20, 30, 12).unwrap();
        assert_eq!(write_utc(&value), "2026-10-19T20:30:12Z");
        let value = value.with_nanosecond(123_456_700).unwrap();
        assert_eq!(write_utc(&value), "2026-10-19T20:30:12.1234567Z");
        let value = value.with_nanosecond(500_000_000).unwrap();
        assert_eq!(write_utc(&value), "2026-10-19T20:30:12.5Z");
        assert_eq!(write_utc(&min_value()), "0001-01-01T00:00:00");
    }

    #[test]
    fn local_values_carry_their_offset() {
        let offset = FixedOffset::east_opt(2 * 3600).unwrap();
        let value = offset
            .with_ymd_and_hms(2026, 10, 19, 22, 30, 12)
            .unwrap()
            .with_nanosecond(100)
            .unwrap();
        assert_eq!(write_local(&value), "2026-10-19T22:30:12.0000001+02:00");
        let west = FixedOffset::west_opt(5 * 3600 + 1800).unwrap();
        let value = west.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
        assert_eq!(write_local(&value), "2026-01-02T03:04:05-05:30");
    }

    #[test]
    fn every_shape_reads_back() {
        let expected = Utc.with_ymd_and_hms(2026, 10, 19, 20, 30, 12).unwrap();
        assert_eq!(parse("2026-10-19T20:30:12Z", true), Some(expected));
        assert_eq!(parse("2026-10-19T22:30:12+02:00", true), Some(expected));
        assert_eq!(parse("2026-10-19T22:30:12+0200", true), Some(expected));
        assert_eq!(parse("2026-10-19T20:30:12", true), Some(expected));
        assert_eq!(
            parse("2026-10-19T20:30:12.1234567Z", true),
            Some(expected.with_nanosecond(123_456_700).unwrap())
        );
        assert_eq!(parse("0001-01-01T00:00:00", false), Some(min_value()));
        assert_eq!(parse("", true), None);
        assert_eq!(parse("yesterday", true), None);
    }

    #[test]
    fn ticks_drop_what_dotnet_cannot_hold() {
        let value = Utc
            .with_ymd_and_hms(2026, 10, 19, 20, 30, 12)
            .unwrap()
            .with_nanosecond(123_456_789)
            .unwrap();
        assert_eq!(to_ticks_precision(value).nanosecond(), 123_456_700);
    }

    #[test]
    fn guids_are_32_hex_digits() {
        let id = new_guid_n();
        assert_eq!(id.len(), 32);
        assert!(
            id.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert_ne!(id, new_guid_n());
    }
}
