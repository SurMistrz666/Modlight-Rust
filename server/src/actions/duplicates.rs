//! Port of `app/actions/duplicates.ts`: retroactive duplicate cleanup.
//!
//! Every action answers with DATA, never a throw: Next masks a thrown
//! message in production, and this screen's whole job is saying precisely
//! what it is about to delete and why it refused. No session is
//! `{ok: false, error: "unauthorized"}`.

use serde_json::{Value, json};

use crate::actions::js_value::{prop, to_number};
use crate::data::duplicates::{
    DismissalInput, dismiss_checksums, parse_upload_ids, restore_checksums, scan_duplicates,
};
use crate::db::types::JsDate;
use crate::media::derived_cache::purge_derived_lazy;
use crate::rpc::{ActionCtx, ActionResult, RpcRegistry};
use crate::upload::duplicate_check::parse_checksums;

pub fn register(reg: &mut RpcRegistry) {
    reg.register("duplicates", "scanMyDuplicates", scan_my_duplicates);
    reg.register("duplicates", "trashDuplicates", trash_duplicates);
    reg.register("duplicates", "keepDuplicateSets", keep_duplicate_sets);
    reg.register("duplicates", "countDuplicateSetsAgain", count_duplicate_sets_again);
}

/// How many storage prefixes are cleared at a time (as lib/media/actions.ts).
const PURGE_CONCURRENCY: usize = 16;
/// Ids kept in the audit entry. The count is the fact; the list is a courtesy.
const AUDIT_ID_SAMPLE: usize = 50;
/// Checksums named in one audit entry.
const AUDIT_CHECKSUM_SAMPLE: usize = 20;

fn refusal(error: &str) -> ActionResult {
    Ok(json!({ "ok": false, "error": error }))
}

/// Read-only scan of the signed-in account. Never touches another account.
async fn scan_my_duplicates(ctx: ActionCtx) -> ActionResult {
    let Some(session) = ctx.get_session().await? else {
        return refusal("unauthorized");
    };
    match scan_duplicates(&ctx.state.db, &ctx.state.config, &session.user.id).await {
        Ok(scan) => Ok(json!({ "ok": true, "scan": serde_json::to_value(&scan)? })),
        Err(err) => {
            tracing::error!(error = ?err, "[duplicates] scan failed");
            refusal("failed")
        }
    }
}

/// Move a caller-supplied list of uploads to the trash: a soft delete, never
/// a hard one. Ownership lives in the WHERE clause of ONE statement, and its
/// RETURNING is the authoritative record of what happened (the byte total
/// included), not what the client believed it was selecting.
async fn trash_duplicates(ctx: ActionCtx) -> ActionResult {
    let Some(session) = ctx.get_session().await? else {
        return refusal("unauthorized");
    };
    let ids = match parse_upload_ids(ctx.args.raw(0)) {
        Ok(ids) => ids,
        Err(issue) => return refusal(issue.as_str()),
    };
    let user_id = &session.user.id;

    let trashed: Vec<(uuid::Uuid, i64, String)> = match sqlx::query_as(
        r#"update "uploads" set "deleted_at" = $1
           where ("uploads"."id" = any($2::uuid[]) and "uploads"."owner_id" = $3 and "uploads"."deleted_at" is null)
           returning "id", "size", "code""#,
    )
    .bind(JsDate::now())
    .bind(&ids)
    .bind(user_id)
    .fetch_all(&ctx.state.db)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(error = ?err, "[duplicates] trash failed");
            return refusal("failed");
        }
    };

    let bytes: i64 = trashed.iter().map(|(_, size, _)| *size).sum();

    // Cached conversions are pure derivatives of a file the owner no longer
    // wants served, so they go now; best-effort, never a failed delete.
    let storage = &ctx.state.storage;
    for slice in trashed.chunks(PURGE_CONCURRENCY) {
        futures_util::future::join_all(
            slice.iter().map(|(id, _, _)| async move { purge_derived_lazy(storage, &id.to_string()).await }),
        )
        .await;
    }

    if !trashed.is_empty() {
        let codes: Vec<&str> = trashed.iter().take(AUDIT_ID_SAMPLE).map(|(_, _, code)| code.as_str()).collect();
        crate::audit::record_audit(
            &ctx.state.db,
            user_id,
            "uploads.duplicates_trashed",
            json!({ "count": trashed.len(), "bytes": bytes, "requested": ids.len(), "codes": codes }),
            &ctx.client_ip(),
        )
        .await;
    }

    ctx.revalidate_path("/dashboard");
    ctx.revalidate_path("/dashboard/trash");
    ctx.revalidate_path("/dashboard/profile");
    ctx.revalidate_path("/dashboard/duplicates");
    // The reminder strip lives in the panel shell.
    ctx.revalidate_path("/dashboard");

    Ok(json!({
        "ok": true,
        "trashed": trashed.len(),
        "bytes": bytes,
        "skipped": ids.len() as i64 - trashed.len() as i64,
    }))
}

