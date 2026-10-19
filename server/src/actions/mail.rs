//! Port of `app/actions/mail.ts`: administration of outgoing mail.
//!
//! Three rules run through every function here:
//!
//!  1. **The SMTP password never travels back.** Not to the browser, not into
//!     an audit entry, not into a log line. The panel learns a STATE
//!     ("set" / "unset" / "undecryptable") and nothing else.
//!  2. **Failures travel as data, not as thrown messages.** Next masks the
//!     message of an error raised inside a Server Action in production, so an
//!     operator debugging SMTP would receive "An error occurred" instead of the
//!     server's own explanation.
//!  3. **Every write is audited**, by field name. Which knob was turned is
//!     operationally useful; what it was turned to is, in one case, a
//!     credential.
//!
//! Arguments arrive as tagged JSON from any caller, so the JavaScript reading
//! of them is reproduced where upstream relies on it (`input.host.trim()` is a
//! TypeError for anything but a string, `Number.isFinite(input.port)` only
//! accepts a real number, `Boolean(input.password)` is truthiness).

use serde_json::{Map, Value, json};

use super::registration::{js_truthy, json_of};
use crate::config::{js_number, js_trim};
use crate::mail::{MailMessage, SmtpFailure, get_transport, send_with_transport};
use crate::mail_settings::{
    MailPasswordState, MailSettingsPatch, MailSource, PasswordPatch, clear_mail_password, clear_mail_settings,
    invalidate_mail_settings, load_mail_settings, save_mail_settings, set_email_verification_required, to_mail_status,
};
use crate::mail_templates::{MailTexts, pick_mail_locale, render_smtp_test};
use crate::password::check_rate_limit;
use crate::rpc::tagged::{Num, Undefined};
use crate::rpc::{ActionCtx, ActionError, ActionResult, RpcRegistry};
use crate::secret_box::{redact_secrets, smtp_secret_forms};

pub fn register(reg: &mut RpcRegistry) {
    reg.register("mail", "loadMailStatus", load_mail_status);
    reg.register("mail", "saveSmtpSettings", save_smtp_settings);
    reg.register("mail", "clearSmtpSettings", clear_smtp_settings);
    reg.register("mail", "clearSmtpPassword", clear_smtp_password);
    reg.register("mail", "setVerificationRequired", set_verification_required);
    reg.register("mail", "sendTestMail", send_test_mail);
}

/// `toMailStatus(await loadMailSettings(true))`
async fn fresh_status(ctx: &ActionCtx) -> ActionResult {
    let resolved = load_mail_settings(&ctx.state, true).await?;
    crate::rpc::ok(&to_mail_status(&resolved, &ctx.state.config))
}

/// `{ by, byEmail, ...more }`
fn detail(by: &str, by_email: &str, more: Vec<(&str, Value)>) -> Value {
    let mut map = Map::new();
    map.insert("by".into(), Value::String(by.to_string()));
    map.insert("byEmail".into(), Value::String(by_email.to_string()));
    for (k, v) in more {
        map.insert(k.into(), v);
    }
    Value::Object(map)
}

async fn audit(ctx: &ActionCtx, actor: &str, action: &str, detail: Value) {
    crate::audit::record_audit(&ctx.state.db, actor, action, detail, &ctx.client_ip()).await;
}

/// `loadMailStatus()`: what the panel is allowed to know. Never contains a
/// credential.
async fn load_mail_status(ctx: ActionCtx) -> ActionResult {
    ctx.require_admin().await?;
    fresh_status(&ctx).await
}

/// Hostname or IP literal (`/^[a-z0-9._:-]+$/i`). Deliberately permissive:
/// internal names are valid.
fn host_pattern(host: &str) -> bool {
    !host.is_empty() && host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
}

/// `/^[^\s@]+@[^\s@]+\.[^\s@]+$/` (JavaScript `\s`).
fn email_pattern(from: &str) -> bool {
    if from.chars().any(crate::config::is_js_whitespace) {
        return false;
    }
    let mut parts = from.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    // `[^\s@]+\.[^\s@]+`: a dot with at least one character on each side.
    !local.is_empty() && domain.char_indices().any(|(i, c)| c == '.' && i > 0 && i + 1 < domain.len())
}

