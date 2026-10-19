//! better-call's `APIError` and Better Auth's error codes, with the exact wire
//! form the TypeScript server answered with:
//!
//! - status code, and the HTTP reason phrase = the status NAME the error was
//!   created with (`400 BAD_REQUEST`), or the number when it was created with a
//!   number (`400 400` for validation errors);
//! - body = the APIError body serialised as JSON (`{"message":..,"code":..}`
//!   for `APIError.from(status, CODE)`), or no body at all, still with
//!   `content-type: application/json`;
//! - extra headers (a `location` for `ctx.redirect`).

use axum::http::StatusCode;
use serde_json::{Map, Value, json};

/// One entry of Better Auth's error-code tables (`defineErrorCodes`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorCode {
    pub code: &'static str,
    pub message: &'static str,
}

const fn ec(code: &'static str, message: &'static str) -> ErrorCode {
    ErrorCode { code, message }
}

/// `BASE_ERROR_CODES` (@better-auth/core 1.6.13), the ones this port uses.
pub mod base {
    use super::{ErrorCode, ec};
    pub const USER_NOT_FOUND: ErrorCode = ec("USER_NOT_FOUND", "User not found");
    pub const PROVIDER_NOT_FOUND: ErrorCode = ec("PROVIDER_NOT_FOUND", "Provider not found");
    pub const FAILED_TO_CREATE_USER: ErrorCode = ec("FAILED_TO_CREATE_USER", "Failed to create user");
    pub const FAILED_TO_CREATE_SESSION: ErrorCode = ec("FAILED_TO_CREATE_SESSION", "Failed to create session");
    pub const FAILED_TO_GET_SESSION: ErrorCode = ec("FAILED_TO_GET_SESSION", "Failed to get session");
    pub const INVALID_PASSWORD: ErrorCode = ec("INVALID_PASSWORD", "Invalid password");
    pub const INVALID_EMAIL: ErrorCode = ec("INVALID_EMAIL", "Invalid email");
    pub const INVALID_EMAIL_OR_PASSWORD: ErrorCode = ec("INVALID_EMAIL_OR_PASSWORD", "Invalid email or password");
    pub const INVALID_USER: ErrorCode = ec("INVALID_USER", "Invalid user");
    pub const INVALID_TOKEN: ErrorCode = ec("INVALID_TOKEN", "Invalid token");
    pub const TOKEN_EXPIRED: ErrorCode = ec("TOKEN_EXPIRED", "Token expired");
    pub const EMAIL_NOT_VERIFIED: ErrorCode = ec("EMAIL_NOT_VERIFIED", "Email not verified");
    pub const PASSWORD_TOO_SHORT: ErrorCode = ec("PASSWORD_TOO_SHORT", "Password too short");
    pub const PASSWORD_TOO_LONG: ErrorCode = ec("PASSWORD_TOO_LONG", "Password too long");
    pub const USER_ALREADY_EXISTS_USE_ANOTHER_EMAIL: ErrorCode =
        ec("USER_ALREADY_EXISTS_USE_ANOTHER_EMAIL", "User already exists. Use another email.");
    pub const EMAIL_CAN_NOT_BE_UPDATED: ErrorCode = ec("EMAIL_CAN_NOT_BE_UPDATED", "Email can not be updated");
    pub const CHANGE_EMAIL_DISABLED: ErrorCode = ec("CHANGE_EMAIL_DISABLED", "Change email is disabled");
    pub const CREDENTIAL_ACCOUNT_NOT_FOUND: ErrorCode =
        ec("CREDENTIAL_ACCOUNT_NOT_FOUND", "Credential account not found");
    pub const SESSION_EXPIRED: ErrorCode =
        ec("SESSION_EXPIRED", "Session expired. Re-authenticate to perform this action.");
    pub const FAILED_TO_UNLINK_LAST_ACCOUNT: ErrorCode =
        ec("FAILED_TO_UNLINK_LAST_ACCOUNT", "You can't unlink your last account");
    pub const ACCOUNT_NOT_FOUND: ErrorCode = ec("ACCOUNT_NOT_FOUND", "Account not found");
    pub const CROSS_SITE_NAVIGATION_LOGIN_BLOCKED: ErrorCode = ec(
        "CROSS_SITE_NAVIGATION_LOGIN_BLOCKED",
        "Cross-site navigation login blocked. This request appears to be a CSRF attack.",
    );
    pub const VERIFICATION_EMAIL_NOT_ENABLED: ErrorCode =
        ec("VERIFICATION_EMAIL_NOT_ENABLED", "Verification email isn't enabled");
    pub const EMAIL_ALREADY_VERIFIED: ErrorCode = ec("EMAIL_ALREADY_VERIFIED", "Email is already verified");
    pub const EMAIL_MISMATCH: ErrorCode = ec("EMAIL_MISMATCH", "Email mismatch");
    pub const SESSION_NOT_FRESH: ErrorCode = ec("SESSION_NOT_FRESH", "Session is not fresh");
    pub const INVALID_ORIGIN: ErrorCode = ec("INVALID_ORIGIN", "Invalid origin");
    pub const INVALID_CALLBACK_URL: ErrorCode = ec("INVALID_CALLBACK_URL", "Invalid callbackURL");
    pub const INVALID_REDIRECT_URL: ErrorCode = ec("INVALID_REDIRECT_URL", "Invalid redirectURL");
    pub const INVALID_ERROR_CALLBACK_URL: ErrorCode = ec("INVALID_ERROR_CALLBACK_URL", "Invalid errorCallbackURL");
    pub const INVALID_NEW_USER_CALLBACK_URL: ErrorCode =
        ec("INVALID_NEW_USER_CALLBACK_URL", "Invalid newUserCallbackURL");
    pub const MISSING_OR_NULL_ORIGIN: ErrorCode = ec("MISSING_OR_NULL_ORIGIN", "Missing or null Origin");
    pub const FIELD_NOT_ALLOWED: ErrorCode = ec("FIELD_NOT_ALLOWED", "Field not allowed to be set");
    pub const VALIDATION_ERROR: ErrorCode = ec("VALIDATION_ERROR", "Validation Error");
    pub const MISSING_FIELD: ErrorCode = ec("MISSING_FIELD", "Field is required");
    pub const METHOD_NOT_ALLOWED_DEFER_SESSION_REQUIRED: ErrorCode = ec(
        "METHOD_NOT_ALLOWED_DEFER_SESSION_REQUIRED",
        "POST method requires deferSessionRefresh to be enabled in session config",
    );
    pub const BODY_MUST_BE_AN_OBJECT: ErrorCode = ec("BODY_MUST_BE_AN_OBJECT", "Body must be an object");
    pub const PASSWORD_ALREADY_SET: ErrorCode = ec("PASSWORD_ALREADY_SET", "User already has a password set");
}

