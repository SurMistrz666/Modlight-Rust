//! `api/routes/sign-up.mjs` (`/sign-up/email`) and lib/auth.ts's
//! `hooks.before`, the self-service registration gate.
//!
//! The gate sits on this one path, not in the user-create hook: that hook runs
//! for every way an account comes into existence (the admin panel's
//! `admin.createUser` included), and an administrator must not have to open
//! registration to the whole internet to create one account by hand.
//! `/sign-up/email` is the only endpoint a stranger can reach unauthenticated.
//!
//! lib/auth.ts makes `requireEmailVerification` a live getter, which also
//! turns on Better Auth's "generic duplicate" answer: while confirmation is
//! required (and mail works) a sign-up for an existing address answers like a
//! successful one with a synthetic user and no session, so the endpoint cannot
//! be used to find out who has an account.

use axum::http::Method;
use futures_util::future::BoxFuture;
use serde_json::{Map, Value, json};

use super::set_session_cookie;
use super::sign_in::banned_error;
use crate::auth::adapter::{self, CreateSessionError, NewUser, NotFirstAccount, SqlValue, USER_INPUT_FIELDS, pg_text};
use crate::auth::endpoint::{Endpoint, EndpointCtx, HandlerResult, Middleware};
use crate::auth::error::{ApiError, EndpointError, base};
use crate::auth::session::{AuthSession, UserOut};
use crate::auth::validate::{Schema, is_zod_email};
use crate::registration::{SignUpDecision, SignUpRefusal};

/// `emailAndPassword.minPasswordLength` / `maxPasswordLength` defaults.
pub const MIN_PASSWORD_LENGTH: usize = 8;
pub const MAX_PASSWORD_LENGTH: usize = 128;

fn body() -> Schema {
    Schema::Intersection(
        Box::new(Schema::object(vec![
            ("name", Schema::string()),
            ("email", Schema::Email),
            ("password", Schema::nonempty_string()),
            ("image", Schema::string().optional()),
            ("callbackURL", Schema::string().optional()),
            ("rememberMe", Schema::Boolean.optional()),
        ])),
        Box::new(Schema::Record(Box::new(Schema::Any))),
    )
}

pub fn endpoints() -> Vec<Endpoint> {
    vec![Endpoint {
        path: "/sign-up/email",
        methods: &[Method::POST],
        body: Some(body),
        query: None,
        media_types: &["application/x-www-form-urlencoded", "application/json"],
        middleware: &[Middleware::FormCsrf],
        handler: sign_up_email,
    }]
}

/// lib/auth.ts `hooks.before` for `/sign-up/email` (runs on the RAW body,
/// before validation).
///
/// Not upstream: a sign-up let through only because no account exists yet is
/// marked on `ctx` ([`EndpointCtx::sign_up_only_as_first`]), and the account
/// is then created only if the table is still empty under the creation lock.
pub async fn registration_gate(ctx: &mut EndpointCtx) -> Result<(), EndpointError> {
    let verdict = gate_verdict(ctx).await?;
    let refusal = match verdict.decision {
        SignUpDecision::Allow => {
            ctx.sign_up_only_as_first = verdict.only_as_first_account;
            return Ok(());
        }
        SignUpDecision::Refuse(r) => r,
    };
    Err(refusal_error(refusal))
}

/// The refusal for a sign-up the gate allowed only as the first account, once
/// another account won that race: what the gate answers now that an account
/// exists (registration closed, or a key missing), never a second account.
async fn late_refusal(ctx: &EndpointCtx) -> EndpointError {
    match gate_verdict(ctx).await {
        Ok(v) => match v.decision {
            SignUpDecision::Refuse(r) => refusal_error(r),
            // Opened in the meantime: still not this request's account.
            SignUpDecision::Allow => refusal_error(SignUpRefusal::Closed),
        },
        Err(e) => EndpointError::Internal(e),
    }
}

/// `evaluateSignUp` for this request (the RAW body's `registrationKey`).
async fn gate_verdict(ctx: &EndpointCtx) -> anyhow::Result<crate::registration::SignUpVerdict> {
    let supplied = match ctx.body.as_ref().and_then(|b| b.get("registrationKey")) {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    };
    let ip = crate::password::client_ip(&ctx.headers, ctx.peer, ctx.state.config.trust_proxy);
    crate::registration::evaluate_sign_up_gate(
        &ctx.state.db,
        &ctx.state.config.auth.secret,
        ctx.state.config.auth.registration_enabled,
        &ip,
        supplied.as_deref(),
    )
    .await
}

