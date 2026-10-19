//! `api/routes/account.mjs` (`/list-accounts`, `/unlink-account`,
//! `/link-social`, `/get-access-token`, `/refresh-token`, `/account-info`),
//! plus `/sign-in/social` (sign-in.mjs) and `/callback/:id` (callback.mjs).
//!
//! Modlight configures no social provider, so every social endpoint ends in
//! its "no such provider" (or "no state") branch; those branches, and the
//! validation and session checks before them, are ported as upstream runs
//! them. Credential accounts are what `/list-accounts` and
//! `/unlink-account` see.

use axum::http::Method;
use futures_util::future::BoxFuture;
use serde_json::{Map, Value, json};

use crate::auth::adapter;
use crate::auth::endpoint::{Endpoint, EndpointCtx, HandlerResult, Middleware};
use crate::auth::error::{ApiError, EndpointError, base};
use crate::auth::validate::{Schema, js_truthy, parse};
use crate::db::models::Account;

/// `SocialProviderListEnum` = `z.enum(providers).or(z.string())`: any string.
fn provider_schema() -> Schema {
    Schema::Union(vec![Schema::string(), Schema::string()])
}

fn social_sign_in_body() -> Schema {
    Schema::object(vec![
        ("callbackURL", Schema::string().optional()),
        ("newUserCallbackURL", Schema::string().optional()),
        ("errorCallbackURL", Schema::string().optional()),
        ("provider", provider_schema()),
        ("disableRedirect", Schema::Boolean.optional()),
        (
            "idToken",
            Schema::object(vec![
                ("token", Schema::string()),
                ("nonce", Schema::string().optional()),
                ("accessToken", Schema::string().optional()),
                ("refreshToken", Schema::string().optional()),
                ("expiresAt", Schema::Number.optional()),
                (
                    "user",
                    Schema::object(vec![
                        (
                            "name",
                            Schema::object(vec![
                                ("firstName", Schema::string().optional()),
                                ("lastName", Schema::string().optional()),
                            ])
                            .optional(),
                        ),
                        ("email", Schema::string().optional()),
                    ])
                    .optional(),
                ),
            ])
            .optional(),
        ),
        ("scopes", Schema::Array(Box::new(Schema::string())).optional()),
        ("requestSignUp", Schema::Boolean.optional()),
        ("loginHint", Schema::string().optional()),
        ("additionalData", Schema::Record(Box::new(Schema::Any)).optional()),
    ])
}

fn link_social_body() -> Schema {
    Schema::object(vec![
        ("callbackURL", Schema::string().optional()),
        ("provider", provider_schema()),
        (
            "idToken",
            Schema::object(vec![
                ("token", Schema::string()),
                ("nonce", Schema::string().optional()),
                ("accessToken", Schema::string().optional()),
                ("refreshToken", Schema::string().optional()),
                ("scopes", Schema::Array(Box::new(Schema::string())).optional()),
            ])
            .optional(),
        ),
        ("requestSignUp", Schema::Boolean.optional()),
        ("scopes", Schema::Array(Box::new(Schema::string())).optional()),
        ("errorCallbackURL", Schema::string().optional()),
        ("disableRedirect", Schema::Boolean.optional()),
        ("additionalData", Schema::Record(Box::new(Schema::Any)).optional()),
    ])
}

fn unlink_body() -> Schema {
    Schema::object(vec![("providerId", Schema::string()), ("accountId", Schema::string().optional())])
}

fn token_body() -> Schema {
    Schema::object(vec![
        ("providerId", Schema::string()),
        ("accountId", Schema::string().optional()),
        ("userId", Schema::string().optional()),
    ])
}

fn account_info_query() -> Schema {
    Schema::object(vec![
        ("accountId", Schema::string().optional()),
        ("providerId", Schema::string().optional()),
        ("userId", Schema::string().optional()),
    ])
    .optional()
}

fn callback_schema() -> Schema {
    Schema::object(vec![
        ("code", Schema::string().optional()),
        ("error", Schema::string().optional()),
        ("device_id", Schema::string().optional()),
        ("error_description", Schema::string().optional()),
        ("state", Schema::string().optional()),
        ("user", Schema::string().optional()),
    ])
}

fn callback_optional() -> Schema {
    callback_schema().optional()
}

/// `/sign-in/social` and `/callback/:id` (registered first, like upstream).
pub fn social_endpoints() -> Vec<Endpoint> {
    vec![
        Endpoint {
            path: "/sign-in/social",
            methods: &[Method::POST],
            body: Some(social_sign_in_body),
            query: None,
            media_types: &[],
            middleware: &[],
            handler: sign_in_social,
        },
        Endpoint {
            path: "/callback/:id",
            methods: &[Method::GET, Method::POST],
            body: Some(callback_optional),
            query: Some(callback_optional),
            media_types: &["application/x-www-form-urlencoded", "application/json"],
            middleware: &[],
            handler: callback_oauth,
        },
    ]
}

