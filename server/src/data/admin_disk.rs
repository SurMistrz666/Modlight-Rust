//! Port of `lib/data/admin-disk.ts`: what the disk is actually holding, for
//! the administrator, plus `serverStorageUsage` of `lib/data/admin.ts` (the
//! other half of the same report; it moves to `data/admin.rs` when that
//! module is ported).
//!
//! `server_storage_usage` answers "how full is the machine"; `disk_insights`
//! the question that follows: WHAT is in there. Every number is an aggregate
//! the database computes, never an extrapolation from a sample. Says nothing
//! about who may see it: the caller does the guarding.

use indexmap::IndexMap;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::{Config, js_number};
use crate::db::types::JsDate;
use crate::maintenance::trash::TRASH_RETENTION_MS;
use crate::media::mime::media_kind;
use crate::upload::disk::{FilesystemSpace, storage_filesystem_space};

/// How long a file has to go untouched before it is worth listing: six
/// months, so an install younger than a year still reports something.
pub const COLD_DAYS: i64 = 180;

/// A trashed file with this many days left is about to be purged for real.
pub const TRASH_DUE_SOON_DAYS: i64 = 7;

/// Rows per list.
const LIST_LIMIT: i64 = 6;

/// Accounts shown by name before the tail is folded into one row.
pub const ACCOUNT_LIMIT: usize = 6;

const DAY_MS: i64 = 86_400_000;

// ── Shapes ──────────────────────────────────────────────────────────────────

/// `ServerStorageUsage` (lib/data/admin.ts).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStorageUsage {
    /// Where the bytes live. "s3" means the filesystem numbers are absent.
    pub driver: String,
    pub live_files: i64,
    pub live_bytes: i64,
    pub trashed_files: i64,
    pub trashed_bytes: i64,
    /// The filesystem holding storage, or `None` for an object store or a
    /// failed statfs.
    pub disk: Option<FilesystemSpace>,
    /// The operator's floor, so the panel can say WHY a number is alarming.
    pub min_free_bytes: i64,
}

/// `HeavyFile`
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeavyFile {
    pub id: Uuid,
    pub code: String,
    pub filename: String,
    pub ext: String,
    pub mime_type: String,
    pub size: i64,
    /// `None` for an orphan (owner deleted, files kept).
    pub owner_id: Option<String>,
    pub owner_tag: Option<String>,
    pub owner_name: Option<String>,
    pub created_at: Option<String>,
    pub views: i64,
    /// `None` is NOT proof the file was never read: access has only been
    /// counted since the row was written here.
    pub last_accessed_at: Option<String>,
}

/// `KindUsage`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KindUsage {
    pub kind: &'static str,
    pub files: i64,
    pub bytes: i64,
}

/// `RankedAccount`: an `AccountUsage` before the names are looked up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankedAccount {
    /// `None` for the folded tail and for the orphan row.
    pub id: Option<String>,
    /// The pseudo-row for files with no owner.
    pub orphaned: bool,
    /// The folded tail; `accounts` says how many were folded in.
    pub rest: bool,
    pub accounts: i64,
    pub live_files: i64,
    pub live_bytes: i64,
    pub trashed_files: i64,
    pub trashed_bytes: i64,
}

/// `AccountUsage`: `{ ...ranked, name, email, tag }` (in that key order).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AccountUsage {
    #[serde(flatten)]
    pub ranked: RankedAccount,
    pub name: Option<String>,
    pub email: Option<String>,
    pub tag: Option<String>,
}

/// `TrashOutlook`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashOutlook {
    pub files: i64,
    pub bytes: i64,
    /// ISO of the longest-standing trashed row.
    pub oldest: Option<String>,
    /// Purged within `TRASH_DUE_SOON_DAYS`.
    pub due_soon_files: i64,
    pub due_soon_bytes: i64,
    /// Past the retention window and STILL here: should be zero (the sweeper
    /// runs every six hours); anything else says the sweeper is not running.
    pub overdue_files: i64,
    pub overdue_bytes: i64,
}

