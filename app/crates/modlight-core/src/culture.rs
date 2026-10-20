//! Number and date formatting: .NET's `CurrentCulture`.
//!
//! On Windows the culture is the user's region format, independent of the interface
//! language: the upstream screenshots show an English interface with Polish numbers
//! ("5,8 MB used"). On macOS the counterpart is the region of the system locale
//! (`AppleLocale`, e.g. `en_PL`). The two cultures the app ships texts for are modelled
//! exactly (en-US and pl-PL, as .NET formats them); any other region formats like en-US.

use std::cell::Cell;
use std::sync::atomic::{AtomicU8, Ordering};

use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CultureId {
    EnglishUs,
    PolishPl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Culture {
    pub id: CultureId,
    pub decimal: char,
    pub group: char,
}

static CULTURE: AtomicU8 = AtomicU8::new(0);

thread_local! {
    static THREAD_CULTURE: Cell<Option<Culture>> = const { Cell::new(None) };
    static THREAD_UTC: Cell<bool> = const { Cell::new(false) };
}

/// Show times in UTC instead of the local zone on this thread (QA renders, which must
/// not depend on where the machine is).
pub fn set_thread_utc(utc: bool) {
    THREAD_UTC.with(|cell| cell.set(utc));
}

fn thread_utc() -> bool {
    THREAD_UTC.with(Cell::get)
}

/// Set the process culture.
pub fn set_culture(culture: Culture) {
    CULTURE.store(culture.id as u8, Ordering::Relaxed);
}

/// Override the culture on this thread only (tests and previews).
pub fn set_thread_culture(culture: Option<Culture>) {
    THREAD_CULTURE.with(|cell| cell.set(culture));
}

const EN_DAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const EN_MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const PL_DAYS: [&str; 7] = [
    "poniedziałek",
    "wtorek",
    "środa",
    "czwartek",
    "piątek",
    "sobota",
    "niedziela",
];
/// Genitive forms, which is what .NET's pl-PL "f" pattern ("d MMMM yyyy") prints.
const PL_MONTHS_GENITIVE: [&str; 12] = [
    "stycznia",
    "lutego",
    "marca",
    "kwietnia",
    "maja",
    "czerwca",
    "lipca",
    "sierpnia",
    "września",
    "października",
    "listopada",
    "grudnia",
];

impl Culture {
    pub fn english() -> Culture {
        Culture {
            id: CultureId::EnglishUs,
            decimal: '.',
            group: ',',
        }
    }

    pub fn polish() -> Culture {
        Culture {
            id: CultureId::PolishPl,
            decimal: ',',
            group: '\u{a0}',
        }
    }

    pub fn from_id(id: CultureId) -> Culture {
        match id {
            CultureId::EnglishUs => Culture::english(),
            CultureId::PolishPl => Culture::polish(),
        }
    }

    /// Pick the culture for a system locale identifier such as `pl_PL`, `en_PL`, `en-US`.
    pub fn for_locale(locale: &str) -> Culture {
        let normalized = locale.replace('-', "_");
        let mut parts = normalized.split(['_', '@', '.']);
        let language = parts.next().unwrap_or_default().to_ascii_lowercase();
        let region = parts.next().unwrap_or_default().to_ascii_uppercase();
        if region == "PL" || (region.is_empty() && language == "pl") {
            Culture::polish()
        } else {
            Culture::english()
        }
    }

    pub fn current() -> Culture {
        THREAD_CULTURE.with(|cell| cell.get()).unwrap_or_else(|| {
            if CULTURE.load(Ordering::Relaxed) == CultureId::PolishPl as u8 {
                Culture::polish()
            } else {
                Culture::english()
            }
        })
    }

    /// `ToString("N0")`: grouped thousands, no decimals.
    pub fn n0(&self, value: i64) -> String {
        let digits = value.unsigned_abs().to_string();
        let mut out = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                out.push(self.group);
            }
            out.push(c);
        }
        if value < 0 { format!("-{out}") } else { out }
    }

    /// `ToString("0.0")` style: fixed decimals, midpoints away from zero, no grouping.
    pub fn fixed(&self, value: f64, decimals: usize) -> String {
        let scale = 10f64.powi(decimals as i32);
        let rounded = (value * scale).round() / scale;
        let text = format!("{rounded:.decimals$}");
        if self.decimal == '.' {
            text
        } else {
            text.replace('.', &self.decimal.to_string())
        }
    }

    /// `ToString()` of a double: shortest round-trip form, culture decimal separator.
    pub fn general(&self, value: f64) -> String {
        let text = if value.fract() == 0.0 && value.abs() < 1e15 {
            format!("{}", value as i64)
        } else {
            format!("{value}")
        };
        if self.decimal == '.' {
            text
        } else {
            text.replace('.', &self.decimal.to_string())
        }
    }

    /// The "g" pattern: short date and short time.
    pub fn short_date_time<Tz: TimeZone>(&self, at: &DateTime<Tz>) -> String {
        match self.id {
            CultureId::EnglishUs => format!(
                "{}/{}/{} {}",
                at.month(),
                at.day(),
                at.year(),
                english_time(at.hour(), at.minute())
            ),
            CultureId::PolishPl => format!(
                "{:02}.{:02}.{} {:02}:{:02}",
                at.day(),
                at.month(),
                at.year(),
                at.hour(),
                at.minute()
            ),
        }
    }

    /// The "t" pattern: the short time.
    pub fn short_time<Tz: TimeZone>(&self, at: &DateTime<Tz>) -> String {
        match self.id {
            CultureId::EnglishUs => english_time(at.hour(), at.minute()),
            CultureId::PolishPl => format!("{:02}:{:02}", at.hour(), at.minute()),
        }
    }

    /// The "f" pattern: long date and short time.
    pub fn full_date_time<Tz: TimeZone>(&self, at: &DateTime<Tz>) -> String {
        let weekday = at.weekday().num_days_from_monday() as usize;
        let month = at.month0() as usize;
        match self.id {
            CultureId::EnglishUs => format!(
                "{}, {} {}, {} {}",
                EN_DAYS[weekday],
                EN_MONTHS[month],
                at.day(),
                at.year(),
                english_time(at.hour(), at.minute())
            ),
            CultureId::PolishPl => format!(
                "{}, {} {} {} {:02}:{:02}",
                PL_DAYS[weekday],
                at.day(),
                PL_MONTHS_GENITIVE[month],
                at.year(),
                at.hour(),
                at.minute()
            ),
        }
    }
}

