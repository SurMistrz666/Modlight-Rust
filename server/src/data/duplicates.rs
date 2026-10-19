//! Port of `lib/data/duplicates.ts`: the retroactive duplicate scan, the
//! panel's reminder, and the "these copies are meant to exist" dismissals.
//! (`isSnoozed` of `components/duplicates/snooze.ts` is here too: the server
//! decides the (app) layout's DuplicateReminderSlot.)

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::Config;
use crate::db::types::JsDate;
use crate::media::urls::{LinkOwner, media_urls};

/// Sets listed per scan.
pub const MAX_GROUPS: i64 = 100;
/// Rows fetched for those sets.
pub const MAX_ROWS: i64 = 2000;
/// Copies listed per set.
pub const MAX_COPIES_LISTED: usize = 50;

/// `DuplicateFile`
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateFile {
    pub id: Uuid,
    pub code: String,
    pub filename: String,
    pub ext: String,
    pub mime_type: String,
    pub size: i64,
    pub created_at: String,
    pub folder_id: Option<Uuid>,
    pub folder_name: Option<String>,
    pub views: i32,
    pub downloads: i32,
    pub url: String,
    pub blurhash: Option<String>,
}

/// `DuplicateGroup`
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DuplicateGroup {
    pub checksum: String,
    pub copies: i64,
    pub size: i64,
    pub bytes: i64,
    pub reclaimable: i64,
    pub dismissed: bool,
    pub files: Vec<DuplicateFile>,
}

/// `DuplicateTotals`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct DuplicateTotals {
    pub groups: i64,
    pub redundant: i64,
    pub reclaimable: i64,
}

/// `DuplicateScan`
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateScan {
    pub groups: Vec<DuplicateGroup>,
    pub total_groups: i64,
    pub total_files: i64,
    pub reclaimable: i64,
    pub dismissed_groups: i64,
    pub active: DuplicateTotals,
    pub truncated: bool,
    pub unchecksummed: i64,
    pub scanned_at: String,
}

/// `DuplicateRow`
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct DuplicateRow {
    pub id: Uuid,
    pub code: String,
    pub filename: String,
    pub ext: String,
    pub mime_type: String,
    pub size: i64,
    pub created_at: JsDate,
    pub folder_id: Option<Uuid>,
    pub folder_name: Option<String>,
    pub views: i32,
    pub downloads: i32,
    pub checksum: Option<String>,
    pub blurhash: Option<String>,
}

/// Per-checksum figures from the aggregate (`GroupStats`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupStat {
    pub copies: i64,
    pub bytes: i64,
    pub dismissed: bool,
}

pub fn has_usable_checksum(checksum: Option<&str>) -> bool {
    checksum.is_some_and(|c| !crate::config::js_trim(c).is_empty())
}

