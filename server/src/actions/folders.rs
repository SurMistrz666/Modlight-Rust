//! Port of `app/actions/folders.ts`: the panel's half of the folder surface.
//!
//! Every rule (the depth ceiling, slug uniqueness, ownership, cycle
//! prevention, the two delete modes) lives in [`crate::folders`], which
//! `/api/folders` calls too. What stays here is the session (`requireUser`)
//! and `revalidatePath`.
//!
//! Arguments reach the core with upstream's JavaScript semantics: an id that
//! is not a string is "Folder not found" (`getOwnedFolder` checks
//! `typeof id`), a falsy non-null parent reaches Postgres and fails there.

use serde_json::{Value, json};

use crate::folders::{
    FolderError, count_unfiled_uploads_for, create_folder_for, delete_folder_for, get_owned_folder, move_folder_for,
    move_unfiled_to_folder_for, move_uploads_to_folder_for, rename_folder_for, set_folder_visibility_for,
};
use crate::js_value::{is_nullish, prop, truthy};
use crate::rpc::{ActionCtx, ActionError, ActionResult, RpcRegistry};

pub fn register(reg: &mut RpcRegistry) {
    reg.register("folders", "createFolder", create_folder);
    reg.register("folders", "renameFolder", rename_folder);
    reg.register("folders", "deleteFolder", delete_folder);
    reg.register("folders", "moveFolder", move_folder);
    reg.register("folders", "setFolderVisibility", set_folder_visibility);
    reg.register("folders", "moveUploadsToFolder", move_uploads_to_folder);
    reg.register("folders", "countUnfiledUploads", count_unfiled_uploads);
    reg.register("folders", "moveUnfiledToFolder", move_unfiled_to_folder);
    reg.register("folders", "listFolders", list_folders);
}

impl From<FolderError> for ActionError {
    fn from(e: FolderError) -> Self {
        match e {
            FolderError::Op(op) => ActionError::Thrown(op.message),
            FolderError::Other(err) => ActionError::Internal(err),
        }
    }
}

/// An id argument as `getOwnedFolder` sees it: anything but a string is
/// refused like an unknown id, so it is handed on as "" (not a uuid).
fn id_arg(v: Option<&Value>) -> &str {
    v.and_then(Value::as_str).unwrap_or("")
}

/// A parent argument (`string | null`): nullish is "no parent"; a truthy
/// non-string fails the ownership check; a falsy non-string (`false`, `0`)
/// reaches the uuid column upstream and Postgres refuses it.
fn parent_arg(v: Option<&Value>) -> Result<Option<String>, ActionError> {
    match v {
        _ if is_nullish(v) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        _ if truthy(v) => Err(ActionError::thrown("Folder not found")),
        _ => Err(ActionError::thrown("invalid input syntax for type uuid")),
    }
}

async fn create_folder(ctx: ActionCtx) -> ActionResult {
    let s = ctx.require_user().await?;
    let parent = parent_arg(ctx.args.raw(1))?;
    let folder = create_folder_for(&ctx.state.db, &s.user.id, ctx.args.raw(0), parent.as_deref()).await?;
    ctx.revalidate_path("/dashboard");
    crate::rpc::ok(&folder)
}

async fn rename_folder(ctx: ActionCtx) -> ActionResult {
    let s = ctx.require_user().await?;
    rename_folder_for(&ctx.state.db, &s.user.id, id_arg(ctx.args.raw(0)), ctx.args.raw(1)).await?;
    ctx.revalidate_path("/dashboard");
    crate::rpc::void()
}

/// `deleteFolder(id, opts = {})`: `opts.withContents` picks the mode; a
/// `null` opts is a TypeError once the folder was found.
async fn delete_folder(ctx: ActionCtx) -> ActionResult {
    let s = ctx.require_user().await?;
    let id = id_arg(ctx.args.raw(0));
    let opts = ctx.args.raw(1);
    if matches!(opts, Some(Value::Null)) {
        get_owned_folder(&ctx.state.db, &s.user.id, id).await?;
        return Err(ActionError::thrown("Cannot read properties of null (reading 'withContents')"));
    }
    let with_contents = truthy(prop(opts, "withContents"));
    let storage = ctx.state.storage.get().ok();
    let result = delete_folder_for(&ctx.state.db, storage.as_deref(), &s.user.id, id, with_contents).await?;
    ctx.revalidate_path("/dashboard");
    crate::rpc::ok(&result)
}