/// `input.<key>` of a plain object argument (`undefined` otherwise).
fn field<'a>(input: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    match input {
        Some(v @ Value::Object(map)) if map.len() != 1 || !is_tag(v) => map.get(key),
        _ => None,
    }
}

/// One of the four tagged-JSON wrappers (a Date, undefined, a bigint, a
/// non-finite number): none of them has the properties an input object has.
fn is_tag(v: &Value) -> bool {
    matches!(v, Value::Object(map) if map.len() == 1
        && ["$date", "$undefined", "$bigint", "$num"].iter().any(|k| map.contains_key(*k)))
}

/// `input.<key>.trim()`: a TypeError for anything but a string.
fn string_field<'a>(input: Option<&'a Value>, key: &str) -> Result<&'a str, ActionError> {
    match field(input, key) {
        Some(Value::String(s)) => Ok(s),
        _ => Err(ActionError::thrown(format!("Cannot read properties of input.{key} (reading 'trim')"))),
    }
}

/// `patch.password`, as `saveMailSettings` reads it:
/// `Boolean(patch.password && patch.password.length > 0)` decides whether it
/// is written, and `encryptSecret` throws on anything but a string.
fn password_patch(v: Option<&Value>) -> PasswordPatch {
    if !js_truthy(v) {
        return PasswordPatch::Keep;
    }
    match v {
        Some(Value::String(s)) => PasswordPatch::Set(s.clone()),
        Some(Value::Array(items)) if !items.is_empty() => PasswordPatch::Unencryptable,
        Some(o @ Value::Object(map)) if !is_tag(o) => {
            let positive = match map.get("length") {
                Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n > 0.0),
                Some(Value::String(s)) => js_number(s) > 0.0,
                Some(Value::Bool(b)) => *b,
                _ => false,
            };
            if positive { PasswordPatch::Unencryptable } else { PasswordPatch::Keep }
        }
        // A number, `true`, a Date or a bigint: `.length` is undefined.
        _ => PasswordPatch::Keep,
    }
}

/// `saveSmtpSettings(input)`
async fn save_smtp_settings(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_admin().await?;
    let input = ctx.args.raw(0);

    let host = js_trim(string_field(input, "host")?).to_string();
    let from = js_trim(string_field(input, "from")?).to_string();
    if !host_pattern(&host) {
        return Ok(json!({ "ok": false, "reason": "invalid-host" }));
    }
    if from.is_empty() || !email_pattern(&from) {
        return Ok(json!({ "ok": false, "reason": "invalid-from" }));
    }
    // `Number.isFinite(input.port)`: only a real number counts.
    let port = match field(input, "port") {
        Some(Value::Number(n)) => n.as_f64().filter(|p| p.is_finite() && (1.0..=65535.0).contains(p)),
        _ => None,
    };
    let Some(port) = port else {
        return Ok(json!({ "ok": false, "reason": "invalid-port" }));
    };

    // A user name with no password anywhere is a block that can never send,
    // and saving one is how "configure here" on an environment-backed install
    // used to silently kill working mail: the form seeds the host and the user
    // from the environment but NEVER the password (it is write-only), so an
    // operator who clicks straight through arrives at panel rows with a user
    // and no credential. Refusing here keeps the environment configuration in
    // charge until a real password has been typed.
    let current = load_mail_settings(&ctx.state, true).await?;
    let user_raw = string_field(input, "user")?;
    let user = js_trim(user_raw);
    // The stored password survives a save only while it still belongs to the
    // same server and account: `saveMailSettings` drops it when either
    // changes, so "there is one stored" is not on its own a reason to believe
    // there will be one afterwards.
    let identity_unchanged = current.source == MailSource::Panel && current.host == host && current.user == user;
    let will_have_password =
        js_truthy(field(input, "password")) || (identity_unchanged && current.password_state == MailPasswordState::Set);
    if !user.is_empty() && !will_have_password {
        return Ok(json!({ "ok": false, "reason": "password-required" }));
    }

    // `patch.fromName.trim()` throws inside saveMailSettings before any write.
    let from_name = string_field(input, "fromName")?;
    let patch = MailSettingsPatch {
        host,
        port: port.trunc(),
        security: match field(input, "security") {
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        },
        user: user_raw.to_string(),
        password: password_patch(field(input, "password")),
        from,
        from_name: from_name.to_string(),
    };
    let (changed, password_changed) = save_mail_settings(&ctx.state, &patch).await?;

    // Names only. The values include a credential.
    let detail = detail(
        &session.user.id,
        &session.user.email,
        vec![("changed", json!(changed)), ("passwordChanged", Value::Bool(password_changed))],
    );
    audit(&ctx, &session.user.id, "mail.settings_changed", detail).await;

    ctx.revalidate_path("/dashboard/admin");
    let resolved = load_mail_settings(&ctx.state, true).await?;
    crate::rpc::ok(&json!({
        "ok": true,
        "status": to_mail_status(&resolved, &ctx.state.config),
        "changed": changed,
    }))
}