/// Group rows by checksum, oldest first inside each group; single-row groups
/// are dropped; dismissed sets last, then biggest saving, then checksum.
pub fn bucket_by_checksum(
    rows: &[DuplicateRow],
    url_for: impl Fn(&str, &str) -> String,
    stats: Option<&IndexMap<String, GroupStat>>,
) -> Vec<DuplicateGroup> {
    let mut buckets: IndexMap<String, Vec<&DuplicateRow>> = IndexMap::new();
    for row in rows {
        let Some(checksum) = row.checksum.as_deref().filter(|c| has_usable_checksum(Some(c))) else {
            continue;
        };
        buckets.entry(checksum.to_string()).or_default().push(row);
    }
    let mut groups: Vec<DuplicateGroup> = Vec::new();
    for (checksum, mut bucket) in buckets {
        bucket.sort_by(|a, b| {
            a.created_at.millis().cmp(&b.created_at.millis()).then_with(|| a.id.to_string().cmp(&b.id.to_string()))
        });
        let stat = stats.and_then(|s| s.get(&checksum));
        let copies = stat.map(|s| s.copies).unwrap_or(bucket.len() as i64);
        // A single row is not a duplicate (a copy was trashed in between).
        if copies < 2 || bucket.len() < 2 {
            continue;
        }
        let bytes = stat.map(|s| s.bytes).unwrap_or_else(|| bucket.iter().map(|r| r.size).sum());
        let kept = bucket[0].size;
        groups.push(DuplicateGroup {
            checksum: checksum.clone(),
            copies,
            size: kept,
            bytes,
            reclaimable: (bytes - kept).max(0),
            dismissed: stat.is_some_and(|s| s.dismissed),
            files: bucket
                .iter()
                .take(MAX_COPIES_LISTED)
                .map(|row| DuplicateFile {
                    id: row.id,
                    code: row.code.clone(),
                    filename: row.filename.clone(),
                    ext: row.ext.clone(),
                    mime_type: row.mime_type.clone(),
                    size: row.size,
                    created_at: row.created_at.to_iso_string(),
                    folder_id: row.folder_id,
                    folder_name: row.folder_name.clone(),
                    views: row.views,
                    downloads: row.downloads,
                    url: url_for(&row.code, &row.ext),
                    blurhash: row.blurhash.clone(),
                })
                .collect(),
        });
    }
    groups.sort_by(|a, b| {
        a.dismissed.cmp(&b.dismissed).then(b.reclaimable.cmp(&a.reclaimable)).then_with(|| a.checksum.cmp(&b.checksum))
    });
    groups
}

/// The duplicate sets of one owner (`dupesCte`).
const DUPES_CTE: &str = r#"
    select "uploads"."checksum" as checksum, "uploads"."owner_id" as owner_id, count(*)::int as copies,
           sum("uploads"."size")::bigint as bytes, max("uploads"."size")::bigint as keep
    from "uploads"
    where "uploads"."owner_id" = $1 and "uploads"."deleted_at" is null
      and "uploads"."checksum" is not null and length("uploads"."checksum") > 0
    group by "uploads"."checksum", "uploads"."owner_id"
    having count(*) > 1"#;

fn empty_scan(unchecksummed: i64) -> DuplicateScan {
    DuplicateScan {
        groups: Vec::new(),
        total_groups: 0,
        total_files: 0,
        reclaimable: 0,
        dismissed_groups: 0,
        active: DuplicateTotals::default(),
        truncated: false,
        unchecksummed,
        scanned_at: JsDate::now().to_iso_string(),
    }
}

/// More than this many redundant copies, or more than a gibibyte
/// reclaimable, and the reminder speaks.
pub const REMINDER_MIN_REDUNDANT: i64 = 10;
pub const REMINDER_MIN_BYTES: i64 = 1024 * 1024 * 1024;

/// "Not now" for the reminder (components/duplicates/snooze.ts).
pub const SNOOZE_COOKIE: &str = "ml_dupes_snooze";
pub const SNOOZE_DAYS: i64 = 7;

/// `isSnoozed(value, now)`: a non-empty value whose `Number()` is finite and
/// in the future.
pub fn is_snoozed(value: Option<&str>, now_ms: i64) -> bool {
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return false;
    };
    let until = crate::config::js_number(value);
    until.is_finite() && until > now_ms as f64
}

pub fn reminder_due(totals: &DuplicateTotals) -> bool {
    totals.redundant > REMINDER_MIN_REDUNDANT || totals.reclaimable > REMINDER_MIN_BYTES
}

/// The reminder's aggregate: the duplicate sets of one owner, minus the ones
/// waved off, in ONE statement. Returns the totals and whether they are due.
pub async fn duplicate_reminder(db: &PgPool, user_id: &str) -> sqlx::Result<(DuplicateTotals, bool)> {
    let row: (Option<i32>, Option<i32>, Option<i64>) = sqlx::query_as(&format!(
        r#"with dupes as ({DUPES_CTE})
           select count(*)::int as groups,
                  coalesce(sum(d.copies - 1), 0)::int as redundant,
                  coalesce(sum(d.bytes - d.keep), 0)::bigint as reclaimable
           from dupes d
           where not exists (select 1 from "duplicate_dismissals"
                             where "duplicate_dismissals"."owner_id" = d.owner_id
                               and "duplicate_dismissals"."checksum" = d.checksum)"#
    ))
    .bind(user_id)
    .fetch_one(db)
    .await?;
    let totals = DuplicateTotals {
        groups: i64::from(row.0.unwrap_or(0)),
        redundant: i64::from(row.1.unwrap_or(0)),
        reclaimable: row.2.unwrap_or(0),
    };
    Ok((totals, reminder_due(&totals)))
}

