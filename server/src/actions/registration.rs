//! Port of `app/actions/registration.ts`: administration of who may sign
//! themselves up.
//!
//! The two settings here answer different questions and are deliberately kept
//! apart:
//!
//! ```text
//!   register_enabled     - may a stranger create an account at all?
//!   registration key     - WHICH stranger, once the answer above is yes?
//! ```
//!
//! The key never opens a closed registration (see [`crate::registration`]).
//! That is what keeps the toggle usable as an emergency stop: one switch, one
//! meaning.
//!
//! Arguments arrive as tagged JSON from any caller, so the JavaScript reading
//! of them is reproduced where upstream relies on it: `enabled ? "on" :
//! "off"` is truthiness, `key?.trim()` throws a TypeError on anything that is
//! neither a string nor nullish, and an audit detail is what `JSON.stringify`
//! makes of the raw value (`undefined` drops the key).

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::registration::{
    REGISTRATION_KEY_MAX, RegistrationKeyState, read_registration_key, registration_key_state, set_registration_key,
};
use crate::rpc::{ActionCtx, ActionError, ActionResult, RpcRegistry};
use crate::settings::{is_registration_enabled, keys, set_setting};

pub fn register(reg: &mut RpcRegistry) {
    reg.register("registration", "loadRegistrationSettings", load_registration_settings);
    reg.register("registration", "setRegistrationEnabled", set_registration_enabled);
    reg.register("registration", "saveRegistrationKey", save_registration_key);
    reg.register("registration", "revealRegistrationKey", reveal_registration_key);
}

/// `RegistrationSettings`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrationSettings {
    pub enabled: bool,
    pub key_state: RegistrationKeyState,
}

async fn current_settings(ctx: &ActionCtx) -> Result<RegistrationSettings, ActionError> {
    let config = &ctx.state.config;
    let enabled = is_registration_enabled(&ctx.state.db, config.auth.registration_enabled).await?;
    let key_state = registration_key_state(&ctx.state.db, &config.auth.secret).await?;
    Ok(RegistrationSettings { enabled, key_state })
}

/// JavaScript truthiness of a tagged argument (`undefined` when absent).
pub(crate) fn js_truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(v @ Value::Object(map)) if map.len() == 1 => {
            if crate::rpc::tagged::is_undefined(v) {
                false
            } else if let Some(n) = crate::rpc::tagged::raw_num(v) {
                !n.is_nan() && n != 0.0
            } else if let Some(Value::String(b)) = map.get("$bigint") {
                !b.trim_start_matches('-').trim_start_matches('0').is_empty()
            } else {
                true
            }
        }
        Some(_) => true,
    }
}

/// Why a value cannot go into an audit detail (`JSON.stringify` throws on a
/// bigint, and `recordAudit` then logs instead of writing).
#[derive(Debug)]
pub(crate) struct NotSerializable;

/// `JSON.stringify` of a tagged value, as drizzle stores it in a jsonb column:
/// `Ok(None)` for `undefined` (the key disappears), dates as ISO strings,
/// NaN and the infinities as `null`.
pub(crate) fn json_of(v: &Value) -> Result<Option<Value>, NotSerializable> {
    match v {
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(json_of(item)?.unwrap_or(Value::Null));
            }
            Ok(Some(Value::Array(out)))
        }
        Value::Object(map) => {
            if map.len() == 1 {
                if crate::rpc::tagged::is_undefined(v) {
                    return Ok(None);
                }
                if let Some(d) = map.get("$date") {
                    return Ok(Some(d.clone()));
                }
                if map.contains_key("$num") {
                    return Ok(Some(Value::Null));
                }
                if map.contains_key("$bigint") {
                    return Err(NotSerializable);
                }
            }
            let mut out = Map::new();
            for (k, x) in map {
                if let Some(x) = json_of(x)? {
                    out.insert(k.clone(), x);
                }
            }
            Ok(Some(Value::Object(out)))
        }
        other => Ok(Some(other.clone())),
    }
}

/// `{ by, byEmail, [key]: value }` with `value` as JSON.stringify writes it,
/// or `None` when the insert would fail.
fn detail_with(by: &str, by_email: &str, key: &str, value: Option<&Value>) -> Option<Value> {
    let mut detail = Map::new();
    detail.insert("by".into(), Value::String(by.to_string()));
    detail.insert("byEmail".into(), Value::String(by_email.to_string()));
    if let Some(v) = value {
        match json_of(v) {
            Ok(Some(v)) => {
                detail.insert(key.into(), v);
            }
            Ok(None) => {}
            Err(NotSerializable) => return None,
        }
    }
    Some(Value::Object(detail))
}

async fn audit(ctx: &ActionCtx, actor: &str, action: &str, detail: Option<Value>) {
    match detail {
        Some(detail) => crate::audit::record_audit(&ctx.state.db, actor, action, detail, &ctx.client_ip()).await,
        None => tracing::error!("[audit] could not record {action}: Do not know how to serialize a BigInt"),
    }
}

/// `loadRegistrationSettings()`
async fn load_registration_settings(ctx: ActionCtx) -> ActionResult {
    ctx.require_admin().await?;
    crate::rpc::ok(&current_settings(&ctx).await?)
}