/// The gate's APIError for a refusal.
fn refusal_error(refusal: SignUpRefusal) -> EndpointError {
    match refusal {
        SignUpRefusal::RateLimited => {
            ApiError::with_message("TOO_MANY_REQUESTS", "SIGNUP_RATE_LIMITED", "Too many attempts. Try again later.")
        }
        SignUpRefusal::Closed => {
            ApiError::with_message("FORBIDDEN", "REGISTRATION_DISABLED", "Registration is currently disabled.")
        }
        SignUpRefusal::KeyRequired => {
            ApiError::with_message("BAD_REQUEST", "REGISTRATION_KEY_REQUIRED", "A registration key is required.")
        }
        SignUpRefusal::KeyInvalid => {
            ApiError::with_message("BAD_REQUEST", "REGISTRATION_KEY_INVALID", "That registration key is not valid.")
        }
    }
    .into()
}

/// `parseUserInput(options, rest, "create")` over lib/auth.ts's user schema:
/// fields a client may not set are replaced by their default, or refused when
/// set to a truthy value without one; client-settable ones pass through
/// untouched (their JSON type is NOT checked: Postgres decides).
pub fn parse_user_input_create(rest: &Map<String, Value>) -> Result<NewUser, EndpointError> {
    let mut out = NewUser::default();
    for field in USER_INPUT_FIELDS {
        let input = matches!(*field, "hideUploads" | "copyRaw" | "theme" | "locale");
        let default: Option<SqlValue> = match *field {
            "usedQuota" => Some(SqlValue::BigInt(0)),
            "maxQuota" => Some(SqlValue::BigInt(-1)),
            "hideUploads" | "copyRaw" | "banned" | "twoFactorEnabled" => Some(SqlValue::Bool(false)),
            "theme" => Some(SqlValue::Text("modern".into())),
            _ => None,
        };
        match rest.get(*field) {
            Some(v) if !input => {
                if let Some(d) = default {
                    out.set(field, d);
                    continue;
                }
                if crate::auth::validate::js_truthy(Some(v)) {
                    return Err(ApiError::with_message(
                        "BAD_REQUEST",
                        base::FIELD_NOT_ALLOWED.code,
                        format!("{field} is not allowed to be set"),
                    )
                    .into());
                }
            }
            Some(v) => {
                let value = match pg_text(v) {
                    None => SqlValue::Null,
                    Some(text) => SqlValue::Cast(text, "text"),
                };
                out.set(field, value);
            }
            None => {
                if let Some(d) = default {
                    out.set(field, d);
                }
            }
        }
    }
    Ok(out)
}