/// `{"redundant", "reclaimable"}` for the (app) layout, or `None`:
/// DuplicateReminderSlot's decision (snooze cookie first, no query then; any
/// error is logged and reads as `None`).
pub async fn duplicate_reminder_slot(db: &PgPool, user_id: &str, snooze_cookie: Option<&str>) -> Option<Value> {
    if is_snoozed(snooze_cookie, JsDate::now().millis()) {
        return None;
    }
    match duplicate_reminder(db, user_id).await {
        Ok((totals, true)) => {
            Some(serde_json::json!({ "redundant": totals.redundant, "reclaimable": totals.reclaimable }))
        }
        Ok(_) => None,
        Err(err) => {
            tracing::error!("[duplicates] reminder failed: {err}");
            None
        }
    }
}

/// `DismissalInput`
#[derive(Debug, Clone, PartialEq)]
pub struct DismissalInput {
    pub checksum: String,
    pub copies: f64,
    pub bytes: f64,
}

/// `whole(value)`: `Math.trunc(Number(value))`, positive and finite, else 0.
fn whole(value: f64) -> i64 {
    let n = value.trunc();
    if n.is_finite() && n > 0.0 { n as i64 } else { 0 }
}

/// Mark sets as "meant to exist": upsert per checksum, keeping `created_at`
/// on conflict. Returns how many rows were written.
pub async fn dismiss_checksums(db: &PgPool, user_id: &str, entries: &[DismissalInput]) -> sqlx::Result<i64> {
    let mut by_checksum: IndexMap<String, &DismissalInput> = IndexMap::new();
    for entry in entries {
        if entry.checksum.is_empty() {
            continue;
        }
        by_checksum.insert(entry.checksum.clone(), entry);
    }
    if by_checksum.is_empty() {
        return Ok(0);
    }
    let checksums: Vec<String> = by_checksum.keys().cloned().collect();
    let copies: Vec<i32> = by_checksum.values().map(|e| whole(e.copies) as i32).collect();
    let bytes: Vec<i64> = by_checksum.values().map(|e| whole(e.bytes)).collect();
    let rows: Vec<String> = sqlx::query_scalar(
        r#"insert into "duplicate_dismissals" ("owner_id", "checksum", "copies", "bytes")
           select $1, c, n, b from unnest($2::varchar[], $3::int[], $4::bigint[]) as t(c, n, b)
           on conflict ("owner_id", "checksum") do update set "copies" = excluded.copies, "bytes" = excluded.bytes
           returning "checksum""#,
    )
    .bind(user_id)
    .bind(&checksums)
    .bind(&copies)
    .bind(&bytes)
    .fetch_all(db)
    .await?;
    Ok(rows.len() as i64)
}

/// Count sets again: drop their dismissals.
pub async fn restore_checksums(db: &PgPool, user_id: &str, checksums: &[String]) -> sqlx::Result<i64> {
    if checksums.is_empty() {
        return Ok(0);
    }
    let rows: Vec<String> = sqlx::query_scalar(
        r#"delete from "duplicate_dismissals" where ("duplicate_dismissals"."owner_id" = $1 and "duplicate_dismissals"."checksum" = any($2)) returning "checksum""#,
    )
    .bind(user_id)
    .bind(checksums)
    .fetch_all(db)
    .await?;
    Ok(rows.len() as i64)
}

type AggregateRow = (String, i32, i64, i64, bool, i32, i32, Option<i64>, i32, i32, Option<i64>);