pub fn endpoints() -> Vec<Endpoint> {
    vec![
        Endpoint {
            path: "/link-social",
            methods: &[Method::POST],
            body: Some(link_social_body),
            query: None,
            media_types: &[],
            middleware: &[Middleware::Session],
            handler: link_social,
        },
        Endpoint {
            path: "/list-accounts",
            methods: &[Method::GET],
            body: None,
            query: None,
            media_types: &[],
            middleware: &[Middleware::Session],
            handler: list_user_accounts,
        },
        Endpoint {
            path: "/unlink-account",
            methods: &[Method::POST],
            body: Some(unlink_body),
            query: None,
            media_types: &[],
            middleware: &[Middleware::FreshSession],
            handler: unlink_account,
        },
        Endpoint {
            path: "/get-access-token",
            methods: &[Method::POST],
            body: Some(token_body),
            query: None,
            media_types: &[],
            middleware: &[],
            handler: get_access_token,
        },
        Endpoint {
            path: "/refresh-token",
            methods: &[Method::POST],
            body: Some(token_body),
            query: None,
            media_types: &[],
            middleware: &[],
            handler: refresh_token,
        },
        Endpoint {
            path: "/account-info",
            methods: &[Method::GET],
            body: None,
            query: Some(account_info_query),
            media_types: &[],
            middleware: &[],
            handler: account_info,
        },
    ]
}

fn provider_not_found() -> EndpointError {
    tracing::error!("Provider not found. Make sure to add the provider in your auth config");
    ApiError::from("NOT_FOUND", base::PROVIDER_NOT_FOUND).into()
}

fn sign_in_social(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let _ = ctx;
        Err(provider_not_found())
    })
}

fn link_social(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let _ = ctx;
        Err(provider_not_found())
    })
}

/// `parseAccountOutput` + `scopes`: what `/list-accounts` returns per row.
fn account_output(a: &Account) -> Value {
    // The adapter's row order (schema fields, then `id`), minus the secrets.
    let mut out = Map::new();
    out.insert("accountId".into(), Value::String(a.account_id.clone()));
    out.insert("providerId".into(), Value::String(a.provider_id.clone()));
    out.insert("userId".into(), Value::String(a.user_id.clone()));
    out.insert("createdAt".into(), Value::String(a.created_at.to_iso_string()));
    out.insert("updatedAt".into(), Value::String(a.updated_at.to_iso_string()));
    out.insert("id".into(), Value::String(a.id.clone()));
    let scopes: Vec<Value> = match a.scope.as_deref().filter(|s| !s.is_empty()) {
        Some(s) => s.split(',').map(|x| Value::String(x.to_string())).collect(),
        None => Vec::new(),
    };
    out.insert("scopes".into(), Value::Array(scopes));
    Value::Object(out)
}

fn list_user_accounts(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let user_id = ctx.session.as_ref().map(|s| s.user.id.clone()).unwrap_or_default();
        let accounts = adapter::find_accounts(ctx.db(), &user_id).await?;
        ctx.json(Value::Array(accounts.iter().map(account_output).collect()))
    })
}

fn unlink_account(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let provider_id = ctx.body_str("providerId").unwrap_or_default().to_string();
        let account_id = ctx.body_str("accountId").filter(|a| !a.is_empty()).map(str::to_string);
        let user_id = ctx.session.as_ref().map(|s| s.user.id.clone()).unwrap_or_default();
        let accounts = adapter::find_accounts(ctx.db(), &user_id).await?;
        // `account.accountLinking.allowUnlinkingAll` is not set.
        if accounts.len() == 1 {
            return Err(ApiError::from("BAD_REQUEST", base::FAILED_TO_UNLINK_LAST_ACCOUNT).into());
        }
        let existing = accounts.iter().find(|a| match &account_id {
            Some(id) => &a.account_id == id && a.provider_id == provider_id,
            None => a.provider_id == provider_id,
        });
        let Some(existing) = existing else {
            return Err(ApiError::from("BAD_REQUEST", base::ACCOUNT_NOT_FOUND).into());
        };
        adapter::delete_account(ctx.db(), &existing.id).await?;
        ctx.json(json!({ "status": true }))
    })
}

/// `resolveUserId(ctx, userId)`: an HTTP caller needs a session (its user
/// wins); a server-side caller may name the user.
async fn resolve_user_id(ctx: &mut EndpointCtx, user_id: Option<String>) -> Result<String, EndpointError> {
    let session = ctx.session_from_ctx().await;
    if session.is_none() && (ctx.is_request || ctx.has_headers) {
        return Err(ApiError::named("UNAUTHORIZED", None).into());
    }
    match session.map(|s| s.user.id).or(user_id.filter(|u| !u.is_empty())) {
        Some(id) => Ok(id),
        None => Err(ApiError::with_message(
            "BAD_REQUEST",
            "USER_ID_OR_SESSION_REQUIRED",
            "Either userId or session is required",
        )
        .into()),
    }
}

