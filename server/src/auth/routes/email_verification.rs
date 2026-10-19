//! `api/routes/email-verification.mjs`: `/send-verification-email` and
//! `/verify-email` (the link in the mail).
//!
//! lib/auth.ts turns on `autoSignInAfterVerification`: following a valid link
//! confirms the address and signs the browser in. Modlight never issues
//! e-mail-change tokens (`user.changeEmail` is off), but the `updateTo`
//! branches are ported as upstream has them, for links minted elsewhere with
//! the same secret.

use axum::http::Method;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use super::set_session_cookie;
use super::sign_in::banned_error;
use crate::auth::adapter::{self, CreateSessionError, SqlValue};
use crate::auth::crypto::{JwtError, verify_jwt};
use crate::auth::email::{create_email_verification_token, send_verification_email_fn, send_verification_mail};
use crate::auth::endpoint::{Endpoint, EndpointCtx, HandlerResult, Middleware};
use crate::auth::error::{ApiError, EndpointError, ErrorCode, base};
use crate::auth::session::{AuthSession, UserOut};
use crate::auth::validate::{Schema, is_zod_email};
use crate::db::types::JsDate;
use crate::http::cookies::encode_uri_component;

fn send_body() -> Schema {
    Schema::object(vec![("email", Schema::Email), ("callbackURL", Schema::string().optional())])
}

fn verify_query() -> Schema {
    Schema::object(vec![("token", Schema::string()), ("callbackURL", Schema::string().optional())])
}

pub fn endpoints() -> Vec<Endpoint> {
    vec![
        Endpoint {
            path: "/send-verification-email",
            methods: &[Method::POST],
            body: Some(send_body),
            query: None,
            media_types: &[],
            middleware: &[],
            handler: send_verification_email,
        },
        Endpoint {
            path: "/verify-email",
            methods: &[Method::GET],
            body: None,
            query: Some(verify_query),
            media_types: &[],
            middleware: &[Middleware::OriginCheckQueryCallback],
            handler: verify_email,
        },
    ]
}

fn send_verification_email(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let email = ctx.body_str("email").unwrap_or_default().to_string();
        let callback = ctx.body_str("callbackURL").filter(|c| !c.is_empty()).unwrap_or("/").to_string();
        let Some(session) = ctx.session_from_ctx().await else {
            let user = adapter::find_user_by_email(ctx.db(), &email).await?;
            match user {
                Some(user) if !user.email_verified => {
                    send_verification_email_fn(ctx, &user, &callback).await?;
                }
                _ => {
                    // Same work as the real branch, nothing sent.
                    let _ = create_email_verification_token(ctx.secret(), &email, None, None);
                }
            }
            return ctx.json(json!({ "status": true }));
        };
        if session.user.email.to_lowercase() != email.to_lowercase() {
            return Err(ApiError::from("BAD_REQUEST", base::EMAIL_MISMATCH).into());
        }
        if session.user.email_verified {
            return Err(ApiError::from("BAD_REQUEST", base::EMAIL_ALREADY_VERIFIED).into());
        }
        send_verification_email_fn(ctx, &session.user, &callback).await?;
        ctx.json(json!({ "status": true }))
    })
}

/// `redirectOnError(error)`: to the callback with `error=<code>`, or a 401.
fn redirect_on_error(ctx: &mut EndpointCtx, error: ErrorCode) -> EndpointError {
    if let Some(cb) = ctx.query_str("callbackURL").filter(|c| !c.is_empty()).map(str::to_string) {
        let sep = if cb.contains('?') { '&' } else { '?' };
        return ctx.redirect(format!("{cb}{sep}error={}", error.code));
    }
    ApiError::from("UNAUTHORIZED", error).into()
}

