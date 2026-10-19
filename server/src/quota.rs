//! Port of `lib/quota.ts`: disk-quota parsing / formatting.
//!
//! Quotas are stored on `user.max_quota` as a **bigint byte count**, with `-1`
//! meaning "unlimited" (a convention stored rows rely on).
//!
//! Admins think in "20G", not in 21474836480, so the UI takes human input and
//! this module is the single place that translates in both directions.

use crate::config::js_round;

/// Sentinel stored in `user.max_quota` for accounts with no limit.
pub const UNLIMITED_QUOTA: i64 = -1;

const UNLIMITED_WORDS: &[&str] = &["-1", "unlimited", "none", "no limit", "\u{221e}", "inf"];

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

/// `Number.isSafeInteger`
pub fn is_safe_integer(n: f64) -> bool {
    n.is_finite() && n.trunc() == n && n.abs() <= 9_007_199_254_740_991.0
}

/// Parse an admin-typed size into bytes (`20G`, `512 MB`, `1.5TiB`, `1,5G`, a
/// bare byte count, the "unlimited" spellings). `None` when the input cannot
/// be understood, so callers can reject instead of silently writing a wrong
/// limit.
pub fn parse_quota(input: &str) -> Option<i64> {
    let raw = crate::config::js_trim(input).to_lowercase();
    if raw.is_empty() || UNLIMITED_WORDS.contains(&raw.as_str()) {
        return Some(UNLIMITED_QUOTA);
    }
    // /^(\d+(?:[.,]\d+)?)\s*([a-z]+)?$/
    let re = regex::Regex::new(r"^([0-9]+(?:[.,][0-9]+)?)[\t\n\x0B\x0C\r \u{a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}]*([a-z]+)?$")
        .expect("valid regex");
    let m = re.captures(&raw)?;
    let amount: f64 = m.get(1)?.as_str().replacen(',', ".", 1).parse().ok()?;
    if !amount.is_finite() || amount < 0.0 {
        return None;
    }
    let unit = m.get(2).map(|u| u.as_str()).unwrap_or("b");
    let factor = unit_factor(unit)?;
    let bytes = js_round(amount * factor);
    // Postgres bigint would take more, but anything past 2^53 stops being an
    // exact JS number and the quota arithmetic is plain numbers.
    if !is_safe_integer(bytes) {
        return None;
    }
    Some(bytes as i64)
}

/// True when the stored value means "no limit".
pub fn is_unlimited(bytes: Option<i64>) -> bool {
    bytes.is_none_or(|b| b < 0)
}

/// Render a stored quota back into something an admin can edit: the inverse
/// of [`parse_quota`] for round numbers (`21474836480` -> `20G`).
pub fn format_quota_input(bytes: Option<i64>) -> String {
    let Some(n) = bytes.filter(|b| *b >= 0) else {
        return String::new();
    };
    if n == 0 {
        return "0".into();
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
