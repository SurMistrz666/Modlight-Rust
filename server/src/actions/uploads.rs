//! Port of `app/actions/uploads.ts`: the panel's half of the upload surface.
//!
//! Every rule (the vanity slug and its clash check, `passwordVersion` cleared
//! with a per-file password, the view-cap floor, the audit entry an admin's
//! touch leaves) lives in [`crate::uploads`], which `/api/uploads` calls too.
//! What stays here is the session and `revalidatePath`.
//!
//! Every action is session-or-throw: no session throws `Unauthorized` (masked
//! in production), never a redirect. The actor is `{userId, email, isAdmin}`;
//! `isAdmin` is what lets the admin file browser touch another account's file,
//! and the audit entry in the core is what makes that leave a trace.

use serde_json::{Map, Value, json};

use crate::auth::session::AuthSession;
use crate::media::actions::{DeletableMedia, delete_media_hard, restore_media, soft_delete_media_lazy};
use crate::rpc::{ActionCtx, ActionError, ActionResult, RpcRegistry};
use crate::uploads::{
    UploadActor, UploadError, audit_foreign_upload, empty_trash_for, get_actionable_upload, set_expiry_for,
    set_max_views_for, set_password_for, set_published_for, set_vanity_for, trash_totals_for,
};

pub fn register(reg: &mut RpcRegistry) {
    reg.register("uploads", "togglePublish", toggle_publish);
    reg.register("uploads", "trashUpload", trash_upload);
    reg.register("uploads", "restoreUpload", restore_upload);
    reg.register("uploads", "deleteUploadForever", delete_upload_forever);
    reg.register("uploads", "trashSummary", trash_summary);
    reg.register("uploads", "emptyTrash", empty_trash);
    reg.register("uploads", "setVanity", set_vanity);
    reg.register("uploads", "setUploadExpiry", set_upload_expiry);
    reg.register("uploads", "setUploadMaxViews", set_upload_max_views);
    reg.register("uploads", "setUploadPassword", set_upload_password);
}

impl From<UploadError> for ActionError {
    fn from(e: UploadError) -> Self {
        match e {
            // `UploadOpError` is an Error with its message: a plain throw.
            UploadError::Op(op) => ActionError::Thrown(op.message),
            UploadError::Other(err) => ActionError::Internal(err),
        }
    }
}

/// The session, or a refusal (`throw new Error("Unauthorized")`).
async fn current_session(ctx: &ActionCtx) -> Result<AuthSession, ActionError> {
    ctx.get_session().await?.ok_or_else(|| ActionError::thrown("Unauthorized"))
}

fn to_actor(ctx: &ActionCtx, session: &AuthSession) -> UploadActor {
    UploadActor {
        user_id: session.user.id.clone(),
        email: Some(session.user.email.clone()),
        is_admin: session.is_admin(),
        ip: ctx.client_ip(),
    }
}

async fn actor(ctx: &ActionCtx) -> Result<UploadActor, ActionError> {
    let session = current_session(ctx).await?;
    Ok(to_actor(ctx, &session))
}

/// Whose trash a trash-wide call is about. The tag comes from the CALLER (the
/// owner's own page and an administrator's view of somebody else's render the
/// same list); omitted means "my own", which is what the sidebar shortcut asks
/// for. Whether this actor may reach that tag is the core's decision.
fn trash_target(session: &AuthSession, owner_tag: Option<&Value>) -> String {
    let asked = match owner_tag {
        Some(Value::String(s)) => crate::config::js_trim(s).to_string(),
        _ => String::new(),
    };
    if asked.is_empty() { session.user.user_code.clone().unwrap_or_default() } else { asked }
}

/// `[id]`: the core takes a list of raw ids.
fn one(ctx: &ActionCtx) -> Vec<Value> {
    vec![ctx.args.raw(0).cloned().unwrap_or(Value::Null)]
}

async fn toggle_publish(ctx: ActionCtx) -> ActionResult {
    let a = actor(&ctx).await?;
    // Read first: this is a TOGGLE, so the new value is a function of the old one.
    let media = get_actionable_upload(&ctx.state.db, &a, ctx.args.raw(0)).await?;
    let published = !media.published;
    set_published_for(&ctx.state.db, &a, &one(&ctx), published).await?;
    ctx.revalidate_path("/dashboard");
    Ok(json!({ "published": published }))
}

async fn trash_upload(ctx: ActionCtx) -> ActionResult {
    let a = actor(&ctx).await?;
    let media = get_actionable_upload(&ctx.state.db, &a, ctx.args.raw(0)).await?;
    soft_delete_media_lazy(&ctx.state.db, &ctx.state.storage, media.id).await?;
    audit_foreign_upload(&ctx.state.db, &a, &media, "admin.upload.trashed", Map::new()).await;
    ctx.revalidate_path("/dashboard");
    crate::rpc::void()
}