/// The retroactive scan: the aggregate, the rows of those checksums, and the
/// count of files no lookup can see.
pub async fn scan_duplicates(db: &PgPool, config: &Config, user_id: &str) -> anyhow::Result<DuplicateScan> {
    let aggregate_sql = format!(
        r#"with dupes as ({DUPES_CTE}),
           marked as (
             select d.checksum, d.copies, d.bytes, d.keep, ("duplicate_dismissals"."checksum" is not null) as dismissed
             from dupes d
             left join "duplicate_dismissals"
               on "duplicate_dismissals"."owner_id" = d.owner_id and "duplicate_dismissals"."checksum" = d.checksum)
           select checksum, copies, bytes, keep, dismissed,
                  (count(*) over ())::int as total_groups,
                  (sum(copies) over ())::int as total_files,
                  (sum(bytes - keep) over ())::bigint as total_reclaimable,
                  (count(*) filter (where not dismissed) over ())::int as active_groups,
                  coalesce(sum(copies - 1) filter (where not dismissed) over (), 0)::int as active_redundant,
                  coalesce(sum(bytes - keep) filter (where not dismissed) over (), 0)::bigint as active_reclaimable
           from marked
           order by dismissed asc, (bytes - keep) desc, checksum
           limit {MAX_GROUPS}"#
    );
    let (summaries, prefs, unchecksummed) = tokio::try_join!(
        async { sqlx::query_as::<_, AggregateRow>(&aggregate_sql).bind(user_id).fetch_all(db).await },
        crate::data::user_prefs::get_user_prefs(db, user_id),
        crate::upload::duplicate_lookup::count_unchecksummed(db, user_id),
    )?;
    if summaries.is_empty() {
        return Ok(empty_scan(unchecksummed));
    }
    let first = &summaries[0];
    let total_groups = i64::from(first.5);
    let total_files = i64::from(first.6);
    let reclaimable = first.7.unwrap_or(0);
    let active = DuplicateTotals {
        groups: i64::from(first.8),
        redundant: i64::from(first.9),
        reclaimable: first.10.unwrap_or(0),
    };
    let stats: IndexMap<String, GroupStat> = summaries
        .iter()
        .map(|s| (s.0.clone(), GroupStat { copies: i64::from(s.1), bytes: s.2, dismissed: s.4 }))
        .collect();
    let checksums: Vec<String> = stats.keys().cloned().collect();
    let rows: Vec<DuplicateRow> = sqlx::query_as(&format!(
        r#"select "uploads"."id", "uploads"."code", "uploads"."filename", "uploads"."ext", "uploads"."mime_type",
                  "uploads"."size", "uploads"."created_at", "uploads"."folder_id", "folders"."name" as folder_name,
                  "uploads"."views", "uploads"."downloads", "uploads"."checksum", "uploads"."blurhash"
           from "uploads" left join "folders" on "folders"."id" = "uploads"."folder_id"
           where ("uploads"."owner_id" = $1 and "uploads"."deleted_at" is null and "uploads"."checksum" = any($2))
           order by "uploads"."checksum" asc, "uploads"."created_at" asc, "uploads"."id" asc
           limit {MAX_ROWS}"#
    ))
    .bind(user_id)
    .bind(&checksums)
    .fetch_all(db)
    .await?;
    let owner = LinkOwner {
        user_code: prefs.as_ref().and_then(|p| p.user_code.clone()),
        preferred_domain: prefs.as_ref().and_then(|p| p.preferred_domain.clone()),
        skip_tag_in_url: Some(prefs.as_ref().is_some_and(|p| p.skip_tag_in_url)),
    };
    let groups = bucket_by_checksum(&rows, |code, ext| media_urls(config, &owner, code, ext).url, Some(&stats));
    Ok(DuplicateScan {
        truncated: total_groups > groups.len() as i64,
        groups,
        total_groups,
        total_files,
        reclaimable,
        dismissed_groups: (total_groups - active.groups).max(0),
        active,
        unchecksummed,
        scanned_at: JsDate::now().to_iso_string(),
    })
}

/// Ids one trash request may name.
pub const MAX_TRASH_IDS: usize = MAX_ROWS as usize;