async fn move_folder(ctx: ActionCtx) -> ActionResult {
    let s = ctx.require_user().await?;
    let id = id_arg(ctx.args.raw(0));
    if ctx.args.is_undefined(1) {
        // Upstream with `newParentId` undefined: no ownership, cycle or depth
        // refusal applies, `freeSlug` compares `parent_id = NULL` (no clash
        // ever, so the bare base slug), and drizzle drops the undefined
        // `parentId` from the SET: only the slug is written, which a sibling
        // root folder holding it refuses through the unique index.
        let f = get_owned_folder(&ctx.state.db, &s.user.id, id).await?;
        let base = crate::naming::slugify_name(&f.name);
        let slug = if base.is_empty() { "folder".to_string() } else { base };
        sqlx::query(r#"update "folders" set "slug" = $1 where ("folders"."id" = $2 and "folders"."owner_id" = $3)"#)
            .bind(&slug)
            .bind(f.id)
            .bind(&s.user.id)
            .execute(&ctx.state.db)
            .await?;
    } else {
        let parent = parent_arg(ctx.args.raw(1))?;
        move_folder_for(&ctx.state.db, &s.user.id, id, parent.as_deref()).await?;
    }
    ctx.revalidate_path("/dashboard");
    crate::rpc::void()
}

/// What Postgres' `boolin` makes of a value bound to the boolean column
/// (drizzle passes `isPublic` through untyped).
fn pg_bool(v: Option<&Value>) -> Option<bool> {
    let text = match v? {
        Value::Bool(b) => return Some(*b),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let t = text.trim().to_ascii_lowercase();
    if t.is_empty() {
        return None;
    }
    // Unique prefixes are accepted, except "o" (on/off).
    let matches = |word: &str| word.starts_with(&t) && (t.len() >= 2 || !word.starts_with('o'));
    if ["t", "true", "y", "yes", "on", "1"].iter().any(|w| matches(w)) {
        return Some(true);
    }
    if ["f", "false", "n", "no", "off", "0"].iter().any(|w| matches(w)) {
        return Some(false);
    }
    None
}

async fn set_folder_visibility(ctx: ActionCtx) -> ActionResult {
    let s = ctx.require_user().await?;
    let id = id_arg(ctx.args.raw(0));
    let Some(is_public) = pg_bool(ctx.args.raw(1)) else {
        // undefined: drizzle has no column to set; null: NOT NULL; anything
        // else: not a boolean literal. All throw, after the ownership check.
        get_owned_folder(&ctx.state.db, &s.user.id, id).await?;
        return Err(ActionError::thrown("Invalid value for is_public"));
    };
    set_folder_visibility_for(&ctx.state.db, &s.user.id, id, is_public).await?;
    ctx.revalidate_path("/dashboard");
    crate::rpc::void()
}

async fn move_uploads_to_folder(ctx: ActionCtx) -> ActionResult {
    let s = ctx.require_user().await?;
    // `uploadIds.filter(...)`: anything but an array is a TypeError.
    let Some(Value::Array(ids)) = ctx.args.raw(0) else {
        return Err(ActionError::thrown("uploadIds.filter is not a function"));
    };
    // No usable id answers `{moved: 0}` before the folder is even looked at.
    let any_id = ids.iter().any(|v| v.as_str().is_some_and(crate::upload::folder_target::is_uuid));
    let moved = if !any_id {
        0
    } else if ctx.args.is_undefined(1) {
        // `set({ folderId: undefined, updatedAt })`: drizzle drops an
        // undefined column, so only `updated_at` is written.
        touch_uploads(&ctx.state.db, &s.user.id, ids).await?
    } else {
        let folder = parent_arg(ctx.args.raw(1))?;
        move_uploads_to_folder_for(&ctx.state.db, &s.user.id, ids, folder.as_deref()).await?
    };
    ctx.revalidate_path("/dashboard");
    Ok(json!({ "moved": moved }))
}

/// `moveUploadsToFolderFor` with an undefined folder: the same id filter and
/// ownership scope, and an UPDATE that only stamps `updated_at`.
async fn touch_uploads(db: &sqlx::PgPool, user_id: &str, upload_ids: &[Value]) -> Result<i64, ActionError> {
    let ids: Vec<uuid::Uuid> = upload_ids
        .iter()
        .filter_map(Value::as_str)
        .filter(|s| crate::upload::folder_target::is_uuid(s))
        .take(crate::folders::MOVE_UPLOADS_MAX)
        .filter_map(|s| uuid::Uuid::parse_str(s).ok())
        .collect();
    let moved: Vec<uuid::Uuid> = sqlx::query_scalar(
        r#"update "uploads" set "updated_at" = $1 where ("uploads"."id" = any($2) and "uploads"."owner_id" = $3) returning "id""#,
    )
    .bind(crate::db::types::JsDate::now())
    .bind(&ids)
    .bind(user_id)
    .fetch_all(db)
    .await?;
    Ok(moved.len() as i64)
}

async fn count_unfiled_uploads(ctx: ActionCtx) -> ActionResult {
    let s = ctx.require_user().await?;
    Ok(json!(count_unfiled_uploads_for(&ctx.state.db, &s.user.id).await?))
}

/// `moveUnfiledToFolder(folderName = "Wszystko")`
async fn move_unfiled_to_folder(ctx: ActionCtx) -> ActionResult {
    let s = ctx.require_user().await?;
    let name = if ctx.args.is_undefined(0) { None } else { ctx.args.raw(0) };
    let result = move_unfiled_to_folder_for(&ctx.state.db, &s.user.id, name).await?;
    ctx.revalidate_path("/dashboard");
    crate::rpc::ok(&result)
}

/// No ORDER BY, as upstream: rows come back in whatever order Postgres finds them.
///
/// `count` is 0 for every folder, as upstream answers it: drizzle renders the
/// columns of a single-table select's SQL fields WITHOUT their table
/// (`buildSelection(.., { isSingleTable: true })`), so the subquery upstream
/// meant as "live files in this folder" reaches Postgres as
/// `where "folder_id" = "id"`, and inside it both names resolve to `uploads`
/// (a file whose folder is itself). Verified on the reference; kept 1:1.
async fn list_folders(ctx: ActionCtx) -> ActionResult {
    let s = ctx.require_user().await?;
    let rows: Vec<(uuid::Uuid, Option<uuid::Uuid>, String, String, bool, i32)> = sqlx::query_as(
        r#"select "id", "parent_id", "name", "slug", "is_public", (
             select count(*)::int from "uploads"
             where "folder_id" = "id" and "deleted_at" is null
           ) from "folders" where "folders"."owner_id" = $1"#,
    )
    .bind(&s.user.id)
    .fetch_all(&ctx.state.db)
    .await?;
    let out: Vec<Value> = rows
        .into_iter()
        .map(|(id, parent_id, name, slug, is_public, count)| {
            json!({ "id": id, "parentId": parent_id, "name": name, "slug": slug, "isPublic": is_public, "count": count })
        })
        .collect();
    Ok(Value::Array(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boolean_literals_as_postgres_reads_them() {
        for (v, want) in [
            (json!(true), Some(true)),
            (json!(false), Some(false)),
            (json!("yes"), Some(true)),
            (json!(" TRUE "), Some(true)),
            (json!("of"), Some(false)),
            (json!("o"), None),
            (json!("2"), None),
            (json!(1), Some(true)),
            (json!(0), Some(false)),
            (json!(null), None),
            (json!([]), None),
        ] {
            assert_eq!(pg_bool(Some(&v)), want, "{v}");
        }
        assert_eq!(pg_bool(None), None);
    }

    #[test]
    fn parents() {
        assert_eq!(parent_arg(None).unwrap(), None);
        assert_eq!(parent_arg(Some(&json!({"$undefined": 1}))).unwrap(), None);
        assert_eq!(parent_arg(Some(&json!("x"))).unwrap().as_deref(), Some("x"));
        assert!(matches!(parent_arg(Some(&json!(5))), Err(ActionError::Thrown(m)) if m == "Folder not found"));
        assert!(parent_arg(Some(&json!(false))).is_err());
    }
}