/// `TWO_FACTOR_ERROR_CODES`.
pub mod two_factor {
    use super::{ErrorCode, ec};
    pub const OTP_HAS_EXPIRED: ErrorCode = ec("OTP_HAS_EXPIRED", "OTP has expired");
    pub const TOTP_NOT_ENABLED: ErrorCode = ec("TOTP_NOT_ENABLED", "TOTP not enabled");
    pub const TWO_FACTOR_NOT_ENABLED: ErrorCode = ec("TWO_FACTOR_NOT_ENABLED", "Two factor isn't enabled");
    pub const BACKUP_CODES_NOT_ENABLED: ErrorCode = ec("BACKUP_CODES_NOT_ENABLED", "Backup codes aren't enabled");
    pub const INVALID_BACKUP_CODE: ErrorCode = ec("INVALID_BACKUP_CODE", "Invalid backup code");
    pub const INVALID_CODE: ErrorCode = ec("INVALID_CODE", "Invalid code");
    pub const INVALID_TWO_FACTOR_COOKIE: ErrorCode = ec("INVALID_TWO_FACTOR_COOKIE", "Invalid two factor cookie");
    pub const TOO_MANY_ATTEMPTS_REQUEST_NEW_CODE: ErrorCode =
        ec("TOO_MANY_ATTEMPTS_REQUEST_NEW_CODE", "Too many attempts. Please request a new code.");
}

