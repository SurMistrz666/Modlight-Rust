//! Port of `lib/upload/duplicate-lookup.ts`: the queries behind "do I already
//! have this file?". Every query folds ownership into the WHERE clause: a
//! checksum is a global fact about bytes, and a public host must never answer
//! "does anybody here hold this file".

use indexmap::IndexMap;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::types::JsDate;

/// Hard ceiling on rows fetched for one request (whole checksums are cut off
/// the TAIL, and the caller is told).
pub const MAX_MATCH_ROWS: i64 = 2000;

/// A live upload of the caller's that matched (`ChecksumRow`).
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct ChecksumRow {
    pub id: Uuid,
    pub code: String,
    pub filename: String,
    pub ext: String,
    pub mime_type: String,
    pub size: i64,
    pub created_at: JsDate,
    pub folder_id: Option<Uuid>,
    pub checksum: Option<String>,
}

const ROW_FIELDS: &str =
    r#""id", "code", "filename", "ext", "mime_type", "size", "created_at", "folder_id", "checksum""#;

/// `ChecksumMatches`: keyed by checksum (oldest copy first), plus the ceiling bit.
#[derive(Debug, Clone, Default)]
pub struct ChecksumMatches {
    pub by_checksum: IndexMap<String, Vec<ChecksumRow>>,
    pub truncated: bool,
}

/// Which of these checksums does `user_id` already hold, live? Trashed files
/// are not duplicates (they are about to stop existing).
pub async fn matches_by_checksum(db: &PgPool, user_id: &str, checksums: &[String]) -> sqlx::Result<ChecksumMatches> {
    let mut out = ChecksumMatches::default();
    if checksums.is_empty() {
        return Ok(out);
    }
    let rows: Vec<ChecksumRow> = sqlx::query_as(&format!(
        r#"select {ROW_FIELDS} from "uploads"
           where ("uploads"."owner_id" = $1 and "uploads"."deleted_at" is null and "uploads"."checksum" = any($2))
           order by "uploads"."checksum" asc, "uploads"."created_at" asc, "uploads"."id" asc limit {MAX_MATCH_ROWS}"#
    ))
    .bind(user_id)
    .bind(checksums)
    .fetch_all(db)
    .await?;
    out.truncated = rows.len() as i64 >= MAX_MATCH_ROWS;
    for row in rows {
        let Some(checksum) = row.checksum.clone().filter(|c| !c.is_empty()) else {
            continue;
        };
        out.by_checksum.entry(checksum).or_default().push(row);
    }
    Ok(out)
}

/// The oldest live file of `user_id`'s carrying exactly these bytes. For the
/// ShareX path, where it can only ever be a REPORT, never a refusal.
pub async fn first_match_for(db: &PgPool, user_id: &str, checksum: &str) -> sqlx::Result<Option<ChecksumRow>> {
    sqlx::query_as(&format!(
        r#"select {ROW_FIELDS} from "uploads"
           where ("uploads"."owner_id" = $1 and "uploads"."deleted_at" is null and "uploads"."checksum" = $2)
           order by "uploads"."created_at" asc, "uploads"."id" asc limit 1"#
    ))
    .bind(user_id)
    .bind(checksum)
    .fetch_optional(db)
    .await
}

/// Live files of this account that carry no checksum, and are therefore
/// invisible to every lookup above.
pub async fn count_unchecksummed(db: &PgPool, user_id: &str) -> sqlx::Result<i64> {
    let n: i32 = sqlx::query_scalar(
        r#"select count(*)::int from "uploads"
           where ("uploads"."owner_id" = $1 and "uploads"."deleted_at" is null
                  and ("uploads"."checksum" is null or "uploads"."checksum" = ''))"#,
    )
    .bind(user_id)
    .fetch_one(db)
    .await?;
    Ok(i64::from(n))
}
