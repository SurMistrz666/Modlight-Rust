//! Not upstream: per-account file encryption (docs/ENCRYPTION.md section 8),
//! module `encryption`, called by the SPA's `src/encryption/actions.ts`.
//!
//! Failures travel as data (`{ok: false, reason}`), never as thrown errors, so
//! the card can say exactly why in production too. The file password never
//! leaves the request that carried it: not into a log line, not into an audit
//! entry, not into an answer.

use serde_json::{Value, json};
use uuid::Uuid;

use crate::audit::record_audit;
use crate::data::sessions::describe_device;
use crate::db::models::FileVault;
use crate::media::serve::first_header;
use crate::rpc::{ActionCtx, ActionResult, RpcRegistry};
use crate::vault::VaultState;
use crate::vault::access::offered_grant_ids;
use crate::vault::status::vault_status;
use crate::vault::store::{self, GrantKind};
use crate::vault::unlock::{
    self, Asker, Granted, UnlockError, check_new_password, clear_cookie, create_vault, grant_cookie, new_vault_params,
};

pub fn register(reg: &mut RpcRegistry) {
    reg.register("encryption", "loadEncryptionStatus", load_encryption_status);
    reg.register("encryption", "enableEncryption", enable_encryption);
    reg.register("encryption", "unlockMyFiles", unlock_my_files);
    reg.register("encryption", "lockMyFiles", lock_my_files);
    reg.register("encryption", "changeFilePassword", change_file_password);
    reg.register("encryption", "disableEncryption", disable_encryption);
    reg.register("encryption", "revokeVaultGrant", revoke_vault_grant);
    reg.register("encryption", "revokeAllVaultGrants", revoke_all_vault_grants);
    reg.register("encryption", "unlockOwnerFiles", unlock_owner_files);
}