/// `clearSmtpSettings()`: hand the configuration back to the environment by
/// deleting the panel rows.
async fn clear_smtp_settings(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_admin().await?;
    clear_mail_settings(&ctx.state).await?;
    audit(&ctx, &session.user.id, "mail.settings_cleared", detail(&session.user.id, &session.user.email, vec![])).await;
    ctx.revalidate_path("/dashboard/admin");
    fresh_status(&ctx).await
}

/// `clearSmtpPassword()`: forget the stored password, keeping the rest of
/// the configuration.
async fn clear_smtp_password(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_admin().await?;
    clear_mail_password(&ctx.state).await?;
    audit(&ctx, &session.user.id, "mail.password_cleared", detail(&session.user.id, &session.user.email, vec![])).await;
    ctx.revalidate_path("/dashboard/admin");
    fresh_status(&ctx).await
}

/// `setVerificationRequired(required)`: turn address confirmation on or off
/// without touching the SMTP block.
async fn set_verification_required(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_admin().await?;
    let required = ctx.args.raw(0);
    set_email_verification_required(&ctx.state, js_truthy(required)).await?;
    // `required` as JSON.stringify writes the raw argument.
    match required.map(json_of).transpose() {
        Ok(value) => {
            let more = value.flatten().map(|v| vec![("required", v)]).unwrap_or_default();
            let detail = detail(&session.user.id, &session.user.email, more);
            audit(&ctx, &session.user.id, "mail.verification_toggled", detail).await;
        }
        Err(_) => {
            tracing::error!("[audit] could not record mail.verification_toggled: Do not know how to serialize a BigInt")
        }
    }
    ctx.revalidate_path("/dashboard/admin");
    fresh_status(&ctx).await
}

// ── The test button ─────────────────────────────────────────────────────────

/// Map nodemailer's error vocabulary onto something an operator can act on.
pub fn classify(failure: &SmtpFailure) -> (&'static str, Option<f64>) {
    let code = failure.code.unwrap_or("");
    let smtp_code = failure.response_code;
    let is = |n: f64| smtp_code == Some(n);
    let kind = if code == "EAUTH" || is(535.0) || is(534.0) {
        "auth"
    } else if code == "EENVELOPE" || is(550.0) || is(553.0) || is(554.0) {
        "recipient"
    } else if code == "ETLS" || code == "ESOCKET" {
        "tls"
    } else if code == "ECONNECTION" || code == "ETIMEDOUT" || code == "EDNS" {
        "connection"
    } else {
        "unknown"
    };
    (kind, smtp_code)
}

/// `s.slice(0, n)` in UTF-16 code units (a split surrogate pair keeps its
/// high half in JavaScript; here it becomes U+FFFD).
fn js_slice(s: &str, n: usize) -> String {
    let mut out = String::new();
    let mut units = 0;
    for c in s.chars() {
        let w = c.len_utf16();
        if units + w > n {
            if units < n {
                out.push('\u{FFFD}');
            }
            break;
        }
        out.push(c);
        units += w;
    }
    out
}

fn now_ms() -> i64 {
    crate::db::types::JsDate::now().millis()
}

