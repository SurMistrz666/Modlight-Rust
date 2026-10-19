//! E-mail verification and password-reset mail: Better Auth's token side
//! (`api/routes/email-verification.mjs` `createEmailVerificationToken`,
//! `sendVerificationEmailFn`) and lib/auth.ts's two mail callbacks
//! (`emailVerification.sendVerificationEmail`,
//! `emailAndPassword.sendResetPassword`).
//!
//! The verification link is `<APP_URL>/api/auth/verify-email?token=<JWT>&callbackURL=<enc>`;
//! the JWT is HS256 over AUTH_SECRET with `{ email, iat, exp }` (1 hour), or
//! `{ email, updateTo, requestType }` for e-mail changes.
//!
//! The mail language follows lib/auth.ts `localeFor(user, request)`: the
//! account's own `locale` when it is "pl" or "en", else the request's
//! Accept-Language, which exists only for calls that came over HTTP (a
//! server-side `auth.api.*` call has no Request, so it is English unless the
//! account says otherwise).

use serde_json::{Map, Value};

use super::endpoint::EndpointCtx;
use super::error::EndpointError;
use crate::db::models::User;
use crate::db::types::JsDate;
use crate::http::cookies::encode_uri_component;
use crate::mail::{MailMessage, send_mail};
use crate::mail_templates::{MailTexts, pick_mail_locale, render_reset_password, render_verify_email};

/// `emailVerification.expiresIn` default.
pub const VERIFICATION_EXPIRES_IN: i64 = 3600;

/// `createEmailVerificationToken(secret, email, updateTo, expiresIn, extra)`.
pub fn create_email_verification_token(
    secret: &str,
    email: &str,
    update_to: Option<&str>,
    extra: Option<(&str, &str)>,
) -> String {
    let mut payload = Map::new();
    payload.insert("email".into(), Value::String(email.to_lowercase()));
    if let Some(u) = update_to {
        payload.insert("updateTo".into(), Value::String(u.to_lowercase()));
    }
    if let Some((k, v)) = extra {
        payload.insert(k.into(), Value::String(v.to_string()));
    }
    super::crypto::sign_jwt(&payload, secret, VERIFICATION_EXPIRES_IN, JsDate::now().millis())
}

/// The language of a mail to `user` (`localeFor(user, request)`).
fn locale_for(ctx: &EndpointCtx, user: &User) -> crate::mail_templates::MailLocale {
    let accept = if ctx.is_request {
        ctx.headers.get("accept-language").and_then(|v| v.to_str().ok()).map(str::to_string)
    } else {
        None
    };
    pick_mail_locale(user.locale.as_deref(), accept.as_deref())
}

fn texts(ctx: &EndpointCtx) -> MailTexts {
    MailTexts::from_messages(&ctx.state.messages, &ctx.state.config.app_name)
}

/// lib/auth.ts `emailVerification.sendVerificationEmail`: nothing to send
/// when mail is off (verification is then not required either).
pub async fn send_verification_mail(ctx: &EndpointCtx, user: &User, url: &str) -> Result<(), EndpointError> {
    if !crate::mail_settings::mail_ready(&ctx.state).await? {
        return Ok(());
    }
    let mail = render_verify_email(&texts(ctx), locale_for(ctx, user), &user.email, url);
    send_mail(&ctx.state, &MailMessage::from_rendered(&user.email, &mail)).await?;
    Ok(())
}

/// `sendVerificationEmailFn(ctx, user)` with the callback URL the caller
/// resolved (`ctx.body.callbackURL` or "/").
pub async fn send_verification_email_fn(
    ctx: &EndpointCtx,
    user: &User,
    callback_url: &str,
) -> Result<(), EndpointError> {
    let token = create_email_verification_token(ctx.secret(), &user.email, None, None);
    let url =
        format!("{}/verify-email?token={token}&callbackURL={}", ctx.base_url(), encode_uri_component(callback_url));
    send_verification_mail(ctx, user, &url).await
}

/// lib/auth.ts `emailAndPassword.sendResetPassword`. No `mailReady()` guard
/// on purpose: a reset that cannot be sent must fail rather than report
/// success (and /forgot does not render its form when mail is off).
pub async fn send_reset_password_mail(ctx: &EndpointCtx, user: &User, url: &str) -> Result<(), EndpointError> {
    let mail = render_reset_password(&texts(ctx), locale_for(ctx, user), &user.email, url);
    send_mail(&ctx.state, &MailMessage::from_rendered(&user.email, &mail)).await?;
    Ok(())
}
