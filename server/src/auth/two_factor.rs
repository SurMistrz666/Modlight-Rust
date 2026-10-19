//! The twoFactor plugin, `twoFactor({ issuer: APP_NAME })`
//! (better-auth 1.6.13 `plugins/two-factor/`: `index.mjs`, `totp/`,
//! `backup-codes/`, `otp/`, `verify-two-factor.mjs`).
//!
//! Byte-for-byte with the rows the TypeScript server writes:
//! - `two_factor.secret` = `symmetricEncrypt(AUTH_SECRET, <32 x [a-z0-9A-Z-_]>)`,
//!   `two_factor.backup_codes` = the same over `JSON.stringify(codes)`, ten
//!   codes `xxxxx-xxxxx` over `[a-z0-9A-Z]`;
//! - TOTP: HMAC-SHA1 over the secret STRING, 6 digits, 30 s, one step either
//!   side; the `otpauth://` URI names APP_NAME;
//! - `/sign-in/email` for an account with 2FA ends in the after hook below:
//!   the new session is deleted again, a `verification` row
//!   `2fa-<20 chars>` (10 min) holds the user id and the signed
//!   `better-auth.two_factor` cookie points at it; a valid
//!   `better-auth.trust_device` cookie (an HMAC of `<userId>!<identifier>`
//!   plus a `trust-device-<32>` verification row, 30 days) skips that and is
//!   rotated instead;
//! - enabling stores an UNVERIFIED secret; the first correct TOTP flips
//!   `user.two_factor_enabled` (replacing the session) and marks it verified.
//!
//! The OTP channel (`/two-factor/send-otp`) is not configured (no `sendOTP`):
//! it answers OTP_NOT_CONFIGURED, and `/two-factor/verify-otp` can only ever
//! find no code.

use axum::http::Method;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use super::adapter::{self, CreateSessionError, SessionOverride, SqlValue};
use super::crypto::{
    DIGITS, LOWER, UPPER, constant_time_eq, generate_random_string, hmac_sha256_base64url, random_string,
    symmetric_decrypt, symmetric_encrypt, totp_uri, verify_totp,
};
use super::endpoint::{Endpoint, EndpointCtx, HandlerResult, Middleware, Out};
use super::error::{ApiError, EndpointError, ErrorCode, base, two_factor as codes};
use super::routes::password::{check_password, validate_password};
use super::routes::sign_in::banned_error;
use super::routes::{auth_cookie, delete_session_cookie, set_session_cookie};
use super::session::{AuthSession, UserOut};
use super::validate::Schema;
use crate::db::models::{TwoFactor, User};
use crate::db::types::JsDate;

pub const TWO_FACTOR_COOKIE_NAME: &str = "two_factor";
pub const TRUST_DEVICE_COOKIE_NAME: &str = "trust_device";
/// `trustDeviceMaxAge` default: 30 days.
pub const TRUST_DEVICE_MAX_AGE: i64 = 2_592_000;
/// `twoFactorCookieMaxAge` default: 10 minutes.
pub const TWO_FACTOR_COOKIE_MAX_AGE: i64 = 600;

fn password_body() -> Schema {
    Schema::object(vec![("password", Schema::string())])
}

fn enable_body() -> Schema {
    Schema::object(vec![("password", Schema::string()), ("issuer", Schema::string().optional())])
}

fn verify_totp_body() -> Schema {
    Schema::object(vec![("code", Schema::string()), ("trustDevice", Schema::Boolean.optional())])
}

fn verify_backup_body() -> Schema {
    Schema::object(vec![
        ("code", Schema::string()),
        ("disableSession", Schema::Boolean.optional()),
        ("trustDevice", Schema::Boolean.optional()),
    ])
}

fn send_otp_body() -> Schema {
    Schema::object(vec![("trustDevice", Schema::Boolean.optional())]).optional()
}

fn verify_otp_body() -> Schema {
    Schema::object(vec![("code", Schema::string()), ("trustDevice", Schema::Boolean.optional())])
}

