//! Port of `lib/media/sharp-runtime.ts`: size libvips to the cgroup, once,
//! before anything decodes a picture.
//!
//! sharp reads `os.cpus().length` to pick its thread count, and inside the
//! production container that returns 6 (the host) against a `cpus: 2.0` quota.
//! Six libvips worker threads on a two-core slice do not go faster; they take
//! three times as long to be scheduled, hold three times as many strip buffers
//! while they wait, and drag every other request on the box down with them.
//!
//! Upstream applied the setting on import; here `media::vips::init` applies it
//! the first time libvips is used, which is the same guarantee (no path can
//! decode a picture before it) without a boot-order rule anybody has to
//! remember.

use std::sync::LazyLock;

use crate::runtime::limits::{cgroup_memory_limit, effective_cpu_count};

/// Threads libvips may use per operation.
pub static SHARP_CONCURRENCY: LazyLock<usize> = LazyLock::new(effective_cpu_count);

/// Memory ceiling this process runs under, or `None` when it is not capped.
pub static MEMORY_LIMIT_BYTES: LazyLock<Option<f64>> = LazyLock::new(cgroup_memory_limit);

// libvips also keeps an operation cache. The defaults (50 MB / 20 files / 100
// items) are sane for a server that re-reads the same few pictures; ours reads a
// different file almost every time, so the cache is close to pure overhead.
// Files are what matter most: a cached open descriptor pins the whole decoder
// state, and `pids_limit: 512` is not a lot of room to waste.

/// `sharp.cache({ memory: 32 })`, in megabytes.
pub const CACHE_MEMORY_MB: usize = 32;
/// `sharp.cache({ files: 10 })`.
pub const CACHE_FILES: i32 = 10;
/// `sharp.cache({ items: 50 })`.
pub const CACHE_ITEMS: i32 = 50;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libvips_is_sized_by_the_cgroup_not_the_host() {
        crate::media::vips::init().expect("libvips");
        assert!(*SHARP_CONCURRENCY >= 1);
        assert_eq!(crate::media::vips::concurrency().unwrap() as usize, *SHARP_CONCURRENCY);
    }
}
