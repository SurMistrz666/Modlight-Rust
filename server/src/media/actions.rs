//! Port of `lib/media/actions.ts`: permanent and soft deletes of media rows,
//! with their bytes, derived files and the owners' quota.

use sqlx::PgPool;
use uuid::Uuid;

use crate::db::types::JsDate;
use crate::media::derived_cache::purge_derived;
use crate::storage::StorageDriver;

/// The columns a permanent delete actually needs (`DeletableMedia`).
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct DeletableMedia {
    pub id: Uuid,
    pub owner_id: Option<String>,
    pub size: i64,
    pub storage_key: String,
    pub thumb_key: Option<String>,
}

impl From<&crate::db::models::Upload> for DeletableMedia {
    /// A full row carries everything a delete needs.
    fn from(u: &crate::db::models::Upload) -> Self {
        DeletableMedia {
            id: u.id,
            owner_id: u.owner_id.clone(),
            size: u.size,
            storage_key: u.storage_key.clone(),
            thumb_key: u.thumb_key.clone(),
        }
    }
}

/// `BulkDeleteResult`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct BulkDeleteResult {
    /// Rows removed from the database.
    pub deleted: i64,
    /// Bytes reclaimed (sum of the removed rows' sizes).
    pub bytes: i64,
    /// Rows left alone because storage refused to give up their object.
    pub failed: i64,
}

/// How many storage objects we delete at once.
const STORAGE_CONCURRENCY: usize = 16;

/// Rows deleted per DELETE statement.
const ROW_BATCH: usize = 500;

/// Local ENOENT / S3 NoSuchKey: the object is already gone, which is success.
pub fn is_missing_object_error(err: &anyhow::Error) -> bool {
    crate::storage::is_not_found(err)
}

/// The refund statement: `used_quota = GREATEST(0, used_quota - amount)`
/// (drizzle's `$onUpdate` stamps `updated_at` on every user update).
async fn refund_quota(db: &PgPool, owner_id: &str, amount: i64) -> sqlx::Result<()> {
    sqlx::query(
        r#"update "user" set "used_quota" = GREATEST(0, "user"."used_quota" - $1), "updated_at" = $2 where "user"."id" = $3"#,
    )
    .bind(amount)
    .bind(JsDate::now())
    .bind(owner_id)
    .execute(db)
    .await?;
    Ok(())
}

/// Permanently delete a batch of media: bytes first, then rows, then quota.
///
/// Deleting the object before the row means a crash in between leaves a row
/// pointing at nothing (visible, fixable) rather than bytes nobody remembers.
/// When storage refuses, the row is KEPT and counted in `failed`. Quota
/// refunds are grouped per owner and skip orphans.
pub async fn delete_media_bulk(
    db: &PgPool,
    storage: &dyn StorageDriver,
    rows: &[DeletableMedia],
) -> anyhow::Result<BulkDeleteResult> {
    if rows.is_empty() {
        return Ok(BulkDeleteResult::default());
    }
    let mut removable: Vec<Uuid> = Vec::new();
    // owner id -> bytes to refund, in first-seen order (a JS Map).
    let mut refunds: indexmap::IndexMap<String, i64> = indexmap::IndexMap::new();
    let mut bytes = 0i64;
    let mut failed = 0i64;
    for slice in rows.chunks(STORAGE_CONCURRENCY) {
        let outcomes = futures_util::future::join_all(slice.iter().map(|row| async move {
            if let Err(err) = storage.delete(&row.storage_key).await
                && !is_missing_object_error(&err)
            {
                return false;
            }
            if let Some(thumb) = row.thumb_key.as_deref() {
                // A stale thumbnail is cosmetic - never block reclaiming the row.
                let _ = storage.delete(thumb).await;
            }
            // Cached conversions live under derived/{id}/ and belong to this file.
            purge_derived(storage, &row.id.to_string()).await;
            true
        }))
        .await;
        for (ok, row) in outcomes.into_iter().zip(slice) {
            if !ok {
                failed += 1;
                continue;
            }
            removable.push(row.id);
            bytes += row.size;
            if let Some(owner) = &row.owner_id {
                *refunds.entry(owner.clone()).or_insert(0) += row.size;
            }
        }
    }
    for batch in removable.chunks(ROW_BATCH) {
        sqlx::query(r#"delete from "uploads" where "uploads"."id" = any($1)"#).bind(batch).execute(db).await?;
    }
    for (owner, amount) in &refunds {
        refund_quota(db, owner, *amount).await?;
    }
    Ok(BulkDeleteResult { deleted: removable.len() as i64, bytes, failed })
}

/// Permanently delete a single media: storage object + row, and refund quota.
pub async fn delete_media_hard(db: &PgPool, storage: &dyn StorageDriver, media: &DeletableMedia) -> anyhow::Result<()> {
    // The file may already be gone.
    let _ = storage.delete(&media.storage_key).await;
    if let Some(thumb) = media.thumb_key.as_deref() {
        let _ = storage.delete(thumb).await;
    }
    purge_derived(storage, &media.id.to_string()).await;
    sqlx::query(r#"delete from "uploads" where "uploads"."id" = $1"#).bind(media.id).execute(db).await?;
    // No owner means the file outlived its account: no quota counter to credit.
    if let Some(owner) = &media.owner_id {
        refund_quota(db, owner, media.size).await?;
    }
    Ok(())
}

/// Soft-delete (move to trash). `WHERE deleted_at IS NULL` so trashing
/// something already in the trash does not re-stamp it (the stamp is what the
/// thirty-day sweeper counts from). The cached conversions go now.
pub async fn soft_delete_media(db: &PgPool, storage: &dyn StorageDriver, id: Uuid) -> anyhow::Result<()> {
    sqlx::query(
        r#"update "uploads" set "deleted_at" = $1 where ("uploads"."id" = $2 and "uploads"."deleted_at" is null)"#,
    )
    .bind(JsDate::now())
    .bind(id)
    .execute(db)
    .await?;
    purge_derived(storage, &id.to_string()).await;
    Ok(())
}

pub async fn restore_media(db: &PgPool, id: Uuid) -> anyhow::Result<()> {
    sqlx::query(r#"update "uploads" set "deleted_at" = null where "uploads"."id" = $1"#).bind(id).execute(db).await?;
    Ok(())
}
