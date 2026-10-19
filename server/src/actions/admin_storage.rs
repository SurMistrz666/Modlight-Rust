//! Port of the storage and deletion half of `app/actions/admin.ts`: orphaned
//! files, account deletion, the quota recalculation and the storage scan,
//! plus `components/admin/confirm.ts` (`confirmationMatches`).
//!
//! Registered under the upstream module name `admin`. The rest of that file
//! (account editing, invitations, verification mails) belongs to the accounts
//! port, whose `src/actions/admin.rs` registers its own exports; the split
//! keeps the two branches from colliding.
//!
//! Every action re-checks the admin role (`requireAdmin`): the panel hides
//! these controls from everyone else, but a server action is a public POST
//! endpoint. The two that destroy bytes (account deletion, orphan purge)
//! refuse ambiguous input rather than guessing.

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::actions::js_value::{js_string, truthy};
use crate::data::admin_storage::{
    ORPHAN_PAGE_SIZE, list_orphans, orphan_summary, recalculate_all_quotas, scan_storage_orphans, user_deletion_impact,
};
use crate::db::types::JsDate;
use crate::media::actions::{DeletableMedia, delete_media_bulk, delete_media_hard};
use crate::rpc::{ActionCtx, ActionError, ActionResult, RpcRegistry};

pub fn register(reg: &mut RpcRegistry) {
    reg.register("admin", "loadOrphans", load_orphans);
    reg.register("admin", "deleteOrphan", delete_orphan);
    reg.register("admin", "purgeAllOrphans", purge_all_orphans);
    reg.register("admin", "loadDeletionImpact", load_deletion_impact);
    reg.register("admin", "deleteUserAccount", delete_user_account);
    reg.register("admin", "recalculateEveryQuota", recalculate_every_quota);
    reg.register("admin", "runStorageScan", run_storage_scan);
}

/// Does the typed confirmation authorise destroying `count` files?
///
/// The gate is retyping the exact number of files: a value the operator has
/// to have read to know, which changes every time (a word would become muscle
/// memory). Digit grouping is stripped first, so the "8,630" / "8 630" /
/// "8.630" the dialog printed can be retyped as-is. Zero never confirms
/// anything: an empty list plus an empty input must not read as consent.
pub fn confirmation_matches(input: &str, count: f64) -> bool {
    if !count.is_finite() || count <= 0.0 {
        return false;
    }
    // input.replace(/[\s,._']/g, "")
    let clean: String = input
        .chars()
        .filter(|c| !(crate::config::is_js_whitespace(*c) || matches!(c, ',' | '.' | '_' | '\'')))
        .collect();
    if clean.is_empty() || !clean.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    clean.parse::<f64>().is_ok_and(|n| n == count)
}

// ---------------------------------------------------------------------------
// Orphaned files
// ---------------------------------------------------------------------------

async fn load_orphans(ctx: ActionCtx) -> ActionResult {
    ctx.require_admin().await?;
    let (summary, items) =
        tokio::try_join!(orphan_summary(&ctx.state.db), list_orphans(&ctx.state.db, ORPHAN_PAGE_SIZE))?;
    Ok(json!({ "summary": serde_json::to_value(&summary)?, "items": serde_json::to_value(&items)? }))
}

/// A value bound to a uuid column the way drizzle binds an untyped argument:
/// its text, which Postgres then parses (and refuses when malformed).
fn uuid_text(v: &Value) -> Result<String, ActionError> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Number(_) | Value::Bool(_) => Ok(js_string(v)),
        _ => Err(ActionError::thrown("invalid input syntax for type uuid")),
    }
}

/// Purge one orphan. The `owner_id IS NULL` predicate is part of the WHERE,
/// not a pre-flight check: a stale id from a tab left open cannot delete a
/// file that has since been re-homed to a real account.
async fn delete_orphan(ctx: ActionCtx) -> ActionResult {
    ctx.require_admin().await?;
    let Some(id) = ctx.args.raw(0).filter(|v| truthy(Some(v))) else {
        return Ok(json!({ "ok": false }));
    };
    let row: Option<DeletableMedia> = sqlx::query_as(
        r#"select "id", "owner_id", "size", "storage_key", "thumb_key" from "uploads"
           where ("uploads"."id" = $1::uuid and "uploads"."owner_id" is null) limit 1"#,
    )
    .bind(uuid_text(id)?)
    .fetch_optional(&ctx.state.db)
    .await?;
    let Some(row) = row else {
        return Ok(json!({ "ok": false }));
    };
    let storage = ctx.state.storage.get()?;
    delete_media_hard(&ctx.state.db, storage.as_ref(), &row).await?;
    ctx.revalidate_path("/dashboard/admin");
    Ok(json!({ "ok": true }))
}