/// The plugin's endpoints, in its `endpoints` order (totp, otp, backup
/// codes, enable, disable).
pub fn endpoints() -> Vec<Endpoint> {
    vec![
        Endpoint {
            path: "/two-factor/get-totp-uri",
            methods: &[Method::POST],
            body: Some(password_body),
            query: None,
            media_types: &[],
            middleware: &[Middleware::Session],
            handler: get_totp_uri,
        },
        Endpoint {
            path: "/two-factor/verify-totp",
            methods: &[Method::POST],
            body: Some(verify_totp_body),
            query: None,
            media_types: &[],
            middleware: &[],
            handler: verify_totp_endpoint,
        },
        Endpoint {
            path: "/two-factor/send-otp",
            methods: &[Method::POST],
            body: Some(send_otp_body),
            query: None,
            media_types: &[],
            middleware: &[],
            handler: send_otp,
        },
        Endpoint {
            path: "/two-factor/verify-otp",
            methods: &[Method::POST],
            body: Some(verify_otp_body),
            query: None,
            media_types: &[],
            middleware: &[],
            handler: verify_otp,
        },
        Endpoint {
            path: "/two-factor/verify-backup-code",
            methods: &[Method::POST],
            body: Some(verify_backup_body),
            query: None,
            media_types: &[],
            middleware: &[],
            handler: verify_backup_code,
        },
        Endpoint {
            path: "/two-factor/generate-backup-codes",
            methods: &[Method::POST],
            body: Some(password_body),
            query: None,
            media_types: &[],
            middleware: &[Middleware::Session],
            handler: generate_backup_codes_endpoint,
        },
        Endpoint {
            path: "/two-factor/enable",
            methods: &[Method::POST],
            body: Some(enable_body),
            query: None,
            media_types: &[],
            middleware: &[Middleware::Session],
            handler: enable,
        },
        Endpoint {
            path: "/two-factor/disable",
            methods: &[Method::POST],
            body: Some(password_body),
            query: None,
            media_types: &[],
            middleware: &[Middleware::SensitiveSession],
            handler: disable,
        },
    ]
}

// ── Backup codes ────────────────────────────────────────────────────────────

/// `generateBackupCodesFn()`: ten `xxxxx-xxxxx` codes over `[a-z0-9A-Z]`.
pub fn generate_backup_codes() -> Vec<String> {
    let alphabet = format!("{LOWER}{DIGITS}{UPPER}");
    (0..10)
        .map(|_| {
            let code = random_string(10, &alphabet);
            format!("{}-{}", &code[..5], &code[5..])
        })
        .collect()
}

/// `encodeBackupCodes(codes, secret, { storeBackupCodes: "encrypted" })`.
pub fn encode_backup_codes(secret: &str, codes: &[String]) -> String {
    symmetric_encrypt(secret, &serde_json::to_string(codes).unwrap_or_else(|_| "[]".into()))
}

/// `getBackupCodes(...)`: `safeJSONParse(symmetricDecrypt(...))`, `None` when
/// either fails.
pub fn decode_backup_codes(secret: &str, stored: &str) -> Option<Vec<String>> {
    let json = symmetric_decrypt(secret, stored).ok()?;
    let v: Value = serde_json::from_str(&json).ok()?;
    let items = v.as_array()?;
    Some(items.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect())
}

// ── verifyTwoFactor ─────────────────────────────────────────────────────────

/// What `verifyTwoFactor(ctx)` resolved: a signed-in caller, or the pending
/// sign-in of the `two_factor` cookie.
enum TwoFactorSubject {
    Session(AuthSession),
    Pending { user: User, verification_identifier: String, dont_remember: bool },
}

impl TwoFactorSubject {
    fn user(&self) -> &User {
        match self {
            TwoFactorSubject::Session(s) => &s.user,
            TwoFactorSubject::Pending { user, .. } => user,
        }
    }