/// `ColdTotals`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ColdTotals {
    pub files: i64,
    pub bytes: i64,
    /// The window the two numbers above were measured over.
    pub days: i64,
}

/// `DiskInsights`
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskInsights {
    pub driver: String,
    /// True when the numbers can be compared against a real filesystem.
    pub local: bool,
    pub kinds: Vec<KindUsage>,
    /// Live bytes the kind breakdown adds up to.
    pub kind_bytes: i64,
    pub accounts: Vec<AccountUsage>,
    pub biggest: Vec<HeavyFile>,
    pub cold: Vec<HeavyFile>,
    pub cold_totals: ColdTotals,
    pub trash: TrashOutlook,
}

/// `Number(x)` of a numeric aggregate read as text (sum(bigint) is numeric).
fn num(text: Option<String>) -> i64 {
    text.as_deref().map(js_number).unwrap_or(0.0) as i64
}

// ── Queries ─────────────────────────────────────────────────────────────────

/// Disk usage for the whole install: what the database accounts for (one
/// pass with filtered aggregates, so live and trashed come from one instant)
/// and what the machine has left (the same statfs the upload guard runs).
/// Orphans count: they occupy disk like any other file.
pub async fn server_storage_usage(db: &PgPool, config: &Config) -> anyhow::Result<ServerStorageUsage> {
    let totals = async {
        sqlx::query_as::<_, (i32, String, i32, String)>(
            r#"select (count(*) filter (where "uploads"."deleted_at" is null))::int,
                      coalesce(sum("uploads"."size") filter (where "uploads"."deleted_at" is null), 0)::text,
                      (count(*) filter (where "uploads"."deleted_at" is not null))::int,
                      coalesce(sum("uploads"."size") filter (where "uploads"."deleted_at" is not null), 0)::text
               from "uploads""#,
        )
        .fetch_one(db)
        .await
    };
    let (totals, disk) = tokio::join!(totals, storage_filesystem_space(config));
    let (live_files, live_bytes, trashed_files, trashed_bytes) = totals?;
    Ok(ServerStorageUsage {
        driver: config.storage.driver.clone(),
        live_files: i64::from(live_files),
        live_bytes: num(Some(live_bytes)),
        trashed_files: i64::from(trashed_files),
        trashed_bytes: num(Some(trashed_bytes)),
        disk,
        min_free_bytes: config.uploads.min_free_bytes,
    })
}

/// "Last time we know something touched this file", falling back to the
/// upload date (the truthful lower bound). Written once so the list and its
/// total agree about which files are cold.
const LAST_TOUCH: &str = r#"coalesce("uploads"."last_accessed_at", "uploads"."created_at")"#;

const FILE_COLUMNS: &str = r#""uploads"."id", "uploads"."code", "uploads"."filename", "uploads"."ext", "uploads"."mime_type",
    "uploads"."size", "uploads"."owner_id", "user"."user_code", "user"."name", "uploads"."created_at",
    "uploads"."views", "uploads"."last_accessed_at""#;

type FileRow = (
    Uuid,
    String,
    String,
    Option<String>,
    String,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<JsDate>,
    Option<i32>,
    Option<JsDate>,
);

fn to_heavy_file(row: FileRow) -> HeavyFile {
    let (id, code, filename, ext, mime_type, size, owner_id, owner_tag, owner_name, created_at, views, last) = row;
    HeavyFile {
        id,
        code,
        filename,
        ext: ext.unwrap_or_default(),
        mime_type,
        size: size.unwrap_or(0),
        owner_id,
        owner_tag,
        owner_name,
        created_at: created_at.map(|d| d.to_iso_string()),
        views: i64::from(views.unwrap_or(0)),
        last_accessed_at: last.map(|d| d.to_iso_string()),
    }
}

