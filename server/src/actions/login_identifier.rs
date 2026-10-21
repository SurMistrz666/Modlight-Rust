//! Port of `app/actions/login-identifier.ts`: sign in with a username OR an
//! email.
//!
//! People know their account as `alicja`, `bobby` or `admin` far more often
//! than as the address they registered with years ago. Better Auth signs in by
//! email only, so the username has to be resolved first.
//!
//! That resolution happens HERE, on the server, and the email never travels
//! back to the browser. An earlier version returned it to the client so the
//! client could call sign-in itself; that turned an unauthenticated Server
//! Action into two gifts at once: a directory of every account's email
//! address, and a reliable "does this username exist?" oracle (an answer
//! containing `@` meant yes). Server Actions are callable directly with a
//! `Next-Action` header, so a form was never needed to farm it.
//!
//! Now the caller learns exactly one thing: whether these credentials work.
//!
//! Not upstream: with Cloudflare Turnstile on (src/turnstile.rs) the request
//! must also carry a token Cloudflare accepts (`x-captcha-response`), else
//! `{status: "captcha"}`, or `{status: "captcha-unavailable"}` when Cloudflare
//! cannot be asked.

use serde_json::{Value, json};
use sqlx::PgPool;

use crate::actions::account::js::{Js, js};
use crate::auth::server::{ServerCallError, call_endpoint};
use crate::password::check_rate_limit;
use crate::rpc::{ActionCtx, ActionError, ActionResult, RpcRegistry};
use crate::turnstile::Verdict;

pub fn register(reg: &mut RpcRegistry) {
    reg.register("login-identifier", "signInWithIdentifier", sign_in_with_identifier);
}

/// Better Auth serialises an APIError as `{ message, code }`.
const EMAIL_NOT_VERIFIED: &str = "EMAIL_NOT_VERIFIED";

fn status(s: &str) -> ActionResult {
    Ok(json!({ "status": s }))
}

/// Look up the email behind a username. Never leaves the server.
///
/// The two columns are NOT equal in standing, and treating them as one `OR`
/// with `LIMIT 1` was a real hazard: `username` is unique and case-folded on
/// write (app/actions/admin.ts), while `name` is a free-text display name that
/// anybody can set to anything, including somebody else's username. A single
/// OR let the database pick either row arbitrarily, so setting your display
/// name to `bobby` gave you a coin-flip's chance of standing between bobby and
/// their account, and a certainty of it if the planner happened to prefer
/// your row.
///
/// So: the unique column decides whenever it can, and the legacy one is
/// consulted only when it answers unambiguously.
pub async fn resolve_email(db: &PgPool, identifier: &str) -> sqlx::Result<String> {
    if identifier.contains('@') {
        return Ok(identifier.to_string());
    }
    let needle = identifier.to_lowercase();

    let by_username: Option<String> =
        sqlx::query_scalar(r#"SELECT "email" FROM "user" WHERE lower("user"."username") = $1 LIMIT 1"#)
            .bind(&needle)
            .fetch_optional(db)
            .await?;
    if let Some(email) = by_username {
        return Ok(email);
    }

    // Accounts created before the username column existed carry their login
    // in `name`. Two rows means the name identifies nobody, and guessing
    // between them would hand one account's login attempts to another.
    let by_name: Vec<String> =
        sqlx::query_scalar(r#"SELECT "email" FROM "user" WHERE lower("user"."name") = $1 LIMIT 2"#)
            .bind(&needle)
            .fetch_all(db)
            .await?;
    if by_name.len() == 1 {
        return Ok(by_name.into_iter().next().unwrap_or_default());
    }

    // Unknown or ambiguous falls through unchanged: sign-in then fails with
    // the same generic result as a wrong password, so there is nothing to
    // distinguish.
    Ok(identifier.to_string())
}

async fn sign_in_with_identifier(ctx: ActionCtx) -> ActionResult {
    let raw = match js(ctx.args.raw(0)) {
        Js::Str(s) => crate::config::js_trim(s).to_string(),
        Js::Undefined | Js::Null => {
            return Err(ActionError::thrown("TypeError: Cannot read properties of undefined (reading 'trim')"));
        }
        _ => return Err(ActionError::thrown("TypeError: identifier.trim is not a function")),
    };
    let password = js(ctx.args.raw(1));
    if raw.is_empty() || !password.truthy() {
        return status("invalid");
    }

    // Better Auth's own limiter only covers /api/auth/*; a Server Action is a
    // different door into the same lock, so it needs its own bolt.
    let ip = ctx.client_ip();
    if !check_rate_limit(&format!("{ip}|signin"), 10, 15 * 60 * 1000).allowed {
        return status("rate-limited");
    }

    // Not upstream: the optional Cloudflare Turnstile check (src/turnstile.rs),
    // after the throttle so a flood never reaches Cloudflare, before anything
    // is looked up. The sign-in below is a server-side call, which the check
    // on /api/auth/sign-in/email does not see: this token is spent once.
    match crate::turnstile::check_action(&ctx).await {
        Verdict::Off | Verdict::Passed => {}
        Verdict::Unavailable => return status("captcha-unavailable"),
        Verdict::Missing | Verdict::Failed => return status("captcha"),
    }

    let email = match resolve_email(&ctx.state.db, &raw).await {
        Ok(email) => email,
        Err(err) => {
            tracing::error!("[auth] sign-in failed: {err}");
            return status("error");
        }
    };

    // asResponse so Better Auth's Set-Cookie headers are produced and applied
    // by the framework exactly as they are on the HTTP route. The password
    // goes to the endpoint as the caller sent it: anything but a string fails
    // its body schema (`password: z.string()`), an APIError, so "invalid".
    let password = match password {
        Js::Str(s) => Value::String(s.to_string()),
        _ => Value::Null,
    };
    let body = json!({ "email": email, "password": password });
    let call = call_endpoint(&ctx.state, Some(&ctx.headers), ctx.peer, "/sign-in/email", Some(body), None).await;
    call.apply_cookies(&ctx);

    match call.json() {
        Ok(data) => {
            if crate::auth::validate::js_truthy(data.get("twoFactorRedirect")) {
                status("two-factor")
            } else {
                status("ok")
            }
        }
        Err(ServerCallError::Api(_)) => {
            if call.error_code().as_deref() == Some(EMAIL_NOT_VERIFIED) {
                status("unverified")
            } else {
                status("invalid")
            }
        }
        Err(ServerCallError::Other(message)) => {
            // Bad credentials arrive as an APIError; anything else is a real
            // fault and must not be reported as "wrong password".
            tracing::error!("[auth] sign-in failed: {message}");
            status("error")
        }
    }
}
