//! `api/routes/update-user.mjs`: `/update-user`, `/change-password`,
//! `/delete-user`, `/delete-user/callback`, `/change-email`.
//!
//! lib/auth.ts enables neither `user.deleteUser` nor `user.changeEmail`, so
//! those three endpoints only ever refuse (after validation and, for the POSTs,
//! the session check), exactly as upstream does.

use axum::http::Method;
use futures_util::future::BoxFuture;
use serde_json::{Map, Value, json};

use super::sign_up::{MAX_PASSWORD_LENGTH, MIN_PASSWORD_LENGTH};
use super::{delete_session_cookie, set_session_cookie};
use crate::auth::adapter::{self, CreateSessionError, SqlValue, pg_text};
use crate::auth::endpoint::{Endpoint, EndpointCtx, HandlerResult, Middleware};
use crate::auth::error::{ApiError, EndpointError, base};
use crate::auth::session::{AuthSession, UserOut};
use crate::auth::validate::{Schema, js_truthy};

fn update_user_body() -> Schema {
    Schema::Record(Box::new(Schema::Any))
}

fn change_password_body() -> Schema {
    Schema::object(vec![
        ("newPassword", Schema::string()),
        ("currentPassword", Schema::string()),
        ("revokeOtherSessions", Schema::Boolean.optional()),
    ])
}

fn delete_user_body() -> Schema {
    Schema::object(vec![
        ("callbackURL", Schema::string().optional()),
        ("password", Schema::string().optional()),
        ("token", Schema::string().optional()),
    ])
}

fn delete_callback_query() -> Schema {
    Schema::object(vec![("token", Schema::string()), ("callbackURL", Schema::string().optional())])
}

fn change_email_body() -> Schema {
    Schema::object(vec![("newEmail", Schema::Email), ("callbackURL", Schema::string().optional())])
}

pub fn endpoints() -> Vec<Endpoint> {
    vec![
        Endpoint {
            path: "/update-user",
            methods: &[Method::POST],
            body: Some(update_user_body),
            query: None,
            media_types: &[],
            middleware: &[Middleware::Session],
            handler: update_user,
        },
        Endpoint {
            path: "/change-password",
            methods: &[Method::POST],
            body: Some(change_password_body),
            query: None,
            media_types: &[],
            middleware: &[Middleware::SensitiveSession],
            handler: change_password,
        },
        Endpoint {
            path: "/delete-user",
            methods: &[Method::POST],
            body: Some(delete_user_body),
            query: None,
            media_types: &[],
            middleware: &[Middleware::SensitiveSession],
            handler: delete_user,
        },
        Endpoint {
            path: "/delete-user/callback",
            methods: &[Method::GET],
            body: None,
            query: Some(delete_callback_query),
            media_types: &[],
            middleware: &[Middleware::OriginCheckQueryCallback],
            handler: delete_user_callback,
        },
        Endpoint {
            path: "/change-email",
            methods: &[Method::POST],
            body: Some(change_email_body),
            query: None,
            media_types: &[],
            middleware: &[Middleware::SensitiveSession],
            handler: change_email,
        },
    ]
}

/// `parseUserInput(options, rest, "update")`: input-false fields are refused
/// when truthy (defaults do not apply on update), the rest pass through.
fn parse_user_input_update(rest: &Map<String, Value>) -> Result<Vec<(&'static str, SqlValue)>, EndpointError> {
    let mut out = Vec::new();
    for field in adapter::USER_INPUT_FIELDS {
        let Some(v) = rest.get(*field) else {
            continue;
        };
        let input = matches!(*field, "hideUploads" | "copyRaw" | "theme" | "locale");
        if !input {
            if js_truthy(Some(v)) {
                return Err(ApiError::with_message(
                    "BAD_REQUEST",
                    base::FIELD_NOT_ALLOWED.code,
                    format!("{field} is not allowed to be set"),
                )
                .into());
            }
            continue;
        }
        let value = match pg_text(v) {
            None => SqlValue::Null,
            Some(text) => SqlValue::Cast(text, "text"),
        };
        out.push((*field, value));
    }
    Ok(out)
}

