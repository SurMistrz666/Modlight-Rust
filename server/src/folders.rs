//! Port of `lib/folders.ts`: folder operations, with no idea who is logged in.
//!
//! Each function takes the owner's id instead of calling `requireUser()`, so
//! the panel's server actions and `app/api/folders/route.ts` run the same code.
//! The rules that must not diverge are the depth ceiling, slug uniqueness,
//! cycle prevention, the two delete modes, and above all OWNERSHIP - which is
//! folded into every WHERE clause, never checked after a row comes back.
//!
//! Ids the upstream code passes to Postgres unchecked are bound as text and
//! cast (`$1::uuid`), so a malformed one fails the same way it did upstream.

use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use crate::data::folders::{descendant_folder_ids, would_create_cycle};
use crate::db::models::{Columns, Folder, JsDate};
use crate::naming::{js_slice, slugify_name};
use crate::storage::StorageDriver;
use crate::upload::folder_target::is_uuid;

/// How deep folders may nest. Depth 1 is a root folder.
pub const MAX_FOLDER_DEPTH: usize = 8;
/// Longest folder name we store (`folders.name` is varchar(120)).
pub const FOLDER_NAME_MAX: usize = 120;
/// Ceiling on one "move these files" call.
pub const MOVE_UPLOADS_MAX: usize = 1000;

/// Why an operation was refused (`FolderIssue`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderIssue {
    NotFound,
    InvalidName,
    TooDeep,
    Cycle,
}

impl FolderIssue {
    pub fn as_str(self) -> &'static str {
        match self {
            FolderIssue::NotFound => "not-found",
            FolderIssue::InvalidName => "invalid-name",
            FolderIssue::TooDeep => "too-deep",
            FolderIssue::Cycle => "cycle",
        }
    }
}

/// `FolderOpError`: a refusal with a reason (mapped to a status by the HTTP
/// surface) and the message the panel shows.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct FolderOpError {
    pub reason: FolderIssue,
    pub message: String,
}

fn not_found() -> FolderOpError {
    FolderOpError { reason: FolderIssue::NotFound, message: "Folder not found".into() }
}

fn too_deep() -> FolderOpError {
    FolderOpError {
        reason: FolderIssue::TooDeep,
        message: format!("Folders can be nested at most {MAX_FOLDER_DEPTH} levels deep"),
    }
}