type ScalarRow = (i32, String, Option<JsDate>, i32, String, i32, String, i32, String);

/// Everything the disk panel needs, read as ONE instant: a read-only
/// REPEATABLE READ transaction on a single pooled connection, so the
/// breakdowns add up to the totals above them and one admin refresh never
/// takes half the pool.
pub async fn disk_insights(db: &PgPool, config: &Config, now: JsDate) -> anyhow::Result<DiskInsights> {
    let at = |ms: i64| JsDate::from_millis(ms).unwrap_or(now);
    let cold_cutoff = at(now.millis() - COLD_DAYS * DAY_MS);
    let purge_cutoff = at(now.millis() - TRASH_RETENTION_MS);
    let due_soon_cutoff = at(now.millis() - TRASH_RETENTION_MS + TRASH_DUE_SOON_DAYS * DAY_MS);

    let mut tx = db.begin().await?;
    sqlx::query("set transaction isolation level repeatable read, read only").execute(&mut *tx).await?;

    // One scan, eight filtered aggregates: the trash outlook and the cold
    // totals are different questions about the same table.
    let scalars: ScalarRow = sqlx::query_as(&format!(
        r#"select (count(*) filter (where "uploads"."deleted_at" is not null))::int,
                  coalesce(sum("uploads"."size") filter (where "uploads"."deleted_at" is not null), 0)::text,
                  min("uploads"."deleted_at"),
                  (count(*) filter (where "uploads"."deleted_at" < $1 and "uploads"."deleted_at" >= $2))::int,
                  coalesce(sum("uploads"."size") filter (where "uploads"."deleted_at" < $1 and "uploads"."deleted_at" >= $2), 0)::text,
                  (count(*) filter (where "uploads"."deleted_at" < $2))::int,
                  coalesce(sum("uploads"."size") filter (where "uploads"."deleted_at" < $2), 0)::text,
                  (count(*) filter (where "uploads"."deleted_at" is null and {LAST_TOUCH} < $3))::int,
                  coalesce(sum("uploads"."size") filter (where "uploads"."deleted_at" is null and {LAST_TOUCH} < $3), 0)::text
           from "uploads""#
    ))
    .bind(due_soon_cutoff)
    .bind(purge_cutoff)
    .bind(cold_cutoff)
    .fetch_one(&mut *tx)
    .await?;

    // Bytes per (stored type, extension), folded into media kinds afterwards:
    // what a file IS follows from `effective_type(mime, ext)`, a rule the app
    // owns, not `split_part(mime_type, '/', 1)`.
    let kind_rows: Vec<(String, Option<String>, i32, String)> = sqlx::query_as(
        r#"select "uploads"."mime_type", "uploads"."ext", count(*)::int, coalesce(sum("uploads"."size"), 0)::text
           from "uploads" where "uploads"."deleted_at" is null
           group by "uploads"."mime_type", "uploads"."ext""#,
    )
    .fetch_all(&mut *tx)
    .await?;

    // Per-owner totals, live and trashed apart; the tail is folded below
    // where the leftovers can still be summed honestly.
    let account_rows: Vec<(Option<String>, i32, String, i32, String)> = sqlx::query_as(
        r#"select "uploads"."owner_id",
                  (count(*) filter (where "uploads"."deleted_at" is null))::int,
                  coalesce(sum("uploads"."size") filter (where "uploads"."deleted_at" is null), 0)::text,
                  (count(*) filter (where "uploads"."deleted_at" is not null))::int,
                  coalesce(sum("uploads"."size") filter (where "uploads"."deleted_at" is not null), 0)::text
           from "uploads" group by "uploads"."owner_id""#,
    )
    .fetch_all(&mut *tx)
    .await?;

    // The heaviest live files.
    let biggest_rows: Vec<FileRow> = sqlx::query_as(&format!(
        r#"select {FILE_COLUMNS} from "uploads" left join "user" on "uploads"."owner_id" = "user"."id"
           where "uploads"."deleted_at" is null order by "uploads"."size" desc limit {LIST_LIMIT}"#
    ))
    .fetch_all(&mut *tx)
    .await?;

    // The longest-untouched live files, by the same definition as the totals.
    let cold_rows: Vec<FileRow> = sqlx::query_as(&format!(
        r#"select {FILE_COLUMNS} from "uploads" left join "user" on "uploads"."owner_id" = "user"."id"
           where ("uploads"."deleted_at" is null and {LAST_TOUCH} < $1) order by {LAST_TOUCH} asc limit {LIST_LIMIT}"#
    ))
    .bind(cold_cutoff)
    .fetch_all(&mut *tx)
    .await?;

    let kinds = fold_kinds(
        kind_rows
            .into_iter()
            .map(|(mime_type, ext, files, bytes)| TypeGroup {
                mime_type,
                ext,
                files: i64::from(files),
                bytes: num(Some(bytes)),
            })
            .collect(),
    );
    let ranked = rank_accounts(
        account_rows
            .into_iter()
            .map(|(owner_id, live_files, live_bytes, trashed_files, trashed_bytes)| OwnerGroup {
                owner_id,
                live_files: i64::from(live_files),
                live_bytes: num(Some(live_bytes)),
                trashed_files: i64::from(trashed_files),
                trashed_bytes: num(Some(trashed_bytes)),
            })
            .collect(),
        ACCOUNT_LIMIT,
    );

    // Only the accounts that survived folding need a name, read inside the
    // same snapshot.
    let named: Vec<String> = ranked.iter().filter_map(|a| a.id.clone()).collect();
    let owners: Vec<(String, String, String, Option<String>)> = if named.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as(r#"select "id", "name", "email", "user_code" from "user" where "user"."id" = any($1)"#)
            .bind(&named)
            .fetch_all(&mut *tx)
            .await?
    };
    tx.commit().await?;

    let by_id: IndexMap<String, (String, String, Option<String>)> =
        owners.into_iter().map(|(id, name, email, tag)| (id, (name, email, tag))).collect();
    let accounts = ranked
        .into_iter()
        .map(|a| {
            let owner = a.id.as_ref().and_then(|id| by_id.get(id));
            AccountUsage {
                name: owner.map(|o| o.0.clone()),
                email: owner.map(|o| o.1.clone()),
                tag: owner.and_then(|o| o.2.clone()),
                ranked: a,
            }
        })
        .collect();

    let (trash_files, trash_bytes, trash_oldest, due_files, due_bytes, over_files, over_bytes, cold_files, cold_bytes) =
        scalars;
    Ok(DiskInsights {
        driver: config.storage.driver.clone(),
        local: config.storage.driver == "local",
        kind_bytes: kinds.iter().map(|k| k.bytes).sum(),
        kinds,
        accounts,
        biggest: biggest_rows.into_iter().map(to_heavy_file).collect(),
        cold: cold_rows.into_iter().map(to_heavy_file).collect(),
        cold_totals: ColdTotals { files: i64::from(cold_files), bytes: num(Some(cold_bytes)), days: COLD_DAYS },
        trash: TrashOutlook {
            files: i64::from(trash_files),
            bytes: num(Some(trash_bytes)),
            oldest: trash_oldest.map(|d| d.to_iso_string()),
            due_soon_files: i64::from(due_files),
            due_soon_bytes: num(Some(due_bytes)),
            overdue_files: i64::from(over_files),
            overdue_bytes: num(Some(over_bytes)),
        },
    })
}