fn sign_up_email(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let body = ctx.body.clone().and_then(|b| b.as_object().cloned()).unwrap_or_default();
        let name = body.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        let email = body.get("email").and_then(Value::as_str).unwrap_or_default().to_string();
        let password = body.get("password").and_then(Value::as_str).unwrap_or_default().to_string();
        if !is_zod_email(&email) {
            return Err(ApiError::from("BAD_REQUEST", base::INVALID_EMAIL).into());
        }
        if password.is_empty() {
            return Err(ApiError::from("BAD_REQUEST", base::INVALID_PASSWORD).into());
        }
        let length = password.encode_utf16().count();
        if length < MIN_PASSWORD_LENGTH {
            tracing::error!("Password is too short");
            return Err(ApiError::from("BAD_REQUEST", base::PASSWORD_TOO_SHORT).into());
        }
        if length > MAX_PASSWORD_LENGTH {
            tracing::error!("Password is too long");
            return Err(ApiError::from("BAD_REQUEST", base::PASSWORD_TOO_LONG).into());
        }
        let require_verification = crate::mail_settings::verification_required_sync(&ctx.state.config);
        let mut rest = body.clone();
        for k in ["name", "email", "password", "image", "callbackURL", "rememberMe"] {
            rest.remove(k);
        }
        let additional = parse_user_input_create(&rest)?;
        let normalized = email.to_lowercase();
        if adapter::find_user_by_email(ctx.db(), &normalized).await?.is_some() {
            tracing::info!("Sign-up attempt for existing email");
            if require_verification {
                crate::auth::password::hash_password(&password).await?;
                return ctx
                    .json(json!({ "token": null, "user": synthetic_user(&name, &normalized, &body, &additional) }));
            }
            return Err(ApiError::from("UNPROCESSABLE_ENTITY", base::USER_ALREADY_EXISTS_USE_ANOTHER_EMAIL).into());
        }
        let hash = crate::auth::password::hash_password(&password).await?;
        let mut data = NewUser::default();
        data.set("email", SqlValue::Text(normalized.clone()));
        data.set("name", SqlValue::Text(name.clone()));
        if let Some(Value::String(image)) = body.get("image") {
            data.set("image", SqlValue::Text(image.clone()));
        }
        for (f, v) in additional.fields {
            data.set(&f, v);
        }
        data.set("emailVerified", SqlValue::Bool(false));
        let only_as_first = ctx.sign_up_only_as_first;
        let user =
            match adapter::create_user_with(ctx.db(), ctx.state.config.default_user_quota, data, only_as_first).await {
                Ok(u) => u,
                Err(e) if e.is::<NotFirstAccount>() => {
                    // Another sign-up created the first account in the meantime:
                    // answer what the gate answers now that one exists.
                    tracing::info!("Sign-up allowed as the first account lost the race to another one");
                    return Err(late_refusal(ctx).await);
                }
                Err(e) => {
                    tracing::error!("Failed to create user: {e:#}");
                    return Err(ApiError::from("UNPROCESSABLE_ENTITY", base::FAILED_TO_CREATE_USER).into());
                }
            };
        adapter::create_account(ctx.db(), &user.id, "credential", &user.id, Some(&hash)).await?;
        // sendOnSignUp
        let callback = body.get("callbackURL").and_then(Value::as_str).filter(|c| !c.is_empty()).unwrap_or("/");
        crate::auth::email::send_verification_email_fn(ctx, &user, callback).await?;
        let user_out = serde_json::to_value(UserOut::from(&user)).map_err(anyhow::Error::from)?;
        if require_verification {
            return ctx.json(json!({ "token": null, "user": user_out }));
        }
        let dont_remember = body.get("rememberMe") == Some(&Value::Bool(false));
        let ip = ctx.ip().unwrap_or_default();
        let ua = ctx.user_agent();
        let session = match adapter::create_session(ctx.db(), &user.id, dont_remember, &ip, &ua, None, false).await {
            Ok(s) => s,
            Err(CreateSessionError::Banned) => return Err(banned_error()),
            Err(CreateSessionError::Db(e)) => return Err(e.into()),
        };
        let token = session.token.clone();
        set_session_cookie(ctx, AuthSession { session, user }, Some(dont_remember), None);
        ctx.json(json!({ "token": token, "user": user_out }))
    })
}

/// `buildSyntheticUserOutput(options, { ...coreFields, ...additionalUserFields, id })`:
/// every returned user field, provided values first, then defaults, then null.
fn synthetic_user(name: &str, email: &str, body: &Map<String, Value>, additional: &NewUser) -> Value {
    let now = crate::db::types::JsDate::now().to_iso_string();
    let value_of = |field: &str| -> Option<Value> {
        additional.get(field).map(|v| match v {
            SqlValue::Null | SqlValue::NullOf(_) => Value::Null,
            SqlValue::Text(s) => Value::String(s.clone()),
            SqlValue::Bool(b) => Value::Bool(*b),
            SqlValue::BigInt(n) => json!(n),
            SqlValue::Int(n) => json!(n),
            SqlValue::Date(d) => Value::String(d.to_iso_string()),
            // The client's own value, as it sent it.
            SqlValue::Cast(_, _) => body.get(field).cloned().unwrap_or(Value::Null),
        })
    };
    let mut out = Map::new();
    out.insert("name".into(), Value::String(name.to_string()));
    out.insert("email".into(), Value::String(email.to_string()));
    out.insert("emailVerified".into(), Value::Bool(false));
    out.insert("image".into(), body.get("image").cloned().filter(|v| !v.is_null()).unwrap_or(Value::Null));
    out.insert("createdAt".into(), Value::String(now.clone()));
    out.insert("updatedAt".into(), Value::String(now));
    for field in [
        "role",
        "banned",
        "banReason",
        "banExpires",
        "twoFactorEnabled",
        "userCode",
        "usedQuota",
        "maxQuota",
        "hideUploads",
        "copyRaw",
        "theme",
        "locale",
    ] {
        let v = value_of(field).filter(|v| !v.is_null()).unwrap_or_else(|| match field {
            "banned" | "twoFactorEnabled" | "hideUploads" | "copyRaw" => Value::Bool(false),
            "usedQuota" => json!(0),
            "maxQuota" => json!(-1),
            "theme" => Value::String("modern".into()),
            _ => Value::Null,
        });
        out.insert(field.into(), v);
    }
    out.insert("id".into(), Value::String(crate::auth::crypto::generate_id(32)));
    Value::Object(out)
}
