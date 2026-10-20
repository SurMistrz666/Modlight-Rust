//! Port of `app/actions/convert.ts`: converting files from the panel - one
//! file, or a whole selection.
//!
//! Nothing but identifiers crosses this boundary - an upload id and a target
//! name - so the 1 MB body limit an action carries never enters into it,
//! however large the file being converted is. The bytes are read from storage
//! and written back to storage entirely on the server (`convert_in_place`).
//!
//! These are WRITES, and destructive ones in `replace` mode, so every entry
//! point resolves the session itself.

use serde_json::{Value, json};

use crate::db::types::JsDate;
use crate::media::convert_in_place::{ConvertActor, ConvertInPlaceInput, convert_upload_in_place};
use crate::media::convert_plan::{
    ConvertCandidate, ConvertFailure, ConvertMode, bulk_target_summary, convert_targets_for_row,
};
use crate::rpc::tagged;
use crate::rpc::{ActionCtx, ActionError, ActionResult, RpcRegistry, ok};

pub fn register(reg: &mut RpcRegistry) {
    reg.register("convert", "convertTargetsForUpload", convert_targets_for_upload);
    reg.register("convert", "convertTargetsForSelection", convert_targets_for_selection);
    reg.register("convert", "convertUpload", convert_upload);
    reg.register("convert", "convertUploads", convert_uploads);
}

/// How many files one call may convert. The limiter admits two transforms at
/// a time across the whole server and an encode may take a minute, so a
/// selection of a hundred would be a request that cannot finish.
const MAX_BATCH: usize = 25;

fn unauthorized() -> ActionError {
    ActionError::thrown("Unauthorized")
}

/// `[...new Set(value)]`: an array's items, a string's characters, nothing
/// for null/undefined; anything else is not iterable (a TypeError upstream).
fn unique(value: Option<&Value>) -> Result<Vec<Value>, ActionError> {
    let items: Vec<Value> = match value {
        None => Vec::new(),
        Some(v) if v.is_null() || tagged::is_undefined(v) => Vec::new(),
        Some(Value::Array(items)) => items.clone(),
        Some(Value::String(s)) => s.chars().map(|c| Value::String(c.to_string())).collect(),
        Some(_) => return Err(ActionError::thrown("ids is not iterable")),
    };
    let mut out: Vec<Value> = Vec::new();
    for item in items {
        if !out.contains(&item) {
            out.push(item);
        }
    }
    Ok(out)
}

/// An id as the database will see it: a string, or a failure Postgres would
/// have answered with ("invalid input syntax for type uuid").
fn id_string(v: &Value) -> Result<String, ActionError> {
    match v {
        Value::String(s) => Ok(s.clone()),
        other => Err(ActionError::Internal(anyhow::anyhow!(
            "invalid input syntax for type uuid: \"{}\"",
            crate::js_value::js_string(other)
        ))),
    }
}

/// `loadOwned(ids)`: the rows the caller may act on - their own, or anybody's
/// for an admin. Repeated here so the MENU cannot advertise targets for a file
/// the caller is not allowed to touch.
async fn load_owned(ctx: &ActionCtx, ids: Option<&Value>) -> Result<Vec<ConvertCandidate>, ActionError> {
    let Some(session) = ctx.get_session().await? else {
        return Err(unauthorized());
    };
    let unique: Vec<Value> = unique(ids)?.into_iter().filter(|v| crate::js_value::truthy(Some(v))).take(500).collect();
    if unique.is_empty() {
        return Ok(Vec::new());
    }
    let ids = unique.iter().map(id_string).collect::<Result<Vec<_>, _>>()?;
    // `inArray(uploads.id, ids)`: the same `in ($1, $2, ...)` drizzle writes,
    // so the rows come back in the order Postgres picks for that statement.
    // `$n::uuid`: node-postgres sends the values untyped and Postgres infers
    // uuid from the column, so a malformed id fails the same way.
    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("${i}::uuid")).collect();
    let sql = format!(
        r#"select "id", "owner_id", "ext", "mime_type", "frames", "width", "height", "size", "kind", "deleted_at", "expires_at"
           from "uploads" where "uploads"."id" in ({})"#,
        placeholders.join(", ")
    );
    type Row = (
        uuid::Uuid,
        Option<String>,
        String,
        String,
        Option<i32>,
        Option<i32>,
        Option<i32>,
        i64,
        String,
        Option<JsDate>,
        Option<JsDate>,
    );
    let mut query = sqlx::query_as::<_, Row>(&sql);
    for id in &ids {
        query = query.bind(id);
    }
    let rows = query.fetch_all(&ctx.state.db).await.map_err(|e| ActionError::Internal(e.into()))?;
    let is_admin = session.user.role.as_deref() == Some("admin");
    Ok(rows
        .into_iter()
        .filter(|r| is_admin || r.1.as_deref() == Some(session.user.id.as_str()))
        .map(|(id, _owner, ext, mime, frames, width, height, size, kind, deleted_at, expires_at)| ConvertCandidate {
            id: id.to_string(),
            ext: Some(ext),
            mime_type: Some(mime),
            frames: frames.map(f64::from),
            width: width.map(f64::from),
            height: height.map(f64::from),
            size: size as f64,
            kind,
            deleted_at,
            expires_at,
        })
        .collect())
}

