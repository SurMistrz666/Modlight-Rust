//! Port of `lib/upload/finalize.ts`: the single finalize pipeline shared by
//! the ShareX (multipart) and web (tus) paths.
//!
//! Ordering is deliberate: the quota reservation happens BEFORE any bytes
//! move, and every step after it is undone on failure. A crash between the
//! storage write and the DB insert must not leave a file nobody can see or
//! delete, and a crash after the insert must not leave the quota
//! un-incremented.

use std::path::Path;

use bytes::Bytes;
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::Config;
use crate::db::models::{JsDate, User};
use crate::media::mime::{MediaKind, media_kind};
use crate::media::sniff::{UploadTypeInput, resolve_upload_type};
use crate::media::urls::{LinkOwner, delete_url, ext_of, media_urls, storage_key_for, user_base_url};
use crate::naming::{CodeBaseInput, allocate_code, is_naming_scheme, is_reserved_tag, local_now, plan_code};
use crate::storage::StorageDriver;
use crate::upload::disk::{fits_on_disk, free_bytes_for_uploads, same_filesystem};

/// Why finalize refused, in the words the clients have always received.
#[derive(Debug, thiserror::Error)]
pub enum FinalizeError {
    #[error("User disk quota exceeded.")]
    QuotaExceeded,
    /// The host is running out of room, regardless of the user's quota.
    #[error("Server is low on disk space; upload rejected.")]
    OutOfSpace,
    #[error("Custom url code already exists.")]
    CodeTaken,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<sqlx::Error> for FinalizeError {
    fn from(e: sqlx::Error) -> Self {
        FinalizeError::Other(e.into())
    }
}

/// `FinalizeInput`
pub struct FinalizeInput<'a> {
    pub owner: &'a User,
    pub filename: &'a str,
    /// What the CLIENT claimed. The stored type is decided here.
    pub mime: &'a str,
    pub size: i64,
    /// First bytes of the payload, when the caller already had them in hand.
    pub head: Option<Bytes>,
    /// Temp file to ingest (multipart / local tus).
    pub local_path: Option<&'a Path>,
    /// Already in storage at this key (S3 tus): used as is, no copy.
    pub existing_key: Option<&'a str>,
    /// Optional precomputed sha256.
    pub checksum: Option<String>,
    /// Optional ShareX custom URL code.
    pub custom_code: Option<&'a str>,
    /// Where the file should land, ALREADY checked against the owner: `None`
    /// keeps the account default, `Some(None)` is the root.
    pub folder_id: Option<Option<Uuid>>,
}

/// `FinalizeResult`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizeResult {
    pub id: Uuid,
    pub code: String,
    pub ext: String,
    pub storage_key: String,
    pub url: String,
    pub raw_url: String,
    pub deletion_url: String,
    /// The link a client should put on the clipboard, from `user.copy_raw`.
    pub copy_url: String,
    /// Folder the row ended up in (`None` = root).
    pub folder_id: Option<Uuid>,
}

// ---------------------------------------------------------------------------
// Quota
// ---------------------------------------------------------------------------

/// Bytes this user may still store, or `None` when unlimited (`max_quota` of
/// -1, or NULL for rows predating the column).
pub fn remaining_quota(used: Option<i64>, max: Option<i64>) -> Option<i64> {
    let max = max.unwrap_or(-1);
    if max < 0 {
        return None;
    }
    Some((max - used.unwrap_or(0)).max(0))
}

/// Whether an upload of `size` bytes fits, given the same conventions.
pub fn quota_fits(used: Option<i64>, max: Option<i64>, size: i64) -> bool {
    remaining_quota(used, max).is_none_or(|left| size <= left)
}

/// Reserve `size` bytes against the owner's quota, atomically: the predicate
/// is in the UPDATE's WHERE, so two concurrent uploads cannot both pass.
async fn reserve_quota(db: &PgPool, user_id: &str, size: i64) -> Result<(), FinalizeError> {
    let reserved: Option<Option<i64>> = sqlx::query_scalar(
        r#"update "user" set "used_quota" = coalesce("user"."used_quota", 0) + $1, "updated_at" = $2
           where ("user"."id" = $3 and (
             "user"."max_quota" is null
             or "user"."max_quota" < 0
             or coalesce("user"."used_quota", 0) + $1 <= "user"."max_quota"))
           returning "used_quota""#,
    )
    .bind(size)
    .bind(JsDate::now())
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    if reserved.is_none() {
        return Err(FinalizeError::QuotaExceeded);
    }
    Ok(())
}