    /// `key`: the OTP storage key.
    fn key(&self) -> String {
        match self {
            TwoFactorSubject::Session(s) => format!("{}!{}", s.user.id, s.session.id),
            TwoFactorSubject::Pending { verification_identifier, .. } => verification_identifier.clone(),
        }
    }
}

fn invalid(code: ErrorCode) -> EndpointError {
    ApiError::from("UNAUTHORIZED", code).into()
}

async fn verify_two_factor(ctx: &mut EndpointCtx) -> Result<TwoFactorSubject, EndpointError> {
    if let Some(session) = ctx.session_from_ctx().await {
        return Ok(TwoFactorSubject::Session(session));
    }
    let cookie = auth_cookie(ctx, TWO_FACTOR_COOKIE_NAME, None);
    let Some(identifier) = ctx.signed_cookie_value(&cookie.name) else {
        return Err(invalid(codes::INVALID_TWO_FACTOR_COOKIE));
    };
    let Some(verification) = adapter::find_verification_value(ctx.db(), &identifier).await? else {
        return Err(invalid(codes::INVALID_TWO_FACTOR_COOKIE));
    };
    let Some(user) = adapter::find_user_by_id(ctx.db(), &verification.value).await? else {
        return Err(invalid(codes::INVALID_TWO_FACTOR_COOKIE));
    };
    let dont_remember = ctx.signed_cookie_value(&ctx.state.auth_cookies.dont_remember.name).is_some();
    Ok(TwoFactorSubject::Pending { user, verification_identifier: identifier, dont_remember })
}

/// `valid(ctx)`: a pending sign-in becomes a session (optionally trusting
/// the device); a signed-in caller just gets its own token back.
async fn valid(ctx: &mut EndpointCtx, subject: TwoFactorSubject) -> HandlerResult {
    match subject {
        TwoFactorSubject::Session(s) => {
            let user = serde_json::to_value(UserOut::from(&s.user)).map_err(anyhow::Error::from)?;
            ctx.json(json!({ "token": s.session.token, "user": user }))
        }
        TwoFactorSubject::Pending { user, verification_identifier, dont_remember } => {
            let ip = ctx.ip().unwrap_or_default();
            let ua = ctx.user_agent();
            let session = match adapter::create_session(ctx.db(), &user.id, dont_remember, &ip, &ua, None, false).await
            {
                Ok(s) => s,
                Err(CreateSessionError::Banned) => return Err(banned_error()),
                Err(CreateSessionError::Db(e)) => return Err(e.into()),
            };
            adapter::delete_verification_by_identifier(ctx.db(), &verification_identifier).await?;
            let token = session.token.clone();
            set_session_cookie(ctx, AuthSession { session, user: user.clone() }, None, None);
            let two_factor_cookie = auth_cookie(ctx, TWO_FACTOR_COOKIE_NAME, None);
            ctx.expire_cookie(&two_factor_cookie);
            if ctx.body_field("trustDevice") == Some(&Value::Bool(true)) {
                let trust_cookie = auth_cookie(ctx, TRUST_DEVICE_COOKIE_NAME, Some(TRUST_DEVICE_MAX_AGE));
                let identifier = format!("trust-device-{}", generate_random_string(32));
                let mac = hmac_sha256_base64url(ctx.secret(), &format!("{}!{identifier}", user.id));
                let expires = JsDate::from_millis(JsDate::now().millis() + TRUST_DEVICE_MAX_AGE * 1000)
                    .unwrap_or_else(JsDate::now);
                adapter::create_verification_value(ctx.db(), &identifier, &user.id, expires).await?;
                let value = super::cookies::signed_value(&format!("{mac}!{identifier}"), ctx.secret());
                ctx.set_cookie(trust_cookie.set(value));
                let dont_remember_cookie = ctx.state.auth_cookies.dont_remember.clone();
                ctx.expire_cookie(&dont_remember_cookie);
            }
            let user_out = serde_json::to_value(UserOut::from(&user)).map_err(anyhow::Error::from)?;
            ctx.json(json!({ "token": token, "user": user_out }))
        }
    }
}

