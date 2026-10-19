//! Port of `lib/upload/disk.ts`: free-space rules for uploads.
//!
//! A full disk is not a degraded state: Postgres stops accepting writes,
//! backups truncate mid-file, and every other application on the host goes
//! down with us. So both upload paths refuse an oversized transfer at the door
//! rather than after it, and the admin panel reads the same single statfs the
//! guard runs on.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::Config;

/// What one statfs call has to say about a filesystem. `free + used` does NOT
/// add up to `total`: the gap is the root-reserved portion (this matches `df`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesystemSpace {
    /// Size of the whole filesystem.
    pub total_bytes: u64,
    /// Bytes an unprivileged process may still write (df's "Avail").
    pub free_bytes: u64,
    /// Bytes occupied by everything on this filesystem, not just by us.
    pub used_bytes: u64,
}

/// The one place a statfs is issued. `None` when the path cannot be measured
/// (an unmeasurable filesystem is "no answer", and callers let it through).
pub fn filesystem_space_at(path: &Path) -> Option<FilesystemSpace> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c_path` is a valid NUL-terminated string and `st` a properly
    // sized, writable statvfs struct.
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut st) };
    if rc != 0 {
        return None;
    }
    let frsize = if st.f_frsize > 0 { st.f_frsize as u64 } else { st.f_bsize as u64 };
    let blocks = st.f_blocks as u64;
    let bfree = st.f_bfree as u64;
    let bavail = st.f_bavail as u64;
    Some(FilesystemSpace {
        total_bytes: blocks * frsize,
        free_bytes: bavail * frsize,
        used_bytes: blocks.saturating_sub(bfree) * frsize,
    })
}

async fn filesystem_space(path: PathBuf) -> Option<FilesystemSpace> {
    tokio::task::spawn_blocking(move || filesystem_space_at(&path)).await.ok().flatten()
}

/// Free bytes on the filesystem holding `path`, or `None`.
async fn free_bytes_at(path: &str) -> Option<u64> {
    filesystem_space(PathBuf::from(path)).await.map(|s| s.free_bytes)
}

/// Free bytes on the filesystem that holds uploads, or `None` when the
/// question does not apply (S3) or cannot be answered.
pub async fn free_bytes_for_uploads(config: &Config) -> Option<u64> {
    if config.storage.driver != "local" {
        return None;
    }
    free_bytes_at(&config.storage.local_path).await
}

/// Everything the admin panel needs about the storage filesystem, or `None`
/// (S3, or the syscall failed). Same statfs the upload guard runs on.
pub async fn storage_filesystem_space(config: &Config) -> Option<FilesystemSpace> {
    if config.storage.driver != "local" {
        return None;
    }
    filesystem_space(PathBuf::from(&config.storage.local_path)).await
}

/// Whether `size` more bytes fit without eating into the operator's reserve.
/// `free == None` means we could not measure and must not guess.
pub fn fits_on_disk(size: f64, free: Option<f64>, min_free: f64) -> bool {
    let Some(free) = free else {
        return true;
    };
    if min_free <= 0.0 {
        return free - size >= 0.0;
    }
    free - size >= min_free
}

/// How worried an operator should be about the storage filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiskPressure {
    Unknown,
    Ok,
    Low,
    Critical,
}

/// Below this share of the disk, say something.
pub const LOW_FREE_RATIO: f64 = 0.1;
/// Below this share, say it loudly.
pub const CRITICAL_FREE_RATIO: f64 = 0.05;

/// Severity for the admin panel, from the same two numbers the upload guard
/// uses: the reserve (the operationally real line) and the ratio (the slow
/// fill on a big disk).
pub fn disk_pressure(free: Option<f64>, total: Option<f64>, min_free: f64) -> DiskPressure {
    let (Some(free), Some(total)) = (free, total) else {
        return DiskPressure::Unknown;
    };
    if !free.is_finite() || !total.is_finite() || total <= 0.0 || free < 0.0 {
        return DiskPressure::Unknown;
    }
    let ratio = free / total;
    if ratio <= CRITICAL_FREE_RATIO {
        return DiskPressure::Critical;
    }
    if min_free > 0.0 && free <= min_free {
        return DiskPressure::Critical;
    }
    if ratio <= LOW_FREE_RATIO {
        return DiskPressure::Low;
    }
    if min_free > 0.0 && free <= min_free * 2.0 {
        return DiskPressure::Low;
    }
    DiskPressure::Ok
}

/// Can the host accept a file of `size` bytes, given where it will be staged?
/// Both filesystems are checked with the FULL size (when they differ, the copy
/// has both on the floor at the same moment). Runs BEFORE the first byte is
/// accepted.
pub async fn has_room_for(config: &Config, size: f64, staging_path: &str) -> bool {
    if config.storage.driver != "local" {
        return true;
    }
    if !size.is_finite() || size <= 0.0 {
        return true;
    }
    let min_free = config.uploads.min_free_bytes as f64;
    let mut paths: Vec<&str> = vec![config.storage.local_path.as_str()];
    if staging_path != config.storage.local_path {
        paths.push(staging_path);
    }
    for path in paths {
        if !fits_on_disk(size, free_bytes_at(path).await.map(|f| f as f64), min_free) {
            return false;
        }
    }
    true
}

/// Device id of the nearest existing ancestor of `path` (storage and staging
/// directories are created lazily, and a missing directory must not be
/// mistaken for "a different disk").
fn device_of(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let mut current = crate::storage::local::resolve_lexically(&cwd, path);
    loop {
        if let Ok(meta) = std::fs::metadata(&current) {
            return Some(meta.dev());
        }
        let parent = current.parent()?.to_path_buf();
        if parent == current {
            return None;
        }
        current = parent;
    }
}

/// Whether two paths live on the same filesystem (a free rename or a full
/// copy). Unknown answers come back `false`: over-reserving is safe.
pub async fn same_filesystem(a: &Path, b: &Path) -> bool {
    let (a, b) = (a.to_path_buf(), b.to_path_buf());
    tokio::task::spawn_blocking(move || match (device_of(&a), device_of(&b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    })
    .await
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    //! tests/unit/upload-disk.test.ts
    use super::*;

    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const FREE: f64 = 35.0 * GIB;
    const RESERVE: f64 = 10.0 * GIB;

    #[test]
    fn fits_on_disk_rules() {
        assert!(fits_on_disk(25.0 * GIB, Some(FREE), RESERVE));
        assert!(fits_on_disk(FREE - RESERVE, Some(FREE), RESERVE));
        assert!(!fits_on_disk(FREE - RESERVE + 1.0, Some(FREE), RESERVE));
        assert!(!fits_on_disk(50.0 * GIB, Some(FREE), RESERVE));
        assert!(fits_on_disk(50.0 * GIB, Some(99.0 * GIB), RESERVE));
        assert!(fits_on_disk(50.0 * GIB, None, RESERVE));
        assert!(fits_on_disk(FREE, Some(FREE), 0.0));
        assert!(!fits_on_disk(FREE + 1.0, Some(FREE), 0.0));
        assert!(fits_on_disk(0.0, Some(FREE), RESERVE));
        assert!(!fits_on_disk(0.0, Some(5.0 * GIB), RESERVE));
    }

    #[test]
    fn pressure() {
        let total = Some(96.0 * GIB);
        assert_eq!(disk_pressure(Some(FREE), total, RESERVE), DiskPressure::Ok);
        assert_eq!(disk_pressure(Some(RESERVE - 1.0), total, RESERVE), DiskPressure::Critical);
        assert_eq!(disk_pressure(Some(RESERVE), total, RESERVE), DiskPressure::Critical);
        assert_eq!(disk_pressure(Some(15.0 * GIB), total, RESERVE), DiskPressure::Low);
        let big = Some(4096.0 * GIB);
        assert_eq!(disk_pressure(Some(300.0 * GIB), big, RESERVE), DiskPressure::Low);
        assert_eq!(disk_pressure(Some(150.0 * GIB), big, RESERVE), DiskPressure::Critical);
        assert_eq!(disk_pressure(Some(900.0 * GIB), big, RESERVE), DiskPressure::Ok);
        assert_eq!(disk_pressure(Some(2.0 * GIB), total, 0.0), DiskPressure::Critical);
        assert_eq!(disk_pressure(Some(8.0 * GIB), total, 0.0), DiskPressure::Low);
        assert_eq!(disk_pressure(Some(FREE), total, 0.0), DiskPressure::Ok);
        assert_eq!(disk_pressure(Some(FREE), total, 200.0 * GIB), DiskPressure::Critical);
        assert_eq!(disk_pressure(None, total, RESERVE), DiskPressure::Unknown);
        assert_eq!(disk_pressure(Some(FREE), None, RESERVE), DiskPressure::Unknown);
        assert_eq!(disk_pressure(Some(FREE), Some(0.0), RESERVE), DiskPressure::Unknown);
        assert_eq!(disk_pressure(Some(f64::NAN), total, RESERVE), DiskPressure::Unknown);
        assert_eq!(disk_pressure(Some(-1.0), total, RESERVE), DiskPressure::Unknown);
        assert_eq!(disk_pressure(Some(0.0), total, RESERVE), DiskPressure::Critical);
    }

    #[tokio::test]
    async fn same_filesystem_for_missing_and_sibling_dirs() {
        let root = tempfile::tempdir().unwrap();
        assert!(same_filesystem(&root.path().join("storage"), &root.path().join("tmp")).await);
        let deep = root.path().join("a/b/c/d");
        assert!(same_filesystem(&deep, root.path()).await);
        assert!(filesystem_space_at(root.path()).unwrap().total_bytes > 0);
    }
}

#[cfg(test)]
mod srv_auth_tests {
    //! The srv-auth port's tests of its narrow version of this module, kept
    //! against the reconciled implementation.
    use super::*;

    #[test]
    fn measures_an_existing_directory_and_refuses_a_missing_one() {
        let dir = tempfile::tempdir().unwrap();
        let space = filesystem_space_at(dir.path()).expect("statfs works on a temp dir");
        assert!(space.total_bytes > 0);
        assert!(space.free_bytes <= space.total_bytes);
        assert!(space.used_bytes <= space.total_bytes);
        assert_eq!(filesystem_space_at(std::path::Path::new("/nonexistent/modlight/storage")), None);
        assert_eq!(filesystem_space_at(std::path::Path::new("bad\0path")), None);
    }

    #[test]
    fn serializes_like_upstream() {
        let v = serde_json::to_value(FilesystemSpace { total_bytes: 3, free_bytes: 1, used_bytes: 2 }).unwrap();
        assert_eq!(v, serde_json::json!({ "totalBytes": 3, "freeBytes": 1, "usedBytes": 2 }));
    }
}
