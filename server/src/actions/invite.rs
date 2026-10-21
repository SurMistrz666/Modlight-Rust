//! Port of `app/actions/invite.ts`: the public half of the invitation flow,
//! plus the "send it again" button for address confirmation.
//!
//! Both are unauthenticated by necessity: the person using them cannot sign
//! in yet. They are therefore throttled by caller, and neither reveals whether
//! an account exists.
//!
//! Arguments arrive as tagged JSON from any caller, so where upstream would
//! throw on a wrongly-typed one (`email.trim()` on a number, `password.length`
//! on undefined, `createHash().update()` on anything but a string) the action
//! throws too, at the same point, after the same side effects.

use serde::Serialize;
use serde_json::{Value, json};

use crate::actions::account::js::{Js, js};
use crate::auth::server::{send_verification_email, sign_in_email};
use crate::auth_helpers::headers_without_session;
use crate::invites::{AcceptInviteResult, INVITE_PASSWORD_MIN, accept_invite, read_invite};
use crate::mail_settings::mail_ready;
use crate::password::check_rate_limit;
use crate::rpc::{ActionCtx, ActionError, ActionResult, RpcRegistry};
use crate::turnstile::Verdict;

pub fn register(reg: &mut RpcRegistry) {
    reg.register("invite", "previewInvite", preview_invite);
    reg.register("invite", "acceptInvitation", accept_invitation);
    reg.register("invite", "resendVerification", resend_verification);
}

/// `InvitePreview`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InvitePreview {
    pub email: String,
    pub name: String,
}

/// The TypeError a JavaScript property read or method call on a value of the
/// wrong type throws (the message never reaches the client in production).
fn type_error(what: &str) -> ActionError {
    ActionError::thrown(format!("TypeError: {what}"))
}

/// The token as `readInvite(token)` consumes it: a falsy value is "no
/// invitation"; any other non-string makes `createHash().update()` throw.
fn invite_token(v: Js<'_>) -> Result<Option<&str>, ActionError> {
    match v {
        Js::Str(s) if !s.is_empty() => Ok(Some(s)),
        other if !other.truthy() => Ok(None),
        _ => Err(type_error(
            "The \"data\" argument must be of type string or an instance of Buffer, TypedArray, or DataView.",
        )),
    }
}

/// `previewInvite(token)`: what the invitation page shows before a password
/// is chosen.
pub async fn preview_invite_for(ctx_db: &sqlx::PgPool, token: &str) -> sqlx::Result<Option<InvitePreview>> {
    Ok(read_invite(ctx_db, token).await?.map(|s| InvitePreview { email: s.email, name: s.name }))
}

async fn preview_invite(ctx: ActionCtx) -> ActionResult {
    let Some(token) = invite_token(js(ctx.args.raw(0)))? else {
        return Ok(Value::Null);
    };
    let preview = preview_invite_for(&ctx.state.db, token).await?;
    crate::rpc::ok(&preview)
}

fn refused(reason: &str) -> ActionResult {
    Ok(json!({ "ok": false, "reason": reason }))
}

/// `password.length` as JavaScript reads it (`None` = undefined); reading it
/// on null or undefined throws.
fn js_length(v: Js<'_>) -> Result<Option<f64>, ActionError> {
    Ok(match v {
        Js::Undefined | Js::Null => return Err(type_error("Cannot read properties of undefined (reading 'length')")),
        Js::Str(s) => Some(s.encode_utf16().count() as f64),
        Js::Array(items) => Some(items.len() as f64),
        Js::Object(_) => match v.prop("length") {
            Js::Undefined => None,
            other => Some(other.to_number()),
        },
        _ => None,
    })
}

