//! `api/routes/password.mjs`: `/request-password-reset`,
//! `/reset-password/:token` (the link in the mail), `/reset-password` and
//! `/verify-password`.
//!
//! A reset token is a `verification` row `reset-password:<24 x [A-Za-z0-9]>`
//! holding the user id, valid for an hour. Asking for a reset never says
//! whether the address exists. lib/auth.ts sets no `revokeSessionsOnPasswordReset`
//! and no `onPasswordReset`, so a reset leaves the account's sessions alone.

use axum::http::Method;
use futures_util::future::BoxFuture;
use serde_json::json;

use super::sign_up::{MAX_PASSWORD_LENGTH, MIN_PASSWORD_LENGTH};
use crate::auth::adapter;
use crate::auth::crypto::generate_id;
use crate::auth::endpoint::{Endpoint, EndpointCtx, HandlerResult, Middleware};
use crate::auth::error::{ApiError, EndpointError, base};
use crate::auth::validate::Schema;
use crate::db::types::JsDate;
use crate::http::cookies::encode_uri_component;

/// `emailAndPassword.resetPasswordTokenExpiresIn` default: one hour.
pub const RESET_TOKEN_EXPIRES_IN: i64 = 3600;

fn request_reset_body() -> Schema {
    Schema::object(vec![("email", Schema::Email), ("redirectTo", Schema::string().optional())])
}

fn reset_body() -> Schema {
    Schema::object(vec![("newPassword", Schema::string()), ("token", Schema::string().optional())])
}

fn reset_query() -> Schema {
    Schema::object(vec![("token", Schema::string().optional())]).optional()
}

fn verify_body() -> Schema {
    Schema::object(vec![("password", Schema::string())])
}

pub fn endpoints() -> Vec<Endpoint> {
    vec![
        Endpoint {
            path: "/request-password-reset",
            methods: &[Method::POST],
            body: Some(request_reset_body),
            query: None,
            media_types: &[],
            middleware: &[Middleware::OriginCheckBodyRedirectTo],
            handler: request_password_reset,
        },
        Endpoint {
            path: "/reset-password/:token",
            methods: &[Method::GET],
            body: None,
            query: None,
            media_types: &[],
            middleware: &[Middleware::OriginCheckQueryCallback],
            handler: request_password_reset_callback,
        },
        Endpoint {
            path: "/reset-password",
            methods: &[Method::POST],
            body: Some(reset_body),
            query: Some(reset_query),
            media_types: &[],
            middleware: &[],
            handler: reset_password,
        },
        Endpoint {
            path: "/verify-password",
            methods: &[Method::POST],
            body: Some(verify_body),
            query: None,
            media_types: &[],
            middleware: &[Middleware::SensitiveSession],
            handler: verify_password,
        },
    ]
}

const NEUTRAL: &str = "If this email exists in our system, check your email for the reset link";

fn request_password_reset(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let email = ctx.body_str("email").unwrap_or_default().to_string();
        let redirect_to = ctx.body_str("redirectTo").map(str::to_string);
        let Some(user) = adapter::find_user_by_email(ctx.db(), &email).await? else {
            // Equalise the work of the two branches.
            let _ = generate_id(24);
            adapter::find_verification_value(ctx.db(), "dummy-verification-token").await?;
            tracing::error!("Reset Password: User not found");
            return ctx.json(json!({ "status": true, "message": NEUTRAL }));
        };
        let expires =
            JsDate::from_millis(JsDate::now().millis() + RESET_TOKEN_EXPIRES_IN * 1000).unwrap_or_else(JsDate::now);
        let token = generate_id(24);
        adapter::create_verification_value(ctx.db(), &format!("reset-password:{token}"), &user.id, expires).await?;
        let callback = redirect_to.filter(|r| !r.is_empty()).map(|r| encode_uri_component(&r)).unwrap_or_default();
        let url = format!("{}/reset-password/{token}?callbackURL={callback}", ctx.base_url());
        crate::auth::email::send_reset_password_mail(ctx, &user, &url).await?;
        ctx.json(json!({ "status": true, "message": NEUTRAL }))
    })
}

/// `redirectError(ctx, callbackURL, query)` / `redirectCallback(..)`: the
/// callback resolved against the base URL, with the query parameters set.
fn redirect_url(ctx: &EndpointCtx, callback: Option<&str>, key: &str, value: &str) -> Result<String, EndpointError> {
    let base = ctx.base_url();
    let mut url = match callback {
        Some(cb) => url::Url::parse(&base).and_then(|b| b.join(cb)),
        None => url::Url::parse(&format!("{base}/error")),
    }
    .map_err(|e| anyhow::anyhow!("Invalid URL: {e}"))?;
    // `url.searchParams.set(k, v)`: replaces every existing `k`.
    let mut pairs: Vec<(String, String)> = url.query_pairs().into_owned().filter(|(k, _)| k != key).collect();
    let insert_at = url.query_pairs().position(|(k, _)| k == key).unwrap_or(pairs.len());
    pairs.insert(insert_at.min(pairs.len()), (key.to_string(), value.to_string()));
    url.query_pairs_mut().clear().extend_pairs(pairs);
    Ok(url.to_string())
}