/// `convertTargetsForUpload(id)`: targets for one file, in the same order
/// the "Download as..." menu uses.
async fn convert_targets_for_upload(ctx: ActionCtx) -> ActionResult {
    let id = ctx.args.raw(0).cloned().unwrap_or_else(tagged::undefined);
    let rows = load_owned(&ctx, Some(&Value::Array(vec![id]))).await?;
    match rows.first() {
        Some(row) => ok(&convert_targets_for_row(row)),
        None => ok(&Vec::<Value>::new()),
    }
}

/// `convertTargetsForSelection(ids, mode = "replace")`: targets for a
/// SELECTION, each carrying how many of the selected files it applies to.
async fn convert_targets_for_selection(ctx: ActionCtx) -> ActionResult {
    let mode = if ctx.args.is_undefined(1) { ConvertMode::Replace } else { mode_of(ctx.args.raw(1)) };
    let rows = load_owned(&ctx, ctx.args.raw(0)).await?;
    ok(&bulk_target_summary(&rows, mode, JsDate::now()))
}

fn mode_of(v: Option<&Value>) -> ConvertMode {
    match v {
        Some(Value::String(s)) => ConvertMode::from_js(s),
        _ => ConvertMode::Other,
    }
}

/// The target name as `planConversion` reads it: `(to ?? "")`, and a value
/// that is not a string has no `.replace` (a TypeError upstream).
fn target_of(v: Option<&Value>) -> Result<String, ActionError> {
    match v {
        None => Ok(String::new()),
        Some(Value::String(s)) => Ok(s.clone()),
        Some(v) if v.is_null() || tagged::is_undefined(v) => Ok(String::new()),
        Some(_) => Err(ActionError::thrown("(ext ?? \"\").replace is not a function")),
    }
}

/// One conversion. A null or undefined id finds no row (`eq(id, null)`), so it
/// is reported as not found with the id echoed as it came.
async fn convert_one(
    ctx: &ActionCtx,
    id: &Value,
    to: &str,
    mode: ConvertMode,
    actor: &ConvertActor,
) -> Result<Value, ActionError> {
    if id.is_null() || tagged::is_undefined(id) {
        let outcome = tagged_json_outcome_not_found(id);
        return Ok(outcome);
    }
    let upload_id = id_string(id)?;
    let outcome = convert_upload_in_place(
        &ctx.state,
        ConvertInPlaceInput { upload_id, to: to.to_string(), mode, actor: actor.clone(), signal: None },
    )
    .await
    .map_err(ActionError::Internal)?;
    tagged::to_tagged_value(&outcome).map_err(|e| ActionError::Internal(e.into()))
}

fn tagged_json_outcome_not_found(id: &Value) -> Value {
    json!({ "status": "failed", "uploadId": id, "reason": ConvertFailure::NotFound.as_str() })
}

/// `convertUpload(id, to, mode)`: convert one file, replacing it or leaving
/// a copy beside it.
async fn convert_upload(ctx: ActionCtx) -> ActionResult {
    let Some(session) = ctx.get_session().await? else {
        return Err(unauthorized());
    };
    let to = target_of(ctx.args.raw(1))?;
    let mode = mode_of(ctx.args.raw(2));
    let actor = ConvertActor { id: session.user.id.clone(), role: session.user.role.clone() };
    let id = ctx.args.raw(0).cloned().unwrap_or_else(tagged::undefined);
    let outcome = convert_one(&ctx, &id, &to, mode, &actor).await?;
    after_conversion(&ctx, &session.user.id, std::slice::from_ref(&outcome), ctx.args.raw(1), mode).await;
    Ok(outcome)
}

