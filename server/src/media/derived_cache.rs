//! Port of `lib/media/derived-cache.ts`, the part the library needs: where
//! derived files live and how they are dropped with their source
//! (`purgeDerived`), plus `contentHash`. Reading and writing cached
//! conversions belongs to the media port, which extends this module.

use sha2::{Digest, Sha256};

use crate::storage::StorageDriver;

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

/// sha256 of the bytes, hex. The content half of the address.
pub fn content_hash(buf: &[u8]) -> String {
    hex::encode(Sha256::digest(buf))
}

/// Drop everything derived from one upload. Safe to call for an id that has
/// none; a cache we failed to clear is dead weight, never a failed delete.
pub async fn purge_derived(storage: &dyn StorageDriver, upload_id: &str) {
    let _ = storage.delete_prefix(&derived_prefix_for(upload_id)).await;
}
