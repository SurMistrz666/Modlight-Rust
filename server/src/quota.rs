//! Port of `lib/quota.ts`: disk-quota parsing / formatting.
//!
//! Quotas are stored on `user.max_quota` as a **bigint byte count**, with `-1`
//! meaning "unlimited". Admins think in "20G", not in 21474836480, so the UI
//! takes human input and this module translates in both directions.

/// Sentinel stored in `user.max_quota` for accounts with no limit.
pub const UNLIMITED_QUOTA: i64 = -1;

fn unit_factor(unit: &str) -> Option<f64> {
    let k = 1024f64;
    Some(match unit {
        "b" => 1.0,
        "k" | "kb" | "kib" => k,
        "m" | "mb" | "mib" => k.powi(2),
        "g" | "gb" | "gib" => k.powi(3),
        "t" | "tb" | "tib" => k.powi(4),
        "p" | "pb" | "pib" => k.powi(5),
        _ => return None,
    })
}

const UNLIMITED_WORDS: &[&str] = &["-1", "unlimited", "none", "no limit", "∞", "inf"];

/// Largest integer JavaScript represents exactly (`Number.MAX_SAFE_INTEGER`).
pub const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// `Number.isSafeInteger`
pub fn is_safe_integer(n: f64) -> bool {
    n.is_finite() && n.trunc() == n && n.abs() <= 9_007_199_254_740_991.0
}

/// Parse an admin-typed size into bytes: `20G`, `512 MB`, `1.5TiB`, `1,5G`, a
/// bare byte count, and the "unlimited" spellings. `None` when the input
/// cannot be understood.
pub fn parse_quota(input: &str) -> Option<i64> {
    let raw = crate::config::js_trim(input).to_lowercase();
    if raw.is_empty() || UNLIMITED_WORDS.contains(&raw.as_str()) {
        return Some(UNLIMITED_QUOTA);
    }
    // /^(\d+(?:[.,]\d+)?)\s*([a-z]+)?$/
    let re = regex::Regex::new(r"^([0-9]+(?:[.,][0-9]+)?)[\t\n\x0B\x0C\r \u{a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}]*([a-z]+)?$")
        .expect("static regex");
    let m = re.captures(&raw)?;
    let amount = crate::config::js_number(&m[1].replacen(',', ".", 1));
    if !amount.is_finite() || amount < 0.0 {
        return None;
    }
    let unit = m.get(2).map(|u| u.as_str()).unwrap_or("b");
    let factor = unit_factor(unit)?;
    let bytes = crate::naming::js_round(amount * factor);
    // Anything past 2^53 stops being an exact JS number.
    if bytes.is_nan() || bytes.abs() > MAX_SAFE_INTEGER {
        return None;
    }
    Some(bytes as i64)
}

/// True when the stored value means "no limit".
pub fn is_unlimited(bytes: Option<i64>) -> bool {
    bytes.is_none_or(|b| b < 0)
}

/// Render a stored quota back into something an admin can edit - the inverse
/// of [`parse_quota`] for round numbers.
pub fn format_quota_input(bytes: Option<i64>) -> String {
    let Some(n) = bytes.filter(|b| *b >= 0) else {
        return String::new();
    };
    if n == 0 {
        return "0".to_string();
    }
    for (factor, suffix) in [(1i64 << 40, "T"), (1 << 30, "G"), (1 << 20, "M"), (1 << 10, "K")] {
        if n % factor == 0 {
            return format!("{}{suffix}", n / factor);
        }
    }
    n.to_string()
}

/// Fraction of the quota consumed (0..1), or `None` when unlimited.
pub fn quota_ratio(used: f64, max: Option<i64>) -> Option<f64> {
    let max = max.filter(|m| *m > 0)?;
    Some((used / max as f64).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    //! tests/unit/quota.test.ts
    use super::*;

    #[test]
    fn parses_suffixes_spellings_and_commas() {
        assert_eq!(parse_quota("1K"), Some(1024));
        assert_eq!(parse_quota("512M"), Some(512 * 1024i64.pow(2)));
        assert_eq!(parse_quota("20G"), Some(20 * 1024i64.pow(3)));
        assert_eq!(parse_quota("1T"), Some(1024i64.pow(4)));
        assert_eq!(parse_quota("512mb"), Some(512 * 1024i64.pow(2)));
        assert_eq!(parse_quota("512MiB"), Some(512 * 1024i64.pow(2)));
        assert_eq!(parse_quota("2 GB"), Some(2 * 1024i64.pow(3)));
        assert_eq!(parse_quota("1024"), Some(1024));
        assert_eq!(parse_quota("0"), Some(0));
        assert_eq!(parse_quota("1,5G"), Some((1.5 * 1024f64.powi(3)) as i64));
        assert_eq!(parse_quota("1.5G"), Some((1.5 * 1024f64.powi(3)) as i64));
        for v in ["", "-1", "unlimited", "None", "no limit", "∞", "inf"] {
            assert_eq!(parse_quota(v), Some(UNLIMITED_QUOTA), "{v}");
        }
        for v in ["abc", "20 gigs", "12X", "1e3", "9007199254740993", "1024P"] {
            assert_eq!(parse_quota(v), None, "{v}");
        }
    }

    #[test]
    fn formats_round_trip_and_ratios() {
        for text in ["1K", "512M", "20G", "1T", "0", "1500"] {
            assert_eq!(format_quota_input(parse_quota(text)), text);
        }
        assert_eq!(format_quota_input(Some(-1)), "");
        assert_eq!(format_quota_input(None), "");
        assert!(is_unlimited(Some(-1)));
        assert!(is_unlimited(None));
        assert!(!is_unlimited(Some(0)));
        assert!(!is_unlimited(Some(1024)));
        assert_eq!(quota_ratio(1024.0, Some(-1)), None);
        assert_eq!(quota_ratio(1024.0, None), None);
        assert_eq!(quota_ratio(512.0, Some(1024)), Some(0.5));
        assert_eq!(quota_ratio(4096.0, Some(1024)), Some(1.0));
        assert_eq!(quota_ratio(0.0, Some(1024)), Some(0.0));
    }
}

#[cfg(test)]
mod srv_auth_tests {
    //! The srv-auth port's tests of its narrow version of this module, kept
    //! against the reconciled implementation.
    use super::*;

    #[test]
    fn parses_like_upstream() {
        assert_eq!(parse_quota("20G"), Some(21_474_836_480));
        assert_eq!(parse_quota("512 MB"), Some(536_870_912));
        assert_eq!(parse_quota("1,5G"), Some(1_610_612_736));
        assert_eq!(parse_quota("1.5TiB"), Some(1_649_267_441_664));
        assert_eq!(parse_quota("123"), Some(123));
        assert_eq!(parse_quota(""), Some(-1));
        assert_eq!(parse_quota(" Unlimited "), Some(-1));
        assert_eq!(parse_quota("5 parsecs"), None);
        assert_eq!(parse_quota("-5G"), None);
        assert_eq!(format_quota_input(Some(21_474_836_480)), "20G");
        assert_eq!(format_quota_input(Some(-1)), "");
        assert_eq!(format_quota_input(Some(1000)), "1000");
        assert!(is_unlimited(None));
        assert!(is_unlimited(Some(-1)));
        assert!(!is_unlimited(Some(0)));
    }
}
