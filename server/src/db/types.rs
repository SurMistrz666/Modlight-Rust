//! Column types shared by every model.
//!
//! [`JsDate`] is how a timestamp crosses every boundary of this server the way
//! a JavaScript `Date` crossed the upstream ones:
//!
//! - **Database.** Upstream mixes `timestamp` (the Better Auth tables) and
//!   `timestamp with time zone` (the app tables). Drizzle writes a `Date` into a
//!   `timestamp` column as its UTC wall time and reads it back as UTC
//!   (`value + "+0000"`), so both kinds hold UTC. `JsDate` decodes either kind
//!   as UTC and binds as `timestamptz`; Postgres converts that into a
//!   `timestamp` column using the session time zone, which sqlx pins to UTC.
//! - **JSON.** `JSON.stringify(date)` is `toISOString()`: always UTC, always
//!   three fraction digits (`2026-10-18T12:00:00.000Z`). Inside an RPC or
//!   page-data payload a `Date` is tagged instead (`{"$date": "..."}`, PORT.md
//!   section 7); [`crate::rpc::tagged::to_tagged_value`] switches that on.

use std::fmt;

use chrono::{DateTime, NaiveDateTime, SubsecRound, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use sqlx::postgres::{PgArgumentBuffer, PgTypeInfo, PgValueRef};
use sqlx::{Decode, Encode, Postgres, Type, TypeInfo, ValueRef};

/// A point in time with JavaScript `Date` semantics at every boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JsDate(pub DateTime<Utc>);

impl JsDate {
    /// `new Date()`: now, at JavaScript's millisecond precision.
    pub fn now() -> JsDate {
        JsDate(Utc::now().trunc_subsecs(3))
    }

    /// Milliseconds since the epoch (`date.getTime()`).
    pub fn millis(&self) -> i64 {
        self.0.timestamp_millis()
    }

    pub fn from_millis(ms: i64) -> Option<JsDate> {
        DateTime::<Utc>::from_timestamp_millis(ms).map(JsDate)
    }

    /// `date.toISOString()`.
    pub fn to_iso_string(&self) -> String {
        self.0.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
    }

    /// `new Date(string)` for the ISO forms clients send.
    pub fn parse_iso(s: &str) -> Option<JsDate> {
        if let Ok(d) = DateTime::parse_from_rfc3339(s) {
            return Some(JsDate(d.with_timezone(&Utc)));
        }
        // Date-only and zone-less forms: `new Date("2026-10-18")` is UTC
        // midnight; a date-time without a zone is local time in JavaScript,
        // which on the server (TZ=UTC) is UTC as well.
        if let Ok(d) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
            return Some(JsDate(d.and_utc()));
        }
        if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
            return d.and_hms_opt(0, 0, 0).map(|d| JsDate(d.and_utc()));
        }
        None
    }

    /// The UTC wall time, for code that binds a `timestamp` column explicitly.
    pub fn naive_utc(&self) -> NaiveDateTime {
        self.0.naive_utc()
    }
}

impl From<DateTime<Utc>> for JsDate {
    fn from(d: DateTime<Utc>) -> Self {
        JsDate(d)
    }
}

impl From<JsDate> for DateTime<Utc> {
    fn from(d: JsDate) -> Self {
        d.0
    }
}

impl fmt::Display for JsDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_iso_string())
    }
}

impl Serialize for JsDate {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if crate::rpc::tagged::is_tagged_mode() {
            use serde::ser::SerializeMap;
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("$date", &self.to_iso_string())?;
            map.end()
        } else {
            serializer.serialize_str(&self.to_iso_string())
        }
    }
}

impl<'de> Deserialize<'de> for JsDate {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Tagged {
                #[serde(rename = "$date")]
                date: String,
            },
            Plain(String),
            Millis(i64),
        }
        let s = match Repr::deserialize(deserializer)? {
            Repr::Tagged { date } | Repr::Plain(date) => date,
            Repr::Millis(ms) => {
                return JsDate::from_millis(ms).ok_or_else(|| serde::de::Error::custom("Invalid Date"));
            }
        };
        JsDate::parse_iso(&s).ok_or_else(|| serde::de::Error::custom("Invalid Date"))
    }
}

impl Type<Postgres> for JsDate {
    fn type_info() -> PgTypeInfo {
        PgTypeInfo::with_name("TIMESTAMPTZ")
    }

    fn compatible(ty: &PgTypeInfo) -> bool {
        matches!(ty.name(), "TIMESTAMPTZ" | "TIMESTAMP")
    }
}

impl Encode<'_, Postgres> for JsDate {
    fn encode_by_ref(&self, buf: &mut PgArgumentBuffer) -> Result<IsNull, BoxDynError> {
        <DateTime<Utc> as Encode<Postgres>>::encode_by_ref(&self.0, buf)
    }
}

impl<'r> Decode<'r, Postgres> for JsDate {
    fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
        let is_naive = value.type_info().name() == "TIMESTAMP";
        if is_naive {
            let naive = <NaiveDateTime as Decode<Postgres>>::decode(value)?;
            Ok(JsDate(naive.and_utc()))
        } else {
            Ok(JsDate(<DateTime<Utc> as Decode<Postgres>>::decode(value)?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn iso_strings_like_javascript() {
        let d = JsDate(Utc.with_ymd_and_hms(2026, 10, 18, 12, 0, 0).unwrap());
        assert_eq!(d.to_iso_string(), "2026-10-18T12:00:00.000Z");
        assert_eq!(serde_json::to_string(&d).unwrap(), "\"2026-10-18T12:00:00.000Z\"");
        let back: JsDate = serde_json::from_str("{\"$date\":\"2026-10-18T12:00:00.000Z\"}").unwrap();
        assert_eq!(back, d);
        let plain: JsDate = serde_json::from_str("\"2026-10-18T14:00:00+02:00\"").unwrap();
        assert_eq!(plain, d);
        assert_eq!(JsDate::parse_iso("2026-10-18").unwrap().to_iso_string(), "2026-10-18T00:00:00.000Z");
    }
}
