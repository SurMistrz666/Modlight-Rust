//! Port of `app/actions/avatar.ts`: setting an avatar from a file the account
//! has already uploaded (the picker), and reading the current one back.
//!
//! The chosen upload is COPIED (re-encoded to a 256 px WebP), never
//! referenced: deleting a screenshot must not blank the avatar, the panel
//! must not fetch a 12 MB source on every page, and the upload's own state
//! (unpublished, password, expiry, view budget) stays out of a decoration.

use serde::Serialize;
use serde_json::Value;

use crate::avatar::{AVATAR_MAX_BYTES, avatar_url};
use crate::avatar_store::save_avatar;
use crate::data::uploads::{ListParams, list_uploads};
use crate::media::mime::effective_type;
use crate::rpc::{ActionCtx, ActionError, ActionResult, RpcRegistry, ok, tagged};
use crate::upload::folder_target::is_uuid;

pub fn register(reg: &mut RpcRegistry) {
    reg.register("avatar", "listAvatarPicks", list_avatar_picks);
    reg.register("avatar", "setAvatarFromUpload", set_avatar_from_upload);
    reg.register("avatar", "myAvatar", my_avatar);
}

const PICKER_PER_PAGE: i64 = 24;

/// `AvatarPick`
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarPick {
    pub id: uuid::Uuid,
    pub filename: String,
    /// The dashboard's own relative `/raw?w&h` tile (relative: the session
    /// cookie is host-only).
    pub thumb_url: String,
    pub blurhash: Option<String>,
    pub size: i64,
    pub published: bool,
    pub too_large: bool,
}

/// `AvatarPickPage`
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarPickPage {
    pub items: Vec<AvatarPick>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
}

/// Images we are willing to re-encode: an image by its EFFECTIVE type (a
/// .avif stored as octet-stream counts), never SVG.
pub fn is_pickable_image(mime: Option<&str>, ext: Option<&str>) -> bool {
    let t = effective_type(mime, ext);
    t.starts_with("image/") && t != "image/svg+xml"
}

/// `params.page` / `params.search?.trim()` with JavaScript's property access
/// on whatever was sent (`undefined` is `{}`; `null` throws).
fn picker_params(raw: Option<&Value>) -> Result<(Option<f64>, Option<String>), ActionError> {
    let params = match raw {
        None => return Ok((None, None)),
        Some(v) if tagged::is_undefined(v) => return Ok((None, None)),
        Some(v) => tagged::normalize(v.clone()),
    };
    let params = match params {
        Value::Null => {
            return Err(ActionError::thrown("Cannot read properties of null (reading 'page')"));
        }
        v => v,
    };
    let page = params.get("page").and_then(|p| if p.is_number() { p.as_f64() } else { None });
    let search = match params.get("search") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(crate::config::js_trim(s).to_string()).filter(|s| !s.is_empty()),
        Some(_) => return Err(ActionError::thrown("params.search?.trim is not a function")),
    };
    Ok((page, search))
}

/// `listAvatarPicks(params?)`: the owner's own images, newest first, through
/// the same scoped listing the dashboard uses.
async fn list_avatar_picks(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let (page, search) = picker_params(ctx.args.raw(0))?;
    let db = &ctx.state.db;
    let result = list_uploads(
        db,
        &ListParams {
            user_id: session.user.id.clone(),
            page,
            per_page: Some(PICKER_PER_PAGE),
            search,
            kind: Some("image".into()),
            sort: Some("newest".into()),
            ..Default::default()
        },
    )
    .await?;
    let tag = owner_tag(db, &session.user.id).await?;
    let items = result
        .items
        .iter()
        .filter(|u| is_pickable_image(Some(&u.mime_type), Some(&u.ext)))
        .map(|u| AvatarPick {
            id: u.id,
            filename: u.filename.clone(),
            thumb_url: format!(
                "/{tag}/{}/raw{}?w=200&h=200",
                u.code,
                if u.ext.is_empty() { String::new() } else { format!(".{}", u.ext) }
            ),
            blurhash: u.blurhash.clone(),
            size: u.size,
            published: u.published,
            too_large: u.size as f64 > AVATAR_MAX_BYTES as f64,
        })
        .collect();
    ok(&AvatarPickPage { items, total: result.total, page: result.page, per_page: result.per_page })
}