/// The caller's session when an endpoint enables 2FA for the first time:
/// a new session row carrying the old one's details, the user flag flipped,
/// the old row deleted.
async fn replace_session_with_two_factor(
    ctx: &mut EndpointCtx,
    session: &AuthSession,
    enabled: bool,
) -> Result<User, EndpointError> {
    let updated = adapter::update_user(ctx.db(), &session.user.id, &[("twoFactorEnabled", SqlValue::Bool(enabled))])
        .await?
        .unwrap_or_else(|| {
            let mut u = session.user.clone();
            u.two_factor_enabled = Some(enabled);
            u
        });
    let ip = ctx.ip().unwrap_or_default();
    let ua = ctx.user_agent();
    let over = SessionOverride::from_session(&session.session);
    let new_session = match adapter::create_session(ctx.db(), &updated.id, false, &ip, &ua, Some(&over), false).await {
        Ok(s) => s,
        Err(CreateSessionError::Banned) => return Err(banned_error()),
        Err(CreateSessionError::Db(e)) => return Err(e.into()),
    };
    set_session_cookie(ctx, AuthSession { session: new_session, user: updated.clone() }, None, None);
    adapter::delete_session(ctx.db(), &session.session.token).await?;
    Ok(updated)
}

/// `shouldRequirePassword(...)` is true (`allowPasswordless` off): the body's
/// password must be present and right.
async fn require_password_valid(ctx: &EndpointCtx, user_id: &str) -> Result<(), EndpointError> {
    let password = ctx.body_str("password").unwrap_or_default().to_string();
    if password.is_empty() {
        return Err(ApiError::from("BAD_REQUEST", base::INVALID_PASSWORD).into());
    }
    if !validate_password(ctx, user_id, &password).await? {
        return Err(ApiError::from("BAD_REQUEST", base::INVALID_PASSWORD).into());
    }
    Ok(())
}

// ── Endpoints ───────────────────────────────────────────────────────────────

fn enable(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let session = ctx.session.clone().expect("session middleware ran");
        require_password_valid(ctx, &session.user.id).await?;
        let secret = generate_random_string(32);
        let encrypted_secret = symmetric_encrypt(ctx.secret(), &secret);
        let backup_codes = generate_backup_codes();
        let encrypted_codes = encode_backup_codes(ctx.secret(), &backup_codes);
        let existing = adapter::find_two_factor(ctx.db(), &session.user.id).await?;
        adapter::delete_two_factor(ctx.db(), &session.user.id).await?;
        let verified = existing.is_some_and(|e| e.verified != Some(false));
        adapter::create_two_factor(ctx.db(), &session.user.id, &encrypted_secret, &encrypted_codes, verified).await?;
        let issuer = ctx
            .body_str("issuer")
            .filter(|i| !i.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| ctx.state.config.app_name.clone());
        let uri = totp_uri(&secret, &issuer, &session.user.email);
        ctx.json(json!({ "totpURI": uri, "backupCodes": backup_codes }))
    })
}

fn disable(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let session = ctx.session.clone().expect("session middleware ran");
        require_password_valid(ctx, &session.user.id).await?;
        let updated = adapter::update_user(ctx.db(), &session.user.id, &[("twoFactorEnabled", SqlValue::Bool(false))])
            .await?
            .ok_or_else(|| anyhow::anyhow!("Cannot read properties of null (reading 'id')"))?;
        adapter::delete_two_factor(ctx.db(), &updated.id).await?;
        let ip = ctx.ip().unwrap_or_default();
        let ua = ctx.user_agent();
        let over = SessionOverride::from_session(&session.session);
        let new_session =
            match adapter::create_session(ctx.db(), &updated.id, false, &ip, &ua, Some(&over), false).await {
                Ok(s) => s,
                Err(CreateSessionError::Banned) => return Err(banned_error()),
                Err(CreateSessionError::Db(e)) => return Err(e.into()),
            };
        set_session_cookie(ctx, AuthSession { session: new_session, user: updated }, None, None);
        adapter::delete_session(ctx.db(), &session.session.token).await?;
        let trust_cookie = auth_cookie(ctx, TRUST_DEVICE_COOKIE_NAME, Some(TRUST_DEVICE_MAX_AGE));
        if let Some(value) = ctx.signed_cookie_value(&trust_cookie.name) {
            if let Some(trust_id) = value.split('!').nth(1).filter(|t| !t.is_empty()) {
                adapter::delete_verification_by_identifier(ctx.db(), trust_id).await?;
            }
            ctx.expire_cookie(&trust_cookie);
        }
        ctx.json(json!({ "status": true }))
    })
}