/// Set the password an invitation was issued for, then sign the person in.
///
/// Not upstream: with Cloudflare Turnstile on, also `{ok: false, reason:
/// "captcha"}` (no token Cloudflare accepts) and `"captcha-unavailable"`.
///
/// Signing in here rather than redirecting to /login is what makes the flow
/// one step instead of three, and it is safe for the same reason the
/// invitation is: possession of the token proved control of the mailbox, and
/// the password was just chosen by whoever is holding the session.
async fn accept_invitation(ctx: ActionCtx) -> ActionResult {
    let gate = check_rate_limit(&format!("invite:{}", ctx.client_ip()), 10, 15 * 60_000);
    if !gate.allowed {
        return refused("rate-limited");
    }

    // Not upstream: the optional Cloudflare Turnstile check (src/turnstile.rs),
    // before the invitation is read (reading it may delete an expired one).
    match crate::turnstile::check_action(&ctx).await {
        Verdict::Off | Verdict::Passed => {}
        Verdict::Unavailable => return refused("captcha-unavailable"),
        Verdict::Missing | Verdict::Failed => return refused("captcha"),
    }

    let token = js(ctx.args.raw(0));
    let password = js(ctx.args.raw(1));
    // `password.length < INVITE_PASSWORD_MIN` (undefined < 8 is false).
    if js_length(password)?.is_some_and(|n| n < INVITE_PASSWORD_MIN as f64) {
        return refused("weak-password");
    }

    // acceptInvite(token, password): `!password || password.length < MIN`
    // first, then the token.
    if !password.truthy() {
        return refused("weak-password");
    }
    let result = match password {
        Js::Str(pw) => match invite_token(token)? {
            Some(t) => accept_invite(&ctx.state.db, t, pw).await?,
            None => AcceptInviteResult::Invalid,
        },
        _ => {
            // A truthy non-string password long enough to pass the checks:
            // readInvite runs (with its deletes), then hashing it throws.
            let subject = match invite_token(token)? {
                Some(t) => read_invite(&ctx.state.db, t).await?,
                None => None,
            };
            if subject.is_none() {
                return refused("invalid");
            }
            return Err(type_error("password.normalize is not a function"));
        }
    };
    let (email, password) = match (result, password) {
        (AcceptInviteResult::Ok { email, .. }, Js::Str(pw)) => (email, pw.to_string()),
        (AcceptInviteResult::WeakPassword, _) => return refused("weak-password"),
        _ => return refused("invalid"),
    };

    // nextCookies() is what turns the Set-Cookie of this call into a real
    // cookie on the action's response.
    let call = sign_in_email(&ctx.state, &ctx.headers, ctx.peer, &email, &password).await;
    call.apply_cookies(&ctx);
    if call.json().is_err() {
        // The password is set either way, so this is recoverable at /login.
        return refused("sign-in-failed");
    }
    Ok(json!({ "ok": true }))
}

/// Send the confirmation link again.
///
/// Always reports success once it has been allowed to try, whatever the
/// address turns out to be: an answer that differed for a known and an
/// unknown address would turn this into an account-enumeration endpoint.
async fn resend_verification(ctx: ActionCtx) -> ActionResult {
    let address = match js(ctx.args.raw(0)) {
        Js::Str(s) => crate::config::js_trim(s).to_lowercase(),
        Js::Undefined | Js::Null => return Err(type_error("Cannot read properties of undefined (reading 'trim')")),
        _ => return Err(type_error("email.trim is not a function")),
    };

    // Keyed on the address as well as the caller, so one mailbox cannot be
    // flooded from a rotating set of addresses.
    let by_ip = check_rate_limit(&format!("verify-resend-ip:{}", ctx.client_ip()), 5, 15 * 60_000);
    let by_address = check_rate_limit(&format!("verify-resend:{address}"), 3, 15 * 60_000);
    if !by_ip.allowed || !by_address.allowed {
        return refused("rate-limited");
    }

    if !mail_ready(&ctx.state).await? {
        return refused("mail-unavailable");
    }

    // Session-less by design. Normally there is no session here anyway (the
    // caller cannot sign in yet), but a signed-in visitor who lands on
    // /register would otherwise hit the "resend MY link" branch and get
    // EMAIL_MISMATCH for anybody else's address, swallowed below into a
    // silent success.
    let headers = headers_without_session(&ctx.headers);
    let call = send_verification_email(&ctx.state, &headers, ctx.peer, &address, "/dashboard").await;
    call.apply_cookies(&ctx);
    // Errors swallowed on purpose: "no such account" and "sent" must look
    // identical.
    Ok(json!({ "ok": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_as_read_invite_consumes_them() {
        assert_eq!(invite_token(Js::Str("abc")).unwrap(), Some("abc"));
        assert_eq!(invite_token(Js::Str("")).unwrap(), None);
        assert_eq!(invite_token(Js::Undefined).unwrap(), None);
        assert_eq!(invite_token(Js::Num(0.0)).unwrap(), None);
        assert!(invite_token(Js::Num(5.0)).is_err());
        assert!(invite_token(Js::Bool(true)).is_err());
    }

    #[test]
    fn password_length_as_javascript_reads_it() {
        assert_eq!(js_length(Js::Str("abc")).unwrap(), Some(3.0));
        assert_eq!(js_length(Js::Str("😀")).unwrap(), Some(2.0));
        assert_eq!(js_length(Js::Num(5.0)).unwrap(), None);
        assert!(js_length(Js::Undefined).is_err());
        assert!(js_length(Js::Null).is_err());
    }
}
