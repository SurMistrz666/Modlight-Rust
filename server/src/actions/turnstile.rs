//! Not upstream: administration of the optional Cloudflare Turnstile check
//! (src/turnstile.rs), the "Cloudflare Turnstile" card under Admin ->
//! Settings. Module `turnstile`, called by the SPA's `src/turnstile/actions.ts`.
//!
//! The rules of the SMTP card (`actions/mail.rs`) hold here too:
//!
//!  1. **The secret key never travels back.** Not to the browser, not into an
//!     audit entry, not into a log line; the panel learns a state.
//!  2. **Failures travel as data** (`{ok: false, reason}`), so an operator
//!     sees why in production too.
//!  3. **Every write is audited**, by field name only.
//!
//! One rule of its own: a save that turns the check on first asks Cloudflare
//! whether it accepts the secret. A secret Cloudflare rejects would refuse
//! every sign-in, the administrator's own included, and the switch to undo it
//! sits behind a sign-in.

use serde_json::{Value, json};

use crate::config::js_trim;
use crate::password::check_rate_limit;
use crate::rpc::{ActionCtx, ActionError, ActionResult, RpcRegistry};
use crate::turnstile::{
    Probe, SecretState, TurnstilePatch, TurnstileSource, audit_detail, clear_settings, is_key_shaped, save_settings,
    to_status,
};

pub fn register(reg: &mut RpcRegistry) {
    reg.register("turnstile", "loadTurnstileStatus", load_turnstile_status);
    reg.register("turnstile", "saveTurnstileSettings", save_turnstile_settings);
    reg.register("turnstile", "clearTurnstileSettings", clear_turnstile_settings);
    reg.register("turnstile", "testTurnstileSecret", test_turnstile_secret);
}

/// Longest site key / secret accepted (Cloudflare's are about 35 characters).
const KEY_MAX: usize = 128;
/// Calls to Cloudflare per administrator ("Test" and enabling saves).
const PROBES_PER_WINDOW: u32 = 10;
const PROBE_WINDOW_MS: i64 = 10 * 60_000;

fn now_ms() -> i64 {
    crate::db::types::JsDate::now().millis()
}

/// `input.<key>` of a plain object argument.
fn field<'a>(input: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    match input {
        Some(Value::Object(map)) => map.get(key),
        _ => None,
    }
}

/// An optional string field: absent, null or undefined is `None`; any other
/// non-string is a thrown TypeError.
fn optional_string(input: Option<&Value>, key: &str) -> Result<Option<String>, ActionError> {
    match field(input, key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) if crate::rpc::tagged::is_undefined(v) => Ok(None),
        Some(Value::String(s)) => Ok(Some(js_trim(s).to_string())),
        Some(_) => Err(ActionError::thrown(format!("TypeError: input.{key} must be a string"))),
    }
}

async fn fresh_status(ctx: &ActionCtx) -> Result<Value, ActionError> {
    let resolved = ctx.state.turnstile.load(&ctx.state.db, &ctx.state.config, true).await?;
    Ok(serde_json::to_value(to_status(&resolved, &ctx.state.config)).map_err(anyhow::Error::from)?)
}

async fn audit(ctx: &ActionCtx, actor: &str, action: &str, detail: Value) {
    crate::audit::record_audit(&ctx.state.db, actor, action, detail, &ctx.client_ip()).await;
}

fn probe_allowed(admin_id: &str) -> bool {
    check_rate_limit(&format!("turnstile-test:{admin_id}"), PROBES_PER_WINDOW, PROBE_WINDOW_MS).allowed
}

/// `loadTurnstileStatus(): TurnstileStatus`
async fn load_turnstile_status(ctx: ActionCtx) -> ActionResult {
    ctx.require_admin().await?;
    fresh_status(&ctx).await
}