/// `ADMIN_ERROR_CODES`.
pub mod admin {
    use super::{ErrorCode, ec};
    pub const FAILED_TO_CREATE_USER: ErrorCode = ec("FAILED_TO_CREATE_USER", "Failed to create user");
    pub const USER_ALREADY_EXISTS_USE_ANOTHER_EMAIL: ErrorCode =
        ec("USER_ALREADY_EXISTS_USE_ANOTHER_EMAIL", "User already exists. Use another email.");
    pub const YOU_CANNOT_BAN_YOURSELF: ErrorCode = ec("YOU_CANNOT_BAN_YOURSELF", "You cannot ban yourself");
    pub const YOU_ARE_NOT_ALLOWED_TO_CHANGE_USERS_ROLE: ErrorCode =
        ec("YOU_ARE_NOT_ALLOWED_TO_CHANGE_USERS_ROLE", "You are not allowed to change users role");
    pub const YOU_ARE_NOT_ALLOWED_TO_CREATE_USERS: ErrorCode =
        ec("YOU_ARE_NOT_ALLOWED_TO_CREATE_USERS", "You are not allowed to create users");
    pub const YOU_ARE_NOT_ALLOWED_TO_LIST_USERS: ErrorCode =
        ec("YOU_ARE_NOT_ALLOWED_TO_LIST_USERS", "You are not allowed to list users");
    pub const YOU_ARE_NOT_ALLOWED_TO_LIST_USERS_SESSIONS: ErrorCode =
        ec("YOU_ARE_NOT_ALLOWED_TO_LIST_USERS_SESSIONS", "You are not allowed to list users sessions");
    pub const YOU_ARE_NOT_ALLOWED_TO_BAN_USERS: ErrorCode =
        ec("YOU_ARE_NOT_ALLOWED_TO_BAN_USERS", "You are not allowed to ban users");
    pub const YOU_ARE_NOT_ALLOWED_TO_IMPERSONATE_USERS: ErrorCode =
        ec("YOU_ARE_NOT_ALLOWED_TO_IMPERSONATE_USERS", "You are not allowed to impersonate users");
    pub const YOU_ARE_NOT_ALLOWED_TO_REVOKE_USERS_SESSIONS: ErrorCode =
        ec("YOU_ARE_NOT_ALLOWED_TO_REVOKE_USERS_SESSIONS", "You are not allowed to revoke users sessions");
    pub const YOU_ARE_NOT_ALLOWED_TO_DELETE_USERS: ErrorCode =
        ec("YOU_ARE_NOT_ALLOWED_TO_DELETE_USERS", "You are not allowed to delete users");
    pub const YOU_ARE_NOT_ALLOWED_TO_SET_USERS_PASSWORD: ErrorCode =
        ec("YOU_ARE_NOT_ALLOWED_TO_SET_USERS_PASSWORD", "You are not allowed to set users password");
    pub const YOU_ARE_NOT_ALLOWED_TO_GET_USER: ErrorCode =
        ec("YOU_ARE_NOT_ALLOWED_TO_GET_USER", "You are not allowed to get user");
    pub const NO_DATA_TO_UPDATE: ErrorCode = ec("NO_DATA_TO_UPDATE", "No data to update");
    pub const YOU_ARE_NOT_ALLOWED_TO_UPDATE_USERS: ErrorCode =
        ec("YOU_ARE_NOT_ALLOWED_TO_UPDATE_USERS", "You are not allowed to update users");
    pub const YOU_CANNOT_REMOVE_YOURSELF: ErrorCode = ec("YOU_CANNOT_REMOVE_YOURSELF", "You cannot remove yourself");
    pub const YOU_CANNOT_IMPERSONATE_ADMINS: ErrorCode =
        ec("YOU_CANNOT_IMPERSONATE_ADMINS", "You cannot impersonate admins");
    pub const INVALID_ROLE_TYPE: ErrorCode = ec("INVALID_ROLE_TYPE", "Invalid role type");
}

/// better-call's `statusCodes` table, name -> number.
pub fn status_from_name(name: &str) -> StatusCode {
    let n = match name {
        "OK" => 200,
        "CREATED" => 201,
        "FOUND" => 302,
        "BAD_REQUEST" => 400,
        "UNAUTHORIZED" => 401,
        "FORBIDDEN" => 403,
        "NOT_FOUND" => 404,
        "METHOD_NOT_ALLOWED" => 405,
        "CONFLICT" => 409,
        "UNSUPPORTED_MEDIA_TYPE" => 415,
        "UNPROCESSABLE_ENTITY" => 422,
        "TOO_MANY_REQUESTS" => 429,
        "INTERNAL_SERVER_ERROR" => 500,
        _ => 500,
    };
    StatusCode::from_u16(n).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

/// A thrown `APIError` (or a redirect via `ctx.redirect`, which is one too).
#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    pub status: StatusCode,
    /// The reason phrase: the status name, or the number as text.
    pub status_text: String,
    /// The JSON body; `None` sends no body.
    pub body: Option<Value>,
    /// `location` for redirects.
    pub location: Option<String>,
}