fn update_user(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let Some(Value::Object(body)) = ctx.body.clone() else {
            return Err(ApiError::from("BAD_REQUEST", base::BODY_MUST_BE_AN_OBJECT).into());
        };
        if js_truthy(body.get("email")) {
            return Err(ApiError::from("BAD_REQUEST", base::EMAIL_CAN_NOT_BE_UPDATED).into());
        }
        let mut rest = body.clone();
        let name = rest.remove("name");
        let image = rest.remove("image");
        let additional = parse_user_input_update(&rest)?;
        if image.is_none() && name.is_none() && additional.is_empty() {
            return Err(ApiError::message_only("BAD_REQUEST", "No fields to update").into());
        }
        let session = ctx.session.clone().expect("session middleware ran");
        let mut patch: Vec<(&str, SqlValue)> = Vec::new();
        for (field, v) in [("name", &name), ("image", &image)] {
            if let Some(v) = v {
                patch.push((field, pg_text(v).map(|t| SqlValue::Cast(t, "text")).unwrap_or(SqlValue::Null)));
            }
        }
        patch.extend(additional);
        let updated = adapter::update_user(ctx.db(), &session.user.id, &patch).await?;
        let user = updated.unwrap_or_else(|| session.user.clone());
        set_session_cookie(ctx, AuthSession { session: session.session, user }, None, None);
        ctx.json(json!({ "status": true }))
    })
}

fn change_password(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let new_password = ctx.body_str("newPassword").unwrap_or_default().to_string();
        let current_password = ctx.body_str("currentPassword").unwrap_or_default().to_string();
        let revoke = ctx.body_field("revokeOtherSessions") == Some(&Value::Bool(true));
        let session = ctx.session.clone().expect("session middleware ran");
        let length = new_password.encode_utf16().count();
        if length < MIN_PASSWORD_LENGTH {
            tracing::error!("Password is too short");
            return Err(ApiError::from("BAD_REQUEST", base::PASSWORD_TOO_SHORT).into());
        }
        if length > MAX_PASSWORD_LENGTH {
            tracing::error!("Password is too long");
            return Err(ApiError::from("BAD_REQUEST", base::PASSWORD_TOO_LONG).into());
        }
        let accounts = adapter::find_accounts(ctx.db(), &session.user.id).await?;
        let Some(account) = accounts
            .into_iter()
            .find(|a| a.provider_id == "credential" && a.password.as_deref().is_some_and(|p| !p.is_empty()))
        else {
            return Err(ApiError::from("BAD_REQUEST", base::CREDENTIAL_ACCOUNT_NOT_FOUND).into());
        };
        let hash = crate::auth::password::hash_password(&new_password).await?;
        let stored = account.password.clone().unwrap_or_default();
        if !crate::auth::password::verify_user_password(&stored, &current_password).await? {
            return Err(ApiError::from("BAD_REQUEST", base::INVALID_PASSWORD).into());
        }
        adapter::update_account_password(ctx.db(), &account.id, &hash).await?;
        let mut token = Value::Null;
        if revoke {
            adapter::delete_user_sessions(ctx.db(), &session.user.id).await?;
            let ip = ctx.ip().unwrap_or_default();
            let ua = ctx.user_agent();
            let new_session =
                match adapter::create_session(ctx.db(), &session.user.id, false, &ip, &ua, None, false).await {
                    Ok(s) => s,
                    Err(CreateSessionError::Banned) => return Err(super::sign_in::banned_error()),
                    Err(CreateSessionError::Db(e)) => return Err(e.into()),
                };
            token = Value::String(new_session.token.clone());
            set_session_cookie(ctx, AuthSession { session: new_session, user: session.user.clone() }, None, None);
        }
        let user = serde_json::to_value(UserOut::from(&session.user)).map_err(anyhow::Error::from)?;
        ctx.json(json!({ "token": token, "user": user }))
    })
}

fn delete_user(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        // `user.deleteUser.enabled` is not set: `APIError.fromStatus("NOT_FOUND")`, no body.
        tracing::error!("Delete user is disabled. Enable it in the options");
        let _ = ctx;
        Err(ApiError::named("NOT_FOUND", None).into())
    })
}

fn delete_user_callback(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        tracing::error!("Delete user is disabled. Enable it in the options");
        let _ = ctx;
        Err(ApiError::with_message("NOT_FOUND", "NOT_FOUND", "Not found").into())
    })
}

fn change_email(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        tracing::error!("Change email is disabled.");
        let _ = ctx;
        Err(ApiError::from("BAD_REQUEST", base::CHANGE_EMAIL_DISABLED).into())
    })
}

/// `deleteSessionCookie` re-export for the admin plugin's impersonation.
pub fn expire_session(ctx: &mut EndpointCtx) {
    delete_session_cookie(ctx, false);
}