fn get_totp_uri(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let session = ctx.session.clone().expect("session middleware ran");
        let Some(row) = adapter::find_two_factor(ctx.db(), &session.user.id).await? else {
            return Err(ApiError::from("BAD_REQUEST", codes::TOTP_NOT_ENABLED).into());
        };
        let secret = symmetric_decrypt(ctx.secret(), &row.secret).map_err(|e| anyhow::anyhow!("{e}"))?;
        if ctx.body_str("password").is_none_or(str::is_empty) {
            return Err(ApiError::from("BAD_REQUEST", base::INVALID_PASSWORD).into());
        }
        check_password(ctx, &session.user.id).await?;
        let uri = totp_uri(&secret, &ctx.state.config.app_name, &session.user.email);
        ctx.json(json!({ "totpURI": uri }))
    })
}

fn verify_totp_endpoint(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let subject = verify_two_factor(ctx).await?;
        let is_sign_in = matches!(subject, TwoFactorSubject::Pending { .. });
        let user = subject.user().clone();
        let Some(row) = adapter::find_two_factor(ctx.db(), &user.id).await? else {
            return Err(ApiError::from("BAD_REQUEST", codes::TOTP_NOT_ENABLED).into());
        };
        if is_sign_in && row.verified == Some(false) {
            return Err(ApiError::from("BAD_REQUEST", codes::TOTP_NOT_ENABLED).into());
        }
        let secret = symmetric_decrypt(ctx.secret(), &row.secret).map_err(|e| anyhow::anyhow!("{e}"))?;
        let code = ctx.body_str("code").unwrap_or_default().to_string();
        if !verify_totp(&secret, &code, JsDate::now().millis()) {
            return Err(invalid(codes::INVALID_CODE));
        }
        if row.verified != Some(true) {
            if user.two_factor_enabled != Some(true)
                && let TwoFactorSubject::Session(session) = &subject
            {
                replace_session_with_two_factor(ctx, session, true).await?;
            }
            mark_verified(ctx, &row).await?;
        }
        valid(ctx, subject).await
    })
}

async fn mark_verified(ctx: &EndpointCtx, row: &TwoFactor) -> Result<(), EndpointError> {
    sqlx::query("UPDATE \"two_factor\" SET \"verified\" = true WHERE \"id\" = $1")
        .bind(&row.id)
        .execute(ctx.db())
        .await?;
    Ok(())
}

fn send_otp(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        tracing::error!("send otp isn't configured. Please configure the send otp function on otp options.");
        let _ = ctx;
        Err(ApiError::with_message("BAD_REQUEST", "OTP_NOT_CONFIGURED", "otp isn't configured").into())
    })
}

