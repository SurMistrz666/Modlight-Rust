//! Numbers that come from the server and are also shown in the web panel
//! (`ModlightFormat.cs`), and the relative age of a file (`CloudFileViewModel.AgeOf`).

use chrono::{DateTime, Utc};

use crate::culture::Culture;
use crate::l10n::{Arg, format, s, t};

const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];

/// A byte count, spelled the way the panel spells it: 1024 steps, decimal labels, one
/// decimal place above bytes. The panel wins, because it is where the quota lives.
pub fn cloud_size(bytes: i64) -> String {
    if bytes <= 0 {
        return "0 B".to_string();
    }
    let mut n = bytes as f64;
    let mut i = 0;
    while n >= 1024.0 && i < UNITS.len() - 1 {
        n /= 1024.0;
        i += 1;
    }
    let culture = Culture::current();
    let number = if i == 0 {
        culture.fixed(n, 0)
    } else {
        culture.fixed(n, 1)
    };
    format!("{number} {}", UNITS[i])
}

/// `ToString("N0")` in the current culture.
pub fn n0(value: i64) -> String {
    Culture::current().n0(value)
}

/// How long ago, in the panel's words: "now", "5 min ago", "yesterday", "3 years ago".
pub fn age_of(created: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    let Some(created) = created else {
        return String::new();
    };
    let seconds = ((now - created).num_milliseconds() as f64 / 1000.0).max(0.0);
    if seconds < 60.0 {
        return t(s::MySpace_TimeNow).to_string();
    }
    let minutes = (seconds / 60.0).round() as i64;
    if minutes < 60 {
        return format(t(s::MySpace_TimeMinutes), &[Arg::Int(minutes)]);
    }
    let hours = (minutes as f64 / 60.0).round() as i64;
    if hours < 24 {
        return format(t(s::MySpace_TimeHours), &[Arg::Int(hours)]);
    }
    let days = (hours as f64 / 24.0).round() as i64;
    if days == 1 {
        return t(s::MySpace_TimeYesterday).to_string();
    }
    if days < 30 {
        return format(t(s::MySpace_TimeDays), &[Arg::Int(days)]);
    }
    let months = (days as f64 / 30.0).round() as i64;
    if months < 12 {
        return format(t(s::MySpace_TimeMonths), &[Arg::Int(months)]);
    }
    let years = (months as f64 / 12.0).round() as i64;
    format(t(year_form(years)), &[Arg::Int(years)])
}

/// Polish has three plural forms for years; English uses the "many" one for both.
fn year_form(years: i64) -> crate::l10n::Key {
    if years == 1 {
        return s::MySpace_TimeYearsOne;
    }
    let last = years % 10;
    let teens = years % 100;
    if (2..=4).contains(&last) && !(12..=14).contains(&teens) {
        s::MySpace_TimeYearsFew
    } else {
        s::MySpace_TimeYearsMany
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::culture::set_thread_culture;
    use crate::l10n::{Language, set_thread_language};

    #[test]
    fn sizes_match_the_panel() {
        set_thread_culture(Some(Culture::english()));
        assert_eq!(cloud_size(0), "0 B");
        assert_eq!(cloud_size(-5), "0 B");
        assert_eq!(cloud_size(1000), "1000 B");
        assert_eq!(cloud_size(1126), "1.1 KB");
        assert_eq!(cloud_size(5_853_000), "5.6 MB");
        set_thread_culture(Some(Culture::polish()));
        assert_eq!(cloud_size(6_082_150), "5,8 MB");
        set_thread_culture(None);
    }

    #[test]
    fn ages_read_like_the_panel() {
        set_thread_language(Some(Language::English));
        let now = Utc::now();
        assert_eq!(age_of(None, now), "");
        assert_eq!(age_of(Some(now), now), t(s::MySpace_TimeNow));
        assert_eq!(
            age_of(Some(now - chrono::Duration::minutes(22)), now),
            format(t(s::MySpace_TimeMinutes), &[Arg::Int(22)])
        );
        assert_eq!(
            age_of(Some(now - chrono::Duration::hours(30)), now),
            t(s::MySpace_TimeYesterday)
        );
        assert_eq!(year_form(1), s::MySpace_TimeYearsOne);
        assert_eq!(year_form(3), s::MySpace_TimeYearsFew);
        assert_eq!(year_form(13), s::MySpace_TimeYearsMany);
        assert_eq!(year_form(22), s::MySpace_TimeYearsFew);
        assert_eq!(year_form(5), s::MySpace_TimeYearsMany);
        set_thread_language(None);
    }
}