/// `convertUploads(ids, to, mode)`: convert a selection to one target,
/// strictly one file at a time (the shared limiter has room for two
/// transforms across the entire server). Each file is its own transaction, so
/// a failure in the middle keeps everything already done.
async fn convert_uploads(ctx: ActionCtx) -> ActionResult {
    let Some(session) = ctx.get_session().await? else {
        return Err(unauthorized());
    };
    let to = target_of(ctx.args.raw(1))?;
    let mode = mode_of(ctx.args.raw(2));
    let actor = ConvertActor { id: session.user.id.clone(), role: session.user.role.clone() };
    let ids: Vec<Value> = unique(ctx.args.raw(0))?.into_iter().take(MAX_BATCH).collect();

    let mut results = Vec::new();
    for id in &ids {
        results.push(convert_one(&ctx, id, &to, mode, &actor).await?);
    }
    after_conversion(&ctx, &session.user.id, &results, ctx.args.raw(1), mode).await;

    let count =
        |status: &[&str]| results.iter().filter(|r| status.contains(&r["status"].as_str().unwrap_or(""))).count();
    let echo = |v: Option<&Value>| v.cloned().unwrap_or_else(tagged::undefined);
    Ok(json!({
        "to": echo(ctx.args.raw(1)),
        "mode": echo(ctx.args.raw(2)),
        "converted": count(&["converted", "created"]),
        "skipped": count(&["skipped"]),
        "failed": count(&["failed"]),
        "results": results,
    }))
}

/// `afterConversion(actorId, results, to, mode)`: refresh what the
/// conversion changed, and record it when somebody else's file was rewritten
/// (an admin rewriting somebody else's bytes is the one case here that
/// destroys data the actor does not own). Never fatal.
async fn after_conversion(ctx: &ActionCtx, actor_id: &str, results: &[Value], to: Option<&Value>, mode: ConvertMode) {
    let changed: Vec<&Value> =
        results.iter().filter(|r| matches!(r["status"].as_str(), Some("converted") | Some("created"))).collect();
    if changed.is_empty() {
        return;
    }
    ctx.revalidate_path("/dashboard");

    let ids: Vec<String> = changed
        .iter()
        .filter_map(|r| match r["status"].as_str() {
            Some("converted") => r["uploadId"].as_str(),
            Some("created") => r["sourceId"].as_str(),
            _ => None,
        })
        .map(str::to_string)
        .collect();
    if ids.is_empty() {
        return;
    }
    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("${i}::uuid")).collect();
    let sql =
        format!(r#"select "id", "owner_id" from "uploads" where "uploads"."id" in ({})"#, placeholders.join(", "));
    let mut query = sqlx::query_as::<_, (uuid::Uuid, Option<String>)>(&sql);
    for id in &ids {
        query = query.bind(id);
    }
    let foreign: Vec<String> = match query.fetch_all(&ctx.state.db).await {
        Ok(rows) => rows.into_iter().filter(|r| r.1.as_deref() != Some(actor_id)).map(|r| r.0.to_string()).collect(),
        Err(err) => {
            // Upstream let this one escape (it ran outside the try); the
            // conversion has happened either way.
            tracing::error!("[convert] could not read owners for the audit: {err}");
            return;
        }
    };
    if foreign.is_empty() {
        return;
    }
    let action = if mode == ConvertMode::Replace { "convert.replace" } else { "convert.copy" };
    let to = match to {
        Some(v) if !tagged::is_undefined(v) => tagged::normalize(v.clone()),
        _ => Value::Null,
    };
    let detail = if to.is_null() && !matches!(ctx.args.raw(1), Some(Value::Null)) {
        // `{ to: undefined }` is serialised without the key.
        json!({ "uploadIds": foreign })
    } else {
        json!({ "to": to, "uploadIds": foreign })
    };
    crate::audit::record_audit(&ctx.state.db, actor_id, action, detail, &ctx.client_ip()).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_is_a_set_of_what_was_sent() {
        let v = json!(["a", "b", "a", 1, 1]);
        assert_eq!(unique(Some(&v)).unwrap(), vec![json!("a"), json!("b"), json!(1)]);
        assert_eq!(unique(Some(&json!("ab"))).unwrap(), vec![json!("a"), json!("b")]);
        assert!(unique(None).unwrap().is_empty());
        assert!(unique(Some(&json!(5))).is_err());
    }

    #[test]
    fn modes_read_like_upstream() {
        assert_eq!(mode_of(Some(&json!("copy"))), ConvertMode::Copy);
        assert_eq!(mode_of(Some(&json!("replace"))), ConvertMode::Replace);
        assert_eq!(mode_of(Some(&json!("move"))), ConvertMode::Other);
        assert_eq!(mode_of(None), ConvertMode::Other);
        assert_eq!(target_of(Some(&Value::Null)).unwrap(), "");
        assert!(target_of(Some(&json!(5))).is_err());
    }
}