async fn restore_upload(ctx: ActionCtx) -> ActionResult {
    let a = actor(&ctx).await?;
    let media = get_actionable_upload(&ctx.state.db, &a, ctx.args.raw(0)).await?;
    restore_media(&ctx.state.db, media.id).await?;
    audit_foreign_upload(&ctx.state.db, &a, &media, "admin.upload.restored", Map::new()).await;
    ctx.revalidate_path("/dashboard");
    ctx.revalidate_path("/dashboard/trash");
    crate::rpc::void()
}

async fn delete_upload_forever(ctx: ActionCtx) -> ActionResult {
    let a = actor(&ctx).await?;
    let media = get_actionable_upload(&ctx.state.db, &a, ctx.args.raw(0)).await?;
    // Trash rows only: the only screen that offers this is the trash, so a
    // live id reaching it is a stale selection or a hand-made call.
    if media.deleted_at.is_none() {
        return Err(ActionError::thrown("Not found"));
    }
    let storage = ctx.state.storage.get()?;
    delete_media_hard(&ctx.state.db, storage.as_ref(), &DeletableMedia::from(&media)).await?;
    let mut detail = Map::new();
    detail.insert("size".into(), json!(media.size));
    audit_foreign_upload(&ctx.state.db, &a, &media, "admin.upload.deleted", detail).await;
    ctx.revalidate_path("/dashboard");
    ctx.revalidate_path("/dashboard/trash");
    crate::rpc::void()
}

/// What emptying a trash would destroy, counted right now over the WHOLE
/// trash (the view lists 120 at a time; a confirmation that understates what
/// it destroys is worse than none).
async fn trash_summary(ctx: ActionCtx) -> ActionResult {
    let session = current_session(&ctx).await?;
    let target = trash_target(&session, ctx.args.raw(0));
    let totals = trash_totals_for(&ctx.state.db, &to_actor(&ctx, &session), &target).await?;
    Ok(json!({ "files": totals.files, "bytes": totals.bytes }))
}

/// Empty a trash right now instead of waiting out the 30-day sweep. `failed`
/// is the rows storage would not give up: they stay in the trash for a retry.
async fn empty_trash(ctx: ActionCtx) -> ActionResult {
    let session = current_session(&ctx).await?;
    let target = trash_target(&session, ctx.args.raw(0));
    let actor = to_actor(&ctx, &session);
    // An unbuildable storage driver throws here even for an empty trash
    // (upstream's `getStorage()` ran inside the first non-empty batch): a
    // misconfigured install fails either way.
    let storage = ctx.state.storage.get()?;
    let result = empty_trash_for(&ctx.state.db, storage.as_ref(), &actor, &target).await?;
    ctx.revalidate_path("/dashboard");
    ctx.revalidate_path("/dashboard/trash");
    // `deleted` rather than `files`: the sidebar shortcut reads that name.
    Ok(json!({ "deleted": result.files, "bytes": result.bytes, "failed": result.failed }))
}

async fn set_vanity(ctx: ActionCtx) -> ActionResult {
    let a = actor(&ctx).await?;
    let media = set_vanity_for(&ctx.state.db, &a, ctx.args.raw(0), ctx.args.raw(1)).await?;
    ctx.revalidate_path("/dashboard");
    Ok(json!({ "code": media.code }))
}

/// Set or clear a time-based expiry (`iso` = an ISO date string, or null).
async fn set_upload_expiry(ctx: ActionCtx) -> ActionResult {
    let a = actor(&ctx).await?;
    set_expiry_for(&ctx.state.db, &a, &one(&ctx), ctx.args.raw(1)).await?;
    ctx.revalidate_path("/dashboard");
    crate::rpc::void()
}

/// Set a view cap (burn after N views), or null to clear. 1 = one-time link.
async fn set_upload_max_views(ctx: ActionCtx) -> ActionResult {
    let a = actor(&ctx).await?;
    set_max_views_for(&ctx.state.db, &a, &one(&ctx), ctx.args.raw(1)).await?;
    ctx.revalidate_path("/dashboard");
    crate::rpc::void()
}

/// Set or clear a password (scrypt-hashed); null or empty clears it.
async fn set_upload_password(ctx: ActionCtx) -> ActionResult {
    let a = actor(&ctx).await?;
    set_password_for(&ctx.state.db, &a, &one(&ctx), ctx.args.raw(1)).await?;
    ctx.revalidate_path("/dashboard");
    crate::rpc::void()
}
