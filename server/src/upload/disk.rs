//! Port of `lib/upload/disk.ts`, NARROW: `FilesystemSpace` and
//! `storageFilesystemSpace` (the statfs the admin panel reads). The upload
//! guards (`freeBytesForUploads`, `fitsOnDisk`, `hasRoomFor`, ...) belong to
//! the uploads port, which will reconcile this file with its own.

use serde::Serialize;

use crate::config::Config;

/// What one statfs call has to say about a filesystem.
///
/// `free + used` deliberately does NOT add up to `total`: the gap is the
/// root-reserved portion, which the kernel keeps back from unprivileged
/// writers. Reporting it any other way would either overstate what we can
/// still write or understate what is already occupied, so this matches `df`
/// exactly.
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

/// Everything the admin panel needs about the storage filesystem, or `None`
/// when there is no such thing (S3) or the syscall failed.
///
/// Same single statfs the upload guard runs on, so the panel and the 507 can
/// never disagree about how much room is left: one measurement, two readers.
pub async fn storage_filesystem_space(config: &Config) -> Option<FilesystemSpace> {
    if config.storage.driver != "local" {
        return None;
    }
    filesystem_space_at(&config.storage.local_path).await
}

/// The one place a statfs is issued. `None` when the path cannot be measured.
///
/// Never blocks an upload on a failed syscall: an unmeasurable filesystem is
/// reported as "no answer", and the callers treat that as "let it through".
async fn filesystem_space_at(path: &str) -> Option<FilesystemSpace> {
    let path = path.to_string();
    tokio::task::spawn_blocking(move || statfs(&path)).await.ok().flatten()
}

/// `fs.statfs(path)` as Node issues it (libuv `uv_fs_statfs`: `statfs(2)` and
/// its `f_bsize` on Linux and macOS), multiplied out like upstream.
fn statfs(path: &str) -> Option<FilesystemSpace> {
    let c_path = std::ffi::CString::new(path).ok()?;
    let mut buf = std::mem::MaybeUninit::<libc::statfs>::zeroed();
    // SAFETY: `c_path` is a valid NUL-terminated string and `buf` points to
    // writable memory of the size statfs(2) fills in.
    let rc = unsafe { libc::statfs(c_path.as_ptr(), buf.as_mut_ptr()) };
    if rc != 0 {
        return None;
    }
    // SAFETY: statfs returned 0, so it initialised the struct.
    let fs = unsafe { buf.assume_init() };
    #[allow(clippy::unnecessary_cast)] // the field widths differ between platforms
    let (bsize, blocks, bfree, bavail) = (fs.f_bsize as u64, fs.f_blocks as u64, fs.f_bfree as u64, fs.f_bavail as u64);
    Some(FilesystemSpace {
        total_bytes: blocks.saturating_mul(bsize),
        free_bytes: bavail.saturating_mul(bsize),
        used_bytes: blocks.saturating_sub(bfree).saturating_mul(bsize),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn measures_an_existing_directory_and_refuses_a_missing_one() {
        let dir = tempfile::tempdir().unwrap();
        let space = filesystem_space_at(dir.path().to_str().unwrap()).await.expect("statfs works on a temp dir");
        assert!(space.total_bytes > 0);
        assert!(space.free_bytes <= space.total_bytes);
        assert!(space.used_bytes <= space.total_bytes);
        assert_eq!(filesystem_space_at("/nonexistent/modlight/storage").await, None);
        assert_eq!(filesystem_space_at("bad\0path").await, None);
    }

    #[test]
    fn serializes_like_upstream() {
        let v = serde_json::to_value(FilesystemSpace { total_bytes: 3, free_bytes: 1, used_bytes: 2 }).unwrap();
        assert_eq!(v, serde_json::json!({ "totalBytes": 3, "freeBytes": 1, "usedBytes": 2 }));
    }
}