/// `saveTurnstileSettings({ enabled, siteKey, secret? })`:
/// `{ok: true, status, changed}` or `{ok: false, reason}` with reason one of
/// `invalid-site-key`, `invalid-secret`, `site-key-required`,
/// `secret-required`, `secret-rejected`, `unreachable`, `rate-limited`.
async fn save_turnstile_settings(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_admin().await?;
    let input = ctx.args.raw(0);
    let enabled = crate::actions::registration::js_truthy(field(input, "enabled"));
    let site_key = optional_string(input, "siteKey")?.unwrap_or_default();
    // Blank means "keep the stored secret", as for the SMTP password.
    let secret = optional_string(input, "secret")?.filter(|s| !s.is_empty());

    if !site_key.is_empty() && !is_key_shaped(&site_key, KEY_MAX) {
        return Ok(json!({ "ok": false, "reason": "invalid-site-key" }));
    }
    if secret.as_deref().is_some_and(|s| !is_key_shaped(s, KEY_MAX)) {
        return Ok(json!({ "ok": false, "reason": "invalid-secret" }));
    }

    let current = ctx.state.turnstile.load(&ctx.state.db, &ctx.state.config, true).await?;
    // A stored secret survives only while it belongs to the same widget. An
    // environment secret never moves into the panel.
    let keeps_stored = current.source == TurnstileSource::Panel
        && current.site_key == site_key
        && current.secret_state == SecretState::Set;
    let effective_secret = secret.clone().or_else(|| keeps_stored.then(|| current.secret.clone()));

    if enabled {
        if site_key.is_empty() {
            return Ok(json!({ "ok": false, "reason": "site-key-required" }));
        }
        let Some(probe_with) = effective_secret else {
            return Ok(json!({ "ok": false, "reason": "secret-required" }));
        };
        if !probe_allowed(&session.user.id) {
            return Ok(json!({ "ok": false, "reason": "rate-limited" }));
        }
        match ctx.state.turnstile.probe_secret(&probe_with).await {
            Probe::Accepted { .. } => {}
            Probe::Rejected => return Ok(json!({ "ok": false, "reason": "secret-rejected" })),
            Probe::Unreachable => return Ok(json!({ "ok": false, "reason": "unreachable" })),
        }
    }

    let patch = TurnstilePatch { enabled, site_key, secret };
    let (changed, secret_changed) = save_settings(&ctx.state, &patch).await?;
    let detail = audit_detail(&session.user.id, &session.user.email, &changed, secret_changed);
    audit(&ctx, &session.user.id, "turnstile.settings_changed", detail).await;
    ctx.revalidate_path("/dashboard/admin");
    let status = fresh_status(&ctx).await?;
    Ok(json!({ "ok": true, "status": status, "changed": changed }))
}

/// `clearTurnstileSettings(): TurnstileStatus`: hand the configuration back
/// to the environment by deleting the panel rows.
async fn clear_turnstile_settings(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_admin().await?;
    clear_settings(&ctx.state).await?;
    let detail = json!({ "by": session.user.id, "byEmail": session.user.email });
    audit(&ctx, &session.user.id, "turnstile.settings_cleared", detail).await;
    ctx.revalidate_path("/dashboard/admin");
    fresh_status(&ctx).await
}

/// `testTurnstileSecret(secret?)`: does Cloudflare accept this secret (the one
/// typed, else the one in effect)? `{ok: true, alwaysPasses, tookMs}` or
/// `{ok: false, kind, tookMs}` with kind `no-secret`, `invalid-secret`,
/// `rejected`, `unreachable` or `rate-limited`. One siteverify call with
/// Cloudflare's dummy token; nothing is stored.
async fn test_turnstile_secret(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_admin().await?;
    let started = now_ms();
    let typed = match ctx.args.raw(0) {
        Some(Value::String(s)) => Some(js_trim(s).to_string()).filter(|s| !s.is_empty()),
        _ => None,
    };
    let secret = match typed {
        Some(s) => {
            if !is_key_shaped(&s, KEY_MAX) {
                return Ok(json!({ "ok": false, "kind": "invalid-secret", "tookMs": 0 }));
            }
            s
        }
        None => {
            let current = ctx.state.turnstile.load(&ctx.state.db, &ctx.state.config, true).await?;
            if current.secret.is_empty() {
                return Ok(json!({ "ok": false, "kind": "no-secret", "tookMs": 0 }));
            }
            current.secret
        }
    };
    if !probe_allowed(&session.user.id) {
        return Ok(json!({ "ok": false, "kind": "rate-limited", "tookMs": 0 }));
    }
    let probe = ctx.state.turnstile.probe_secret(&secret).await;
    let took = now_ms() - started;
    Ok(match probe {
        Probe::Accepted { always_passes } => json!({ "ok": true, "alwaysPasses": always_passes, "tookMs": took }),
        Probe::Rejected => json!({ "ok": false, "kind": "rejected", "tookMs": took }),
        Probe::Unreachable => json!({ "ok": false, "kind": "unreachable", "tookMs": took }),
    })
}
