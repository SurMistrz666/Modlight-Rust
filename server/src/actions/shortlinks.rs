//! Port of `app/actions/shortlinks.ts`: the panel's entry points into short
//! links. The rules live in [`crate::shortlinks`] because `POST
//! /api/shortlinks` enforces the same ones for the desktop client; what stays
//! here is the session and revalidating the page that lists them.

use serde_json::{Value, json};

use crate::js_value::{is_nullish, prop};
use crate::rpc::{ActionCtx, ActionError, ActionResult, RpcRegistry};
use crate::shortlinks::{CreateShortLinkInput, CreateShortLinkResult, create_short_link_for, normalize_target};

pub fn register(reg: &mut RpcRegistry) {
    reg.register("shortlinks", "createShortLink", create_short_link);
    reg.register("shortlinks", "retargetShortLink", retarget_short_link);
    reg.register("shortlinks", "deleteShortLink", delete_short_link);
    reg.register("shortlinks", "listShortLinks", list_short_links);
}

/// The client's object as the core reads it (`input.target`, ...). Reading a
/// property of `null`/`undefined` is a TypeError; any other value just has
/// no such properties.
fn create_input(input: Option<&Value>) -> Result<CreateShortLinkInput, ActionError> {
    if is_nullish(input) {
        return Err(ActionError::thrown("Cannot read properties of null (reading 'target')"));
    }
    let field = |k: &str| prop(input, k).cloned();
    Ok(CreateShortLinkInput {
        target: field("target").unwrap_or_else(crate::rpc::tagged::undefined),
        custom_code: field("customCode"),
        expires_in_days: field("expiresInDays"),
        password: field("password"),
        use_user_code: field("useUserCode"),
    })
}

async fn create_short_link(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let input = create_input(ctx.args.raw(0))?;
    let result = create_short_link_for(&ctx.state.db, &ctx.state.config, &session.user.id, &input).await?;
    if matches!(result, CreateShortLinkResult::Ok { .. }) {
        ctx.revalidate_path("/dashboard/links");
    }
    Ok(result.to_json())
}

/// `normalizeTarget(target)` on a raw argument: `raw.length` on a nullish
/// value and `raw.trim()` on a non-string are TypeErrors.
fn target_arg(v: Option<&Value>) -> Result<Option<String>, ActionError> {
    match v {
        Some(Value::String(s)) => Ok(normalize_target(s)),
        _ if is_nullish(v) => Err(ActionError::thrown("Cannot read properties of undefined (reading 'length')")),
        _ => Err(ActionError::thrown("raw.trim is not a function")),
    }
}

/// A short-link id bound to the uuid column as drizzle would bind it: a
/// string as is (Postgres refuses a malformed one), null as NULL (matches
/// nothing), anything else refused by Postgres too.
fn id_arg(v: Option<&Value>) -> Result<Option<String>, ActionError> {
    match v {
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => Ok(Some(n.to_string())),
        _ => Err(ActionError::thrown("invalid input syntax for type uuid")),
    }
}

/// Point an existing short link somewhere else, keeping its code (a release
/// channel's `/s/{tag}/manifest` must outlive the file it currently names).
/// The target goes through the same `normalizeTarget` as creation, so a
/// `javascript:` URL cannot come in through the back door of an edit, and
/// ownership is folded into the UPDATE's WHERE.
async fn retarget_short_link(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let Some(normalized) = target_arg(ctx.args.raw(1))? else {
        return Ok(json!({ "ok": false, "reason": "invalid-url" }));
    };
    let id = id_arg(ctx.args.raw(0))?;
    let row: Option<String> = sqlx::query_scalar(
        r#"update "short_links" set "target" = $1 where ("short_links"."id" = $2::uuid and "short_links"."owner_id" = $3) returning "target""#,
    )
    .bind(&normalized)
    .bind(id)
    .bind(&session.user.id)
    .fetch_optional(&ctx.state.db)
    .await?;
    let Some(target) = row else {
        return Ok(json!({ "ok": false, "reason": "not-found" }));
    };
    ctx.revalidate_path("/dashboard/links");
    Ok(json!({ "ok": true, "target": target }))
}

async fn delete_short_link(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let id = id_arg(ctx.args.raw(0))?;
    sqlx::query(r#"delete from "short_links" where ("short_links"."id" = $1::uuid and "short_links"."owner_id" = $2)"#)
        .bind(id)
        .bind(&session.user.id)
        .execute(&ctx.state.db)
        .await?;
    ctx.revalidate_path("/dashboard/links");
    crate::rpc::void()
}

/// Full rows, newest first, expired links included (dates as `Date`s).
async fn list_short_links(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    crate::rpc::ok(&crate::shortlinks::list_short_links(&ctx.state.db, &session.user.id).await?)
}
