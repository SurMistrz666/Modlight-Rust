//! `api/routes/session.mjs`: `/get-session`, `/list-sessions`,
//! `/revoke-session`, `/revoke-sessions`, `/revoke-other-sessions`.
//!
//! The session lookup itself (cookie verification, expiry, refresh) is
//! [`crate::auth::session`]; this module is the endpoints around it.

use axum::http::Method;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::auth::adapter;
use crate::auth::endpoint::{Endpoint, EndpointCtx, HandlerResult, Middleware};
use crate::auth::error::{ApiError, EndpointError, base};
use crate::auth::session::{GetSessionError, RefreshPolicy, SessionOut, get_session_with};
use crate::auth::validate::{Schema, js_truthy};

fn get_session_query() -> Schema {
    // z.optional(z.object({ disableCookieCache: z.coerce.boolean().optional(), disableRefresh: ... }))
    Schema::Optional(Box::new(Schema::object(vec![
        ("disableCookieCache", Schema::CoerceBoolean.optional()),
        ("disableRefresh", Schema::CoerceBoolean.optional()),
    ])))
}

fn revoke_session_body() -> Schema {
    Schema::object(vec![("token", Schema::string())])
}

pub fn endpoints() -> Vec<Endpoint> {
    vec![
        Endpoint {
            path: "/get-session",
            methods: &[Method::GET, Method::POST],
            body: None,
            query: Some(get_session_query),
            media_types: &[],
            middleware: &[],
            handler: get_session,
        },
        Endpoint {
            path: "/list-sessions",
            methods: &[Method::GET],
            body: None,
            query: None,
            media_types: &[],
            middleware: &[Middleware::Session],
            handler: list_sessions,
        },
        Endpoint {
            path: "/revoke-session",
            methods: &[Method::POST],
            body: Some(revoke_session_body),
            query: None,
            media_types: &[],
            middleware: &[Middleware::SensitiveSession],
            handler: revoke_session,
        },
        Endpoint {
            path: "/revoke-sessions",
            methods: &[Method::POST],
            body: None,
            query: None,
            media_types: &[],
            middleware: &[Middleware::SensitiveSession],
            handler: revoke_sessions,
        },
        Endpoint {
            path: "/revoke-other-sessions",
            methods: &[Method::POST],
            body: None,
            query: None,
            media_types: &[],
            middleware: &[Middleware::SensitiveSession],
            handler: revoke_other_sessions,
        },
    ]
}

fn get_session(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        // `deferSessionRefresh` is off in lib/auth.ts.
        if ctx.method == Method::POST {
            return Err(ApiError::from("METHOD_NOT_ALLOWED", base::METHOD_NOT_ALLOWED_DEFER_SESSION_REQUIRED).into());
        }
        let disable_refresh = js_truthy(ctx.query.get("disableRefresh"));
        let cookies = ctx.state.auth_cookies.clone();
        let lookup =
            match get_session_with(&ctx.state.db, &cookies, &ctx.headers, RefreshPolicy::DbAndCookie, disable_refresh)
                .await
            {
                Ok(lookup) => lookup,
                Err(GetSessionError::Unauthorized(expired)) => {
                    for c in expired {
                        ctx.set_cookie(c);
                    }
                    return Err(ApiError::from("UNAUTHORIZED", base::FAILED_TO_GET_SESSION).into());
                }
                Err(GetSessionError::Failed(e)) => {
                    tracing::error!(error = ?e, "INTERNAL_SERVER_ERROR");
                    return Err(ApiError::from("INTERNAL_SERVER_ERROR", base::FAILED_TO_GET_SESSION).into());
                }
            };
        for c in lookup.cookies {
            ctx.set_cookie(c);
        }
        match lookup.session {
            Some(session) => {
                ctx.session = Some(session.clone());
                Ok(crate::auth::endpoint::Out::Json(
                    serde_json::to_value(session.output()).map_err(anyhow::Error::from)?,
                ))
            }
            None => Ok(crate::auth::endpoint::Out::Json(Value::Null)),
        }
    })
}

fn list_sessions(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let user_id = ctx.session.as_ref().map(|s| s.user.id.clone()).unwrap_or_default();
        let sessions = match adapter::list_sessions(ctx.db(), &user_id, true).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::error!("{e}");
                return Err(ApiError::named("INTERNAL_SERVER_ERROR", None).into());
            }
        };
        let now = crate::db::types::JsDate::now().millis();
        let out: Vec<Value> = sessions
            .iter()
            .filter(|s| s.expires_at.millis() > now)
            .map(|s| serde_json::to_value(SessionOut::from(s)).unwrap_or(Value::Null))
            .collect();
        ctx.json(Value::Array(out))
    })
}

fn revoke_session(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let token = ctx.body_str("token").unwrap_or_default().to_string();
        let me = ctx.session.as_ref().map(|s| s.user.id.clone()).unwrap_or_default();
        let found = adapter::find_session(ctx.db(), &token).await?;
        if found.is_some_and(|f| f.session.user_id == me)
            && let Err(e) = adapter::delete_session(ctx.db(), &token).await
        {
            tracing::error!("{e}");
            return Err(internal_server_error());
        }
        ctx.json(json!({ "status": true }))
    })
}

fn revoke_sessions(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let me = ctx.session.as_ref().map(|s| s.user.id.clone()).unwrap_or_default();
        if let Err(e) = adapter::delete_user_sessions(ctx.db(), &me).await {
            tracing::error!("{e}");
            return Err(internal_server_error());
        }
        ctx.json(json!({ "status": true }))
    })
}

fn revoke_other_sessions(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let Some(current) = ctx.session.clone() else {
            return Err(ApiError::with_message("UNAUTHORIZED", "UNAUTHORIZED", "Unauthorized").into());
        };
        let now = crate::db::types::JsDate::now().millis();
        let others: Vec<String> = adapter::list_sessions(ctx.db(), &current.user.id, false)
            .await?
            .into_iter()
            .filter(|s| s.expires_at.millis() > now)
            .filter(|s| s.token != current.session.token)
            .map(|s| s.token)
            .collect();
        for token in others {
            adapter::delete_session(ctx.db(), &token).await?;
        }
        ctx.json(json!({ "status": true }))
    })
}

/// `APIError.from("INTERNAL_SERVER_ERROR", { message: "Internal Server Error", code: "INTERNAL_SERVER_ERROR" })`.
fn internal_server_error() -> EndpointError {
    ApiError::with_message("INTERNAL_SERVER_ERROR", "INTERNAL_SERVER_ERROR", "Internal Server Error").into()
}
