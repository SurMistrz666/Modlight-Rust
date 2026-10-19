//! Port of `lib/maintenance/trash.ts`: trash retention.
//!
//! Deleting a file in Modlight is always a soft-delete: the row gets a
//! `deleted_at` stamp and the bytes stay where they were. This module closes
//! the loop: it permanently removes items that have been sitting in the trash
//! for longer than `TRASH_RETENTION_DAYS`, deletes their bytes from storage,
//! and refunds the owner's used quota. Scheduled by the job runner, never
//! reachable from a request.

use std::time::Duration;

use crate::db::types::JsDate;
use crate::state::AppState;

/// How long a trashed item is kept before it is destroyed for good. Surfaced
/// in the UI verbatim, so this constant and that copy must stay in step.
pub const TRASH_RETENTION_DAYS: i64 = 30;
pub const TRASH_RETENTION_MS: i64 = TRASH_RETENTION_DAYS * 24 * 60 * 60 * 1000;

/// How often the server runs the sweep (`TRASH_SWEEP_INTERVAL_MS`).
pub const TRASH_SWEEP_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// The first sweep waits this long after boot, so a boot loop can never turn
/// into a delete loop (server.ts `setTimeout(sweepTrash, 60_000)`).
pub const TRASH_FIRST_SWEEP_DELAY: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrashSweepResult {
    /// Rows permanently removed.
    pub deleted: u64,
    /// Bytes reclaimed (sum of the removed rows' sizes).
    pub bytes: u64,
    /// Rows left for the next sweep because storage refused to delete them.
    pub skipped: u64,
}

/// Days remaining before an item is purged; 0 once it is due.
pub fn trash_days_left(deleted_at: Option<JsDate>, now_ms: i64) -> Option<i64> {
    let stamp = deleted_at?.millis();
    let day = 24 * 60 * 60 * 1000;
    let left = stamp + TRASH_RETENTION_MS - now_ms;
    // Math.ceil for the division, then Math.max(0, ...)
    Some((left as f64 / day as f64).ceil().max(0.0) as i64)
}

/// Rows handled per round trip.
const BATCH_SIZE: i64 = 200;
/// Hard stop so one invocation can never run away with the database.
const MAX_BATCHES: usize = 250;

/// `purgeExpiredTrash(now)`: permanently delete every trashed upload older
/// than the retention window.
///
/// The storage object goes first, then the row: a crash in between loses a
/// byte-level orphan rather than leaving a row pointing at nothing. When
/// storage refuses to delete an object, the row is *kept* and skipped, so the
/// next sweep tries again.
pub async fn purge_expired_trash(state: &AppState, now: JsDate) -> anyhow::Result<TrashSweepResult> {
    let storage = state.storage.get()?;
    purge_expired_trash_with(&state.db, storage.as_ref(), now).await
}

pub async fn purge_expired_trash_with(
    db: &sqlx::PgPool,
    storage: &dyn crate::storage::StorageDriver,
    now: JsDate,
) -> anyhow::Result<TrashSweepResult> {
    use crate::media::actions::{DeletableMedia, is_missing_object_error};
    let cutoff = JsDate::from_millis(now.millis() - TRASH_RETENTION_MS).unwrap_or(now);
    let mut skipped: indexmap::IndexSet<uuid::Uuid> = indexmap::IndexSet::new();
    let mut deleted = 0u64;
    let mut bytes = 0u64;
    for _ in 0..MAX_BATCHES {
        let skip: Vec<uuid::Uuid> = skipped.iter().copied().collect();
        let batch: Vec<DeletableMedia> = sqlx::query_as(
            r#"select "id", "owner_id", "size", "storage_key", "thumb_key" from "uploads"
               where ("uploads"."deleted_at" is not null and "uploads"."deleted_at" < $1
                      and (cardinality($2::uuid[]) = 0 or "uploads"."id" <> all($2::uuid[])))
               order by "uploads"."deleted_at" asc limit $3"#,
        )
        .bind(cutoff)
        .bind(&skip)
        .bind(BATCH_SIZE)
        .fetch_all(db)
        .await?;
        if batch.is_empty() {
            break;
        }
        let mut removable: Vec<uuid::Uuid> = Vec::new();
        let mut refunds: indexmap::IndexMap<String, i64> = indexmap::IndexMap::new();
        for row in &batch {
            if let Err(err) = storage.delete(&row.storage_key).await
                && !is_missing_object_error(&err)
            {
                // Storage is unhappy: leave the row for the next run.
                skipped.insert(row.id);
                continue;
            }
            if let Some(thumb) = row.thumb_key.as_deref() {
                // A stale thumbnail is cosmetic.
                let _ = storage.delete(thumb).await;
            }
            crate::media::derived_cache::purge_derived(storage, &row.id.to_string()).await;
            removable.push(row.id);
            // `owner_id` is null for an orphan: no quota left to refund.
            if let Some(owner) = &row.owner_id {
                *refunds.entry(owner.clone()).or_insert(0) += row.size;
            }
            bytes += row.size.max(0) as u64;
        }
        if !removable.is_empty() {
            sqlx::query(r#"delete from "uploads" where "uploads"."id" = any($1)"#).bind(&removable).execute(db).await?;
            for (owner, amount) in &refunds {
                sqlx::query(
                    r#"update "user" set "used_quota" = GREATEST(0, "user"."used_quota" - $1), "updated_at" = $2 where "user"."id" = $3"#,
                )
                .bind(*amount)
                .bind(JsDate::now())
                .bind(owner)
                .execute(db)
                .await?;
            }
            deleted += removable.len() as u64;
        }
        // Everything in this batch was skipped: keep going only while progress happens.
        if removable.is_empty() && (batch.len() as i64) < BATCH_SIZE {
            break;
        }
    }
    Ok(TrashSweepResult { deleted, bytes, skipped: skipped.len() as u64 })
}

#[cfg(test)]
mod tests {
    use super::*;

    // tests/unit/trash-retention.test.ts
    const DAY: i64 = 24 * 60 * 60 * 1000;
    const NOW: i64 = 1_760_000_000_000;

    fn left(ago_days: f64) -> Option<i64> {
        trash_days_left(JsDate::from_millis(NOW - (ago_days * DAY as f64) as i64), NOW)
    }

    #[test]
    fn retention_countdown() {
        assert_eq!(TRASH_RETENTION_DAYS, 30);
        assert_eq!(TRASH_RETENTION_MS, 30 * DAY);
        assert_eq!(left(0.0), Some(30));
        assert_eq!(left(1.0), Some(29));
        assert_eq!(left(29.0), Some(1));
        assert_eq!(left(29.5), Some(1));
        assert_eq!(left(30.0), Some(0));
        assert_eq!(left(400.0), Some(0));
        assert_eq!(trash_days_left(None, NOW), None);
    }
}