fn request_password_reset_callback(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let token = ctx.params.get("token").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let callback = ctx.query_str("callbackURL").map(str::to_string);
        let callback = callback.filter(|c| !c.is_empty());
        if token.is_empty() || callback.is_none() {
            let to = redirect_url(ctx, callback.as_deref(), "error", "INVALID_TOKEN")?;
            return Err(ctx.redirect(to));
        }
        let verification = adapter::find_verification_value(ctx.db(), &format!("reset-password:{token}")).await?;
        if verification.is_none_or(|v| v.expires_at.millis() < JsDate::now().millis()) {
            let to = redirect_url(ctx, callback.as_deref(), "error", "INVALID_TOKEN")?;
            return Err(ctx.redirect(to));
        }
        let to = redirect_url(ctx, callback.as_deref(), "token", &token)?;
        Err(ctx.redirect(to))
    })
}

fn reset_password(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let token = ctx
            .body_str("token")
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .or_else(|| ctx.query_str("token").filter(|t| !t.is_empty()).map(str::to_string));
        let Some(token) = token else {
            return Err(ApiError::from("BAD_REQUEST", base::INVALID_TOKEN).into());
        };
        let new_password = ctx.body_str("newPassword").unwrap_or_default().to_string();
        let length = new_password.encode_utf16().count();
        if length < MIN_PASSWORD_LENGTH {
            return Err(ApiError::from("BAD_REQUEST", base::PASSWORD_TOO_SHORT).into());
        }
        if length > MAX_PASSWORD_LENGTH {
            return Err(ApiError::from("BAD_REQUEST", base::PASSWORD_TOO_LONG).into());
        }
        let identifier = format!("reset-password:{token}");
        let verification = adapter::find_verification_value(ctx.db(), &identifier).await?;
        let Some(verification) = verification.filter(|v| v.expires_at.millis() >= JsDate::now().millis()) else {
            return Err(ApiError::from("BAD_REQUEST", base::INVALID_TOKEN).into());
        };
        let user_id = verification.value.clone();
        let hash = crate::auth::password::hash_password(&new_password).await?;
        let accounts = adapter::find_accounts(ctx.db(), &user_id).await?;
        if adapter::credential_account(&accounts).is_none() {
            adapter::create_account(ctx.db(), &user_id, "credential", &user_id, Some(&hash)).await?;
        } else {
            adapter::update_password(ctx.db(), &user_id, &hash).await?;
        }
        adapter::delete_verification_by_identifier(ctx.db(), &identifier).await?;
        ctx.json(json!({ "status": true }))
    })
}

fn verify_password(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let password = ctx.body_str("password").unwrap_or_default().to_string();
        let user_id = ctx.session.as_ref().map(|s| s.user.id.clone()).unwrap_or_default();
        if !validate_password(ctx, &user_id, &password).await? {
            return Err(ApiError::from("BAD_REQUEST", base::INVALID_PASSWORD).into());
        }
        ctx.json(json!({ "status": true }))
    })
}

/// `validatePassword(ctx, { password, userId })`.
pub async fn validate_password(ctx: &EndpointCtx, user_id: &str, password: &str) -> Result<bool, EndpointError> {
    let accounts = adapter::find_accounts(ctx.db(), user_id).await?;
    let Some(hash) = adapter::credential_account(&accounts).and_then(|a| a.password.clone()).filter(|p| !p.is_empty())
    else {
        return Ok(false);
    };
    Ok(crate::auth::password::verify_user_password(&hash, password).await?)
}

/// `checkPassword(userId, ctx)`: the body's `password` must match, else
/// `BAD_REQUEST INVALID_PASSWORD` (hashing it anyway when there is nothing
/// to compare against).
pub async fn check_password(ctx: &EndpointCtx, user_id: &str) -> Result<(), EndpointError> {
    let accounts = adapter::find_accounts(ctx.db(), user_id).await?;
    let hash = adapter::credential_account(&accounts).and_then(|a| a.password.clone()).filter(|p| !p.is_empty());
    let password = ctx.body_str("password").filter(|p| !p.is_empty()).map(str::to_string);
    let (Some(hash), Some(password)) = (hash, password.clone()) else {
        if let Some(p) = password {
            crate::auth::password::hash_password(&p).await?;
        }
        return Err(ApiError::from("BAD_REQUEST", base::INVALID_PASSWORD).into());
    };
    if !crate::auth::password::verify_user_password(&hash, &password).await? {
        return Err(ApiError::from("BAD_REQUEST", base::INVALID_PASSWORD).into());
    }
    Ok(())
}
