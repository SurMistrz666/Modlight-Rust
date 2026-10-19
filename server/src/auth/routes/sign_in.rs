//! `api/routes/sign-in.mjs`: `/sign-in/email` (and `/sign-in/social`, which
//! lives with the other social endpoints in [`super::account`]).
//!
//! lib/auth.ts options that shape it: `requireEmailVerification` is the live
//! mail-settings getter ([`crate::mail_settings::verification_required_sync`]),
//! `sendOnSignIn` re-sends the confirmation link when a CORRECT password is
//! refused for an unconfirmed address, the password verifier accepts the
//! legacy bcrypt hashes, and the twoFactor plugin intercepts the result in an
//! after hook ([`crate::auth::two_factor::sign_in_after_hook`]).

use axum::http::Method;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use super::set_session_cookie;
use crate::auth::adapter::{self, CreateSessionError};
use crate::auth::endpoint::{Endpoint, EndpointCtx, HandlerResult, Middleware};
use crate::auth::error::{ApiError, EndpointError, base};
use crate::auth::session::{AuthSession, BANNED_USER_MESSAGE, UserOut};
use crate::auth::validate::{Schema, is_zod_email};

fn body() -> Schema {
    Schema::object(vec![
        ("email", Schema::string()),
        ("password", Schema::string()),
        ("callbackURL", Schema::string().optional()),
        ("rememberMe", Schema::Default(Box::new(Schema::Boolean), json!(true)).optional()),
    ])
}

pub fn endpoints() -> Vec<Endpoint> {
    vec![Endpoint {
        path: "/sign-in/email",
        methods: &[Method::POST],
        body: Some(body),
        query: None,
        media_types: &["application/x-www-form-urlencoded", "application/json"],
        middleware: &[Middleware::FormCsrf],
        handler: sign_in_email,
    }]
}

/// The admin plugin's `session.create.before` refusal.
pub fn banned_error() -> EndpointError {
    ApiError::with_message("FORBIDDEN", "BANNED_USER", BANNED_USER_MESSAGE).into()
}

/// `ctx.context.password.hash(password)` whose result nobody reads (timing
/// equalisation on the "no such user" paths).
async fn burn_hash(password: &str) -> Result<(), EndpointError> {
    crate::auth::password::hash_password(password).await?;
    Ok(())
}

fn sign_in_email(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let email = ctx.body_str("email").unwrap_or_default().to_string();
        let password = ctx.body_str("password").unwrap_or_default().to_string();
        if !is_zod_email(&email) {
            return Err(ApiError::from("BAD_REQUEST", base::INVALID_EMAIL).into());
        }
        let Some(user) = adapter::find_user_by_email(ctx.db(), &email).await? else {
            burn_hash(&password).await?;
            tracing::error!("User not found");
            return Err(ApiError::from("UNAUTHORIZED", base::INVALID_EMAIL_OR_PASSWORD).into());
        };
        let accounts = adapter::find_accounts(ctx.db(), &user.id).await?;
        let Some(credential) = adapter::credential_account(&accounts).cloned() else {
            burn_hash(&password).await?;
            tracing::error!("Credential account not found");
            return Err(ApiError::from("UNAUTHORIZED", base::INVALID_EMAIL_OR_PASSWORD).into());
        };
        let Some(hash) = credential.password.filter(|p| !p.is_empty()) else {
            burn_hash(&password).await?;
            tracing::error!("Password not found");
            return Err(ApiError::from("UNAUTHORIZED", base::INVALID_EMAIL_OR_PASSWORD).into());
        };
        if !crate::auth::password::verify_user_password(&hash, &password).await? {
            tracing::error!("Invalid password");
            return Err(ApiError::from("UNAUTHORIZED", base::INVALID_EMAIL_OR_PASSWORD).into());
        }
        if crate::mail_settings::verification_required_sync(&ctx.state.config) && !user.email_verified {
            // sendOnSignIn: a fresh link, sent through lib/auth.ts's callback.
            let callback = ctx.body_str("callbackURL").filter(|c| !c.is_empty()).unwrap_or("/").to_string();
            crate::auth::email::send_verification_email_fn(ctx, &user, &callback).await?;
            return Err(ApiError::from("FORBIDDEN", base::EMAIL_NOT_VERIFIED).into());
        }
        let dont_remember = ctx.body_field("rememberMe") == Some(&Value::Bool(false));
        let ip = ctx.ip().unwrap_or_default();
        let ua = ctx.user_agent();
        let session = match adapter::create_session(ctx.db(), &user.id, dont_remember, &ip, &ua, None, false).await {
            Ok(s) => s,
            Err(CreateSessionError::Banned) => return Err(banned_error()),
            Err(CreateSessionError::Db(e)) => return Err(e.into()),
        };
        let callback = ctx.body_str("callbackURL").filter(|c| !c.is_empty()).map(str::to_string);
        let user_out = serde_json::to_value(UserOut::from(&user)).map_err(anyhow::Error::from)?;
        let token = session.token.clone();
        set_session_cookie(ctx, AuthSession { session, user }, Some(dont_remember), None);
        if let Some(cb) = &callback {
            ctx.location = Some(cb.clone());
        }
        let mut out = serde_json::Map::new();
        out.insert("redirect".into(), Value::Bool(callback.is_some()));
        out.insert("token".into(), Value::String(token));
        if let Some(cb) = callback {
            out.insert("url".into(), Value::String(cb));
        } else if ctx.body_field("callbackURL").is_some() {
            // An empty callbackURL: `url: ""` (`redirect` false).
            out.insert("url".into(), ctx.body_field("callbackURL").cloned().unwrap_or(Value::Null));
        }
        out.insert("user".into(), user_out);
        ctx.json(Value::Object(out))
    })
}