/// `(to ?? "").trim()`
fn test_recipient(to: Option<&Value>) -> Result<String, ActionError> {
    match to {
        None | Some(Value::Null) => Ok(String::new()),
        Some(v) if crate::rpc::tagged::is_undefined(v) => Ok(String::new()),
        Some(Value::String(s)) => Ok(js_trim(s).to_string()),
        Some(_) => Err(ActionError::thrown("(to ?? \"\").trim is not a function")),
    }
}

/// `sendTestMail(to?)`
///
/// Sends a real message rather than calling `transporter.verify()`.
/// `verify()` proves the connection and the credentials, and stops exactly
/// one step before the thing that most often goes wrong: whether the server
/// will accept an envelope from the configured From address. "Relay access
/// denied" and "sender address rejected" are invisible to `verify()` and fatal
/// in practice, so the test has to put a message on the wire.
async fn send_test_mail(ctx: ActionCtx) -> ActionResult {
    let session = ctx.require_admin().await?;
    let started = now_ms();

    // This button opens an outbound connection to whatever host:port the panel
    // currently names, so it is a small port scanner if left unthrottled.
    if !check_rate_limit(&format!("mail-test:{}", session.user.id), 5, 15 * 60_000).allowed {
        return Ok(json!({ "ok": false, "kind": "rate-limited", "tookMs": 0 }));
    }

    // Read through, never from the snapshot: the operator has just pressed Save.
    invalidate_mail_settings();
    let Some((transport, mail)) = get_transport(&ctx.state).await? else {
        return Ok(json!({ "ok": false, "kind": "not-configured", "tookMs": now_ms() - started }));
    };

    let to = test_recipient(ctx.args.raw(0))?;
    let recipient = if to.is_empty() { session.user.email.clone() } else { to };
    let accept_language = ctx.headers.get("accept-language").and_then(|v| v.to_str().ok());
    let locale = pick_mail_locale(session.user.locale.as_deref(), accept_language);
    let texts = MailTexts::from_messages(&ctx.state.messages, &ctx.state.config.app_name);
    let message = render_smtp_test(&texts, locale, &mail.host, mail.port, &mail.from);
    let secrets = smtp_secret_forms(&mail.user, &mail.password);

    // Deliberately the transport itself, not `send_mail`: this path needs the
    // transport's own error to classify the failure.
    let sent =
        send_with_transport(&ctx.state, &transport, &mail, &MailMessage::from_rendered(&recipient, &message)).await;
    let by = |more: Vec<(&str, Value)>| detail(&session.user.id, &session.user.email, more);
    match sent {
        Ok(info) => {
            let took_ms = now_ms() - started;
            let detail = by(vec![("to", json!(recipient)), ("ok", json!(true))]);
            audit(&ctx, &session.user.id, "mail.test_sent", detail).await;
            Ok(json!({ "ok": true, "accepted": info.accepted, "tookMs": took_ms }))
        }
        Err(failure) => {
            let (kind, smtp_code) = classify(&failure);
            let took_ms = now_ms() - started;

            // The server's own words are the whole diagnostic value of this
            // button ("535 5.7.8 Username and Password not accepted" answers
            // the question on its own), but they are also where an echoed AUTH
            // line would appear.
            let raw = failure.response.as_deref().filter(|r| !r.is_empty()).unwrap_or(&failure.message);
            let server_says = js_slice(&redact_secrets(raw, &secrets), 400);

            let mut more = vec![("to", json!(recipient)), ("ok", json!(false)), ("kind", json!(kind))];
            if let Some(code) = smtp_code {
                more.push(("smtpCode", serde_json::to_value(Num(code))?));
            }
            audit(&ctx, &session.user.id, "mail.test_sent", by(more)).await;

            let smtp_code = match smtp_code {
                Some(code) => serde_json::to_value(Num(code))?,
                None => crate::rpc::tagged::to_tagged_value(&Undefined)?,
            };
            Ok(json!({
                "ok": false,
                "kind": kind,
                "smtpCode": smtp_code,
                "serverSays": server_says,
                "tookMs": took_ms,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(code: Option<&'static str>, response_code: Option<f64>) -> SmtpFailure {
        SmtpFailure { code, message: String::new(), response: None, response_code, command: None }
    }

    #[test]
    fn classifies_like_upstream() {
        assert_eq!(classify(&failure(Some("EAUTH"), None)).0, "auth");
        assert_eq!(classify(&failure(Some("EMESSAGE"), Some(535.0))).0, "auth");
        assert_eq!(classify(&failure(Some("EENVELOPE"), Some(550.0))).0, "recipient");
        assert_eq!(classify(&failure(Some("EPROTOCOL"), Some(554.0))).0, "recipient");
        // nodemailer labels every socket error ESOCKET, a refused connection
        // included: upstream reports it as a TLS problem.
        assert_eq!(classify(&failure(Some("ESOCKET"), None)).0, "tls");
        assert_eq!(classify(&failure(Some("ETLS"), Some(454.0))), ("tls", Some(454.0)));
        assert_eq!(classify(&failure(Some("ETIMEDOUT"), None)).0, "connection");
        assert_eq!(classify(&failure(Some("EDNS"), None)).0, "connection");
        assert_eq!(classify(&failure(Some("ECONNECTION"), Some(421.0))).0, "connection");
        assert_eq!(classify(&failure(None, None)).0, "unknown");
    }

    #[test]
    fn validates_host_and_sender_like_the_upstream_patterns() {
        for ok in ["smtp.example.com", "10.0.0.1", "[::1]".trim_matches(|c| c == '[' || c == ']'), "MAIL_HOST-1"] {
            assert!(host_pattern(ok), "{ok}");
        }
        for bad in ["", "smtp example", "smtp/example", "smtp@example", "ex\u{e4}mple"] {
            assert!(!host_pattern(bad), "{bad}");
        }
        for ok in ["a@b.c", "no-reply@mail.example.com", "a@b..c", "\"x\"@b.c"] {
            assert!(email_pattern(ok), "{ok}");
        }
        for bad in ["a@b", "a@.c", "a@b.", "@b.c", "a b@c.d", "a@b@c.d", "a@b.c\u{00a0}"] {
            assert!(!email_pattern(bad), "{bad}");
        }
    }

    #[test]
    fn password_values_follow_the_javascript_checks() {
        assert_eq!(password_patch(None), PasswordPatch::Keep);
        assert_eq!(password_patch(Some(&json!(""))), PasswordPatch::Keep);
        assert_eq!(password_patch(Some(&json!("pw"))), PasswordPatch::Set("pw".into()));
        assert_eq!(password_patch(Some(&json!(5))), PasswordPatch::Keep);
        assert_eq!(password_patch(Some(&json!(true))), PasswordPatch::Keep);
        assert_eq!(password_patch(Some(&json!([]))), PasswordPatch::Keep);
        assert_eq!(password_patch(Some(&json!(["x"]))), PasswordPatch::Unencryptable);
        assert_eq!(password_patch(Some(&json!({ "length": 2 }))), PasswordPatch::Unencryptable);
        assert_eq!(password_patch(Some(&json!({}))), PasswordPatch::Keep);
        assert_eq!(password_patch(Some(&json!({ "$date": "2026-10-18T12:00:00.000Z" }))), PasswordPatch::Keep);
    }

    #[test]
    fn inputs_are_read_like_property_accesses() {
        let input = json!({ "host": " smtp.example.com ", "port": 587 });
        assert_eq!(string_field(Some(&input), "host").unwrap(), " smtp.example.com ");
        assert!(string_field(Some(&input), "from").is_err());
        assert!(string_field(Some(&json!("host")), "host").is_err());
        assert!(string_field(None, "host").is_err());
        assert!(field(Some(&json!({ "$undefined": 1 })), "host").is_none());
        assert_eq!(test_recipient(Some(&json!("  a@b.c "))).unwrap(), "a@b.c");
        assert_eq!(test_recipient(None).unwrap(), "");
        assert!(test_recipient(Some(&json!(1))).is_err());
    }

    #[test]
    fn slices_server_text_in_utf16_units() {
        assert_eq!(js_slice("abc", 2), "ab");
        assert_eq!(js_slice("a\u{1F600}", 2), "a\u{FFFD}");
        assert_eq!(js_slice("a\u{1F600}", 3), "a\u{1F600}");
    }
}
