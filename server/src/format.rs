//! Port of `lib/format.ts` (the parts the server uses).

/// `humanFileSize(bytes, precision = 1)`: `0 B`, `512 B`, `1.5 KB`, ...
pub fn human_file_size(bytes: f64, precision: usize) -> String {
    if !bytes.is_finite() || bytes <= 0.0 {
        return "0 B".into();
    }
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut i = 0;
    let mut n = bytes;
    while n >= 1024.0 && i < UNITS.len() - 1 {
        n /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} {}", crate::config::format_js_number(n), UNITS[i])
    } else {
        format!("{} {}", js_to_fixed(n, precision), UNITS[i])
    }
}

/// `Number.prototype.toFixed(digits)` for finite values below 1e21.
///
/// Both V8 and Rust round the EXACT binary value; they differ only on an exact
/// tie, where V8 picks the larger candidate and Rust the even one.
pub fn js_to_fixed(n: f64, digits: usize) -> String {
    let scale = 10f64.powi(digits as i32);
    let twice = n * 2.0 * scale;
    let is_exact_tie = twice.fract() == 0.0 && (twice.abs() as i64) % 2 == 1 && twice / (2.0 * scale) == n;
    if is_exact_tie {
        // n = (k + 0.5) / 10^d exactly: round away from zero for positives
        // ("the larger n"), towards zero for negatives.
        let k = ((twice - 1.0) / 2.0).floor();
        let up = (k + 1.0) / scale;
        return format!("{up:.digits$}");
    }
    format!("{n:.digits$}")
}

#[cfg(test)]
mod tests {
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
