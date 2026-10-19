//! Port of `lib/media/derived-cache.ts`: cache for files we DERIVE from an
//! upload (format conversions, posters, motion tiles, timelines).
//!
//! `/convert` is public for every published file, uncached, and costs seconds
//! of CPU per call. Without a cache the cheapest denial of service against this
//! deployment is a `for` loop over one link, and every honest second click pays
//! full price too.
//!
//! Four properties, and each one is a rule the rest of the app already relies on:
//!
//!  1. CONTENT-ADDRESSED. The key carries a sha256 of the source bytes (or the
//!     row's checksum) plus a variant tag naming the target AND the encoder
//!     settings.
//!  2. OUTSIDE THE QUOTA. Derived objects live under `derived/`, never under a
//!     user's tag, and the admin's stray file scan skips the prefix.
//!  3. CLEANED UP WITH THE SOURCE. One prefix per upload id, one
//!     `delete_prefix` on delete, on trash, and on the retention purge.
//!  4. NEVER FOR A RESTRICTED FILE. A password, an expiry, a view cap or an
//!     unpublished flag means the bytes are gated, and a cache entry is a copy
//!     that outlives the gate.
//!
//! Every read and write takes the lazily built [`Storage`] and swallows a
//! driver that cannot be built, as upstream's `getStorage()` inside its `try`.

use std::path::Path;

use bytes::Bytes;
use sha2::{Digest, Sha256};

use crate::storage::{ByteStream, Storage, StorageDriver, bytes_stream};

/// Everything derived lives under this one prefix. Never a user tag.
pub const DERIVED_PREFIX: &str = "derived";

/// Bumped when the meaning of a cached object changes for reasons the variant
/// tag cannot express.
pub const CACHE_EPOCH: &str = "1";

/// Everything cached for one upload. Handed straight to `delete_prefix`.
pub fn derived_prefix_for(upload_id: &str) -> String {
    format!("{DERIVED_PREFIX}/{upload_id}/")
}

/// Where one derived artefact lives.
pub fn derived_key(upload_id: &str, content_hash: &str, variant: &str, ext: &str) -> String {
    format!("{}{content_hash}.{CACHE_EPOCH}.{variant}.{ext}", derived_prefix_for(upload_id))
}

/// `CacheableMedia`: what decides whether a derivative may be kept.
#[derive(Debug, Clone, Copy)]
pub struct CacheableMedia<'a> {
    pub published: bool,
    pub password_hash: Option<&'a str>,
    pub expires_at: Option<crate::db::types::JsDate>,
    pub max_views: Option<i32>,
}

impl<'a> From<&'a crate::db::models::Upload> for CacheableMedia<'a> {
    fn from(u: &'a crate::db::models::Upload) -> Self {
        CacheableMedia {
            published: u.published,
            password_hash: u.password_hash.as_deref(),
            expires_at: u.expires_at,
            max_views: u.max_views,
        }
    }
}

/// `mayCacheDerived(media)`: only an ordinary public file may have a cached
/// derivative; a copy of a gated file would outlive its gate (and an expiry
/// deletes no row, so nothing would chase it).
pub fn may_cache_derived(media: CacheableMedia<'_>) -> bool {
    media.published
        && media.password_hash.is_none_or(str::is_empty)
        && media.expires_at.is_none()
        && media.max_views.is_none()
}

/// sha256 of the bytes, hex. The content half of the address.
pub fn content_hash(buf: &[u8]) -> String {
    hex::encode(Sha256::digest(buf))
}

/// `readDerived(key)`: cached bytes, or `None` on a miss (or on any storage
/// trouble - a miss is safe).
pub async fn read_derived(storage: &Storage, key: &str) -> Option<Bytes> {
    let driver = storage.get().ok()?;
    driver.read_buffer(key).await.ok()
}

/// `writeDerived(key, body)`: best effort; a cache that cannot be written is
/// still a working conversion.
pub async fn write_derived(storage: &Storage, key: &str, body: Bytes) {
    if let Ok(driver) = storage.get() {
        let _ = driver.write(key, bytes_stream(body)).await;
    }
}

