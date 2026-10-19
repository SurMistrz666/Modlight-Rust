//! Port of `lib/data/admin-disk.ts`: what the disk is actually holding, for
//! the administrator.
//!
//! `server_storage_usage()` in [`crate::data::admin`] answers "how full is the
//! machine". This answers the question that follows it: WHAT is in there, and
//! which end of it is worth pulling on. Every number below is an aggregate
//! the database computes, never an extrapolation from a sample.
//!
//! Cost: five statements over `uploads`, plus one primary-key lookup in
//! `user`, run one after another on a SINGLE pooled connection. Each is a
//! sequential scan of a table of a few hundred heap pages; the comment above
//! each query names the index it would want at a hundred times the size.
//! None of those indexes is worth creating today: every one of them is write
//! amplification on the hottest table in the app, bought to save a few
//! milliseconds on a page only an administrator opens.

use std::collections::HashMap;

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::Config;
use crate::db::types::JsDate;
use crate::maintenance::trash::TRASH_RETENTION_MS;
use crate::media::mime::{MediaKind, media_kind};

/// How long a file has to go untouched before it is worth listing.
///
/// Six months rather than a year: the point is to surface something an
/// operator can act on this quarter, and a year-long window on an install
/// that is itself younger than a year would report nothing at all.
pub const COLD_DAYS: i64 = 180;

/// A trashed file with this many days left is about to be purged for real.
pub const TRASH_DUE_SOON_DAYS: i64 = 7;

/// Rows per list. Long enough to see a pattern, short enough to read at a glance.
const LIST_LIMIT: i64 = 6;

/// Accounts shown by name before the tail is folded into one "everyone else" row.
pub const ACCOUNT_LIMIT: usize = 6;

const DAY_MS: i64 = 86_400_000;

