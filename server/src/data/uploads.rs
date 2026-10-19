//! Port of `lib/data/uploads.ts`: listing a library, the file-view mapping,
//! and the per-account counters (`userStats`, `kindCounts`).

use indexmap::IndexMap;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

pub use super::upload_query::{DEFAULT_PER_PAGE, FILE_KINDS, MAX_PAGE, MAX_PER_PAGE, SORT_KEYS};
use crate::db::models::{Columns, JsDate, Upload};
use crate::media::mime::effective_type;

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

/// Classify a stored row for display. Mirrors the SQL `kind_filter`. The
/// extension is consulted only when the stored type says nothing.
pub fn kind_of(mime_type: &str, kind: &str, ext: Option<&str>) -> &'static str {
    if kind == "paste" {
        return "paste";
    }
    let m = effective_type(Some(mime_type), ext);
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

/// `kindOf(mime, kind)` without an extension (the sidebar counts).
pub fn kind_of_stored(mime_type: &str, kind: &str) -> &'static str {
    kind_of(mime_type, kind, None)
}

fn quoted_list(items: &[&str]) -> String {
    items.iter().map(|m| format!("'{m}'")).collect::<Vec<_>>().join(", ")
}

/// SQL predicate for a file kind (on the RAW `mime_type`).
fn kind_filter(kind: &str) -> Option<String> {
    Some(match kind {
        "image" => r#""uploads"."mime_type" like 'image/%'"#.to_string(),
        "video" => r#""uploads"."mime_type" like 'video/%'"#.to_string(),
        "audio" => r#""uploads"."mime_type" like 'audio/%'"#.to_string(),
        "document" => format!(
            r#"("uploads"."mime_type" like 'text/%' or "uploads"."mime_type" in ({}))"#,
            quoted_list(DOCUMENT_MIMES)
        ),
        "archive" => format!(r#""uploads"."mime_type" in ({})"#, quoted_list(ARCHIVE_MIMES)),
        "paste" => r#""uploads"."kind" = 'paste'"#.to_string(),
        "other" => {
            let all: Vec<&str> = DOCUMENT_MIMES.iter().chain(ARCHIVE_MIMES).copied().collect();
            format!(
                r#"("uploads"."mime_type" not like 'image/%' and "uploads"."mime_type" not like 'video/%' and "uploads"."mime_type" not like 'audio/%' and "uploads"."mime_type" not like 'text/%' and "uploads"."mime_type" not in ({}))"#,
                quoted_list(&all)
            )
        }
        _ => return None,
    })
}

/// `folderId` of `ListParams`: `undefined` (no filter), `null` (loose files)
/// or an id as the caller gave it (compared against a uuid column unchecked).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum FolderFilter {
    #[default]
    Any,
    Unfiled,
    Folder(String),
}

/// `ListParams`
#[derive(Debug, Clone, Default)]
pub struct ListParams {
    pub user_id: String,
    pub page: Option<f64>,
    pub per_page: Option<i64>,
    pub search: Option<String>,
    pub sort: Option<String>,
    pub trashed: bool,
    pub folder: FolderFilter,
    /// Include files in nested folders when a folder is selected.
    pub include_subfolders: bool,
    pub kind: Option<String>,
    /// "public" | "private"
    pub visibility: Option<String>,
    pub min_size: Option<f64>,
    pub max_size: Option<f64>,
    pub before: Option<JsDate>,
    pub after: Option<JsDate>,
    /// `mime_type like 'image/%'` only (the avatar picker's own filter).
    pub images_only: bool,
}

fn order_for(sort: &str) -> &'static str {
    match sort {
        "oldest" => r#""uploads"."created_at" asc"#,
        "largest" => r#""uploads"."size" desc"#,
        "smallest" => r#""uploads"."size" asc"#,
        "name" => r#""uploads"."filename" asc"#,
        "views" => r#""uploads"."views" desc"#,
        _ => r#""uploads"."created_at" desc"#,
    }
}

/// The page number `OFFSET` is computed from, bounded at both ends (NaN and
/// infinities are page 1).
pub fn page_of(raw: Option<f64>) -> i64 {
    match raw {
        Some(n) if n.is_finite() => n.floor().clamp(1.0, MAX_PAGE as f64) as i64,
        _ => 1,
    }
}

/// `{ items, total, page, perPage }` with the EFFECTIVE page and perPage.
#[derive(Debug, Clone)]
pub struct UploadPage {
    pub items: Vec<Upload>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
}

/// A bound parameter of the listing query.
enum Bind {
    Text(String),
    Texts(Vec<String>),
    Bool(bool),
    Num(f64),
    Date(JsDate),
}