/// A refusal or an unexpected failure (database, storage).
#[derive(Debug, thiserror::Error)]
pub enum FolderError {
    #[error(transparent)]
    Op(#[from] FolderOpError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<sqlx::Error> for FolderError {
    fn from(e: sqlx::Error) -> Self {
        FolderError::Other(e.into())
    }
}

pub type FolderResult<T> = Result<T, FolderError>;

/// `FolderRecord = typeof folders.$inferSelect`
pub type FolderRecord = Folder;

fn select_folder(where_clause: &str) -> String {
    format!("select {} from {} where {where_clause}", Folder::COLUMNS, Folder::TABLE)
}

/// Fetch a folder the caller owns, or refuse. Somebody else's id, an id that
/// never existed and a string that is not a uuid at all produce the SAME
/// answer (no oracle for "is this id real, and whose is it").
pub async fn get_owned_folder(db: &PgPool, user_id: &str, id: &str) -> FolderResult<FolderRecord> {
    if !is_uuid(id) {
        return Err(not_found().into());
    }
    let sql = select_folder(r#"("folders"."id" = $1::uuid and "folders"."owner_id" = $2) limit 1"#);
    let row: Option<Folder> = sqlx::query_as(&sql).bind(id).bind(user_id).fetch_optional(db).await?;
    row.ok_or_else(|| not_found().into())
}

/// Slugs are unique per (owner, parent): pick a friendly free slug instead of
/// surfacing a constraint violation. Does NOT exclude the folder itself
/// (renaming to the same name moves the slug to `-2`, as upstream).
async fn free_slug(db: &PgPool, user_id: &str, parent_id: Option<&str>, name: &str) -> FolderResult<String> {
    let base = {
        let s = slugify_name(name);
        if s.is_empty() { "folder".to_string() } else { s }
    };
    let mut slug = base.clone();
    let mut i: u32 = 2;
    loop {
        let clash: Option<Uuid> = match parent_id {
            None => {
                sqlx::query_scalar(
                    r#"select "id" from "folders" where ("folders"."owner_id" = $1 and "folders"."parent_id" is null and "folders"."slug" = $2) limit 1"#,
                )
                .bind(user_id)
                .bind(&slug)
                .fetch_optional(db)
                .await?
            }
            Some(parent) => {
                sqlx::query_scalar(
                    r#"select "id" from "folders" where ("folders"."owner_id" = $1 and "folders"."parent_id" = $2::uuid and "folders"."slug" = $3) limit 1"#,
                )
                .bind(user_id)
                .bind(parent)
                .bind(&slug)
                .fetch_optional(db)
                .await?
            }
        };
        if clash.is_none() {
            return Ok(slug);
        }
        slug = format!("{base}-{i}");
        if i > 500 {
            return Ok(format!("{base}-{}", JsDate::now().millis()));
        }
        i += 1;
    }
}

/// How many levels down `folder_id` sits. 0 for the root (no folder).
async fn depth_of(db: &PgPool, user_id: &str, folder_id: Option<&str>) -> FolderResult<usize> {
    let Some(folder_id) = folder_id.filter(|f| !f.is_empty()) else {
        return Ok(0);
    };
    let rows: Vec<(Uuid, Option<Uuid>)> =
        sqlx::query_as(r#"select "id", "parent_id" from "folders" where "folders"."owner_id" = $1"#)
            .bind(user_id)
            .fetch_all(db)
            .await?;
    let by_id: HashMap<String, Option<Uuid>> = rows.iter().map(|(id, p)| (id.to_string(), *p)).collect();
    let mut depth = 0;
    let mut seen = std::collections::HashSet::new();
    let mut cur = by_id.get_key_value(folder_id).map(|(k, v)| (k.clone(), *v));
    while let Some((id, parent)) = cur {
        if !seen.insert(id) {
            break;
        }
        depth += 1;
        cur = parent.and_then(|p| by_id.get_key_value(&p.to_string()).map(|(k, v)| (k.clone(), *v)));
    }
    Ok(depth)
}

/// Trim and bound a name the way `folders.name` needs it. `name` is the raw
/// argument: anything but a string counts as blank.
pub fn clean_name(name: Option<&serde_json::Value>, fallback: Option<&str>) -> Result<String, FolderOpError> {
    let trimmed = match name {
        Some(serde_json::Value::String(s)) => crate::config::js_trim(s).to_string(),
        _ => String::new(),
    };
    if trimmed.is_empty() {
        return match fallback {
            None => {
                Err(FolderOpError { reason: FolderIssue::InvalidName, message: "Folder name cannot be empty".into() })
            }
            Some(f) => Ok(f.to_string()),
        };
    }
    Ok(js_slice(&trimmed, FOLDER_NAME_MAX).to_string())
}

/// `createFolderFor(userId, name, parentId = null)`. A truthy parent must be
/// owned and shallower than the ceiling; an empty-string parent is not
/// normalised to null (Postgres refuses it, as upstream).
pub async fn create_folder_for(
    db: &PgPool,
    user_id: &str,
    name: Option<&serde_json::Value>,
    parent_id: Option<&str>,
) -> FolderResult<FolderRecord> {
    if let Some(parent) = parent_id.filter(|p| !p.is_empty()) {
        get_owned_folder(db, user_id, parent).await?;
        if depth_of(db, user_id, Some(parent)).await? >= MAX_FOLDER_DEPTH {
            return Err(too_deep().into());
        }
    }
    let clean = clean_name(name, Some("Folder"))?;
    let slug = free_slug(db, user_id, parent_id, &clean).await?;
    let sql = format!(
        r#"insert into "folders" ("owner_id", "parent_id", "name", "slug") values ($1, $2::uuid, $3, $4) returning {}"#,
        Folder::COLUMNS
    );
    Ok(sqlx::query_as(&sql).bind(user_id).bind(parent_id).bind(&clean).bind(&slug).fetch_one(db).await?)
}

pub async fn rename_folder_for(
    db: &PgPool,
    user_id: &str,
    id: &str,
    name: Option<&serde_json::Value>,
) -> FolderResult<FolderRecord> {
    let f = get_owned_folder(db, user_id, id).await?;
    // No fallback: renaming to nothing is a mistake.
    let clean = clean_name(name, None)?;
    let parent = f.parent_id.map(|p| p.to_string());
    let slug = free_slug(db, user_id, parent.as_deref(), &clean).await?;
    let sql = format!(
        r#"update "folders" set "name" = $1, "slug" = $2 where ("folders"."id" = $3::uuid and "folders"."owner_id" = $4) returning {}"#,
        Folder::COLUMNS
    );
    let row: Option<Folder> =
        sqlx::query_as(&sql).bind(&clean).bind(&slug).bind(id).bind(user_id).fetch_optional(db).await?;
    Ok(row.unwrap_or(Folder { name: clean, slug, ..f }))
}

/// `DeleteFolderResult`
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct DeleteFolderResult {
    /// Folders removed (1 for a plain delete, the whole subtree otherwise).
    pub folders: i64,
    /// Files moved to the trash. Always 0 unless `with_contents` was set.
    pub trashed: i64,
}

/// Delete a folder, in one of two modes. Default ("folder only"): files fall
/// back to "no folder" and child folders are re-parented. `with_contents`: the
/// folder and its whole subtree go, and every live file inside lands in the
/// trash (soft-delete; bytes are reclaimed by the trash sweep 30 days later).
pub async fn delete_folder_for(
    db: &PgPool,
    storage: Option<&dyn StorageDriver>,
    user_id: &str,
    id: &str,
    with_contents: bool,
) -> FolderResult<DeleteFolderResult> {
    let f = get_owned_folder(db, user_id, id).await?;
    if !with_contents {
        let mut tx = db.begin().await?;
        sqlx::query(
            r#"update "folders" set "parent_id" = $1 where ("folders"."parent_id" = $2 and "folders"."owner_id" = $3)"#,
        )
        .bind(f.parent_id)
        .bind(f.id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(r#"update "uploads" set "folder_id" = null where ("uploads"."folder_id" = $1 and "uploads"."owner_id" = $2)"#)
            .bind(f.id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(r#"delete from "folders" where ("folders"."id" = $1 and "folders"."owner_id" = $2)"#)
            .bind(f.id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok(DeleteFolderResult { folders: 1, trashed: 0 });
    }
    // descendant_folder_ids only walks rows belonging to this user.
    let subtree: Vec<Uuid> =
        descendant_folder_ids(db, user_id, id).await?.iter().filter_map(|s| Uuid::parse_str(s).ok()).collect();
    let now = JsDate::now();
    let mut tx = db.begin().await?;
    let trashed: Vec<Uuid> = sqlx::query_scalar(
        r#"select "id" from "uploads" where ("uploads"."owner_id" = $1 and "uploads"."folder_id" = any($2) and "uploads"."deleted_at" is null)"#,
    )
    .bind(user_id)
    .bind(&subtree)
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query(
        r#"update "uploads" set "deleted_at" = $1, "updated_at" = $2 where ("uploads"."owner_id" = $3 and "uploads"."folder_id" = any($4) and "uploads"."deleted_at" is null)"#,
    )
    .bind(now)
    .bind(now)
    .bind(user_id)
    .bind(&subtree)
    .execute(&mut *tx)
    .await?;
    sqlx::query(r#"delete from "folders" where ("folders"."owner_id" = $1 and "folders"."id" = any($2))"#)
        .bind(user_id)
        .bind(&subtree)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    // Outside the transaction: storage is not transactional.
    if let Some(storage) = storage {
        for upload_id in &trashed {
            crate::media::derived_cache::purge_derived(storage, &upload_id.to_string()).await;
        }
    }
    Ok(DeleteFolderResult { folders: subtree.len() as i64, trashed: trashed.len() as i64 })
}

pub async fn move_folder_for(
    db: &PgPool,
    user_id: &str,
    id: &str,
    new_parent_id: Option<&str>,
) -> FolderResult<FolderRecord> {
    let f = get_owned_folder(db, user_id, id).await?;
    if let Some(parent) = new_parent_id.filter(|p| !p.is_empty()) {
        get_owned_folder(db, user_id, parent).await?;
    }
    if would_create_cycle(db, user_id, id, new_parent_id).await? {
        return Err(FolderOpError {
            reason: FolderIssue::Cycle,
            message: "A folder cannot be moved inside itself".into(),
        }
        .into());
    }
    if depth_of(db, user_id, new_parent_id).await? >= MAX_FOLDER_DEPTH {
        return Err(too_deep().into());
    }
    let slug = free_slug(db, user_id, new_parent_id, &f.name).await?;
    let sql = format!(
        r#"update "folders" set "parent_id" = $1::uuid, "slug" = $2 where ("folders"."id" = $3::uuid and "folders"."owner_id" = $4) returning {}"#,
        Folder::COLUMNS
    );
    let row: Option<Folder> =
        sqlx::query_as(&sql).bind(new_parent_id).bind(&slug).bind(id).bind(user_id).fetch_optional(db).await?;
    let parent_uuid = new_parent_id.and_then(|p| Uuid::parse_str(p).ok());
    Ok(row.unwrap_or(Folder { parent_id: parent_uuid, slug, ..f }))
}

pub async fn set_folder_visibility_for(
    db: &PgPool,
    user_id: &str,
    id: &str,
    is_public: bool,
) -> FolderResult<FolderRecord> {
    let f = get_owned_folder(db, user_id, id).await?;
    let sql = format!(
        r#"update "folders" set "is_public" = $1 where ("folders"."id" = $2::uuid and "folders"."owner_id" = $3) returning {}"#,
        Folder::COLUMNS
    );
    let row: Option<Folder> = sqlx::query_as(&sql).bind(is_public).bind(id).bind(user_id).fetch_optional(db).await?;
    Ok(row.unwrap_or(Folder { is_public, ..f }))
}

/// Move files into a folder (or out of every folder when `folder_id` is
/// `None`). Ownership is enforced inside the UPDATE; non-uuid ids are dropped
/// before the query; trashed rows move too.
pub async fn move_uploads_to_folder_for(
    db: &PgPool,
    user_id: &str,
    upload_ids: &[serde_json::Value],
    folder_id: Option<&str>,
) -> FolderResult<i64> {
    let ids: Vec<Uuid> = upload_ids
        .iter()
        .filter_map(|v| v.as_str())
        .filter(|s| is_uuid(s))
        .take(MOVE_UPLOADS_MAX)
        .filter_map(|s| Uuid::parse_str(s).ok())
        .collect();
    if ids.is_empty() {
        return Ok(0);
    }
    if let Some(folder) = folder_id.filter(|f| !f.is_empty()) {
        get_owned_folder(db, user_id, folder).await?;
    }
    let moved: Vec<Uuid> = sqlx::query_scalar(
        r#"update "uploads" set "folder_id" = $1::uuid, "updated_at" = $2 where ("uploads"."id" = any($3) and "uploads"."owner_id" = $4) returning "id""#,
    )
    .bind(folder_id)
    .bind(JsDate::now())
    .bind(&ids)
    .bind(user_id)
    .fetch_all(db)
    .await?;
    Ok(moved.len() as i64)
}

/// Files that belong to no folder at all (and aren't in the trash).
const UNFILED_SCOPE: &str =
    r#"("uploads"."owner_id" = $1 and "uploads"."folder_id" is null and "uploads"."deleted_at" is null)"#;

pub async fn count_unfiled_uploads_for(db: &PgPool, user_id: &str) -> FolderResult<i64> {
    let n: i32 = sqlx::query_scalar(&format!(r#"select count(*)::int from "uploads" where {UNFILED_SCOPE}"#))
        .bind(user_id)
        .fetch_one(db)
        .await?;
    Ok(i64::from(n))
}

/// `{ moved, folderId, folderName }`
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveUnfiledResult {
    pub moved: i64,
    pub folder_id: Uuid,
    pub folder_name: String,
}

/// Sweep every loose file into one folder, creating it if needed: a single
/// UPDATE over the whole set, reusing an existing top-level folder with the
/// same slug instead of creating a twin on the second run.
pub async fn move_unfiled_to_folder_for(
    db: &PgPool,
    user_id: &str,
    folder_name: Option<&serde_json::Value>,
) -> FolderResult<MoveUnfiledResult> {
    let default = serde_json::Value::String("Wszystko".into());
    let clean = clean_name(Some(folder_name.unwrap_or(&default)), Some("Wszystko"))?;
    let slug = {
        let s = slugify_name(&clean);
        if s.is_empty() { "folder".to_string() } else { s }
    };
    let sql = select_folder(
        r#"("folders"."owner_id" = $1 and "folders"."parent_id" is null and "folders"."slug" = $2) limit 1"#,
    );
    let existing: Option<Folder> = sqlx::query_as(&sql).bind(user_id).bind(&slug).fetch_optional(db).await?;
    let target = match existing {
        Some(f) => f,
        None => {
            let sql = format!(
                r#"insert into "folders" ("owner_id", "parent_id", "name", "slug") values ($1, null, $2, $3) returning {}"#,
                Folder::COLUMNS
            );
            sqlx::query_as(&sql).bind(user_id).bind(&clean).bind(&slug).fetch_one(db).await?
        }
    };
    let moved = count_unfiled_uploads_for(db, user_id).await?;
    if moved > 0 {
        sqlx::query(&format!(r#"update "uploads" set "folder_id" = $2, "updated_at" = $3 where {UNFILED_SCOPE}"#))
            .bind(user_id)
            .bind(target.id)
            .bind(JsDate::now())
            .execute(db)
            .await?;
    }
    Ok(MoveUnfiledResult { moved, folder_id: target.id, folder_name: target.name })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn clean_names() {
        assert_eq!(clean_name(Some(&json!("  Docs  ")), Some("Folder")).unwrap(), "Docs");
        assert_eq!(clean_name(Some(&json!("   ")), Some("Folder")).unwrap(), "Folder");
        assert_eq!(clean_name(Some(&json!(5)), Some("Folder")).unwrap(), "Folder");
        assert_eq!(clean_name(None, Some("Folder")).unwrap(), "Folder");
        assert_eq!(clean_name(Some(&json!("")), None).unwrap_err().reason, FolderIssue::InvalidName);
        assert_eq!(clean_name(Some(&json!("x".repeat(200))), None).unwrap().len(), 120);
    }
}