// ---------------------------------------------------------------------------
// Shapes (field order = upstream's object literals)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeavyFile {
    pub id: Uuid,
    pub code: String,
    pub filename: String,
    pub ext: String,
    pub mime_type: String,
    pub size: i64,
    /// Owner's id, or null for an orphan (owner deleted, files kept).
    pub owner_id: Option<String>,
    /// Owner's tag, i.e. the first segment of their public links.
    pub owner_tag: Option<String>,
    pub owner_name: Option<String>,
    /// ISO strings: a server component may only hand plain data to the client.
    pub created_at: Option<String>,
    /// Recorded fetches. Zero next to a large size is the clearest case to act on.
    pub views: i64,
    /// Last recorded fetch, or null when nothing has fetched it since it arrived.
    ///
    /// Null is NOT proof the file was never read: access has only been counted
    /// since the row was written here, and rows that predate the counter, or
    /// were imported, carry no history at all.
    pub last_accessed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KindUsage {
    pub kind: MediaKind,
    pub files: i64,
    pub bytes: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUsage {
    /// Null for the folded "everyone else" row and for the orphan row.
    pub id: Option<String>,
    /// True for the pseudo-row that stands for files with no owner.
    pub orphaned: bool,
    /// True for the folded tail. `accounts` says how many were folded in.
    pub rest: bool,
    pub accounts: i64,
    pub live_files: i64,
    pub live_bytes: i64,
    pub trashed_files: i64,
    pub trashed_bytes: i64,
    pub name: Option<String>,
    pub email: Option<String>,
    pub tag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashOutlook {
    pub files: i64,
    pub bytes: i64,
    /// ISO of the longest-standing trashed row, or null for an empty trash.
    pub oldest: Option<String>,
    /// Purged within TRASH_DUE_SOON_DAYS: the bytes coming back on their own.
    pub due_soon_files: i64,
    pub due_soon_bytes: i64,
    /// Past the retention window and STILL here. Should be zero: the sweeper
    /// runs every six hours, so anything else is the panel reporting that the
    /// sweeper is not running, which is the only way an operator would find out.
    pub overdue_files: i64,
    pub overdue_bytes: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColdTotals {
    pub files: i64,
    pub bytes: i64,
    /// The window the two numbers above were measured over.
    pub days: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskInsights {
    /// Storage driver, so the UI can say what it cannot know on an object store.
    pub driver: String,
    /// True when the numbers below can be compared against a real filesystem.
    pub local: bool,
    pub kinds: Vec<KindUsage>,
    /// Live bytes the kind breakdown adds up to, so shares are honest.
    pub kind_bytes: i64,
    pub accounts: Vec<AccountUsage>,
    pub biggest: Vec<HeavyFile>,
    pub cold: Vec<HeavyFile>,
    pub cold_totals: ColdTotals,
    pub trash: TrashOutlook,
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

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
    let (id, code, filename, ext, mime_type, size, owner_id, owner_tag, owner_name, created_at, views, last_accessed) =
        row;
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
        last_accessed_at: last_accessed.map(|d| d.to_iso_string()),
    }
}

/// The columns of a [`HeavyFile`], `uploads` LEFT JOIN `user`.
const FILE_COLUMNS: &str = r#"u."id", u."code", u."filename", u."ext", u."mime_type", u."size", u."owner_id",
       o."user_code", o."name", u."created_at", u."views", u."last_accessed_at"
  FROM "uploads" u LEFT JOIN "user" o ON u."owner_id" = o."id""#;

/// `(mime_type, ext, files, bytes)`
type KindRow = (String, Option<String>, Option<i32>, Option<i64>);

/// `(owner_id, liveFiles, liveBytes, trashedFiles, trashedBytes)`
type AccountRow = (Option<String>, Option<i32>, Option<i64>, Option<i32>, Option<i64>);

type Scalars = (
    Option<i32>,
    Option<i64>,
    Option<JsDate>,
    Option<i32>,
    Option<i64>,
    Option<i32>,
    Option<i64>,
    Option<i32>,
    Option<i64>,
);

/// Everything the disk panel needs, read as one instant (upstream's default
/// `now` is `new Date()`).
///
/// A read-only REPEATABLE READ transaction rather than parallel statements,
/// for two reasons that point the same way:
///
///  - **The panel has to reconcile.** Its entire job is that the breakdowns
///    add up to the total above them. Statements issued in parallel are
///    different snapshots, so an upload landing mid-render puts a file in the
///    type breakdown and not in the account one.
///  - **The pool is shared.** One admin refresh must not take half of the
///    connections to answer a question nobody is waiting on.
///
/// `read only` is belt and braces: this is a read model, and the transaction
/// says so to Postgres rather than only to the reader.
///
/// Says nothing about who may see it: the caller does the guarding.
pub async fn disk_insights(db: &PgPool, config: &Config, now: JsDate) -> sqlx::Result<DiskInsights> {
    let now_ms = now.millis();
    let cold_cutoff = js_date(now_ms - COLD_DAYS * DAY_MS);
    let purge_cutoff = js_date(now_ms - TRASH_RETENTION_MS);
    let due_soon_cutoff = js_date(now_ms - TRASH_RETENTION_MS + TRASH_DUE_SOON_DAYS * DAY_MS);

    // drizzle: `begin isolation level repeatable read read only`
    let mut tx = db.begin_with("begin isolation level repeatable read read only").await?;

    // One scan, eight filtered aggregates: the trash outlook and the cold
    // totals are different questions about the same table, and asking them
    // separately would read it twice for no gain.
    //
    // At a hundred times this size: a partial index on (deleted_at) where
    // deleted_at is not null, plus an expression index on
    // coalesce(last_accessed_at, created_at).
    let scalars: Scalars = sqlx::query_as(
        r#"SELECT (count(*) filter (where "deleted_at" is not null))::int,
                  coalesce(sum("size") filter (where "deleted_at" is not null), 0)::bigint,
                  min("deleted_at"),
                  (count(*) filter (where "deleted_at" < $1 and "deleted_at" >= $2))::int,
                  coalesce(sum("size") filter (where "deleted_at" < $1 and "deleted_at" >= $2), 0)::bigint,
                  (count(*) filter (where "deleted_at" < $2))::int,
                  coalesce(sum("size") filter (where "deleted_at" < $2), 0)::bigint,
                  (count(*) filter (where "deleted_at" is null
                                    and coalesce("last_accessed_at", "created_at") < $3))::int,
                  coalesce(sum("size") filter (where "deleted_at" is null
                                    and coalesce("last_accessed_at", "created_at") < $3), 0)::bigint
           FROM "uploads""#,
    )
    .bind(due_soon_cutoff)
    .bind(purge_cutoff)
    .bind(cold_cutoff)
    .fetch_one(&mut *tx)
    .await?;

    // Bytes per stored type, folded into media kinds afterwards rather than in
    // SQL: what a file IS follows from `effectiveType(mime, ext)`, because a
    // client that sent application/octet-stream told us nothing and the name
    // on disk is then the better evidence. Grouping by the (type, extension)
    // pair keeps that rule available while still costing one scan.
    let kind_rows: Vec<KindRow> = sqlx::query_as(
        r#"SELECT "mime_type", "ext", count(*)::int, coalesce(sum("size"), 0)::bigint
           FROM "uploads" WHERE "deleted_at" IS NULL GROUP BY "mime_type", "ext""#,
    )
    .fetch_all(&mut *tx)
    .await?;

    // Per-owner totals, live and trashed apart. Unlimited on purpose: the
    // group count is the number of accounts that hold files (plus one null
    // group for orphans), so the tail is folded here where the leftovers can
    // still be summed honestly.
    let account_rows: Vec<AccountRow> = sqlx::query_as(
        r#"SELECT "owner_id",
                  (count(*) filter (where "deleted_at" is null))::int,
                  coalesce(sum("size") filter (where "deleted_at" is null), 0)::bigint,
                  (count(*) filter (where "deleted_at" is not null))::int,
                  coalesce(sum("size") filter (where "deleted_at" is not null), 0)::bigint
           FROM "uploads" GROUP BY "owner_id""#,
    )
    .fetch_all(&mut *tx)
    .await?;

    // The heaviest live files. At a hundred times this size: index on
    // (size desc) where deleted_at is null.
    let biggest_rows: Vec<FileRow> = sqlx::query_as(&format!(
        r#"SELECT {FILE_COLUMNS} WHERE u."deleted_at" IS NULL ORDER BY u."size" DESC LIMIT {LIST_LIMIT}"#
    ))
    .fetch_all(&mut *tx)
    .await?;

    // The longest-untouched live files, by the same definition as the totals.
    let cold_rows: Vec<FileRow> = sqlx::query_as(&format!(
        r#"SELECT {FILE_COLUMNS}
           WHERE u."deleted_at" IS NULL AND coalesce(u."last_accessed_at", u."created_at") < $1
           ORDER BY coalesce(u."last_accessed_at", u."created_at") ASC LIMIT {LIST_LIMIT}"#
    ))
    .bind(cold_cutoff)
    .fetch_all(&mut *tx)
    .await?;

    let kinds = fold_kinds(
        &kind_rows
            .into_iter()
            .map(|(mime_type, ext, files, bytes)| TypeGroup {
                mime_type,
                ext,
                files: i64::from(files.unwrap_or(0)),
                bytes: bytes.unwrap_or(0),
            })
            .collect::<Vec<_>>(),
    );

    let ranked = rank_accounts(
        &account_rows
            .into_iter()
            .map(|(owner_id, live_files, live_bytes, trashed_files, trashed_bytes)| OwnerGroup {
                owner_id,
                live_files: i64::from(live_files.unwrap_or(0)),
                live_bytes: live_bytes.unwrap_or(0),
                trashed_files: i64::from(trashed_files.unwrap_or(0)),
                trashed_bytes: trashed_bytes.unwrap_or(0),
            })
            .collect::<Vec<_>>(),
        ACCOUNT_LIMIT,
    );

    // Only the accounts that survived folding need a name, so the user table
    // is read for at most ACCOUNT_LIMIT ids rather than for every group. It is
    // read inside the same snapshot: an account deleted between the grouping
    // and the lookup would otherwise render as an unnamed row.
    let named_ids: Vec<String> = ranked.iter().filter_map(|a| a.id.clone()).collect();
    let owners: Vec<(String, String, String, Option<String>)> = if named_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as(r#"SELECT "id", "name", "email", "user_code" FROM "user" WHERE "id" = ANY($1)"#)
            .bind(&named_ids)
            .fetch_all(&mut *tx)
            .await?
    };
    tx.commit().await?;
    let by_id: HashMap<String, (String, String, Option<String>)> =
        owners.into_iter().map(|(id, name, email, tag)| (id, (name, email, tag))).collect();

    let (
        trash_files,
        trash_bytes,
        trash_oldest,
        due_soon_files,
        due_soon_bytes,
        overdue_files,
        overdue_bytes,
        cold_files,
        cold_bytes,
    ) = scalars;

    Ok(DiskInsights {
        driver: config.storage.driver.clone(),
        local: config.storage.driver == "local",
        kind_bytes: kinds.iter().map(|k| k.bytes).sum(),
        kinds,
        accounts: ranked
            .into_iter()
            .map(|a| {
                // `a.id ? byId.get(a.id) : undefined`
                let owner = a.id.as_deref().filter(|id| !id.is_empty()).and_then(|id| by_id.get(id));
                AccountUsage {
                    name: owner.map(|o| o.0.clone()),
                    email: owner.map(|o| o.1.clone()),
                    tag: owner.and_then(|o| o.2.clone()),
                    id: a.id,
                    orphaned: a.orphaned,
                    rest: a.rest,
                    accounts: a.accounts,
                    live_files: a.live_files,
                    live_bytes: a.live_bytes,
                    trashed_files: a.trashed_files,
                    trashed_bytes: a.trashed_bytes,
                }
            })
            .collect(),
        biggest: biggest_rows.into_iter().map(to_heavy_file).collect(),
        cold: cold_rows.into_iter().map(to_heavy_file).collect(),
        cold_totals: ColdTotals {
            files: i64::from(cold_files.unwrap_or(0)),
            bytes: cold_bytes.unwrap_or(0),
            days: COLD_DAYS,
        },
        trash: TrashOutlook {
            files: i64::from(trash_files.unwrap_or(0)),
            bytes: trash_bytes.unwrap_or(0),
            oldest: trash_oldest.map(|d| d.to_iso_string()),
            due_soon_files: i64::from(due_soon_files.unwrap_or(0)),
            due_soon_bytes: due_soon_bytes.unwrap_or(0),
            overdue_files: i64::from(overdue_files.unwrap_or(0)),
            overdue_bytes: overdue_bytes.unwrap_or(0),
        },
    })
}

fn js_date(ms: i64) -> JsDate {
    JsDate::from_millis(ms).unwrap_or_else(JsDate::now)
}

// ---------------------------------------------------------------------------
// Pure folding (unit-tested)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeGroup {
    pub mime_type: String,
    pub ext: Option<String>,
    pub files: i64,
    pub bytes: i64,
}

/// (stored type, extension) groups collapsed into the media kinds the rest of
/// the app reasons in, heaviest first.
///
/// `mediaKind` is the app's one answer to "what is this file",
/// `effectiveType` included; nothing here re-derives it.
pub fn fold_kinds(groups: &[TypeGroup]) -> Vec<KindUsage> {
    let mut acc: Vec<KindUsage> = Vec::new();
    for g in groups {
        let kind = media_kind(Some(&g.mime_type), g.ext.as_deref());
        match acc.iter_mut().find(|k| k.kind == kind) {
            Some(seen) => {
                seen.files += g.files;
                seen.bytes += g.bytes;
            }
            None => acc.push(KindUsage { kind, files: g.files, bytes: g.bytes }),
        }
    }
    // `b.bytes - a.bytes || a.kind.localeCompare(b.kind)`: the kind names are
    // lower-case ASCII, where localeCompare is plain alphabetical order.
    acc.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.kind.as_str().cmp(b.kind.as_str())));
    acc
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerGroup {
    pub owner_id: Option<String>,
    pub live_files: i64,
    pub live_bytes: i64,
    pub trashed_files: i64,
    pub trashed_bytes: i64,
}

/// `Omit<AccountUsage, "name" | "email" | "tag">`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedAccount {
    pub id: Option<String>,
    pub orphaned: bool,
    pub rest: bool,
    pub accounts: i64,
    pub live_files: i64,
    pub live_bytes: i64,
    pub trashed_files: i64,
    pub trashed_bytes: i64,
}

