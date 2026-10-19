//! Port of `lib/data/uploads.ts` (the parts the panel shell and the profile
//! page need: `kindOf` without an extension, `kindCounts`, `userStats`). The
//! listing and the rest belong to the library port.

use indexmap::IndexMap;
use sqlx::PgPool;

/// `FILE_KINDS` (lib/data/upload-query.ts), in upstream order.
pub const FILE_KINDS: &[&str] = &["image", "video", "audio", "document", "archive", "paste", "other"];

const ARCHIVE_MIMES: &[&str] = &[
    "application/zip",
    "application/x-7z-compressed",
    "application/vnd.rar",
    "application/x-rar-compressed",
    "application/gzip",
    "application/x-tar",
];

const DOCUMENT_MIMES: &[&str] = &[
    "application/pdf",
    "application/msword",
    "application/vnd.ms-excel",
    "application/vnd.ms-powerpoint",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
];

/// `normalizeMime` (lib/media/sniff.ts): strip parameters and casing.
fn normalize_mime(mime: &str) -> String {
    crate::config::js_trim(mime.split(';').next().unwrap_or("")).to_lowercase()
}

/// `kindOf(mimeType, kind)` with NO extension, the way `kindCounts` calls it:
/// `effectiveType(mime)` then returns the normalised stored type whatever it
/// says (the extension lookup has nothing to look up).
pub fn kind_of_stored(mime_type: &str, kind: &str) -> &'static str {
    if kind == "paste" {
        return "paste";
    }
    let m = normalize_mime(mime_type);
    if m.starts_with("image/") {
        "image"
    } else if m.starts_with("video/") {
        "video"
    } else if m.starts_with("audio/") {
        "audio"
    } else if m.starts_with("text/") || DOCUMENT_MIMES.contains(&m.as_str()) {
        "document"
    } else if ARCHIVE_MIMES.contains(&m.as_str()) {
        "archive"
    } else {
        "other"
    }
}

/// `userStats(userId)`: live (not trashed) uploads of one account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserStats {
    pub count: i64,
    pub total_size: i64,
    pub total_views: i64,
}

pub async fn user_stats(db: &PgPool, user_id: &str) -> sqlx::Result<UserStats> {
    let (count, total_size, total_views): (i32, i64, i32) = sqlx::query_as(
        r#"SELECT count(*)::int, coalesce(sum("size"),0)::bigint, coalesce(sum("views"),0)::int
           FROM "uploads" WHERE "owner_id" = $1 AND "deleted_at" IS NULL"#,
    )
    .bind(user_id)
    .fetch_one(db)
    .await?;
    Ok(UserStats { count: i64::from(count), total_size, total_views: i64::from(total_views) })
}

/// Per-kind counts for the sidebar's smart views. One pass over the table.
///
/// No extension fallback here, deliberately: these counts label the
/// sidebar's smart views, and the views themselves are filtered by SQL over
/// mime_type alone. A count that disagreed with the list it opens would be
/// worse than a count that is occasionally conservative.
pub async fn kind_counts(db: &PgPool, user_id: &str) -> sqlx::Result<IndexMap<&'static str, i64>> {
    let rows: Vec<(String, String, i32)> = sqlx::query_as(
        r#"SELECT "mime_type", "kind", count(*)::int FROM "uploads"
           WHERE "owner_id" = $1 AND "deleted_at" IS NULL GROUP BY "mime_type", "kind""#,
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;
    let mut out: IndexMap<&'static str, i64> = FILE_KINDS.iter().map(|k| (*k, 0)).collect();
    for (mime, kind, n) in rows {
        *out.entry(kind_of_stored(&mime, &kind)).or_insert(0) += i64::from(n);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_without_extension() {
        assert_eq!(kind_of_stored("Image/PNG; x=1", "file"), "image");
        assert_eq!(kind_of_stored("text/plain", "paste"), "paste");
        assert_eq!(kind_of_stored("application/pdf", "file"), "document");
        assert_eq!(kind_of_stored("text/csv", "file"), "document");
        assert_eq!(kind_of_stored("application/zip", "file"), "archive");
        assert_eq!(kind_of_stored("application/octet-stream", "file"), "other");
        assert_eq!(kind_of_stored("", "file"), "other");
    }
}
