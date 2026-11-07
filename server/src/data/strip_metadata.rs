//! Not upstream (SKELETON.md 9.47): the account's "Remove metadata from files
//! on upload" switch (`user.strip_metadata`) and the files that kept their
//! metadata anyway (`uploads.metadata_kept`). Read and written by the Settings
//! card (RPC module `strip-metadata`), `GET`/`PUT /api/strip-metadata` and the
//! Settings page's `stripMetadata` prop.

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::types::JsDate;

/// How many of the files that kept their metadata the card lists.
pub const KEPT_LISTED: i64 = 20;

/// The formats the switch cleans, as the card and the docs list them.
pub const FORMATS: &[&str] = &[
    "JPEG", "PNG", "WebP", "GIF", "HEIC", "AVIF", "TIFF", "MP4", "MOV", "M4V", "MKV", "WebM", "AVI", "MP3", "M4A",
    "FLAC", "OGG", "Opus", "WAV", "PDF", "DOCX", "XLSX", "PPTX", "ODT", "ODS", "ODP",
];

/// One file uploaded with the switch on whose metadata could not be removed.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeptFile {
    pub id: Uuid,
    pub code: String,
    pub filename: String,
    pub ext: String,
    /// `media::strip::KeptReason::as_str`.
    pub reason: String,
    /// `toISOString()`.
    pub created_at: String,
}

/// What the Settings card shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StripStatus {
    pub enabled: bool,
    /// Live files that kept their metadata, newest first ([`KEPT_LISTED`] at most).
    pub kept: Vec<KeptFile>,
    /// How many there are in all.
    pub kept_total: i64,
    pub formats: Vec<&'static str>,
}

/// The switch; `None` when the account is gone.
pub async fn read_switch(db: &PgPool, user_id: &str) -> sqlx::Result<Option<bool>> {
    sqlx::query_scalar(r#"select "strip_metadata" from "user" where "user"."id" = $1 limit 1"#)
        .bind(user_id)
        .fetch_optional(db)
        .await
}

/// Set the switch and read it back; `None` when the account is gone. Files
/// already uploaded are not touched.
pub async fn write_switch(db: &PgPool, user_id: &str, enabled: bool) -> sqlx::Result<Option<bool>> {
    sqlx::query_scalar(
        r#"update "user" set "strip_metadata" = $1, "updated_at" = $2 where "user"."id" = $3 returning "strip_metadata""#,
    )
    .bind(enabled)
    .bind(JsDate::now())
    .bind(user_id)
    .fetch_optional(db)
    .await
}

/// The card's status.
pub async fn strip_status(db: &PgPool, user_id: &str) -> sqlx::Result<StripStatus> {
    let enabled = read_switch(db, user_id).await?.unwrap_or(false);
    let rows: Vec<(Uuid, String, String, String, String, JsDate)> = sqlx::query_as(
        r#"select "id", "code", "filename", "ext", "metadata_kept", "created_at" from "uploads"
           where ("uploads"."owner_id" = $1 and "uploads"."deleted_at" is null and "uploads"."metadata_kept" is not null)
           order by "uploads"."created_at" desc, "uploads"."id" desc limit $2"#,
    )
    .bind(user_id)
    .bind(KEPT_LISTED)
    .fetch_all(db)
    .await?;
    let kept_total: i64 = if (rows.len() as i64) < KEPT_LISTED {
        rows.len() as i64
    } else {
        sqlx::query_scalar(
            r#"select count(*) from "uploads"
               where ("uploads"."owner_id" = $1 and "uploads"."deleted_at" is null and "uploads"."metadata_kept" is not null)"#,
        )
        .bind(user_id)
        .fetch_one(db)
        .await?
    };
    let kept = rows
        .into_iter()
        .map(|(id, code, filename, ext, reason, created)| KeptFile {
            id,
            code,
            filename,
            ext,
            reason,
            created_at: created.to_iso_string(),
        })
        .collect();
    Ok(StripStatus { enabled, kept, kept_total, formats: FORMATS.to_vec() })
}