fn verify_otp(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let subject = verify_two_factor(ctx).await?;
        let identifier = format!("2fa-otp-{}", subject.key());
        let stored = adapter::find_verification_value(ctx.db(), &identifier).await?;
        let Some(stored) = stored.filter(|v| v.expires_at.millis() >= JsDate::now().millis()) else {
            if adapter::find_verification_value(ctx.db(), &identifier).await?.is_some() {
                adapter::delete_verification_by_identifier(ctx.db(), &identifier).await?;
            }
            return Err(ApiError::from("BAD_REQUEST", codes::OTP_HAS_EXPIRED).into());
        };
        // Codes are only ever stored by `send-otp`, which is not configured:
        // compare what is there (hashed form `<hash>:<counter>`).
        let mut parts = stored.value.splitn(2, ':');
        let otp = parts.next().unwrap_or_default().to_string();
        let counter: i64 = parts.next().and_then(|c| c.parse().ok()).unwrap_or(0);
        if counter >= 5 {
            adapter::delete_verification_by_identifier(ctx.db(), &identifier).await?;
            return Err(ApiError::from("BAD_REQUEST", codes::TOO_MANY_ATTEMPTS_REQUEST_NEW_CODE).into());
        }
        let input = ctx.body_str("code").unwrap_or_default();
        let hashed = {
            use base64::Engine;
            use sha2::Digest;
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(input.as_bytes()))
        };
        if constant_time_eq(otp.as_bytes(), hashed.as_bytes()) {
            return valid(ctx, subject).await;
        }
        adapter::update_verification_value(ctx.db(), &identifier, &format!("{otp}:{}", counter + 1)).await?;
        Err(invalid(codes::INVALID_CODE))
    })
}

fn verify_backup_code(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let subject = verify_two_factor(ctx).await?;
        let user = subject.user().clone();
        let Some(row) = adapter::find_two_factor(ctx.db(), &user.id).await? else {
            return Err(ApiError::from("BAD_REQUEST", codes::BACKUP_CODES_NOT_ENABLED).into());
        };
        let code = ctx.body_str("code").unwrap_or_default().to_string();
        let Some(codes_now) = decode_backup_codes(ctx.secret(), &row.backup_codes) else {
            return Err(ApiError::from("UNAUTHORIZED", codes::INVALID_BACKUP_CODE).into());
        };
        if !codes_now.contains(&code) {
            return Err(ApiError::from("UNAUTHORIZED", codes::INVALID_BACKUP_CODE).into());
        }
        let remaining: Vec<String> = codes_now.into_iter().filter(|c| c != &code).collect();
        let encoded = encode_backup_codes(ctx.secret(), &remaining);
        let updated = sqlx::query(
            "UPDATE \"two_factor\" SET \"backup_codes\" = $1 WHERE \"id\" = $2 AND \"backup_codes\" = $3 RETURNING \"id\"",
        )
        .bind(&encoded)
        .bind(&row.id)
        .bind(&row.backup_codes)
        .fetch_optional(ctx.db())
        .await?;
        if updated.is_none() {
            return Err(ApiError::message_only("CONFLICT", "Failed to verify backup code. Please try again.").into());
        }
        if ctx.body_field("disableSession") != Some(&Value::Bool(true)) {
            return valid(ctx, subject).await;
        }
        let token = match &subject {
            TwoFactorSubject::Session(s) => Value::String(s.session.token.clone()),
            TwoFactorSubject::Pending { .. } => Value::Null,
        };
        let mut out = serde_json::Map::new();
        if !token.is_null() {
            out.insert("token".into(), token);
        }
        out.insert("user".into(), serde_json::to_value(UserOut::from(&user)).map_err(anyhow::Error::from)?);
        Ok(Out::Json(Value::Object(out)))
    })
}

fn generate_backup_codes_endpoint(ctx: &mut EndpointCtx) -> BoxFuture<'_, HandlerResult> {
    Box::pin(async move {
        let session = ctx.session.clone().expect("session middleware ran");
        if session.user.two_factor_enabled != Some(true) {
            return Err(ApiError::from("BAD_REQUEST", codes::TWO_FACTOR_NOT_ENABLED).into());
        }
        if ctx.body_str("password").is_none_or(str::is_empty) {
            return Err(ApiError::from("BAD_REQUEST", base::INVALID_PASSWORD).into());
        }
        check_password(ctx, &session.user.id).await?;
        let Some(row) = adapter::find_two_factor(ctx.db(), &session.user.id).await? else {
            return Err(ApiError::from("BAD_REQUEST", codes::TWO_FACTOR_NOT_ENABLED).into());
        };
        let backup_codes = generate_backup_codes();
        let encoded = encode_backup_codes(ctx.secret(), &backup_codes);
        sqlx::query("UPDATE \"two_factor\" SET \"backup_codes\" = $1 WHERE \"id\" = $2")
            .bind(&encoded)
            .bind(&row.id)
            .execute(ctx.db())
            .await?;
        ctx.json(json!({ "status": true, "backupCodes": backup_codes }))
    })
}