/// Purge EVERY orphan. Irreversible, so gated twice: the admin retypes the
/// exact file count, and that count is re-read here. If it moved between the
/// dialog opening and the click, the confirmation no longer matches and
/// nothing happens: a stale number in a browser tab is not informed consent.
async fn purge_all_orphans(ctx: ActionCtx) -> ActionResult {
    ctx.require_admin().await?;
    let rows: Vec<DeletableMedia> = sqlx::query_as(
        r#"select "id", "owner_id", "size", "storage_key", "thumb_key" from "uploads" where "uploads"."owner_id" is null"#,
    )
    .fetch_all(&ctx.state.db)
    .await?;
    if rows.is_empty() {
        return Ok(json!({ "ok": false, "reason": "empty", "deleted": 0, "bytes": 0, "failed": 0 }));
    }
    let confirmed = match ctx.args.raw(0) {
        Some(Value::String(input)) => confirmation_matches(input, rows.len() as f64),
        // `input.replace` on anything else is a TypeError.
        _ => return Err(ActionError::thrown("input.replace is not a function")),
    };
    if !confirmed {
        return Ok(json!({ "ok": false, "reason": "confirmation", "deleted": 0, "bytes": 0, "failed": 0 }));
    }
    let storage = ctx.state.storage.get()?;
    let result = delete_media_bulk(&ctx.state.db, storage.as_ref(), &rows).await?;
    ctx.revalidate_path("/dashboard/admin");
    Ok(json!({ "ok": true, "deleted": result.deleted, "bytes": result.bytes, "failed": result.failed }))
}

// ---------------------------------------------------------------------------
// Account deletion
// ---------------------------------------------------------------------------

/// A user id as drizzle binds it to the text column: a string as is, any
/// other value as its text (which no generated id ever equals).
fn user_id_text(v: &Value) -> String {
    js_string(v)
}

async fn load_deletion_impact(ctx: ActionCtx) -> ActionResult {
    ctx.require_admin().await?;
    let Some(user_id) = ctx.args.raw(0).filter(|v| truthy(Some(v))) else {
        return Ok(Value::Null);
    };
    let impact = user_deletion_impact(&ctx.state.db, &user_id_text(user_id)).await?;
    Ok(serde_json::to_value(&impact)?)
}

/// Rows read per round of a "delete the files too" account deletion.
const ACCOUNT_DELETE_BATCH: i64 = 500;

/// The verification rows of pending invitations (lib/invites.ts
/// `revokeInvites`): `verification` has no foreign key to the account, so a
/// pending invitation would outlive it and point at an id that no longer
/// resolves.
async fn revoke_invites(db: &PgPool, user_id: &str) -> sqlx::Result<()> {
    sqlx::query(
        r#"delete from "verification" where ("verification"."identifier" like $1 and "verification"."value" = $2)"#,
    )
    .bind("invite:%")
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(())
}

/// The effect of better-auth 1.6.13's admin `removeUser` called with the
/// request's headers (`auth.api.removeUser({ body: { userId }, headers })`):
/// the caller is an admin (`user:delete` is granted to the admin role), may
/// not remove itself, and the account must exist; then its sessions go, then
/// (internalAdapter.deleteUser) the sessions again, the `account` rows and
/// the `user` row, whose foreign keys cascade or set null. `Err` is the
/// APIError upstream would have thrown.
///
/// The accounts port carries the whole endpoint (`auth::server::remove_user`
/// on its branch); this is the same sequence of statements for the one caller
/// here, and should give way to that when the branches meet.
async fn remove_user(db: &PgPool, caller_id: &str, user_id: &str) -> Result<(), &'static str> {
    if user_id == caller_id {
        return Err("YOU_CANNOT_REMOVE_YOURSELF");
    }
    let exists: Option<String> = sqlx::query_scalar(r#"select "id" from "user" where "id" = $1 limit 1"#)
        .bind(user_id)
        .fetch_optional(db)
        .await
        .map_err(|_| "INTERNAL")?;
    if exists.is_none() {
        return Err("USER_NOT_FOUND");
    }
    let statements = [
        r#"delete from "session" where "user_id" = $1"#,
        r#"delete from "session" where "user_id" = $1"#,
        r#"delete from "account" where "user_id" = $1"#,
        r#"delete from "user" where "id" = $1"#,
    ];
    for sql in statements {
        sqlx::query(sql).bind(user_id).execute(db).await.map_err(|_| "INTERNAL")?;
    }
    Ok(())
}