/// `writeDerivedFile(key, localPath)`: cache a derived file that lives on
/// local disk, without loading it. A remux is the same size as its source - up
/// to 512 MB - so the video path hands storage a STREAM of a COPY (the file
/// itself stays where it is: the response streams it afterwards). Best effort.
pub async fn write_derived_file(storage: &Storage, key: &str, local_path: &Path) {
    let Ok(driver) = storage.get() else { return };
    let Ok(file) = tokio::fs::File::open(local_path).await else { return };
    let body: ByteStream = Box::pin(tokio_util::io::ReaderStream::new(file));
    let _ = driver.write(key, body).await;
}

/// `readDerivedStream(key)`: a cached object as a stream plus its size, or
/// `None` on a miss. Size comes first and separately because the response
/// needs a Content-Length.
pub async fn read_derived_stream(storage: &Storage, key: &str) -> Option<(ByteStream, u64)> {
    let driver = storage.get().ok()?;
    let info = driver.stat(key).await.ok()??;
    if info.size == 0 {
        return None;
    }
    let stream = driver.read_stream(key, None).await.ok()?;
    Some((stream, info.size))
}

/// Drop everything derived from one upload. Safe to call for an id that has
/// none; a cache we failed to clear is dead weight, never a failed delete.
pub async fn purge_derived(storage: &dyn StorageDriver, upload_id: &str) {
    let _ = storage.delete_prefix(&derived_prefix_for(upload_id)).await;
}

/// [`purge_derived`] with upstream's lazy `getStorage()` inside the `try`: a
/// storage driver that cannot be built is swallowed like any other failure.
pub async fn purge_derived_lazy(storage: &crate::storage::Storage, upload_id: &str) {
    if let Ok(driver) = storage.get() {
        purge_derived(driver.as_ref(), upload_id).await;
    }
}

#[cfg(test)]
mod tests {
    //! tests/unit/derived-cache.test.ts
    use super::*;

    const UPLOAD: &str = "6f1c0f2e-0f3a-4a1a-9d3e-2f0d8c9a1b22";

    #[test]
    fn where_a_derived_file_lives() {
        let key = derived_key(UPLOAD, &content_hash(b"x"), "png", "png");
        assert!(key.starts_with(&format!("{DERIVED_PREFIX}/")));
        assert!(key.starts_with(&derived_prefix_for(UPLOAD)));
        let hash = content_hash(b"x");
        for variant in ["png", "pdf", "webp-anim", "avif"] {
            let ext = variant.split('-').next().unwrap();
            assert!(derived_key(UPLOAD, &hash, variant, ext).starts_with(&derived_prefix_for(UPLOAD)));
        }
        assert!(derived_prefix_for(UPLOAD).ends_with('/'));
        let (a, b) = (content_hash(b"one"), content_hash(b"two"));
        assert_ne!(a, b);
        assert_ne!(derived_key(UPLOAD, &a, "png", "png"), derived_key(UPLOAD, &b, "png", "png"));
        assert_ne!(derived_key(UPLOAD, &a, "webp", "webp"), derived_key(UPLOAD, &a, "webp-anim", "webp"));
        let h = content_hash(b"same bytes");
        assert_eq!(derived_key(UPLOAD, &h, "pdf", "pdf"), derived_key(UPLOAD, &h, "pdf", "pdf"));
    }

    #[test]
    fn what_may_be_cached() {
        let open = CacheableMedia { published: true, password_hash: None, expires_at: None, max_views: None };
        assert!(may_cache_derived(open));
        assert!(!may_cache_derived(CacheableMedia { published: false, ..open }));
        assert!(!may_cache_derived(CacheableMedia { password_hash: Some("scrypt$..."), ..open }));
        assert!(!may_cache_derived(CacheableMedia { expires_at: Some(crate::db::types::JsDate::now()), ..open }));
        assert!(!may_cache_derived(CacheableMedia { max_views: Some(1), ..open }));
    }
}