fn english_time(hour: u32, minute: u32) -> String {
    let (h12, suffix) = match hour {
        0 => (12, "AM"),
        1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        _ => (hour - 12, "PM"),
    };
    format!("{h12}:{minute:02} {suffix}")
}

/// A UTC timestamp in local time, "g" pattern of the current culture.
pub fn local_short<Tz: TimeZone>(at: &DateTime<Tz>) -> String {
    if thread_utc() {
        return Culture::current().short_date_time(&at.with_timezone(&chrono::Utc));
    }
    Culture::current().short_date_time(&at.with_timezone(&Local))
}

/// The time of day of a UTC timestamp in local time, "t" pattern of the current culture.
pub fn local_time<Tz: TimeZone>(at: &DateTime<Tz>) -> String {
    if thread_utc() {
        return Culture::current().short_time(&at.with_timezone(&chrono::Utc));
    }
    Culture::current().short_time(&at.with_timezone(&Local))
}

/// A UTC timestamp in local time, "f" pattern of the current culture.
pub fn local_full<Tz: TimeZone>(at: &DateTime<Tz>) -> String {
    if thread_utc() {
        return Culture::current().full_date_time(&at.with_timezone(&chrono::Utc));
    }
    Culture::current().full_date_time(&at.with_timezone(&Local))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, TimeZone};

    #[test]
    fn numbers_follow_the_culture() {
        let en = Culture::english();
        let pl = Culture::polish();
        assert_eq!(en.n0(6926), "6,926");
        assert_eq!(en.n0(-1234567), "-1,234,567");
        assert_eq!(en.n0(12), "12");
        assert_eq!(pl.n0(6926), "6\u{a0}926");
        assert_eq!(en.fixed(5.25, 1), "5.3");
        assert_eq!(pl.fixed(5.84, 1), "5,8");
        assert_eq!(en.fixed(1.0, 0), "1");
        assert_eq!(pl.general(2.5), "2,5");
    }

    #[test]
    fn locales_map_to_cultures() {
        assert_eq!(Culture::for_locale("pl_PL").id, CultureId::PolishPl);
        assert_eq!(Culture::for_locale("en_PL").id, CultureId::PolishPl);
        assert_eq!(Culture::for_locale("pl").id, CultureId::PolishPl);
        assert_eq!(Culture::for_locale("en_US").id, CultureId::EnglishUs);
        assert_eq!(Culture::for_locale("de-DE").id, CultureId::EnglishUs);
    }

    #[test]
    fn dates_follow_the_dotnet_patterns() {
        let at = FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2026, 10, 18, 14, 7, 0)
            .unwrap();
        assert_eq!(
            Culture::english().short_date_time(&at),
            "10/18/2026 2:07 PM"
        );
        assert_eq!(Culture::polish().short_date_time(&at), "18.10.2026 14:07");
        assert_eq!(
            Culture::english().full_date_time(&at),
            "Sunday, October 18, 2026 2:07 PM"
        );
        assert_eq!(
            Culture::polish().full_date_time(&at),
            "niedziela, 18 października 2026 14:07"
        );
    }
}