// ── Pure folding (unit-tested) ──────────────────────────────────────────────

/// `TypeGroup`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeGroup {
    pub mime_type: String,
    pub ext: Option<String>,
    pub files: i64,
    pub bytes: i64,
}

/// (stored type, extension) groups collapsed into media kinds, heaviest
/// first (ties by kind name). `media_kind` is the app's one answer to "what
/// is this file", nothing here re-derives it.
pub fn fold_kinds(groups: Vec<TypeGroup>) -> Vec<KindUsage> {
    let mut acc: IndexMap<&'static str, KindUsage> = IndexMap::new();
    for g in groups {
        let kind = media_kind(Some(&g.mime_type), g.ext.as_deref()).as_str();
        let seen = acc.entry(kind).or_insert(KindUsage { kind, files: 0, bytes: 0 });
        seen.files += g.files;
        seen.bytes += g.bytes;
    }
    let mut kinds: Vec<KindUsage> = acc.into_values().collect();
    kinds.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.kind.cmp(b.kind)));
    kinds
}

/// `OwnerGroup`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerGroup {
    pub owner_id: Option<String>,
    pub live_files: i64,
    pub live_bytes: i64,
    pub trashed_files: i64,
    pub trashed_bytes: i64,
}

/// Per-owner groups ordered by what they occupy (trash included), with the
/// tail FOLDED into one row rather than dropped, so the column still adds up
/// to the total above it. The orphan group stays separate wherever it lands.
pub fn rank_accounts(groups: Vec<OwnerGroup>, limit: usize) -> Vec<RankedAccount> {
    let mut rows: Vec<RankedAccount> = groups
        .into_iter()
        .map(|g| RankedAccount {
            orphaned: g.owner_id.is_none(),
            id: g.owner_id,
            rest: false,
            accounts: 1,
            live_files: g.live_files,
            live_bytes: g.live_bytes,
            trashed_files: g.trashed_files,
            trashed_bytes: g.trashed_bytes,
        })
        .collect();
    // Stable, like Array.prototype.sort: ties keep the order the groups came in.
    rows.sort_by_key(|a| std::cmp::Reverse(a.live_bytes + a.trashed_bytes));
    if rows.len() <= limit {
        return rows;
    }
    let tail = rows.split_off(limit);
    let folded = tail.iter().fold(
        RankedAccount {
            id: None,
            orphaned: false,
            rest: true,
            accounts: 0,
            live_files: 0,
            live_bytes: 0,
            trashed_files: 0,
            trashed_bytes: 0,
        },
        |sum, r| RankedAccount {
            accounts: sum.accounts + 1,
            live_files: sum.live_files + r.live_files,
            live_bytes: sum.live_bytes + r.live_bytes,
            trashed_files: sum.trashed_files + r.trashed_files,
            trashed_bytes: sum.trashed_bytes + r.trashed_bytes,
            ..sum
        },
    );
    rows.push(folded);
    rows
}