/// Give a reservation back when the upload it was made for never landed.
async fn release_quota(db: &PgPool, user_id: &str, size: i64) -> sqlx::Result<()> {
    sqlx::query(
        r#"update "user" set "used_quota" = greatest(coalesce("user"."used_quota", 0) - $1, 0), "updated_at" = $2 where "user"."id" = $3"#,
    )
    .bind(size)
    .bind(JsDate::now())
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(())
}

/// Bytes the ingest is still going to consume on the storage filesystem: 0 for
/// a same-filesystem rename (charging it twice would reject a fully
/// transferred file on a disk that has room for it), the full size for a
/// cross-device copy.
async fn pending_write_cost(config: &Config, input: &FinalizeInput<'_>) -> i64 {
    let Some(local) = input.local_path else {
        return 0; // already in storage (S3 tus): nothing to write
    };
    if same_filesystem(local, Path::new(&config.storage.local_path)).await { 0 } else { input.size }
}

/// Refuse an upload that would leave the filesystem with less than
/// `MIN_FREE_DISK_BYTES` to spare (local driver only). With a cost of 0 this
/// still fires when the disk is already below the reserve.
async fn assert_disk_headroom(config: &Config, cost: i64) -> Result<(), FinalizeError> {
    if config.storage.driver != "local" {
        return Ok(());
    }
    let floor = config.uploads.min_free_bytes;
    if floor <= 0 {
        return Ok(());
    }
    let free = free_bytes_for_uploads(config).await.map(|f| f as f64);
    if !fits_on_disk(cost as f64, free, floor as f64) {
        return Err(FinalizeError::OutOfSpace);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Code allocation
// ---------------------------------------------------------------------------

/// Clean a client-supplied code down to something safe to put in a URL; ""
/// when nothing usable survives or when it would shadow an application route.
fn sanitize_custom_code(raw: &str) -> String {
    let clean: String = crate::ids::slugify(raw).chars().take(60).collect();
    if clean.is_empty() || is_reserved_tag(&clean) {
        return String::new();
    }
    clean
}

pub async fn code_is_taken(db: &PgPool, candidate: &str) -> sqlx::Result<bool> {
    let hit: Option<Uuid> = sqlx::query_scalar(r#"select "id" from "uploads" where "uploads"."code" = $1 limit 1"#)
        .bind(candidate)
        .fetch_optional(db)
        .await?;
    Ok(hit.is_some())
}

/// Allocate the public code for a new upload according to the owner's naming
/// scheme (also used by a conversion made as a COPY).
pub async fn allocate_code_for(db: &PgPool, owner: &User, filename: &str) -> anyhow::Result<String> {
    let scheme = owner.naming_scheme.as_deref().filter(|s| is_naming_scheme(Some(s))).unwrap_or("random");
    let plan = plan_code(&CodeBaseInput {
        scheme,
        pattern: owner.naming_pattern.as_deref(),
        filename,
        now: local_now(),
        word_count: owner.naming_word_count.map(f64::from),
        random_length: owner.naming_random_length.map(f64::from),
    });
    let pool = db.clone();
    allocate_code(
        &plan.base,
        move |candidate| {
            let pool = pool.clone();
            async move { Ok(code_is_taken(&pool, &candidate).await?) }
        },
        plan.regenerate.as_ref(),
    )
    .await
}

// ---------------------------------------------------------------------------
// Finalize
// ---------------------------------------------------------------------------

/// The image branch reads the whole object, so it is bounded.
const IMAGE_META_MAX: i64 = 64 * 1024 * 1024;

/// Finalize an upload whose bytes have arrived: code, headroom, quota, ingest,
/// type, metadata, row, links. Every step after the reservation is undone on
/// failure; only bytes WE wrote are removed (with `existing_key` the object is
/// still the upload's own tus artifact, and the client may resume onto it).
pub async fn finalize_upload(
    db: &PgPool,
    config: &Config,
    storage: &dyn StorageDriver,
    input: FinalizeInput<'_>,
) -> Result<FinalizeResult, FinalizeError> {
    let owner = input.owner;
    let filename = input.filename;
    let size = input.size;

    // 1) code (custom or generated): no side effects, so it comes first.
    let ext = ext_of(filename);
    let code = match input.custom_code.filter(|c| !c.is_empty()) {
        Some(raw) => {
            let custom = sanitize_custom_code(raw);
            if custom.is_empty() || code_is_taken(db, &custom).await? {
                return Err(FinalizeError::CodeTaken);
            }
            custom
        }
        None => allocate_code_for(db, owner, filename).await?,
    };

    // Not upstream: an account with file encryption on stores its uploads
    // encrypted with its public key (docs/ENCRYPTION.md section 5).
    let sealer = crate::vault::write::sealer_for(db, &config.auth.secret, &owner.id).await?;

    // 2) host headroom, before the per-user quota. An encrypted ingest writes a
    // whole new copy, never a rename.
    let cost = if sealer.is_some() && input.local_path.is_some() {
        input.size
    } else {
        pending_write_cost(config, &input).await
    };
    assert_disk_headroom(config, cost).await?;

    // 3) quota: atomic reserve.
    reserve_quota(db, &owner.id, size).await?;

    let storage_key = match (input.existing_key, sealer.is_some()) {
        (Some(k), false) => k.to_string(),
        // The ciphertext of an object already in storage goes beside it.
        _ => storage_key_for(owner.user_code.as_deref().unwrap_or("u"), &code, &ext),
    };
    let mut wrote_bytes = false;
    let mut encryption: Option<String> = None;

    let outcome: Result<FinalizeResult, FinalizeError> = async {
        // Not upstream: with the vault, the plaintext is described first (from
        // the temp file, or the object an S3 tus upload left) and encrypted
        // after; it never lands in storage as it is.
        let plain = sealer.as_ref().map(|_| PlainSource::of(&input));

        // 3) ingest bytes
        if plain.is_none() && input.existing_key.is_none() {
            let Some(local) = input.local_path else {
                return Err(anyhow::anyhow!("finalizeUpload: no source provided").into());
            };
            storage.ingest_local_file(local, &storage_key).await?;
            wrote_bytes = true;
        }

        // 4) what the file actually IS (signature, extension, client claim).
        let head = match (input.head.as_ref().filter(|h| !h.is_empty()), &plain) {
            (Some(h), _) => h.clone(),
            (None, Some(p)) => p.head(storage).await,
            (None, None) => crate::storage::read_head(storage, &storage_key, 4096).await,
        };
        let mime = resolve_upload_type(UploadTypeInput {
            client_mime: Some(input.mime),
            filename: Some(filename),
            ext: None,
            head: Some(&head),
        });

        // 5) image metadata (best-effort, small images only).
        let mut width: Option<i64> = None;
        let mut height: Option<i64> = None;
        let mut blurhash: Option<String> = None;
        let mut frames: Option<i64> = None;
        if mime.starts_with("image/")
            && size <= IMAGE_META_MAX
            && let Ok(buf) = match &plain {
                Some(p) => p.whole(storage).await,
                None => storage.read_buffer(&storage_key).await,
            }
        {
            let meta = crate::media::image::image_meta_from_buffer(buf).await;
            width = meta.width.map(i64::from);
            height = meta.height.map(i64::from);
            // Not upstream: a blurhash is a small picture of the content, never
            // kept for an encrypted file.
            blurhash = if plain.is_some() { None } else { meta.blurhash };
            frames = meta.frames.map(i64::from);
        }

        // 5b) audio/video metadata, from the same ffprobe the conversion path
        // uses. Best-effort in the strongest sense: never fails the upload.
        let mut duration_seconds: Option<f64> = None;
        let mut fps: Option<f64> = None;
        let av_kind = media_kind(Some(&mime), Some(&ext));
        let av_meta = if av_kind == MediaKind::Video || av_kind == MediaKind::Audio {
            match &plain {
                Some(p) => p.av(storage).await,
                None => crate::media::av_meta::probe_stored_av_meta(storage, &storage_key).await,
            }
        } else {
            None
        };
        if let Some(meta) = av_meta {
            width = meta.width.map(i64::from);
            height = meta.height.map(i64::from);
            frames = meta.frames.map(i64::from);
            duration_seconds = meta.duration_seconds;
            fps = meta.fps;
        }

        // 5c) Not upstream: now the plaintext is encrypted into storage.
        if let (Some(sealer), Some(p)) = (&sealer, &plain) {
            let header = match p {
                PlainSource::Stored(existing) => {
                    let Some(stat) = storage.stat(existing).await? else {
                        return Err(anyhow::anyhow!("finalizeUpload: the uploaded object is missing").into());
                    };
                    let body = storage.read_stream(existing, None).await?;
                    sealer.write_stream(storage, &storage_key, body, stat.size).await?
                }
                PlainSource::Local(local) => {
                    let header = sealer.write_local_file(storage, &storage_key, local).await?;
                    // What the ingest would have done: the temp file is gone.
                    let _ = tokio::fs::remove_file(local).await;
                    header
                }
            };
            wrote_bytes = true;
            encryption = Some(header);
        }

        // 6) DB row. The account default password is COPIED onto the file.
        let delete_token = crate::ids::secure_token(16);
        let (password_hash, password_version) = match owner.default_password_hash.as_deref().filter(|h| !h.is_empty()) {
            Some(hash) => (Some(hash.to_string()), Some(owner.default_password_version)),
            None => (None, None),
        };
        // An explicit target wins; without one the account default stands.
        let folder_id: Option<Uuid> = match input.folder_id {
            Some(explicit) => explicit,
            None => owner.default_folder_id,
        };
        // Not upstream: the vault is re-read under a lock with the insert, so a
        // switch on or off that raced with this upload is never missed.
        let mut tx = db.begin().await?;
        crate::vault::write::confirm_vault_for_write(&mut tx, &owner.id, encryption.is_some()).await?;
        let (id, folder): (Uuid, Option<Uuid>) = sqlx::query_as(
            r#"insert into "uploads" ("owner_id", "code", "filename", "ext", "mime_type", "size", "storage_key", "checksum",
                                      "width", "height", "frames", "duration_seconds", "fps", "blurhash", "published",
                                      "delete_token", "password_hash", "password_version", "folder_id", "encryption")
               values ($1, $2, $3, $4, $5, $6, $7, $8, $9::int, $10::int, $11::int, $12, $13, $14, $15, $16, $17, $18, $19, $20)
               returning "id", "folder_id""#,
        )
        .bind(&owner.id)
        .bind(&code)
        .bind(filename)
        .bind(&ext)
        .bind(&mime)
        .bind(size)
        .bind(&storage_key)
        .bind(&input.checksum)
        .bind(width)
        .bind(height)
        .bind(frames)
        .bind(duration_seconds)
        .bind(fps)
        .bind(&blurhash)
        .bind(owner.hide_uploads != Some(true))
        .bind(&delete_token)
        .bind(&password_hash)
        .bind(password_version)
        .bind(folder_id)
        .bind(&encryption)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        // The plaintext an S3 tus upload left in the bucket: its ciphertext now
        // lives at `storage_key`.
        if encryption.is_some()
            && let Some(existing) = input.existing_key
            && existing != storage_key
            && let Err(e) = storage.delete(existing).await
        {
            tracing::error!("[finalize] plaintext object left after encrypting: {existing} {e:#}");
        }

        let uc = owner.user_code.as_deref().unwrap_or("u");
        let base = user_base_url(config, owner.preferred_domain.as_deref());
        let links = media_urls(config, &LinkOwner::of_user(owner), &code, &ext);
        let copy_url = if owner.copy_raw == Some(true) { links.raw_url.clone() } else { links.url.clone() };
        Ok(FinalizeResult {
            id,
            code: code.clone(),
            ext: ext.clone(),
            storage_key: storage_key.clone(),
            url: links.url,
            raw_url: links.raw_url,
            copy_url,
            folder_id: folder,
            // The deletion link always carries the tag.
            deletion_url: delete_url(uc, &code, &delete_token, &base),
        })
    }
    .await;

    match outcome {
        Ok(result) => Ok(result),
        Err(err) => {
            // Unwind in reverse; only bytes WE wrote are removed.
            if wrote_bytes && let Err(e) = storage.delete(&storage_key).await {
                tracing::error!("[finalize] orphaned object left in storage: {storage_key} {e:#}");
            }
            if let Err(e) = release_quota(db, &owner.id, size).await {
                tracing::error!("[finalize] could not release quota reservation: {} {e}", owner.id);
            }
            Err(err)
        }
    }
}

/// Not upstream: where finalize reads an upload's plaintext from when the
/// file vault will encrypt it (docs/ENCRYPTION.md): the temp file, or the
/// object an S3 tus upload left in the bucket.
enum PlainSource<'a> {
    Local(&'a Path),
    Stored(&'a str),
}

impl<'a> PlainSource<'a> {
    fn of(input: &FinalizeInput<'a>) -> PlainSource<'a> {
        match (input.existing_key, input.local_path) {
            (Some(key), _) => PlainSource::Stored(key),
            (None, Some(path)) => PlainSource::Local(path),
            // finalize refuses this shape before anything is read.
            (None, None) => PlainSource::Stored(""),
        }
    }

    async fn head(&self, storage: &dyn StorageDriver) -> Bytes {
        use tokio::io::AsyncReadExt;
        match self {
            PlainSource::Stored(key) => crate::storage::read_head(storage, key, 4096).await,
            PlainSource::Local(path) => {
                let Ok(mut file) = tokio::fs::File::open(path).await else { return Bytes::new() };
                let mut buf = vec![0u8; 4096];
                let mut filled = 0;
                while filled < buf.len() {
                    match file.read(&mut buf[filled..]).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => filled += n,
                    }
                }
                buf.truncate(filled);
                Bytes::from(buf)
            }
        }
    }

    async fn whole(&self, storage: &dyn StorageDriver) -> anyhow::Result<Bytes> {
        match self {
            PlainSource::Stored(key) => storage.read_buffer(key).await,
            PlainSource::Local(path) => Ok(Bytes::from(tokio::fs::read(path).await?)),
        }
    }

    async fn av(&self, storage: &dyn StorageDriver) -> Option<crate::media::av_meta::AvMeta> {
        match self {
            PlainSource::Stored(key) => crate::media::av_meta::probe_stored_av_meta(storage, key).await,
            PlainSource::Local(path) => crate::media::av_meta::probe_local_av_meta(path).await,
        }
    }
}

#[cfg(test)]
mod tests {
    //! tests/unit/upload-quota.test.ts
    use super::*;

    #[test]
    fn quota_conventions() {
        assert_eq!(remaining_quota(Some(5), Some(-1)), None);
        assert_eq!(remaining_quota(Some(5), None), None);
        assert_eq!(remaining_quota(Some(5), Some(10)), Some(5));
        assert_eq!(remaining_quota(Some(50), Some(10)), Some(0));
        assert_eq!(remaining_quota(None, Some(10)), Some(10));
        assert!(quota_fits(Some(5), Some(10), 5));
        assert!(!quota_fits(Some(5), Some(10), 6));
        assert!(quota_fits(Some(5), Some(-1), i64::MAX));
        assert!(quota_fits(Some(0), Some(0), 0));
    }

    #[test]
    fn custom_codes() {
        assert_eq!(sanitize_custom_code("My File!"), "my-file");
        assert_eq!(sanitize_custom_code("login"), "");
        assert_eq!(sanitize_custom_code("a/b"), "a-b");
        assert_eq!(sanitize_custom_code("!!!"), "");
        assert_eq!(sanitize_custom_code(&"x".repeat(100)).len(), 60);
    }
}
