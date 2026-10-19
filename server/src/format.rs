//! Port of `lib/format.ts`, the server-side part: `humanFileSize`.
//!
//! `relativeTime` (Intl.RelativeTimeFormat) is called only by browser
//! components; the SPA carries upstream's module verbatim
//! (`server/web/lib/format.ts`), so nothing here needs it.

/// `Number.prototype.toFixed(digits)`: the exact binary value rounded to
/// `digits` places, an exact tie going to the LARGER magnitude (Rust's
/// formatter breaks ties to even, which differs on dyadic values such as
/// 1.25 - exactly what byte counts divided by 1024 produce).
pub fn js_to_fixed(x: f64, digits: usize) -> String {
    if !x.is_finite() {
        return crate::i18n::format::js_number_to_string(x);
    }
    let exact = format!("{:.1100}", x.abs());
    let frac = exact.split('.').nth(1).unwrap_or("");
    let tie = frac.as_bytes().get(digits) == Some(&b'5') && frac.as_bytes()[digits + 1..].iter().all(|b| *b == b'0');
    let magnitude = if tie {
        format!("{:.*}", digits, x.abs() + 0.5 * 10f64.powi(-(digits as i32)))
    } else {
        format!("{:.*}", digits, x.abs())
    };
    let zero = magnitude.bytes().all(|b| b == b'0' || b == b'.');
    if x < 0.0 && !zero { format!("-{magnitude}") } else { magnitude }
}

/// Human-readable byte size (e.g. 1.5 GB).
pub fn human_file_size(bytes: f64, precision: usize) -> String {
    if !bytes.is_finite() || bytes <= 0.0 {
        return "0 B".to_string();
    }
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut i = 0;
    let mut n = bytes;
    while n >= 1024.0 && i < UNITS.len() - 1 {
        n /= 1024.0;
        i += 1;
    }
    let shown = if i == 0 { crate::i18n::format::js_number_to_string(n) } else { js_to_fixed(n, precision) };
    format!("{shown} {}", UNITS[i])
}

#[cfg(test)]
mod tests {
    //! tests/unit/format.test.ts (humanFileSize)
    use super::*;

    #[test]
    fn human_file_size_like_upstream() {
        assert_eq!(human_file_size(0.0, 1), "0 B");
        assert_eq!(human_file_size(512.0, 1), "512 B");
        assert_eq!(human_file_size(1024.0, 1), "1.0 KB");
        assert_eq!(human_file_size(1536.0, 1), "1.5 KB");
        assert_eq!(human_file_size(60.0 * 1024.0 * 1024.0, 1), "60.0 MB");
        assert_eq!(human_file_size(-5.0, 1), "0 B");
        assert_eq!(human_file_size(f64::NAN, 1), "0 B");
        // 1.25 GiB: toFixed rounds the exact tie up, Rust's formatter would not.
        assert_eq!(human_file_size(1342177280.0, 1), "1.3 GB");
        assert_eq!(human_file_size(1.5 * 1024f64.powi(6), 1), "1536.0 PB");
    }

    #[test]
    fn to_fixed_ties() {
        assert_eq!(js_to_fixed(0.25, 1), "0.3");
        assert_eq!(js_to_fixed(1.005, 2), "1.00"); // 1.005 is below the tie in binary
        assert_eq!(js_to_fixed(2.5, 0), "3");
        assert_eq!(js_to_fixed(-2.5, 0), "-3");
        assert_eq!(js_to_fixed(-0.04, 1), "-0.0".trim_start_matches('-'));
    }
}

#[cfg(test)]
mod srv_auth_tests {
    //! The srv-auth port's tests of its narrow version of this module, kept
    //! against the reconciled implementation.
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(human_file_size(0.0, 1), "0 B");
        assert_eq!(human_file_size(512.0, 1), "512 B");
        assert_eq!(human_file_size(1536.0, 1), "1.5 KB");
        assert_eq!(human_file_size(10_737_418_240.0, 1), "10.0 GB");
        assert_eq!(human_file_size(1024.0 * 1024.0 * 1.25, 1), "1.3 MB");
        assert_eq!(js_to_fixed(0.125, 2), "0.13");
        assert_eq!(js_to_fixed(2.5, 0), "3");
    }
}