/// A string argument, or "" for anything else.
fn text(ctx: &ActionCtx, i: usize) -> String {
    match ctx.args.raw(i) {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

fn flag(ctx: &ActionCtx, i: usize, default: bool) -> bool {
    match ctx.args.raw(i) {
        Some(Value::Bool(b)) => *b,
        _ => default,
    }
}

fn label_of(ctx: &ActionCtx) -> Option<String> {
    let device = describe_device(first_header(&ctx.headers, "user-agent").as_deref());
    match (device.browser, device.os) {
        (Some(b), Some(o)) => Some(format!("{b} · {o}")),
        (Some(b), None) => Some(b),
        (None, Some(o)) => Some(o),
        (None, None) => None,
    }
}

fn refusal(reason: &str) -> ActionResult {
    Ok(json!({ "ok": false, "reason": reason }))
}

fn unlock_refusal(e: UnlockError) -> ActionResult {
    match e {
        UnlockError::Wrong => refusal("wrong"),
        UnlockError::NotEncrypted => refusal("not-encrypted"),
        UnlockError::RateLimited { retry_after_ms } => Ok(json!({
            "ok": false, "reason": "rate-limited",
            "retryAfter": (retry_after_ms as f64 / 1000.0).ceil() as i64,
        })),
    }
}

async fn status_value(ctx: &ActionCtx, user_id: &str) -> anyhow::Result<Value> {
    Ok(serde_json::to_value(vault_status(&ctx.state, &ctx.headers, user_id).await?)?)
}

/// `loadEncryptionStatus()`
async fn load_encryption_status(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    crate::rpc::ok(&vault_status(&ctx.state, &ctx.headers, &session.user.id).await?)
}

fn set_grant_cookie(ctx: &ActionCtx, owner_id: &str, granted: &Granted) {
    ctx.set_cookie(grant_cookie(&ctx.state, owner_id, granted));
}

/// `enableEncryption(password, confirm, acknowledged)`
async fn enable_encryption(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let user_id = session.user.id.clone();
    let (password, confirm) = (text(&ctx, 0), text(&ctx, 1));
    if !flag(&ctx, 2, false) {
        return refusal("not-acknowledged");
    }
    if password != confirm {
        return refusal("mismatch");
    }
    if let Some(existing) = store::load(&ctx.state.db, &user_id).await? {
        return refusal(if store::state_of(&existing) == Some(VaultState::Decrypting) { "busy" } else { "already-on" });
    }
    // Throttled like an unlock: each attempt costs an Argon2 run.
    if let Err(e) = unlock::admit_attempt(&user_id, Asker::Owner, &ctx.client_ip()) {
        return unlock_refusal(e);
    }
    if let Some(problem) = check_new_password(&ctx.state, &user_id, &password).await? {
        return refusal(problem.as_str());
    }
    let ip = ctx.client_ip();
    let label = label_of(&ctx);
    let Some((_, granted)) =
        create_vault(&ctx.state, &user_id, &password, new_vault_params(), label.as_deref(), &ip).await?
    else {
        return refusal("already-on");
    };
    set_grant_cookie(&ctx, &user_id, &granted);
    record_audit(&ctx.state.db, &user_id, "vault.enabled", json!({}), &ip).await;
    ctx.revalidate_path("/dashboard/profile");
    Ok(json!({ "ok": true }))
}

/// The caller's vault, if it asks for a password.
async fn gated_vault(ctx: &ActionCtx, user_id: &str) -> anyhow::Result<Option<FileVault>> {
    Ok(store::load(&ctx.state.db, user_id).await?.filter(|v| store::state_of(v).is_some_and(VaultState::is_gated)))
}

/// `unlockMyFiles(password, remember = true)`
async fn unlock_my_files(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let user_id = session.user.id.clone();
    let Some(vault) = gated_vault(&ctx, &user_id).await? else {
        return refusal("not-encrypted");
    };
    let ip = ctx.client_ip();
    let label = label_of(&ctx);
    let remember = flag(&ctx, 1, true);
    match unlock::unlock(&ctx.state, &vault, &text(&ctx, 0), Asker::Owner, &ip, remember, label.as_deref()).await? {
        Ok(granted) => {
            set_grant_cookie(&ctx, &user_id, &granted);
            record_audit(
                &ctx.state.db,
                &user_id,
                "vault.unlocked",
                unlock::audit_detail(Asker::Owner, Some(granted.id)),
                &ip,
            )
            .await;
            Ok(json!({ "ok": true }))
        }
        Err(e) => {
            if e == UnlockError::Wrong || matches!(e, UnlockError::RateLimited { .. }) {
                record_audit(&ctx.state.db, &user_id, "vault.unlock_failed", json!({ "by": "owner" }), &ip).await;
            }
            unlock_refusal(e)
        }
    }
}

/// `lockMyFiles()`: this browser forgets the password (its grant is revoked).
async fn lock_my_files(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let user_id = session.user.id.clone();
    for id in offered_grant_ids(&ctx.headers, &user_id) {
        store::delete_grant(&ctx.state.db, id, &user_id).await?;
    }
    ctx.set_cookie(clear_cookie(&ctx.state, &user_id));
    Ok(json!({ "ok": true }))
}

/// `changeFilePassword(current, next, confirm)`
async fn change_file_password(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let user_id = session.user.id.clone();
    let (current, next, confirm) = (text(&ctx, 0), text(&ctx, 1), text(&ctx, 2));
    let Some(vault) = gated_vault(&ctx, &user_id).await? else {
        return refusal("not-encrypted");
    };
    if next != confirm {
        return refusal("mismatch");
    }
    if let Some(problem) = check_new_password(&ctx.state, &user_id, &next).await? {
        return refusal(problem.as_str());
    }
    let ip = ctx.client_ip();
    if let Err(e) = unlock::admit_attempt(&user_id, Asker::Owner, &ip) {
        return unlock_refusal(e);
    }
    let secret = match unlock::open_with_password(&vault, &current).await? {
        Ok(s) => s,
        Err(e) => {
            record_audit(&ctx.state.db, &user_id, "vault.unlock_failed", json!({ "by": "owner" }), &ip).await;
            return unlock_refusal(e);
        }
    };
    let vault = unlock::change_password(&ctx.state, &vault, &secret, &next).await?;
    // Every grant is gone; this browser keeps working with a new one.
    let label = label_of(&ctx);
    let granted =
        unlock::grant(&ctx.state, &vault, &secret, GrantKind::Owner, true, label.as_deref(), Some(&ip)).await?;
    set_grant_cookie(&ctx, &user_id, &granted);
    record_audit(&ctx.state.db, &user_id, "vault.password_changed", json!({}), &ip).await;
    Ok(json!({ "ok": true }))
}

/// `disableEncryption(password)`
async fn disable_encryption(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let user_id = session.user.id.clone();
    let Some(vault) = gated_vault(&ctx, &user_id).await? else {
        return refusal("not-encrypted");
    };
    let ip = ctx.client_ip();
    if let Err(e) = unlock::admit_attempt(&user_id, Asker::Owner, &ip) {
        return unlock_refusal(e);
    }
    let secret = match unlock::open_with_password(&vault, &text(&ctx, 0)).await? {
        Ok(s) => s,
        Err(e) => {
            record_audit(&ctx.state.db, &user_id, "vault.unlock_failed", json!({ "by": "owner" }), &ip).await;
            return unlock_refusal(e);
        }
    };
    unlock::start_decrypting(&ctx.state, &vault, &secret).await?;
    ctx.set_cookie(clear_cookie(&ctx.state, &user_id));
    record_audit(&ctx.state.db, &user_id, "vault.disabled", json!({}), &ip).await;
    Ok(json!({ "ok": true }))
}

/// `revokeVaultGrant(id)`
async fn revoke_vault_grant(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let user_id = session.user.id.clone();
    let Ok(id) = Uuid::parse_str(&text(&ctx, 0)) else {
        return refusal("not-found");
    };
    if !store::delete_grant(&ctx.state.db, id, &user_id).await? {
        return refusal("not-found");
    }
    if offered_grant_ids(&ctx.headers, &user_id).contains(&id) {
        ctx.set_cookie(clear_cookie(&ctx.state, &user_id));
    }
    record_audit(&ctx.state.db, &user_id, "vault.grant_revoked", json!({ "grant": id.to_string() }), &ctx.client_ip())
        .await;
    Ok(json!({ "ok": true, "status": status_value(&ctx, &user_id).await? }))
}

/// `revokeAllVaultGrants()`
async fn revoke_all_vault_grants(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_user().await?;
    let user_id = session.user.id.clone();
    let count = store::delete_all_grants(&ctx.state.db, &user_id).await?;
    ctx.set_cookie(clear_cookie(&ctx.state, &user_id));
    record_audit(&ctx.state.db, &user_id, "vault.grants_revoked", json!({ "count": count }), &ctx.client_ip()).await;
    Ok(json!({ "ok": true, "count": count }))
}

/// Whose files a visitor's unlock is for: `{mediaId}` (a file page) or
/// `{tag}` (a gallery or an album), resolved to the owner's id.
async fn target_owner(ctx: &ActionCtx, target: Option<&Value>) -> anyhow::Result<Option<String>> {
    let Some(Value::Object(target)) = target else { return Ok(None) };
    if let Some(Value::String(id)) = target.get("mediaId") {
        let Ok(id) = Uuid::parse_str(id) else { return Ok(None) };
        let owner: Option<Option<String>> =
            sqlx::query_scalar(r#"select "owner_id" from "uploads" where "id" = $1 and "deleted_at" is null limit 1"#)
                .bind(id)
                .fetch_optional(&ctx.state.db)
                .await?;
        return Ok(owner.flatten());
    }
    if let Some(Value::String(path)) = target.get("path") {
        return crate::vault::access::owner_of_path(&ctx.state, path).await;
    }
    if let Some(Value::String(tag)) = target.get("tag") {
        return crate::vault::access::owner_by_tag(&ctx.state, tag).await;
    }
    Ok(None)
}

/// `unlockOwnerFiles(target, password, remember = true)`: the unlock page of
/// a link (file, gallery, album). Unknown targets and accounts without a vault
/// answer like a wrong password, after the same throttle.
async fn unlock_owner_files(ctx: ActionCtx) -> ActionResult {
    let ip = ctx.client_ip();
    let owner = target_owner(&ctx, ctx.args.raw(0)).await?;
    let vault = match owner.as_deref() {
        Some(o) => gated_vault(&ctx, o).await?,
        None => None,
    };
    let Some(vault) = vault else {
        // Spend an attempt anyway: the page must not tell a stranger which
        // links lead to encrypted accounts faster than guessing would.
        let _ = unlock::admit_attempt("unknown", Asker::Visitor, &ip);
        return refusal("wrong");
    };
    let viewer = ctx.get_session().await?;
    let asker = if viewer.as_ref().is_some_and(|s| s.user.id == vault.user_id) { Asker::Owner } else { Asker::Visitor };
    let remember = flag(&ctx, 2, true);
    let label = label_of(&ctx);
    match unlock::unlock(&ctx.state, &vault, &text(&ctx, 1), asker, &ip, remember, label.as_deref()).await? {
        Ok(granted) => {
            set_grant_cookie(&ctx, &vault.user_id, &granted);
            if asker == Asker::Owner {
                record_audit(
                    &ctx.state.db,
                    &vault.user_id,
                    "vault.unlocked",
                    unlock::audit_detail(asker, Some(granted.id)),
                    &ip,
                )
                .await;
            }
            Ok(json!({ "ok": true }))
        }
        Err(e) => {
            if matches!(e, UnlockError::RateLimited { .. }) {
                note_rate_limited(&ctx, &vault.user_id, asker, &ip).await;
            }
            unlock_refusal(e)
        }
    }
}

/// One audit entry per account and quarter of an hour for throttled visitor
/// attempts (a flood must not become a flood of audit rows).
async fn note_rate_limited(ctx: &ActionCtx, owner_id: &str, asker: Asker, ip: &str) {
    let key = format!("vault-audit|{owner_id}");
    if crate::password::check_rate_limit(&key, 1, 15 * 60 * 1000).allowed {
        let by = if asker == Asker::Owner { "owner" } else { "visitor" };
        record_audit(&ctx.state.db, owner_id, "vault.unlock_failed", json!({ "by": by, "rateLimited": true }), ip)
            .await;
    }
}