// ── The sign-in after hook ──────────────────────────────────────────────────

/// The plugin's `hooks.after` on `/sign-in/email`: an account with 2FA does
/// not get the session the sign-in just created, unless the browser carries a
/// valid trust-device cookie (which is then rotated).
pub async fn sign_in_after_hook(ctx: &mut EndpointCtx, result: HandlerResult) -> HandlerResult {
    let Some(data) = ctx.new_session.clone() else {
        return result;
    };
    if data.user.two_factor_enabled != Some(true) {
        return result;
    }
    let trust_cookie = auth_cookie(ctx, TRUST_DEVICE_COOKIE_NAME, Some(TRUST_DEVICE_MAX_AGE));
    if let Some(value) = ctx.signed_cookie_value(&trust_cookie.name) {
        let mut parts = value.split('!');
        let token = parts.next().unwrap_or_default().to_string();
        let identifier = parts.next().unwrap_or_default().to_string();
        if !token.is_empty() && !identifier.is_empty() {
            let expected = hmac_sha256_base64url(ctx.secret(), &format!("{}!{identifier}", data.user.id));
            if token == expected {
                let record = adapter::find_verification_value(ctx.db(), &identifier).await?;
                if record.is_some_and(|r| r.value == data.user.id && r.expires_at.millis() > JsDate::now().millis()) {
                    adapter::delete_verification_by_identifier(ctx.db(), &identifier).await?;
                    let new_identifier = format!("trust-device-{}", generate_random_string(32));
                    let new_token = hmac_sha256_base64url(ctx.secret(), &format!("{}!{new_identifier}", data.user.id));
                    let expires = JsDate::from_millis(JsDate::now().millis() + TRUST_DEVICE_MAX_AGE * 1000)
                        .unwrap_or_else(JsDate::now);
                    adapter::create_verification_value(ctx.db(), &new_identifier, &data.user.id, expires).await?;
                    let signed = super::cookies::signed_value(&format!("{new_token}!{new_identifier}"), ctx.secret());
                    ctx.set_cookie(trust_cookie.set(signed));
                    return result;
                }
            }
        }
        ctx.expire_cookie(&trust_cookie);
    }
    // Remove the session cookie the sign-in set, and the session itself.
    delete_session_cookie(ctx, true);
    adapter::delete_session(ctx.db(), &data.session.token).await?;
    ctx.new_session = None;
    let two_factor_cookie = auth_cookie(ctx, TWO_FACTOR_COOKIE_NAME, Some(TWO_FACTOR_COOKIE_MAX_AGE));
    let identifier = format!("2fa-{}", generate_random_string(20));
    let expires =
        JsDate::from_millis(JsDate::now().millis() + TWO_FACTOR_COOKIE_MAX_AGE * 1000).unwrap_or_else(JsDate::now);
    adapter::create_verification_value(ctx.db(), &identifier, &data.user.id, expires).await?;
    ctx.set_cookie(two_factor_cookie.set(super::cookies::signed_value(&identifier, ctx.secret())));
    // TOTP needs a stored secret; OTP is not configured.
    let mut methods = Vec::new();
    if adapter::find_two_factor(ctx.db(), &data.user.id).await?.is_some_and(|t| t.verified != Some(false)) {
        methods.push(Value::String("totp".into()));
    }
    Ok(Out::Json(json!({ "twoFactorRedirect": true, "twoFactorMethods": methods })))
}