/// `setRegistrationEnabled(enabled)`
async fn set_registration_enabled(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_admin().await?;
    let enabled = ctx.args.raw(0);
    set_setting(&ctx.state.db, keys::REGISTER_ENABLED, Some(if js_truthy(enabled) { "on" } else { "off" })).await?;
    let detail = detail_with(&session.user.id, &session.user.email, "enabled", enabled);
    audit(&ctx, &session.user.id, "admin.registration_toggled", detail).await;
    ctx.revalidate_path("/dashboard/admin");
    // /register, /login and the landing page all read this flag on render.
    ctx.revalidate_path("/");
    // `return loadRegistrationSettings()`: requireAdmin again (the same,
    // memoised session), then the fresh read.
    crate::rpc::ok(&current_settings(&ctx).await?)
}

/// `key?.trim() ?? ""`
fn trimmed_key(key: Option<&Value>) -> Result<String, ActionError> {
    match key {
        None | Some(Value::Null) => Ok(String::new()),
        Some(v) if crate::rpc::tagged::is_undefined(v) => Ok(String::new()),
        Some(Value::String(s)) => Ok(crate::config::js_trim(s).to_string()),
        Some(_) => Err(ActionError::thrown("key?.trim is not a function")),
    }
}

/// `saveRegistrationKey(key)`: set or clear the shared key. An empty string
/// clears it.
async fn save_registration_key(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_admin().await?;
    let value = trimmed_key(ctx.args.raw(0))?;
    if value.encode_utf16().count() > REGISTRATION_KEY_MAX {
        return Ok(json!({ "ok": false, "reason": "too-long" }));
    }

    let secret = &ctx.state.config.auth.secret;
    set_registration_key(&ctx.state.db, secret, if value.is_empty() { None } else { Some(&value) }).await?;
    // The key itself is a secret handed out by hand; the audit records only
    // that it moved.
    let cleared = Value::Bool(value.is_empty());
    let detail = detail_with(&session.user.id, &session.user.email, "cleared", Some(&cleared));
    audit(&ctx, &session.user.id, "admin.registration_key_changed", detail).await;

    ctx.revalidate_path("/dashboard/admin");
    ctx.revalidate_path("/");
    let key_state = registration_key_state(&ctx.state.db, secret).await?;
    crate::rpc::ok(&json!({ "ok": true, "keyState": key_state }))
}

/// `revealRegistrationKey()`: show the administrator the key, on request.
///
/// Deliberately different from the SMTP password, which never comes back.
/// This key exists to be given to people, so an administrator has to be able
/// to read it in order to use it at all. What that buys is a reveal that
/// leaves a trace: every disclosure is an audit entry naming who asked.
async fn reveal_registration_key(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_admin().await?;
    let key = read_registration_key(&ctx.state.db, &ctx.state.config.auth.secret).await?;
    let detail = detail_with(&session.user.id, &session.user.email, "", None);
    audit(&ctx, &session.user.id, "admin.registration_key_revealed", detail).await;
    crate::rpc::ok(&json!({ "key": key }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truthiness_follows_javascript() {
        let falsy = [
            Value::Null,
            json!(false),
            json!(0),
            json!(0.0),
            json!(""),
            json!({ "$undefined": 1 }),
            json!({ "$num": "NaN" }),
            json!({ "$bigint": "0" }),
        ];
        for v in &falsy {
            assert!(!js_truthy(Some(v)), "{v}");
        }
        assert!(!js_truthy(None));
        let truthy = [
            json!(true),
            json!(-1),
            json!("off"),
            json!([]),
            json!({}),
            json!({ "$num": "Infinity" }),
            json!({ "$bigint": "-5" }),
            json!({ "$date": "2026-10-18T12:00:00.000Z" }),
        ];
        for v in &truthy {
            assert!(js_truthy(Some(v)), "{v}");
        }
    }

    #[test]
    fn audit_values_are_what_json_stringify_writes() {
        assert_eq!(json_of(&json!({ "$undefined": 1 })).unwrap(), None);
        assert_eq!(json_of(&json!({ "$num": "NaN" })).unwrap(), Some(Value::Null));
        assert_eq!(
            json_of(&json!({ "$date": "2026-10-18T12:00:00.000Z" })).unwrap(),
            Some(json!("2026-10-18T12:00:00.000Z"))
        );
        assert_eq!(
            json_of(&json!({ "a": { "$undefined": 1 }, "b": [{ "$undefined": 1 }, 1] })).unwrap(),
            Some(json!({ "b": [null, 1] }))
        );
        assert!(json_of(&json!([{ "$bigint": "1" }])).is_err());
        assert_eq!(
            detail_with("u1", "a@b.c", "enabled", Some(&json!({ "$undefined": 1 }))),
            Some(json!({ "by": "u1", "byEmail": "a@b.c" }))
        );
        assert_eq!(detail_with("u1", "a@b.c", "enabled", Some(&json!({ "$bigint": "1" }))), None);
    }

    #[test]
    fn keys_trim_like_optional_chaining() {
        assert_eq!(trimmed_key(None).unwrap(), "");
        assert_eq!(trimmed_key(Some(&Value::Null)).unwrap(), "");
        assert_eq!(trimmed_key(Some(&json!({ "$undefined": 1 }))).unwrap(), "");
        assert_eq!(trimmed_key(Some(&json!("  team-key\u{FEFF} "))).unwrap(), "team-key");
        assert!(trimmed_key(Some(&json!(5))).is_err());
        assert!(trimmed_key(Some(&json!({ "trim": 1 }))).is_err());
    }
}