impl ApiError {
    /// `new APIError(name, body)` / `APIError.fromStatus(name, body)`.
    pub fn named(name: &'static str, body: Option<Value>) -> ApiError {
        ApiError { status: status_from_name(name), status_text: name.to_string(), body, location: None }
    }

    /// `APIError.from(name, { message, code })`: body `{message, code}`.
    pub fn from(name: &'static str, code: ErrorCode) -> ApiError {
        ApiError::named(name, Some(json!({ "message": code.message, "code": code.code })))
    }

    /// `APIError.from(name, { ...code, message })`: a code with its own message.
    pub fn with_message(name: &'static str, code: &str, message: impl Into<String>) -> ApiError {
        ApiError::named(name, Some(json!({ "message": message.into(), "code": code })))
    }

    /// `APIError.fromStatus(name, { message })`: no code.
    pub fn message_only(name: &'static str, message: impl Into<String>) -> ApiError {
        ApiError::named(name, Some(json!({ "message": message.into() })))
    }

    /// `new APIError(400, { message, code: "VALIDATION_ERROR" })` (numeric status).
    pub fn numeric(status: u16, body: Value) -> ApiError {
        ApiError {
            status: StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
            status_text: status.to_string(),
            body: Some(body),
            location: None,
        }
    }

    /// `ctx.redirect(url)`: a 302 `FOUND` with no body.
    pub fn redirect(location: impl Into<String>) -> ApiError {
        ApiError { status: StatusCode::FOUND, status_text: "FOUND".into(), body: None, location: Some(location.into()) }
    }

    pub fn code(&self) -> Option<&str> {
        self.body.as_ref().and_then(|b| b.get("code")).and_then(Value::as_str)
    }

    pub fn message(&self) -> Option<&str> {
        self.body.as_ref().and_then(|b| b.get("message")).and_then(Value::as_str)
    }

    /// `{ code, message }` style body as a map, for server-side callers that
    /// read `err.body.code`.
    pub fn body_map(&self) -> Map<String, Value> {
        match &self.body {
            Some(Value::Object(m)) => m.clone(),
            _ => Map::new(),
        }
    }
}

/// What can end an endpoint other than a value.
#[derive(Debug)]
pub enum EndpointError {
    /// A thrown `APIError` (answered with its status and body).
    Api(ApiError),
    /// Anything else (answered 500 with no body, as better-call does).
    Internal(anyhow::Error),
}

impl From<ApiError> for EndpointError {
    fn from(e: ApiError) -> Self {
        EndpointError::Api(e)
    }
}

impl From<anyhow::Error> for EndpointError {
    fn from(e: anyhow::Error) -> Self {
        EndpointError::Internal(e)
    }
}

impl From<sqlx::Error> for EndpointError {
    fn from(e: sqlx::Error) -> Self {
        EndpointError::Internal(e.into())
    }
}

impl std::fmt::Display for EndpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EndpointError::Api(e) => write!(f, "{} {}", e.status.as_u16(), e.message().unwrap_or("")),
            EndpointError::Internal(e) => write!(f, "{e:#}"),
        }
    }
}

pub type EndpointResult<T> = Result<T, EndpointError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies_and_reason_phrases_match_better_call() {
        let e = ApiError::from("UNAUTHORIZED", base::INVALID_EMAIL_OR_PASSWORD);
        assert_eq!(e.status, StatusCode::UNAUTHORIZED);
        assert_eq!(e.status_text, "UNAUTHORIZED");
        assert_eq!(
            serde_json::to_string(&e.body).unwrap(),
            r#"{"message":"Invalid email or password","code":"INVALID_EMAIL_OR_PASSWORD"}"#
        );
        let v = ApiError::numeric(400, json!({"message": "x", "code": "VALIDATION_ERROR"}));
        assert_eq!(v.status_text, "400");
        let r = ApiError::redirect("/x");
        assert_eq!(r.status.as_u16(), 302);
        assert!(r.body.is_none());
    }
}