fn provider_not_supported(provider_id: &str) -> EndpointError {
    ApiError::with_message("BAD_REQUEST", "PROVIDER_NOT_SUPPORTED", format!("Provider {provider_id} is not supported."))
        .into()
}

fn get_access_token(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let provider_id = ctx.body_str("providerId").unwrap_or_default().to_string();
        let user_id = ctx.body_str("userId").map(str::to_string);
        resolve_user_id(ctx, user_id).await?;
        Err(provider_not_supported(&provider_id))
    })
}

fn refresh_token(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let provider_id = ctx.body_str("providerId").unwrap_or_default().to_string();
        let user_id = ctx.body_str("userId").map(str::to_string);
        resolve_user_id(ctx, user_id).await?;
        Err(provider_not_supported(&provider_id))
    })
}

fn account_info(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let account_id = ctx.query_str("accountId").filter(|a| !a.is_empty()).map(str::to_string);
        let provider_id = ctx.query_str("providerId").filter(|p| !p.is_empty()).map(str::to_string);
        let user_id = ctx.query_str("userId").map(str::to_string);
        let resolved = resolve_user_id(ctx, user_id).await?;
        let account = match &account_id {
            // `storeAccountCookie` is off: no account without an id.
            None => None,
            Some(id) => {
                let matching: Vec<Account> = adapter::find_accounts(ctx.db(), &resolved)
                    .await?
                    .into_iter()
                    .filter(|a| &a.account_id == id && provider_id.as_ref().is_none_or(|p| &a.provider_id == p))
                    .collect();
                if matching.len() > 1 {
                    return Err(ApiError::with_message(
                        "BAD_REQUEST",
                        "AMBIGUOUS_ACCOUNT",
                        "Multiple accounts share this account ID. Pass a providerId to disambiguate.",
                    )
                    .into());
                }
                matching.into_iter().next()
            }
        };
        if account.as_ref().is_none_or(|a| a.user_id != resolved) {
            return Err(ApiError::from("BAD_REQUEST", base::ACCOUNT_NOT_FOUND).into());
        }
        Err(ApiError::with_message(
            "BAD_REQUEST",
            "PROVIDER_NOT_CONFIGURED",
            "Account is not associated with a configured social provider.",
        )
        .into())
    })
}

/// `redirectOnError(ctx, errorURL, error)`.
fn redirect_on_error(ctx: &mut EndpointCtx, error_url: &str, error: &str) -> EndpointError {
    let sep = if error_url.contains('?') { '&' } else { '?' };
    let params = form_urlencoded::Serializer::new(String::new()).append_pair("error", error).finish();
    ctx.redirect(format!("{error_url}{sep}{params}"))
}

fn callback_oauth(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let error_url = format!("{}/error", ctx.base_url());
        let id = ctx.params.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        if ctx.method == Method::POST {
            // `schema.parse(body)`, `schema.parse(query)`, merged (query wins),
            // re-sent as a GET.
            let mut merged = Map::new();
            for source in [ctx.body.clone(), Some(Value::Object(ctx.query.clone()))] {
                if let Some(v) = source.filter(|v| js_truthy(Some(v))) {
                    let (parsed, issues) = parse(&callback_schema(), Some(&v));
                    if !issues.is_empty() {
                        return Err(EndpointError::Internal(anyhow::anyhow!("ZodError")));
                    }
                    if let Some(Value::Object(m)) = parsed {
                        merged.extend(m);
                    }
                }
            }
            let mut params = form_urlencoded::Serializer::new(String::new());
            for (k, v) in &merged {
                if let Value::String(s) = v {
                    params.append_pair(k, s);
                }
            }
            let to = format!("{}/callback/{id}?{}", ctx.base_url(), params.finish());
            return Err(ctx.redirect(to));
        }
        // parseState: the OAuth state lives in `verification` (database strategy).
        let state = ctx.query_str("state").filter(|s| !s.is_empty()).map(str::to_string);
        let Some(state) = state else {
            tracing::error!("Failed to parse state: State not found in OAuth callback");
            return Err(redirect_on_error(ctx, &error_url, "state_not_found"));
        };
        let found = adapter::find_verification_value(ctx.db(), &state).await?;
        let code = match found {
            None => "state_mismatch",
            // No OAuth flow ever stores one: whatever is there does not parse.
            Some(_) => "internal_server_error",
        };
        tracing::error!("Failed to parse state: {code}");
        Err(redirect_on_error(ctx, &error_url, code))
    })
}