/// `typeof set?.checksum === "string" ? set.checksum.trim().toLowerCase() : ""`
fn normalised_checksum(set: &Value) -> String {
    match prop(Some(set), "checksum") {
        Some(Value::String(s)) => crate::config::js_trim(s).to_lowercase(),
        _ => String::new(),
    }
}

/// Record that these sets are meant to have copies, so the reminder stops
/// counting them. Keyed by checksum, never by upload id: trashing one copy
/// of a dismissed set must not resurrect the nag for the survivors.
async fn keep_duplicate_sets(ctx: ActionCtx) -> ActionResult {
    let Some(session) = ctx.get_session().await? else {
        return refusal("unauthorized");
    };
    let list: &[Value] = match ctx.args.raw(0) {
        Some(Value::Array(items)) => items,
        _ => &[],
    };
    // `list.map((set) => set?.checksum)`
    let raw: Vec<Value> = list.iter().map(|set| prop(Some(set), "checksum").cloned().unwrap_or(Value::Null)).collect();
    let checksums = match parse_checksums(Some(&Value::Array(raw))) {
        Ok(c) => c,
        Err(issue) => return refusal(issue.as_str()),
    };

    // `parseChecksums` lower-cases and de-duplicates, so the copies/bytes note
    // is matched back by the NORMALISED digest (the last entry wins, as Map#set).
    let mut by_checksum: std::collections::HashMap<String, &Value> = std::collections::HashMap::new();
    for set in list {
        let checksum = normalised_checksum(set);
        if !checksum.is_empty() {
            by_checksum.insert(checksum, set);
        }
    }
    let entries: Vec<DismissalInput> = checksums
        .iter()
        .map(|checksum| match by_checksum.get(checksum) {
            Some(set) => DismissalInput {
                checksum: checksum.clone(),
                copies: to_number(prop(Some(set), "copies")),
                bytes: to_number(prop(Some(set), "bytes")),
            },
            None => DismissalInput { checksum: checksum.clone(), copies: 0.0, bytes: 0.0 },
        })
        .collect();

    let kept = match dismiss_checksums(&ctx.state.db, &session.user.id, &entries).await {
        Ok(n) => n,
        Err(err) => {
            tracing::error!(error = ?err, "[duplicates] keep failed");
            return refusal("failed");
        }
    };

    let sample: Vec<&String> = checksums.iter().take(AUDIT_CHECKSUM_SAMPLE).collect();
    crate::audit::record_audit(
        &ctx.state.db,
        &session.user.id,
        "uploads.duplicates_kept",
        json!({ "count": kept, "requested": checksums.len(), "checksums": sample }),
        &ctx.client_ip(),
    )
    .await;

    ctx.revalidate_path("/dashboard/duplicates");
    ctx.revalidate_path("/dashboard");

    Ok(json!({ "ok": true, "kept": kept, "checksums": checksums }))
}

/// Undo the above: start counting these sets again. Ownership is inside the
/// DELETE's WHERE clause: a checksum is a global fact about bytes.
async fn count_duplicate_sets_again(ctx: ActionCtx) -> ActionResult {
    let Some(session) = ctx.get_session().await? else {
        return refusal("unauthorized");
    };
    let checksums = match parse_checksums(ctx.args.raw(0)) {
        Ok(c) => c,
        Err(issue) => return refusal(issue.as_str()),
    };
    let restored = match restore_checksums(&ctx.state.db, &session.user.id, &checksums).await {
        Ok(n) => n,
        Err(err) => {
            tracing::error!(error = ?err, "[duplicates] count-again failed");
            return refusal("failed");
        }
    };

    let sample: Vec<&String> = checksums.iter().take(AUDIT_CHECKSUM_SAMPLE).collect();
    crate::audit::record_audit(
        &ctx.state.db,
        &session.user.id,
        "uploads.duplicates_counted_again",
        json!({ "count": restored, "requested": checksums.len(), "checksums": sample }),
        &ctx.client_ip(),
    )
    .await;

    ctx.revalidate_path("/dashboard/duplicates");
    ctx.revalidate_path("/dashboard");

    Ok(json!({ "ok": true, "restored": restored }))
}
