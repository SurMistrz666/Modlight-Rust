//! Port of `lib/data/admin.ts`: read models and maintenance probes for the
//! admin panel.
//!
//! Like upstream, nothing here decides who may see it: the callers (the admin
//! page behind `requireAdmin`, the admin actions, `GET /api/admin/disk`) do
//! the guarding. The pure helpers at the bottom (key normalisation) are
//! unit-tested.

use std::collections::{HashMap, HashSet};
use std::os::unix::ffi::OsStrExt;

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::avatar::AVATAR_PREFIX;
use crate::background::BACKGROUND_PREFIX;
use crate::config::Config;
use crate::db::types::JsDate;
use crate::upload::disk::{FilesystemSpace, storage_filesystem_space};

/// `DERIVED_PREFIX` of `lib/media/derived-cache.ts` (that module belongs to the
/// library port): everything derived lives under this one prefix.
const DERIVED_PREFIX: &str = "derived";

// ---------------------------------------------------------------------------
// Orphaned uploads: files whose owning account was deleted with "keep files"
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrphanSummary {
    /// Rows with no owner, trash included (they still occupy disk).
    pub files: i64,
    pub bytes: i64,
    /// How many of those are also sitting in the trash.
    pub trashed: i64,
    /// Upload date of the oldest / newest orphan, ISO, or null when there are none.
    pub oldest: Option<String>,
    pub newest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrphanRow {
    pub id: Uuid,
    pub code: String,
    pub filename: String,
    pub ext: String,
    pub mime_type: String,
    pub size: i64,
    pub storage_key: String,
    /// ISO: server actions may only hand plain data to the client.
    pub created_at: Option<String>,
    pub deleted_at: Option<String>,
}

pub const EMPTY_ORPHAN_SUMMARY: OrphanSummary =
    OrphanSummary { files: 0, bytes: 0, trashed: 0, oldest: None, newest: None };

type OrphanSummaryRow = (Option<i32>, Option<i64>, Option<i32>, Option<JsDate>, Option<JsDate>);

/// Headline numbers for the "Orphaned files" section.
///
/// One aggregate query rather than a listing the browser counts: production
/// carries ~8.6k uploads, and the summary has to stay cheap enough to render
/// on every visit to the admin page.
pub async fn orphan_summary(db: &PgPool) -> sqlx::Result<OrphanSummary> {
    // An aggregate without GROUP BY always answers one row; upstream's
    // `if (!row) return EMPTY_ORPHAN_SUMMARY` is kept for the shape only.
    let row: Option<OrphanSummaryRow> = sqlx::query_as(
        r#"SELECT count(*)::int, coalesce(sum("size"), 0)::bigint,
                  (count(*) filter (where "deleted_at" is not null))::int,
                  min("created_at"), max("created_at")
           FROM "uploads" WHERE "owner_id" IS NULL"#,
    )
    .fetch_optional(db)
    .await?;
    let Some((files, bytes, trashed, oldest, newest)) = row else {
        return Ok(EMPTY_ORPHAN_SUMMARY);
    };
    Ok(OrphanSummary {
        files: i64::from(files.unwrap_or(0)),
        bytes: bytes.unwrap_or(0),
        trashed: i64::from(trashed.unwrap_or(0)),
        oldest: iso_or_null(oldest),
        newest: iso_or_null(newest),
    })
}

/// Cap on how many orphan rows travel to the browser at once.
pub const ORPHAN_PAGE_SIZE: i64 = 200;

type OrphanDbRow = (Uuid, String, String, String, String, i64, String, Option<JsDate>, Option<JsDate>);

/// Oldest first: an orphan that has been sitting there for a year is the story.
/// Upstream's default argument is [`ORPHAN_PAGE_SIZE`].
pub async fn list_orphans(db: &PgPool, limit: i64) -> sqlx::Result<Vec<OrphanRow>> {
    let rows: Vec<OrphanDbRow> = sqlx::query_as(
        r#"SELECT "id", "code", "filename", "ext", "mime_type", "size", "storage_key", "created_at", "deleted_at"
           FROM "uploads" WHERE "owner_id" IS NULL ORDER BY "created_at" ASC LIMIT $1"#,
    )
    .bind(ORPHAN_PAGE_SIZE.min(limit.max(1)))
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, code, filename, ext, mime_type, size, storage_key, created_at, deleted_at)| OrphanRow {
            id,
            code,
            filename,
            ext,
            mime_type,
            size,
            storage_key,
            created_at: iso_or_null(created_at),
            deleted_at: iso_or_null(deleted_at),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Server disk: how full the machine is, not just how full an account is
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStorageUsage {
    /// Where the bytes actually live. "s3" means the filesystem numbers are absent.
    pub driver: String,
    /// Uploads that are not in the trash.
    pub live_files: i64,
    pub live_bytes: i64,
    /// Soft-deleted uploads. Still on disk, but reclaimable: that is the point.
    pub trashed_files: i64,
    pub trashed_bytes: i64,
    /// The filesystem holding storage, or null for an object store / a failed statfs.
    pub disk: Option<FilesystemSpace>,
    /// Operator's floor, so the panel can say WHY a number is alarming.
    pub min_free_bytes: i64,
}

/// Disk usage for the whole install: what the database accounts for, and
/// what the machine actually has left.
///
/// The second half is the reason this exists. Per-account quota answers "is
/// this user over their limit"; it says nothing about the host, and a full
/// disk takes every site on it down at once, so the number an administrator
/// needs is the filesystem's, measured through the very same statfs the
/// upload guard refuses 507s with.
///
/// Orphans (owner deleted, files kept) are counted here even though the
/// overview tiles exclude them: they occupy disk exactly like any other file,
/// and the question this answers is "how much disk", not "whose disk".
pub async fn server_storage_usage(db: &PgPool, config: &Config) -> sqlx::Result<ServerStorageUsage> {
    // One pass with filtered aggregates. Live and trashed are split here
    // rather than by two queries so the two numbers can never be read from
    // different instants.
    let totals = async {
        sqlx::query_as::<_, (Option<i32>, Option<i64>, Option<i32>, Option<i64>)>(
            r#"SELECT (count(*) filter (where "deleted_at" is null))::int,
                      coalesce(sum("size") filter (where "deleted_at" is null), 0)::bigint,
                      (count(*) filter (where "deleted_at" is not null))::int,
                      coalesce(sum("size") filter (where "deleted_at" is not null), 0)::bigint
               FROM "uploads""#,
        )
        .fetch_optional(db)
        .await
    };
    let (totals, disk) = tokio::join!(totals, storage_filesystem_space(config));
    let (live_files, live_bytes, trashed_files, trashed_bytes) = totals?.unwrap_or((None, None, None, None));
    Ok(ServerStorageUsage {
        driver: config.storage.driver.clone(),
        live_files: i64::from(live_files.unwrap_or(0)),
        live_bytes: live_bytes.unwrap_or(0),
        trashed_files: i64::from(trashed_files.unwrap_or(0)),
        trashed_bytes: trashed_bytes.unwrap_or(0),
        disk,
        min_free_bytes: config.uploads.min_free_bytes,
    })
}

// ---------------------------------------------------------------------------
// Account deletion: what exactly is about to be destroyed
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserDeletionImpact {
    pub id: String,
    pub email: String,
    pub name: String,
    pub tag: Option<String>,
    /// Uploads owned, trash included: those bytes are on disk either way.
    pub files: i64,
    pub bytes: i64,
    pub trashed: i64,
    /// Rows that always go with the account (FK cascade), shown so it isn't a surprise.
    pub folders: i64,
    pub short_links: i64,
    pub tokens: i64,
}

/// Everything the "delete account" dialog has to state out loud before the
/// admin decides what happens to the files. `None` when the account is gone.
pub async fn user_deletion_impact(db: &PgPool, user_id: &str) -> sqlx::Result<Option<UserDeletionImpact>> {
    let account: Option<(String, String, String, Option<String>)> =
        sqlx::query_as(r#"SELECT "id", "email", "name", "user_code" FROM "user" WHERE "id" = $1 LIMIT 1"#)
            .bind(user_id)
            .fetch_optional(db)
            .await?;
    let Some((id, email, name, tag)) = account else {
        return Ok(None);
    };

    let files = sqlx::query_as::<_, (Option<i32>, Option<i64>, Option<i32>)>(
        r#"SELECT count(*)::int, coalesce(sum("size"), 0)::bigint,
                  (count(*) filter (where "deleted_at" is not null))::int
           FROM "uploads" WHERE "owner_id" = $1"#,
    )
    .bind(user_id)
    .fetch_optional(db);
    let folders = count_where(db, r#"SELECT count(*) FROM "folders" WHERE "owner_id" = $1"#, user_id);
    let links = count_where(db, r#"SELECT count(*) FROM "short_links" WHERE "owner_id" = $1"#, user_id);
    let tokens = count_where(db, r#"SELECT count(*) FROM "api_tokens" WHERE "user_id" = $1"#, user_id);
    let (files, folders, links, tokens) = tokio::try_join!(files, folders, links, tokens)?;
    let (file_count, bytes, trashed) = files.unwrap_or((None, None, None));

    Ok(Some(UserDeletionImpact {
        id,
        email,
        name,
        tag,
        files: i64::from(file_count.unwrap_or(0)),
        bytes: bytes.unwrap_or(0),
        trashed: i64::from(trashed.unwrap_or(0)),
        folders,
        short_links: links,
        tokens,
    }))
}

/// drizzle's `count()`: `count(*)` read through `Number`.
async fn count_where(db: &PgPool, sql: &'static str, id: &str) -> sqlx::Result<i64> {
    let row: Option<(Option<i64>,)> = sqlx::query_as(sql).bind(id).fetch_optional(db).await?;
    Ok(row.and_then(|r| r.0).unwrap_or(0))
}

// ---------------------------------------------------------------------------
// Maintenance: quota recalculation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaRecalcRow {
    pub id: String,
    pub email: String,
    pub before: i64,
    pub after: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaRecalcResult {
    pub accounts: i64,
    /// Accounts whose stored counter disagreed with reality.
    pub corrected: i64,
    /// Signed byte delta applied across all accounts (negative = counter was too high).
    pub drift: i64,
    /// The corrected accounts, for the report. Capped, see [`MAX_RECALC_REPORT`].
    pub changes: Vec<QuotaRecalcRow>,
}

const MAX_RECALC_REPORT: usize = 50;

/// Recompute `used_quota` for EVERY account from the rows that actually exist.
///
/// The counter is maintained incrementally (upload adds, permanent delete
/// subtracts), so it drifts whenever rows move out-of-band: a failed
/// finalize, a hand-written SQL fix, a bulk import, or an account deleted with
/// its files kept. Doing all accounts in one pass is the useful shape: after a
/// manual database operation you do not know which account was affected.
///
/// Trashed files are counted on purpose: they still occupy disk until purged.
/// Orphans are excluded by construction: they group under a NULL owner, which
/// matches no account.
pub async fn recalculate_all_quotas(db: &PgPool) -> sqlx::Result<QuotaRecalcResult> {
    let accounts =
        sqlx::query_as::<_, (String, String, Option<i64>)>(r#"SELECT "id", "email", "used_quota" FROM "user""#)
            .fetch_all(db);
    let sums = sqlx::query_as::<_, (Option<String>, Option<i64>)>(
        r#"SELECT "owner_id", coalesce(sum("size"), 0)::bigint FROM "uploads"
           WHERE "owner_id" IS NOT NULL GROUP BY "owner_id""#,
    )
    .fetch_all(db);
    let (accounts, sums) = tokio::try_join!(accounts, sums)?;

    // `if (s.ownerId)`: a truthy owner only.
    let actual: HashMap<String, i64> = sums
        .into_iter()
        .filter_map(|(owner, total)| owner.filter(|o| !o.is_empty()).map(|o| (o, total.unwrap_or(0))))
        .collect();

    let mut changes = Vec::new();
    let mut corrected = 0;
    let mut drift = 0;
    let total = accounts.len() as i64;
    for (id, email, used) in accounts {
        let before = used.unwrap_or(0);
        let after = actual.get(&id).copied().unwrap_or(0);
        if before == after {
            continue;
        }
        // `user.updated_at` has `$onUpdate(() => new Date())` in the schema, so
        // drizzle's update writes it too.
        sqlx::query(r#"UPDATE "user" SET "used_quota" = $1, "updated_at" = $2 WHERE "id" = $3"#)
            .bind(after)
            .bind(JsDate::now())
            .bind(&id)
            .execute(db)
            .await?;
        corrected += 1;
        drift += after - before;
        if changes.len() < MAX_RECALC_REPORT {
            changes.push(QuotaRecalcRow { id, email, before, after });
        }
    }

    Ok(QuotaRecalcResult { accounts: total, corrected, drift, changes })
}

// ---------------------------------------------------------------------------
// Maintenance: storage objects with no database row
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageOrphan {
    pub key: String,
    pub size: u64,
    /// ISO mtime: how long the file has been lying around unreferenced.
    pub modified_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageScanResult {
    /// False when the configured driver cannot be walked (S3).
    pub supported: bool,
    /// Driver the scan ran against, for the report.
    pub driver: String,
    /// Objects examined on disk.
    pub scanned: u64,
    /// Objects with no row in `uploads` (neither storageKey nor thumbKey).
    pub orphans: u64,
    pub orphan_bytes: u64,
    /// A sample of them, capped at [`MAX_STORAGE_ORPHAN_SAMPLE`].
    pub sample: Vec<StorageOrphan>,
    /// True when the walk hit its file ceiling and stopped early.
    pub truncated: bool,
    /// Files younger than the grace window that were skipped (likely in flight).
    pub skipped_recent: u64,
}

/// Never report an object younger than this: it may be mid-finalize.
pub const STORAGE_SCAN_GRACE_MS: i64 = 60 * 60 * 1000;

/// Top-level storage trees that deliberately have no row in `uploads`.
///
/// `derived/` holds cached conversions, keyed by content rather than by a row.
/// `backgrounds/` and `avatars/` hold per-account decoration, which is kept
/// out of `uploads` on purpose: it must not appear in a dashboard, count
/// against a quota, or be reachable at a /{tag}/{code} link. Each is freed by
/// the feature that wrote it, so none of them is this report's business.
/// Not upstream: `releases/` holds Downloads' builds (`releases` table).
const UNTRACKED_PREFIXES: [&str; 4] =
    [DERIVED_PREFIX, BACKGROUND_PREFIX, AVATAR_PREFIX, crate::downloads::STORAGE_PREFIX];

/// Hard ceiling on the walk, so one click can never wander off into a mount.
const MAX_SCANNED_FILES: u64 = 250_000;

/// How many orphan keys travel to the browser.
pub const MAX_STORAGE_ORPHAN_SAMPLE: usize = 100;

/// Find bytes on disk that no `uploads` row points at.
///
/// These are the mirror image of an orphaned row: an interrupted finalize, a
/// row deleted by hand, an import that half-ran. Nothing reads them and
/// nothing will ever free them.
///
/// Report only, deliberately no delete button: an upload being finalised
/// right now looks exactly like an orphan for a fraction of a second, and
/// "the admin panel ate my screenshot" is a far worse bug than some wasted
/// disk. The grace window makes the report trustworthy; acting on it is a
/// deliberate, manual step.
///
/// Local driver only: S3 listing is a paginated, billed API and belongs in a
/// background job, not in a page the admin clicks. Upstream's default `now`
/// is `Date.now()`.
pub async fn scan_storage_orphans(db: &PgPool, config: &Config, now_ms: i64) -> sqlx::Result<StorageScanResult> {
    let driver = config.storage.driver.clone();
    if driver != "local" {
        return Ok(StorageScanResult {
            supported: false,
            driver,
            scanned: 0,
            orphans: 0,
            orphan_bytes: 0,
            sample: Vec::new(),
            truncated: false,
            skipped_recent: 0,
        });
    }

    let root = resolve_path(&config.storage.local_path);

    // Every key any row references: storageKey AND thumbKey, or every cached
    // thumbnail would be reported as garbage.
    let rows: Vec<(String, Option<String>)> =
        sqlx::query_as(r#"SELECT "storage_key", "thumb_key" FROM "uploads""#).fetch_all(db).await?;
    let mut known: HashSet<String> = HashSet::new();
    for (storage_key, thumb_key) in rows {
        known.insert(normalize_key(&storage_key));
        if let Some(thumb) = thumb_key.filter(|t| !t.is_empty()) {
            known.insert(normalize_key(&thumb));
        }
    }

    let mut result = StorageScanResult {
        supported: true,
        driver,
        scanned: 0,
        orphans: 0,
        orphan_bytes: 0,
        sample: Vec::new(),
        truncated: false,
        skipped_recent: 0,
    };

    let mut queue: Vec<String> = vec![root.clone()];
    'walk: while let Some(dir) = queue.pop() {
        let Ok(entries) = read_dir_sorted(&dir).await else {
            // Storage root missing (fresh install) or unreadable: nothing to report.
            continue;
        };

        for (name, file_type) in entries {
            let full = join_path(&dir, &name);
            if file_type.is_dir() {
                // Top-level trees that no `uploads` row will ever point at, so
                // every single object under them would be reported as garbage.
                if dir == root && UNTRACKED_PREFIXES.contains(&name.as_str()) {
                    continue;
                }
                queue.push(full);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }

            if result.scanned >= MAX_SCANNED_FILES {
                result.truncated = true;
                queue.clear();
                break 'walk;
            }
            result.scanned += 1;

            let Some(key) = to_storage_key(&root, &full) else {
                continue;
            };
            if key.is_empty() || known.contains(&key) {
                continue;
            }

            let Ok(info) = tokio::fs::metadata(&full).await else {
                continue; // vanished between readdir and stat: that is fine
            };
            let mtime_ms = mtime_ms(&info);
            if (now_ms as f64) - mtime_ms < STORAGE_SCAN_GRACE_MS as f64 {
                result.skipped_recent += 1;
                continue;
            }

            result.orphans += 1;
            result.orphan_bytes += info.len();
            if result.sample.len() < MAX_STORAGE_ORPHAN_SAMPLE {
                result.sample.push(StorageOrphan {
                    key,
                    size: info.len(),
                    modified_at: JsDate::from_millis(mtime_ms.trunc() as i64).map(|d| d.to_iso_string()),
                });
            }
        }
    }

    Ok(result)
}

/// `readdir(dir, { withFileTypes: true })`: libuv's scandir hands the names
/// back sorted with `strcmp`, and Node decodes them as UTF-8 (lossily).
async fn read_dir_sorted(dir: &str) -> std::io::Result<Vec<(String, std::fs::FileType)>> {
    let mut reader = tokio::fs::read_dir(dir).await?;
    let mut entries = Vec::new();
    while let Some(entry) = reader.next_entry().await? {
        let Ok(file_type) = entry.file_type().await else {
            continue;
        };
        entries.push((entry.file_name(), file_type));
    }
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    Ok(entries.into_iter().map(|(name, t)| (name.to_string_lossy().into_owned(), t)).collect())
}

/// `stats.mtimeMs`: milliseconds with the fraction Node keeps.
fn mtime_ms(info: &std::fs::Metadata) -> f64 {
    use std::os::unix::fs::MetadataExt;
    info.mtime() as f64 * 1000.0 + info.mtime_nsec() as f64 / 1_000_000.0
}

/// Node's `path.resolve(p)` on POSIX: absolute against the working directory,
/// `.` and `..` folded lexically, no trailing slash.
fn resolve_path(path: &str) -> String {
    let joined = if path.starts_with('/') {
        path.to_string()
    } else {
        let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_else(|_| "/".into());
        format!("{cwd}/{path}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    format!("/{}", parts.join("/"))
}

/// `path.join(dir, name)` for a directory from [`resolve_path`] and a plain
/// directory entry name.
fn join_path(dir: &str, name: &str) -> String {
    if dir.ends_with('/') { format!("{dir}{name}") } else { format!("{dir}/{name}") }
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested; keep free of I/O)
// ---------------------------------------------------------------------------

/// The platform's path separator (`path.sep`).
const SEP: &str = std::path::MAIN_SEPARATOR_STR;

/// Storage keys are stored with forward slashes ("tag/code.png") on every
/// platform; the filesystem walk hands back native separators. Normalising
/// both sides is what keeps the scan from reporting every single file as an
/// orphan on Windows.
pub fn normalize_key(key: &str) -> String {
    let joined = key.split(SEP).collect::<Vec<_>>().join("/");
    // .replace(/^\.?\/+/, "")
    let after_dot = joined.strip_prefix('.').unwrap_or(&joined);
    let trimmed = after_dot.trim_start_matches('/');
    if trimmed.len() < after_dot.len() { trimmed.to_string() } else { joined }
}

/// Relative storage key for an absolute path under `root`, or `None` if outside it.
pub fn to_storage_key(root: &str, absolute_path: &str) -> Option<String> {
    let normalized_root = root.strip_suffix(SEP).unwrap_or(root);
    let prefix = format!("{normalized_root}{SEP}");
    let rest = absolute_path.strip_prefix(&prefix)?;
    Some(normalize_key(rest))
}

/// Date -> ISO for the client boundary, tolerating the null the aggregates return.
fn iso_or_null(value: Option<JsDate>) -> Option<String> {
    value.map(|d| d.to_iso_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // tests/unit/admin-orphans.test.ts: "normalizeKey"
    #[test]
    fn normalize_key_leaves_a_canonical_key_alone() {
        assert_eq!(normalize_key("alicja/quick-fox-42.png"), "alicja/quick-fox-42.png");
    }

    #[test]
    fn normalize_key_converts_native_separators() {
        assert_eq!(normalize_key(&["alicja", "quick-fox-42.png"].join(SEP)), "alicja/quick-fox-42.png");
    }

    #[test]
    fn normalize_key_strips_a_leading_dot_slash_or_slash() {
        assert_eq!(normalize_key("./alicja/a.png"), "alicja/a.png");
        assert_eq!(normalize_key("/alicja/a.png"), "alicja/a.png");
        // The regex is /^\.?\/+/: one optional dot, then at least one slash.
        assert_eq!(normalize_key(".//a.png"), "a.png");
        assert_eq!(normalize_key(".hidden/a.png"), ".hidden/a.png");
        assert_eq!(normalize_key("../a.png"), "../a.png");
    }

    // tests/unit/admin-orphans.test.ts: "toStorageKey"
    const ROOT: &str = "/srv/modlight/storage";

    #[test]
    fn to_storage_key_returns_the_key_a_row_would_hold() {
        assert_eq!(
            to_storage_key(ROOT, &format!("{ROOT}/alicja/quick-fox-42.png")).as_deref(),
            Some("alicja/quick-fox-42.png")
        );
    }

    #[test]
    fn to_storage_key_handles_a_nested_thumbnail_path() {
        assert_eq!(
            to_storage_key(ROOT, &format!("{ROOT}/thumbs/alicja/quick-fox-42.webp")).as_deref(),
            Some("thumbs/alicja/quick-fox-42.webp")
        );
    }

    #[test]
    fn to_storage_key_refuses_a_path_outside_the_root() {
        // A key computed from the wrong root would miss every known entry and
        // the scan would report the whole disk as garbage.
        assert_eq!(to_storage_key(ROOT, "/srv/modlight/tus/incoming.bin"), None);
    }

    #[test]
    fn to_storage_key_is_not_fooled_by_a_sibling_sharing_the_prefix() {
        assert_eq!(to_storage_key(ROOT, "/srv/modlight/storage-old/a.png"), None);
    }

    #[test]
    fn to_storage_key_returns_null_for_the_root_itself() {
        assert_eq!(to_storage_key(ROOT, ROOT), None);
    }

    #[test]
    fn to_storage_key_tolerates_a_root_with_a_trailing_separator() {
        assert_eq!(
            to_storage_key(&format!("{ROOT}/"), &format!("{ROOT}/alicja/a.png")).as_deref(),
            Some("alicja/a.png")
        );
    }

    #[test]
    fn resolves_like_node() {
        assert_eq!(resolve_path("/a/b/../c/./d/"), "/a/c/d");
        assert_eq!(resolve_path("/"), "/");
        assert_eq!(resolve_path("/.."), "/");
        let cwd = std::env::current_dir().unwrap().to_string_lossy().into_owned();
        assert_eq!(resolve_path("./data/storage"), format!("{cwd}/data/storage"));
        assert_eq!(join_path("/", "a"), "/a");
        assert_eq!(join_path("/x", "a"), "/x/a");
    }

    #[test]
    fn shapes_serialize_like_upstream() {
        let usage = ServerStorageUsage {
            driver: "local".into(),
            live_files: 1,
            live_bytes: 2,
            trashed_files: 3,
            trashed_bytes: 4,
            disk: None,
            min_free_bytes: 5,
        };
        let keys: Vec<String> = serde_json::to_value(&usage).unwrap().as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["driver", "liveFiles", "liveBytes", "trashedFiles", "trashedBytes", "disk", "minFreeBytes"]);
        let scan = StorageScanResult {
            supported: false,
            driver: "s3".into(),
            scanned: 0,
            orphans: 0,
            orphan_bytes: 0,
            sample: vec![],
            truncated: false,
            skipped_recent: 0,
        };
        assert_eq!(
            serde_json::to_value(&scan).unwrap(),
            serde_json::json!({
                "supported": false, "driver": "s3", "scanned": 0, "orphans": 0, "orphanBytes": 0,
                "sample": [], "truncated": false, "skippedRecent": 0
            })
        );
    }
}