/// Per-owner groups ordered by what they occupy, with the tail folded into
/// one row rather than dropped.
///
/// Dropping it would make the column stop adding up to the total on the card
/// above, and a breakdown that does not reconcile is worse than no breakdown.
/// The orphan group (no owner) is kept separate wherever it lands: those bytes
/// belong to nobody, which is the whole reason they need looking at.
/// Upstream's default `limit` is [`ACCOUNT_LIMIT`].
pub fn rank_accounts(groups: &[OwnerGroup], limit: usize) -> Vec<RankedAccount> {
    let mut rows: Vec<RankedAccount> = groups
        .iter()
        .map(|g| RankedAccount {
            id: g.owner_id.clone(),
            orphaned: g.owner_id.is_none(),
            rest: false,
            accounts: 1,
            live_files: g.live_files,
            live_bytes: g.live_bytes,
            trashed_files: g.trashed_files,
            trashed_bytes: g.trashed_bytes,
        })
        .collect();
    // Stable, like V8's Array.prototype.sort: equal totals keep the group order.
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
            id: None,
            orphaned: false,
            rest: true,
            accounts: sum.accounts + 1,
            live_files: sum.live_files + r.live_files,
            live_bytes: sum.live_bytes + r.live_bytes,
            trashed_files: sum.trashed_files + r.trashed_files,
            trashed_bytes: sum.trashed_bytes + r.trashed_bytes,
        },
    );
    rows.push(folded);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(mime_type: &str, ext: Option<&str>, files: i64, bytes: i64) -> TypeGroup {
        TypeGroup { mime_type: mime_type.into(), ext: ext.map(Into::into), files, bytes }
    }

    fn ku(kind: MediaKind, files: i64, bytes: i64) -> KindUsage {
        KindUsage { kind, files, bytes }
    }

    // tests/unit/admin-disk.test.ts: "foldKinds"
    #[test]
    fn fold_kinds_sums_the_type_extension_groups() {
        let kinds = fold_kinds(&[
            g("image/png", Some("png"), 3, 300),
            g("image/jpeg", Some("jpg"), 2, 200),
            g("video/mp4", Some("mp4"), 1, 5000),
        ]);
        assert_eq!(kinds, vec![ku(MediaKind::Video, 1, 5000), ku(MediaKind::Image, 5, 500)]);
    }

    #[test]
    fn fold_kinds_believes_the_extension_when_the_stored_type_says_nothing() {
        // ShareX sends application/octet-stream for anything Windows has no
        // registry entry for, and .avif is exactly that.
        let kinds = fold_kinds(&[g("application/octet-stream", Some("avif"), 4, 4000)]);
        assert_eq!(kinds, vec![ku(MediaKind::Image, 4, 4000)]);
    }

    #[test]
    fn fold_kinds_does_not_let_an_extension_override_a_type_that_says_something() {
        let kinds = fold_kinds(&[g("video/mp4", Some("png"), 1, 10)]);
        assert_eq!(kinds[0].kind, MediaKind::Video);
    }

    #[test]
    fn fold_kinds_orders_by_bytes_heaviest_first() {
        let kinds = fold_kinds(&[
            g("text/plain", Some("txt"), 900, 10),
            g("application/pdf", Some("pdf"), 1, 900),
            g("application/zip", Some("zip"), 2, 400),
        ]);
        let order: Vec<MediaKind> = kinds.iter().map(|k| k.kind).collect();
        assert_eq!(order, vec![MediaKind::Pdf, MediaKind::Archive, MediaKind::Text]);
    }

    #[test]
    fn fold_kinds_breaks_byte_ties_by_kind_name() {
        let kinds = fold_kinds(&[g("video/mp4", None, 1, 7), g("audio/mpeg", None, 1, 7), g("image/png", None, 1, 7)]);
        let order: Vec<&str> = kinds.iter().map(|k| k.kind.as_str()).collect();
        assert_eq!(order, vec!["audio", "image", "video"]);
    }

    #[test]
    fn fold_kinds_is_empty_for_an_empty_install() {
        assert_eq!(fold_kinds(&[]), vec![]);
    }

    fn o(owner_id: Option<&str>, live_bytes: i64, trashed_bytes: i64, files: i64) -> OwnerGroup {
        OwnerGroup {
            owner_id: owner_id.map(Into::into),
            live_files: files,
            live_bytes,
            trashed_files: i64::from(trashed_bytes > 0),
            trashed_bytes,
        }
    }

    fn total_bytes(rows: &[RankedAccount]) -> i64 {
        rows.iter().map(|r| r.live_bytes + r.trashed_bytes).sum()
    }

    // tests/unit/admin-disk.test.ts: "rankAccounts"
    #[test]
    fn rank_accounts_orders_by_everything_the_account_occupies_trash_included() {
        let ranked = rank_accounts(&[o(Some("a"), 100, 0, 1), o(Some("b"), 10, 500, 1), o(Some("c"), 200, 0, 1)], 6);
        let ids: Vec<Option<&str>> = ranked.iter().map(|r| r.id.as_deref()).collect();
        assert_eq!(ids, vec![Some("b"), Some("c"), Some("a")]);
    }

    #[test]
    fn rank_accounts_folds_the_tail_instead_of_dropping_it() {
        let groups = [
            o(Some("a"), 500, 0, 1),
            o(Some("b"), 400, 0, 1),
            o(Some("c"), 300, 0, 1),
            o(Some("d"), 200, 0, 1),
            o(Some("e"), 100, 0, 1),
        ];
        let ranked = rank_accounts(&groups, 2);
        assert_eq!(ranked.len(), 3);
        let group_total: i64 = groups.iter().map(|g| g.live_bytes + g.trashed_bytes).sum();
        assert_eq!(total_bytes(&ranked), group_total);
        let rest = ranked.last().unwrap();
        assert!(rest.rest);
        assert_eq!(rest.accounts, 3);
        assert_eq!(rest.live_bytes, 600);
        assert_eq!(rest.live_files, 3);
    }

    #[test]
    fn rank_accounts_keeps_the_ownerless_group_separate_wherever_it_lands() {
        let ranked = rank_accounts(&[o(Some("a"), 10, 0, 1), o(None, 900, 0, 1)], 1);
        assert_eq!(ranked[0].id, None);
        assert!(ranked[0].orphaned);
        assert!(!ranked[0].rest);
        assert_eq!(ranked[0].live_bytes, 900);
        let last = ranked.last().unwrap();
        assert!(last.rest);
        assert_eq!(last.accounts, 1);
        assert_eq!(last.live_bytes, 10);
    }

    #[test]
    fn rank_accounts_adds_no_fold_row_when_everything_fits() {
        let ranked = rank_accounts(&[o(Some("a"), 10, 0, 1), o(Some("b"), 20, 0, 1)], 6);
        assert_eq!(ranked.len(), 2);
        assert!(ranked.iter().all(|r| !r.rest));
        assert!(ranked.iter().all(|r| r.accounts == 1));
    }

    #[test]
    fn rank_accounts_returns_nothing_for_an_install_with_no_files() {
        assert_eq!(rank_accounts(&[], ACCOUNT_LIMIT), vec![]);
    }

    #[test]
    fn rank_accounts_keeps_the_group_order_on_ties() {
        let ranked = rank_accounts(&[o(Some("x"), 5, 0, 1), o(Some("y"), 5, 0, 1), o(Some("z"), 9, 0, 1)], 6);
        let ids: Vec<Option<&str>> = ranked.iter().map(|r| r.id.as_deref()).collect();
        assert_eq!(ids, vec![Some("z"), Some("x"), Some("y")]);
    }

    // tests/unit/admin-disk-api.test.ts, the lib-side promises: the window
    // constants the route sends and the shape the desktop client deserialises.
    #[test]
    fn windows_are_this_deployments_policy() {
        assert_eq!(COLD_DAYS, 180);
        assert_eq!(TRASH_DUE_SOON_DAYS, 7);
        assert_eq!(crate::maintenance::trash::TRASH_RETENTION_DAYS, 30);
    }

    #[test]
    fn insights_serialize_in_the_shape_clients_read_with_nulls_intact() {
        let cold = HeavyFile {
            id: Uuid::nil(),
            code: "hosade34".into(),
            filename: "old.zip".into(),
            ext: "zip".into(),
            mime_type: "application/zip".into(),
            size: 900_000,
            owner_id: None,
            owner_tag: None,
            owner_name: None,
            created_at: Some("2024-01-01T00:00:00.000Z".into()),
            views: 0,
            last_accessed_at: None,
        };
        let insights = DiskInsights {
            driver: "local".into(),
            local: true,
            kinds: vec![ku(MediaKind::Archive, 1, 900_000)],
            kind_bytes: 900_000,
            accounts: vec![],
            biggest: vec![],
            cold: vec![cold],
            cold_totals: ColdTotals { files: 1, bytes: 900_000, days: COLD_DAYS },
            trash: TrashOutlook {
                files: 0,
                bytes: 0,
                oldest: None,
                due_soon_files: 0,
                due_soon_bytes: 0,
                overdue_files: 0,
                overdue_bytes: 0,
            },
        };
        let v = serde_json::to_value(&insights).unwrap();
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            vec!["driver", "local", "kinds", "kindBytes", "accounts", "biggest", "cold", "coldTotals", "trash"]
        );
        assert!(v["cold"][0]["lastAccessedAt"].is_null());
        assert!(v["cold"][0]["ownerTag"].is_null());
        assert_eq!(v["kinds"][0], serde_json::json!({ "kind": "archive", "files": 1, "bytes": 900_000 }));
        let file_keys: Vec<&String> = v["cold"][0].as_object().unwrap().keys().collect();
        assert_eq!(
            file_keys,
            vec![
                "id",
                "code",
                "filename",
                "ext",
                "mimeType",
                "size",
                "ownerId",
                "ownerTag",
                "ownerName",
                "createdAt",
                "views",
                "lastAccessedAt"
            ]
        );
        let trash_keys: Vec<&String> = v["trash"].as_object().unwrap().keys().collect();
        assert_eq!(
            trash_keys,
            vec!["files", "bytes", "oldest", "dueSoonFiles", "dueSoonBytes", "overdueFiles", "overdueBytes"]
        );
    }

    #[test]
    fn account_rows_keep_upstreams_key_order() {
        let a = AccountUsage {
            id: Some("u".into()),
            orphaned: false,
            rest: false,
            accounts: 1,
            live_files: 1,
            live_bytes: 2,
            trashed_files: 0,
            trashed_bytes: 0,
            name: Some("N".into()),
            email: Some("e@x".into()),
            tag: None,
        };
        let v = serde_json::to_value(&a).unwrap();
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            vec![
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