/// Delete an account, with the files going either way.
///
/// Files first, account second: if the account removal fails afterwards, the
/// operator sees `files-removed` and can retry; the other order would leave
/// rows quietly ownerless with no error to act on. "keep" needs no work:
/// `uploads.owner_id` is ON DELETE SET NULL, so dropping the account is what
/// orphans the rows. Self-deletion is refused here as well as by Better Auth:
/// an admin removing their own account is how an install ends up with no
/// administrator at all.
async fn delete_user_account(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_admin().await?;
    let refuse = |reason: &str| Ok(json!({ "ok": false, "reason": reason }));
    let Some(user_arg) = ctx.args.raw(0).filter(|v| truthy(Some(v))) else {
        return refuse("not-found");
    };
    if user_arg.as_str() == Some(session.user.id.as_str()) {
        return refuse("self");
    }
    let user_id = user_id_text(user_arg);
    let Some(impact) = user_deletion_impact(&ctx.state.db, &user_id).await? else {
        return refuse("not-found");
    };
    let disposition = ctx.args.raw(1).and_then(Value::as_str);

    let mut deleted = 0i64;
    let mut bytes = 0i64;
    let mut failed = 0i64;
    if disposition == Some("delete") {
        // Batched: an account can hold thousands of rows. Rows storage refused
        // are parked in `stuck` and excluded from the next query, or they
        // would come back in every batch and spin forever once only they are left.
        let storage = ctx.state.storage.get()?;
        let mut stuck: Vec<Uuid> = Vec::new();
        loop {
            let batch: Vec<DeletableMedia> = sqlx::query_as(
                r#"select "id", "owner_id", "size", "storage_key", "thumb_key" from "uploads"
                   where ("uploads"."owner_id" = $1
                          and (cardinality($2::uuid[]) = 0 or "uploads"."id" <> all($2::uuid[])))
                   limit $3"#,
            )
            .bind(&user_id)
            .bind(&stuck)
            .bind(ACCOUNT_DELETE_BATCH)
            .fetch_all(&ctx.state.db)
            .await?;
            if batch.is_empty() {
                break;
            }
            let result = delete_media_bulk(&ctx.state.db, storage.as_ref(), &batch).await?;
            deleted += result.deleted;
            bytes += result.bytes;
            if result.failed > 0 {
                // delete_media_bulk removes exactly the rows it cleared, so
                // whatever is still there is what it could not touch.
                let ids: Vec<Uuid> = batch.iter().map(|r| r.id).collect();
                let survivors: Vec<Uuid> =
                    sqlx::query_scalar(r#"select "id" from "uploads" where "uploads"."id" = any($1)"#)
                        .bind(&ids)
                        .fetch_all(&ctx.state.db)
                        .await?;
                stuck.extend(survivors);
                failed = stuck.len() as i64;
            }
        }
    }

    revoke_invites(&ctx.state.db, &user_id).await?;

    if remove_user(&ctx.state.db, &session.user.id, &user_id).await.is_err() {
        return refuse(if deleted > 0 { "files-removed" } else { "forbidden" });
    }

    ctx.revalidate_path("/dashboard/admin");
    ctx.revalidate_path("/dashboard");

    Ok(json!({
        "ok": true,
        "deleted": deleted,
        "bytes": bytes,
        "failed": failed,
        "orphaned": if disposition == Some("keep") { impact.files } else { failed },
    }))
}

// ---------------------------------------------------------------------------
// Maintenance
// ---------------------------------------------------------------------------

/// Recompute every account's `used_quota` from the rows that exist.
async fn recalculate_every_quota(ctx: ActionCtx) -> ActionResult {
    ctx.require_admin().await?;
    let result = recalculate_all_quotas(&ctx.state.db).await?;
    ctx.revalidate_path("/dashboard/admin");
    Ok(serde_json::to_value(&result)?)
}

/// Report storage objects that no database row points at. Read-only.
async fn run_storage_scan(ctx: ActionCtx) -> ActionResult {
    ctx.require_admin().await?;
    let result = scan_storage_orphans(&ctx.state.db, &ctx.state.config, JsDate::now().millis()).await?;
    Ok(serde_json::to_value(&result)?)
}

#[cfg(test)]
mod tests {
    //! tests/unit/admin-orphans.test.ts: "confirmationMatches"
    use super::*;

    #[test]
    fn accepts_the_exact_count() {
        assert!(confirmation_matches("42", 42.0));
    }

    #[test]
    fn rejects_a_different_number() {
        // A stale dialog must not confirm.
        assert!(!confirmation_matches("41", 42.0));
        assert!(!confirmation_matches("420", 42.0));
    }

    #[test]
    fn ignores_the_grouping_a_locale_formatted_count_carries() {
        assert!(confirmation_matches("8,630", 8630.0));
        assert!(confirmation_matches("8 630", 8630.0));
        assert!(confirmation_matches("8.630", 8630.0));
        // A narrow no-break space, as some locales print it.
        assert!(confirmation_matches("8\u{202F}630", 8630.0));
        assert!(confirmation_matches("8'630", 8630.0));
        assert!(confirmation_matches("8_630", 8630.0));
    }

    #[test]
    fn tolerates_surrounding_whitespace_from_a_paste() {
        assert!(confirmation_matches("  42  ", 42.0));
    }

    #[test]
    fn refuses_anything_that_is_not_a_plain_number() {
        for bad in ["", "delete", "42 files", "-42", "4e1", "٤٢"] {
            assert!(!confirmation_matches(bad, 42.0), "{bad:?}");
        }
    }

    #[test]
    fn never_confirms_when_there_is_nothing_to_delete() {
        assert!(!confirmation_matches("0", 0.0));
        assert!(!confirmation_matches("", 0.0));
    }

    #[test]
    fn refuses_a_nonsensical_count_instead_of_throwing() {
        assert!(!confirmation_matches("1", f64::NAN));
        assert!(!confirmation_matches("1", -1.0));
    }

    #[test]
    fn leading_zeros_are_the_same_number() {
        assert!(confirmation_matches("042", 42.0));
    }
}