fn verify_email(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let token = ctx.query_str("token").unwrap_or_default().to_string();
        let payload = match verify_jwt(&token, ctx.secret(), JsDate::now().millis()) {
            Ok(p) => p,
            Err(JwtError::Expired) => return Err(redirect_on_error(ctx, base::TOKEN_EXPIRED)),
            Err(_) => return Err(redirect_on_error(ctx, base::INVALID_TOKEN)),
        };
        // z.object({ email: z.email(), updateTo: z.string().optional(), requestType: ... }).parse(payload):
        // a payload of the wrong shape throws a ZodError (an internal error).
        let email = match payload.get("email") {
            Some(Value::String(e)) if is_zod_email(e) => e.clone(),
            _ => return Err(EndpointError::Internal(anyhow::anyhow!("invalid verification token payload"))),
        };
        let update_to = match payload.get("updateTo") {
            None => None,
            Some(Value::String(u)) => Some(u.clone()),
            Some(_) => return Err(EndpointError::Internal(anyhow::anyhow!("invalid updateTo"))),
        };
        let request_type = match payload.get("requestType") {
            None => None,
            Some(Value::String(r)) => Some(r.clone()),
            Some(_) => return Err(EndpointError::Internal(anyhow::anyhow!("invalid requestType"))),
        };
        let Some(user) = adapter::find_user_by_email(ctx.db(), &email).await? else {
            return Err(redirect_on_error(ctx, base::USER_NOT_FOUND));
        };
        let callback = ctx.query_str("callbackURL").filter(|c| !c.is_empty()).map(str::to_string);

        if let Some(update_to) = update_to.filter(|u| !u.is_empty()) {
            let session = ctx.session_from_ctx().await;
            if session.as_ref().is_some_and(|s| s.user.email != email) {
                return Err(redirect_on_error(ctx, base::INVALID_USER));
            }
            match request_type.as_deref() {
                Some("change-email-confirmation") => {
                    let new_token = create_email_verification_token(
                        ctx.secret(),
                        &email,
                        Some(&update_to),
                        Some(("requestType", "change-email-verification")),
                    );
                    let cb = encode_uri_component(callback.as_deref().unwrap_or("/"));
                    let url = format!("{}/verify-email?token={new_token}&callbackURL={cb}", ctx.base_url());
                    let mut target = user.clone();
                    target.email = update_to.clone();
                    send_verification_mail(ctx, &target, &url).await?;
                    if let Some(cb) = callback {
                        return Err(ctx.redirect(cb));
                    }
                    return ctx.json(json!({ "status": true }));
                }
                Some("change-email-verification") => {
                    let active = match session {
                        Some(s) => s,
                        None => new_session_for(ctx, &user).await?,
                    };
                    let updated = adapter::update_user_by_email(
                        ctx.db(),
                        &email,
                        &[("email", SqlValue::Text(update_to.clone())), ("emailVerified", SqlValue::Bool(true))],
                    )
                    .await?;
                    let mut cookie_user = active.user.clone();
                    cookie_user.email = update_to.clone();
                    cookie_user.email_verified = true;
                    set_session_cookie(ctx, AuthSession { session: active.session, user: cookie_user }, None, None);
                    if let Some(cb) = callback {
                        return Err(ctx.redirect(cb));
                    }
                    let user_out =
                        updated.as_ref().map(|u| serde_json::to_value(UserOut::from(u)).unwrap_or(Value::Null));
                    return ctx.json(json!({ "status": true, "user": user_out }));
                }
                _ => {
                    let active = match session {
                        Some(s) => s,
                        None => new_session_for(ctx, &user).await?,
                    };
                    let updated = adapter::update_user_by_email(
                        ctx.db(),
                        &email,
                        &[("email", SqlValue::Text(update_to.clone())), ("emailVerified", SqlValue::Bool(false))],
                    )
                    .await?;
                    let new_token = create_email_verification_token(ctx.secret(), &update_to, None, None);
                    let cb = encode_uri_component(callback.as_deref().unwrap_or("/"));
                    let url = format!("{}/verify-email?token={new_token}&callbackURL={cb}", ctx.base_url());
                    if let Some(u) = &updated {
                        send_verification_mail(ctx, u, &url).await?;
                    }
                    let mut cookie_user = active.user.clone();
                    cookie_user.email = update_to.clone();
                    cookie_user.email_verified = false;
                    set_session_cookie(ctx, AuthSession { session: active.session, user: cookie_user }, None, None);
                    if let Some(cb) = callback {
                        return Err(ctx.redirect(cb));
                    }
                    let user_out =
                        updated.as_ref().map(|u| serde_json::to_value(UserOut::from(u)).unwrap_or(Value::Null));
                    return ctx.json(json!({ "status": true, "user": user_out }));
                }
            }
        }

        if user.email_verified {
            if let Some(cb) = callback {
                return Err(ctx.redirect(cb));
            }
            return ctx.json(json!({ "status": true, "user": null }));
        }
        adapter::update_user_by_email(ctx.db(), &email, &[("emailVerified", SqlValue::Bool(true))]).await?;
        // autoSignInAfterVerification
        let current = ctx.session_from_ctx().await;
        match current {
            Some(current) if current.user.email == email => {
                let mut u = current.user.clone();
                u.email_verified = true;
                set_session_cookie(ctx, AuthSession { session: current.session, user: u }, None, None);
            }
            _ => {
                let mut verified = user.clone();
                verified.email_verified = true;
                let created = new_session_for(ctx, &verified).await?;
                set_session_cookie(ctx, created, None, None);
            }
        }
        if let Some(cb) = callback {
            return Err(ctx.redirect(cb));
        }
        ctx.json(json!({ "status": true, "user": null }))
    })
}

/// `internalAdapter.createSession(user.id)` for a link-follower without a
/// session (failure: 500 FAILED_TO_CREATE_SESSION; a banned account: 403).
async fn new_session_for(ctx: &EndpointCtx, user: &crate::db::models::User) -> Result<AuthSession, EndpointError> {
    let ip = ctx.ip().unwrap_or_default();
    let ua = ctx.user_agent();
    match adapter::create_session(ctx.db(), &user.id, false, &ip, &ua, None, false).await {
        Ok(session) => Ok(AuthSession { session, user: user.clone() }),
        Err(CreateSessionError::Banned) => Err(banned_error()),
        Err(CreateSessionError::Db(e)) => Err(e.into()),
    }
}
