//! Port of `lib/runtime/limits.ts`: what THIS process is actually allowed to
//! use, from cgroup v2 (`cpu.max`, `memory.max`). Everything a runtime sizes
//! itself by looks at the HOST; inside a container that is wrong in the same
//! direction every time. Parsing is separated from reading so the interesting
//! part is a unit test rather than a production surprise.

const CPU_MAX: &str = "/sys/fs/cgroup/cpu.max";
const MEMORY_MAX: &str = "/sys/fs/cgroup/memory.max";

/// Parse `cpu.max`: "<quota> <period>", or "max <period>" when uncapped.
/// Returns cores as a float (200000/100000 -> 2), or `None` with no cap.
pub fn parse_cpu_max(raw: Option<&str>) -> Option<f64> {
    let raw = raw?;
    let mut parts = raw.split_whitespace();
    let quota = parts.next()?;
    if quota == "max" {
        return None;
    }
    let q = crate::config::js_number(quota);
    let p = crate::config::js_number(parts.next().unwrap_or("100000"));
    if !q.is_finite() || !p.is_finite() || q <= 0.0 || p <= 0.0 {
        return None;
    }
    Some(q / p)
}

/// Parse `memory.max`: a byte count, or "max" when uncapped.
pub fn parse_memory_max(raw: Option<&str>) -> Option<f64> {
    let t = raw?.trim();
    if t.is_empty() || t == "max" {
        return None;
    }
    let n = crate::config::js_number(t);
    if n.is_finite() && n > 0.0 { Some(n) } else { None }
}

fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Cores this process may burn; `None` outside a capped container.
pub fn cgroup_cpu_limit() -> Option<f64> {
    parse_cpu_max(read(CPU_MAX).as_deref())
}

/// Bytes this process may hold; `None` outside a capped container.
pub fn cgroup_memory_limit() -> Option<f64> {
    parse_memory_max(read(MEMORY_MAX).as_deref())
}

/// How many cores to size thread pools by: the cgroup quota when there is
/// one, the host count otherwise, never below 1. Rounded DOWN, deliberately.
pub fn effective_cpu_count() -> usize {
    let host = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).max(1);
    match cgroup_cpu_limit() {
        None => host,
        Some(quota) => (quota.floor() as usize).min(host).max(1),
    }
}

#[cfg(test)]
mod tests {
    //! tests/unit/runtime-limits.test.ts
    use super::*;

    #[test]
    fn reads_the_quota_the_production_container_really_has() {
        assert_eq!(parse_cpu_max(Some("200000 100000")), Some(2.0));
        assert_eq!(parse_cpu_max(Some("50000 100000")), Some(0.5));
        assert_eq!(parse_memory_max(Some("2147483648")), Some(2.0 * 1024.0 * 1024.0 * 1024.0));
    }

    #[test]
    fn says_no_cap_rather_than_guessing_when_there_is_none() {
        assert_eq!(parse_cpu_max(Some("max 100000")), None);
        assert_eq!(parse_memory_max(Some("max")), None);
        assert_eq!(parse_cpu_max(None), None);
        assert_eq!(parse_memory_max(None), None);
        assert_eq!(parse_cpu_max(Some("")), None);
        assert_eq!(parse_cpu_max(Some("garbage")), None);
        assert_eq!(parse_memory_max(Some("-1")), None);
        assert_eq!(parse_cpu_max(Some("0 100000")), None);
    }

    #[test]
    fn tolerates_the_trailing_newline_the_kernel_writes() {
        assert_eq!(parse_cpu_max(Some("200000 100000\n")), Some(2.0));
        assert_eq!(parse_memory_max(Some("2147483648\n")), Some(2147483648.0));
    }

    #[test]
    fn effective_count_is_at_least_one() {
        assert!(effective_cpu_count() >= 1);
    }
}