/// `{ ok: false, error }`
fn refused(error: &str) -> ActionResult {
    ok(&serde_json::json!({ "ok": false, "error": error }))
}

/// `setAvatarFromUpload(uploadId)`: one of the caller's own live uploads,
/// ownership in the WHERE clause.
async fn set_avatar_from_upload(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let upload_id = ctx.args.raw(0).cloned().map(tagged::normalize);
    let Some(id) = upload_id.as_ref().and_then(Value::as_str).filter(|s| is_uuid(s)) else {
        return refused("not-found");
    };
    let db = &ctx.state.db;
    let row: Option<(String, String, String, i64)> = sqlx::query_as(
        r#"select "storage_key", "mime_type", "ext", "size" from "uploads"
           where ("uploads"."id" = $1::uuid and "uploads"."owner_id" = $2 and "uploads"."deleted_at" is null) limit 1"#,
    )
    .bind(id)
    .bind(&session.user.id)
    .fetch_optional(db)
    .await?;
    let Some((storage_key, mime, ext, size)) = row else {
        return refused("not-found");
    };
    if !is_pickable_image(Some(&mime), Some(&ext)) {
        return refused("unsupported");
    }
    if size as f64 > AVATAR_MAX_BYTES as f64 {
        return refused("too-large");
    }
    let storage = ctx.state.storage.get()?;
    let Ok(input) = storage.read_buffer(&storage_key).await else {
        // The row exists but its bytes do not.
        return refused("not-found");
    };
    match save_avatar(db, storage.as_ref(), &session.user.id, input, None).await? {
        Err(e) => refused(e.as_str()),
        Ok(saved) => {
            // <html> and the sidebar carry the avatar (root layout).
            ctx.revalidate_path("/");
            ok(&serde_json::json!({
                "ok": true,
                "url": saved.url,
                "animated": saved.animated,
                "degraded": saved.degraded.map(|d| d.as_str()),
            }))
        }
    }
}

/// `myAvatar()`: the current URL, read back rather than guessed.
async fn my_avatar(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let row: Option<(Option<String>, Option<String>)> =
        sqlx::query_as(r#"select "user_code", "image" from "user" where "user"."id" = $1 limit 1"#)
            .bind(&session.user.id)
            .fetch_optional(&ctx.state.db)
            .await?;
    let (code, image) = row.unwrap_or((None, None));
    ok(&serde_json::json!({ "url": avatar_url(code.as_deref(), image.as_deref()) }))
}

/// The tag the owner's own links are built on.
async fn owner_tag(db: &sqlx::PgPool, user_id: &str) -> sqlx::Result<String> {
    let row: Option<Option<String>> =
        sqlx::query_scalar(r#"select "user_code" from "user" where "user"."id" = $1 limit 1"#)
            .bind(user_id)
            .fetch_optional(db)
            .await?;
    Ok(row.flatten().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pickable_images() {
        assert!(is_pickable_image(Some("image/png"), Some("png")));
        assert!(is_pickable_image(Some("application/octet-stream"), Some("avif")));
        assert!(!is_pickable_image(Some("image/svg+xml"), Some("svg")));
        assert!(!is_pickable_image(Some("video/mp4"), Some("mp4")));
    }

    #[test]
    fn picker_params_follow_property_access() {
        assert_eq!(picker_params(None).unwrap(), (None, None));
        assert!(picker_params(Some(&Value::Null)).is_err());
        let v = serde_json::json!({ "page": 2, "search": "  cat " });
        assert_eq!(picker_params(Some(&v)).unwrap(), (Some(2.0), Some("cat".into())));
        let v = serde_json::json!({ "page": "2", "search": "   " });
        assert_eq!(picker_params(Some(&v)).unwrap(), (None, None));
        assert!(picker_params(Some(&serde_json::json!({ "search": 5 }))).is_err());
    }
}