/// `IdListIssue`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdListIssue {
    Empty,
    Malformed,
    TooMany,
}

impl IdListIssue {
    pub fn as_str(self) -> &'static str {
        match self {
            IdListIssue::Empty => "empty",
            IdListIssue::Malformed => "malformed",
            IdListIssue::TooMany => "too-many",
        }
    }
}

/// `parseUploadIds(input)`: an array of uuid strings (trimmed, lower-cased,
/// deduplicated), at most `MAX_TRASH_IDS`.
pub fn parse_upload_ids(input: Option<&Value>) -> Result<Vec<String>, IdListIssue> {
    let Some(Value::Array(items)) = input else {
        return Err(IdListIssue::Empty);
    };
    if items.is_empty() {
        return Err(IdListIssue::Empty);
    }
    if items.len() > MAX_TRASH_IDS {
        return Err(IdListIssue::TooMany);
    }
    let mut ids = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for raw in items {
        let Some(s) = raw.as_str() else {
            return Err(IdListIssue::Malformed);
        };
        let t = crate::config::js_trim(s);
        if !crate::upload::folder_target::is_uuid(t) {
            return Err(IdListIssue::Malformed);
        }
        let id = t.to_lowercase();
        if seen.insert(id.clone()) {
            ids.push(id);
        }
    }
    if ids.is_empty() {
        return Err(IdListIssue::Empty);
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    //! tests/unit/duplicates.test.ts (the pure parts)
    use serde_json::json;

    use super::*;

    fn row(id: u128, checksum: &str, size: i64, at: i64) -> DuplicateRow {
        DuplicateRow {
            id: Uuid::from_u128(id),
            code: format!("c{id}"),
            filename: format!("f{id}.png"),
            ext: "png".into(),
            mime_type: "image/png".into(),
            size,
            created_at: JsDate::from_millis(at).unwrap(),
            folder_id: None,
            folder_name: None,
            views: 0,
            downloads: 0,
            checksum: Some(checksum.into()),
            blurhash: None,
        }
    }

    #[test]
    fn buckets_keep_the_oldest_and_order_by_saving() {
        let rows =
            vec![row(2, "b", 10, 5), row(1, "b", 10, 1), row(3, "a", 100, 1), row(4, "a", 100, 2), row(5, "c", 1, 1)];
        let groups = bucket_by_checksum(&rows, |code, ext| format!("/t/{code}.{ext}"), None);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].checksum, "a");
        assert_eq!(groups[0].reclaimable, 100);
        assert_eq!(groups[1].files[0].id, Uuid::from_u128(1));
        assert_eq!(groups[1].files[0].url, "/t/c1.png");
        let mut stats = IndexMap::new();
        stats.insert("a".to_string(), GroupStat { copies: 2, bytes: 200, dismissed: true });
        let groups = bucket_by_checksum(&rows, |c, _| c.to_string(), Some(&stats));
        assert_eq!(groups.last().unwrap().checksum, "a");
    }

    #[test]
    fn reminder_and_ids() {
        assert!(!reminder_due(&DuplicateTotals { groups: 1, redundant: 10, reclaimable: REMINDER_MIN_BYTES }));
        assert!(reminder_due(&DuplicateTotals { groups: 1, redundant: 11, reclaimable: 0 }));
        assert!(is_snoozed(Some("9999999999999"), 1));
        assert!(!is_snoozed(Some("abc"), 1));
        assert!(!is_snoozed(Some(""), 1));
        let a = "9D2E1A54-0000-4000-8000-000000000001";
        assert_eq!(parse_upload_ids(Some(&json!([a, a.to_lowercase()]))).unwrap(), vec![a.to_lowercase()]);
        assert_eq!(parse_upload_ids(Some(&json!([]))), Err(IdListIssue::Empty));
        assert_eq!(parse_upload_ids(Some(&json!(["x"]))), Err(IdListIssue::Malformed));
        assert_eq!(parse_upload_ids(Some(&json!(null))), Err(IdListIssue::Empty));
    }
}