#[cfg(test)]
mod tests {
    //! tests/unit/admin-disk.test.ts
    use super::*;

    fn g(mime: &str, ext: Option<&str>, files: i64, bytes: i64) -> TypeGroup {
        TypeGroup { mime_type: mime.into(), ext: ext.map(str::to_string), files, bytes }
    }

    fn o(owner: Option<&str>, live_bytes: i64, trashed_bytes: i64, files: i64) -> OwnerGroup {
        OwnerGroup {
            owner_id: owner.map(str::to_string),
            live_files: files,
            live_bytes,
            trashed_files: i64::from(trashed_bytes > 0),
            trashed_bytes,
        }
    }

    fn total(rows: &[RankedAccount]) -> i64 {
        rows.iter().map(|r| r.live_bytes + r.trashed_bytes).sum()
    }

    fn k(kind: &'static str, files: i64, bytes: i64) -> KindUsage {
        KindUsage { kind, files, bytes }
    }

    #[test]
    fn sums_the_type_extension_groups() {
        let kinds = fold_kinds(vec![
            g("image/png", Some("png"), 3, 300),
            g("image/jpeg", Some("jpg"), 2, 200),
            g("video/mp4", Some("mp4"), 1, 5000),
        ]);
        assert_eq!(kinds, vec![k("video", 1, 5000), k("image", 5, 500)]);
    }