/// `listUploads(params)`
pub async fn list_uploads(db: &PgPool, params: &ListParams) -> sqlx::Result<UploadPage> {
    let page = page_of(params.page);
    let per_page = params.per_page.unwrap_or(DEFAULT_PER_PAGE).clamp(1, MAX_PER_PAGE);
    let sort = params.sort.as_deref().unwrap_or("newest");

    let mut binds: Vec<Bind> = vec![Bind::Text(params.user_id.clone())];
    let mut clauses: Vec<String> = vec![r#""uploads"."owner_id" = $1"#.to_string()];
    clauses.push(if params.trashed {
        r#""uploads"."deleted_at" is not null"#.to_string()
    } else {
        r#""uploads"."deleted_at" is null"#.to_string()
    });
    let push = |binds: &mut Vec<Bind>, b: Bind| -> usize {
        binds.push(b);
        binds.len()
    };
    // Search covers the code as well as the filename; `%`, `_` and `\` in the
    // needle are not escaped, as upstream.
    if let Some(search) = params.search.as_deref().filter(|s| !s.is_empty()) {
        let n = push(&mut binds, Bind::Text(format!("%{search}%")));
        clauses.push(format!(r#"("uploads"."filename" ilike ${n} or "uploads"."code" ilike ${n})"#));
    }
    match &params.folder {
        FolderFilter::Any => {}
        FolderFilter::Unfiled => clauses.push(r#""uploads"."folder_id" is null"#.to_string()),
        FolderFilter::Folder(id) if id.is_empty() => {}
        FolderFilter::Folder(id) => {
            if params.include_subfolders {
                let ids = super::folders::descendant_folder_ids(db, &params.user_id, id).await?;
                let n = push(&mut binds, Bind::Texts(ids));
                clauses.push(format!(r#""uploads"."folder_id" = any(${n}::uuid[])"#));
            } else {
                let n = push(&mut binds, Bind::Text(id.clone()));
                clauses.push(format!(r#""uploads"."folder_id" = ${n}::uuid"#));
            }
        }
    }
    if let Some(filter) = params.kind.as_deref().and_then(kind_filter) {
        clauses.push(filter);
    }
    if params.images_only {
        clauses.push(r#""uploads"."mime_type" like 'image/%'"#.to_string());
    }
    match params.visibility.as_deref() {
        Some("public") => {
            let n = push(&mut binds, Bind::Bool(true));
            clauses.push(format!(r#""uploads"."published" = ${n}"#));
        }
        Some("private") => {
            let n = push(&mut binds, Bind::Bool(false));
            clauses.push(format!(r#""uploads"."published" = ${n}"#));
        }
        _ => {}
    }
    if let Some(min) = params.min_size {
        let n = push(&mut binds, Bind::Num(min));
        clauses.push(format!(r#""uploads"."size" >= ${n}"#));
    }
    if let Some(max) = params.max_size {
        let n = push(&mut binds, Bind::Num(max));
        clauses.push(format!(r#""uploads"."size" <= ${n}"#));
    }
    if let Some(after) = params.after {
        let n = push(&mut binds, Bind::Date(after));
        clauses.push(format!(r#""uploads"."created_at" >= ${n}"#));
    }
    if let Some(before) = params.before {
        let n = push(&mut binds, Bind::Date(before));
        clauses.push(format!(r#""uploads"."created_at" <= ${n}"#));
    }
    let where_clause = clauses.join(" and ");

    fn bind_all<'q, O>(
        mut q: sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments>,
        binds: &'q [Bind],
    ) -> sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments> {
        for b in binds {
            q = match b {
                Bind::Text(s) => q.bind(s),
                Bind::Texts(v) => q.bind(v),
                Bind::Bool(v) => q.bind(*v),
                Bind::Num(v) => q.bind(*v),
                Bind::Date(d) => q.bind(*d),
            };
        }
        q
    }

    let count_sql = format!(r#"select count(*)::int from "uploads" where ({where_clause})"#);
    let (total,): (i32,) = bind_all(sqlx::query_as(&count_sql), &binds).fetch_one(db).await?;
    let list_sql = format!(
        r#"select {} from "uploads" where ({where_clause}) order by {} limit {per_page} offset {}"#,
        Upload::COLUMNS,
        order_for(sort),
        (page - 1) * per_page
    );
    let items: Vec<Upload> = bind_all(sqlx::query_as(&list_sql), &binds).fetch_all(db).await?;
    Ok(UploadPage { items, total: i64::from(total), page, per_page })
}

/// `FileItem` (components/dashboard/types.ts): the plain shape the file views
/// consume. `durationSeconds`/`fps` are present only where upstream's
/// `toFileItems` adds them (the admin browser), never on the dashboard.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileItem {
    pub id: Uuid,
    pub code: String,
    pub filename: String,
    pub ext: String,
    pub mime_type: String,
    pub size: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub frames: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<Option<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<Option<f64>>,
    pub blurhash: Option<String>,
    pub published: bool,
    pub views: i32,
    pub downloads: i32,
    pub created_at: String,
    pub kind: &'static str,
    pub folder_id: Option<Uuid>,
    pub folder_name: Option<String>,
    pub expires_at: Option<String>,
    pub max_views: Option<i32>,
    pub has_password: bool,
}

/// Mapping "M" of the dashboard pages (`app/(app)/dashboard/page.tsx`).
pub fn file_item(u: &Upload, folder_name: Option<String>) -> FileItem {
    FileItem {
        id: u.id,
        code: u.code.clone(),
        filename: u.filename.clone(),
        ext: u.ext.clone(),
        mime_type: u.mime_type.clone(),
        size: u.size,
        width: u.width,
        height: u.height,
        frames: u.frames,
        duration_seconds: None,
        fps: None,
        blurhash: u.blurhash.clone(),
        published: u.published,
        views: u.views,
        downloads: u.downloads,
        created_at: u.created_at.to_iso_string(),
        kind: kind_of(&u.mime_type, &u.kind, Some(&u.ext)),
        folder_id: u.folder_id,
        folder_name: if u.folder_id.is_some() { folder_name } else { None },
        expires_at: u.expires_at.map(|d| d.to_iso_string()),
        max_views: u.max_views,
        has_password: u.password_hash.as_deref().is_some_and(|h| !h.is_empty()),
    }
}

/// `toFileItems(rows, folderNames)`: mapping M plus `durationSeconds`/`fps`.
pub fn to_file_items(rows: &[Upload], folder_names: &std::collections::HashMap<Uuid, String>) -> Vec<FileItem> {
    rows.iter()
        .map(|u| {
            let name = u.folder_id.and_then(|f| folder_names.get(&f).cloned());
            FileItem { duration_seconds: Some(u.duration_seconds), fps: Some(u.fps), ..file_item(u, name) }
        })
        .collect()
}

pub async fn get_upload(db: &PgPool, id: Uuid) -> sqlx::Result<Option<Upload>> {
    let sql = format!(r#"select {} from {} where "uploads"."id" = $1 limit 1"#, Upload::COLUMNS, Upload::TABLE);
    sqlx::query_as(&sql).bind(id).fetch_optional(db).await
}

/// `userStats(userId)`: live (not trashed) uploads of one account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserStats {
    pub count: i64,
    pub total_size: i64,
    pub total_views: i64,
}

pub async fn user_stats(db: &PgPool, user_id: &str) -> sqlx::Result<UserStats> {
    let (count, total_size, total_views): (i32, i64, i32) = sqlx::query_as(
        r#"select count(*)::int, coalesce(sum("uploads"."size"),0)::bigint, coalesce(sum("uploads"."views"),0)::int
           from "uploads" where ("uploads"."owner_id" = $1 and "uploads"."deleted_at" is null)"#,
    )
    .bind(user_id)
    .fetch_one(db)
    .await?;
    Ok(UserStats { count: i64::from(count), total_size, total_views: i64::from(total_views) })
}

/// Per-kind counts for the sidebar's smart views, all seven keys in order.
/// No extension fallback, deliberately: the views themselves are filtered by
/// SQL over mime_type alone, and a count that disagreed with the list it opens
/// would be worse than a conservative one.
pub async fn kind_counts(db: &PgPool, user_id: &str) -> sqlx::Result<IndexMap<&'static str, i64>> {
    let rows: Vec<(String, String, i32)> = sqlx::query_as(
        r#"select "mime_type", "kind", count(*)::int from "uploads"
           where ("uploads"."owner_id" = $1 and "uploads"."deleted_at" is null)
           group by "uploads"."mime_type", "uploads"."kind""#,
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
    fn kinds() {
        assert_eq!(kind_of("image/png", "file", None), "image");
        assert_eq!(kind_of("text/plain", "paste", None), "paste");
        assert_eq!(kind_of("application/octet-stream", "file", Some("avif")), "image");
        assert_eq!(kind_of_stored("application/octet-stream", "file"), "other");
        assert_eq!(kind_of("application/pdf", "file", None), "document");
        assert_eq!(kind_of("application/x-rar-compressed", "file", None), "archive");
        assert_eq!(kind_of("Text/Markdown; x", "file", None), "document");
        assert_eq!(page_of(None), 1);
        assert_eq!(page_of(Some(f64::NAN)), 1);
        assert_eq!(page_of(Some(2.7)), 2);
        assert_eq!(page_of(Some(1e30)), MAX_PAGE);
        assert!(kind_filter("other").unwrap().contains("not in ('application/pdf'"));
    }
}

#[cfg(test)]
mod srv_auth_tests {
    //! The srv-auth port's tests of its narrow version of this module, kept
    //! against the reconciled implementation.
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