    #[test]
    fn believes_the_extension_when_the_stored_type_says_nothing() {
        let kinds = fold_kinds(vec![g("application/octet-stream", Some("avif"), 4, 4000)]);
        assert_eq!(kinds, vec![k("image", 4, 4000)]);
    }

    #[test]
    fn does_not_let_an_extension_override_a_type_that_says_something() {
        let kinds = fold_kinds(vec![g("video/mp4", Some("png"), 1, 10)]);
        assert_eq!(kinds[0].kind, "video");
    }

    #[test]
    fn orders_kinds_by_bytes_heaviest_first() {
        let kinds = fold_kinds(vec![
            g("text/plain", Some("txt"), 900, 10),
            g("application/pdf", Some("pdf"), 1, 900),
            g("application/zip", Some("zip"), 2, 400),
        ]);
        assert_eq!(kinds.iter().map(|k| k.kind).collect::<Vec<_>>(), vec!["pdf", "archive", "text"]);
        assert!(fold_kinds(Vec::new()).is_empty());
    }

    #[test]
    fn orders_accounts_by_everything_they_occupy() {
        let ranked = rank_accounts(vec![o(Some("a"), 100, 0, 1), o(Some("b"), 10, 500, 1), o(Some("c"), 200, 0, 1)], 6);
        assert_eq!(ranked.iter().map(|r| r.id.as_deref()).collect::<Vec<_>>(), vec![Some("b"), Some("c"), Some("a")]);
    }

    #[test]
    fn folds_the_tail_instead_of_dropping_it() {
        let groups = vec![
            o(Some("a"), 500, 0, 1),
            o(Some("b"), 400, 0, 1),
            o(Some("c"), 300, 0, 1),
            o(Some("d"), 200, 0, 1),
            o(Some("e"), 100, 0, 1),
        ];
        let all = rank_accounts(groups.clone(), 99);
        let ranked = rank_accounts(groups, 2);
        assert_eq!(ranked.len(), 3);
        assert_eq!(total(&ranked), total(&all));
        let rest = ranked.last().unwrap();
        assert!(rest.rest);
        assert_eq!(rest.accounts, 3);
        assert_eq!(rest.live_bytes, 600);
        assert_eq!(rest.live_files, 3);
    }

    #[test]
    fn keeps_the_ownerless_group_separate() {
        let ranked = rank_accounts(vec![o(Some("a"), 10, 0, 1), o(None, 900, 0, 1)], 1);
        assert_eq!(
            (ranked[0].id.as_deref(), ranked[0].orphaned, ranked[0].rest, ranked[0].live_bytes),
            (None, true, false, 900)
        );
        let last = ranked.last().unwrap();
        assert_eq!((last.rest, last.accounts, last.live_bytes), (true, 1, 10));
    }

    #[test]
    fn adds_no_fold_row_when_everything_fits() {
        let ranked = rank_accounts(vec![o(Some("a"), 10, 0, 1), o(Some("b"), 20, 0, 1)], 6);
        assert_eq!(ranked.len(), 2);
        assert!(ranked.iter().all(|r| !r.rest && r.accounts == 1));
        assert!(rank_accounts(Vec::new(), ACCOUNT_LIMIT).is_empty());
    }

    #[test]
    fn account_rows_keep_the_upstream_key_order() {
        let row = AccountUsage {
            ranked: rank_accounts(vec![o(Some("a"), 1, 0, 1)], 6).remove(0),
            name: Some("N".into()),
            email: None,
            tag: None,
        };
        let keys: Vec<String> = serde_json::to_value(&row).unwrap().as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            [
                "id",
                "orphaned",
                "rest",
                "accounts",
                "liveFiles",
                "liveBytes",
                "trashedFiles",
                "trashedBytes",
                "name",
                "email",
                "tag"
            ]
        );
    }
}
