//! The server-side calls upstream makes through `auth.api.*` (PORT.md 9):
//! the same database effects as the Better Auth endpoints, as plain Rust
//! functions for actions and pages. Session resolution (`getSession` with the
//! request headers) is the caller's [`crate::auth_helpers::RequestSession`];
//! each function takes the session it resolved.

use sqlx::PgPool;

use super::session::AuthSession;
use crate::db::models::{Columns, Session};
use crate::db::types::JsDate;

/// `internalAdapter.listSessions(userId)`: every session row of the user (no
/// ORDER BY upstream; callers sort).
pub async fn list_user_session_rows(db: &PgPool, user_id: &str) -> sqlx::Result<Vec<Session>> {
    let sql = format!("SELECT {} FROM {} WHERE \"user_id\" = $1", Session::COLUMNS, Session::TABLE);
    sqlx::query_as::<_, Session>(&sql).bind(user_id).fetch_all(db).await
}

/// `auth.api.listSessions({ headers })` (`/list-sessions`): the caller's
/// sessions with `expires_at > now`, then the admin plugin's after-hook drops
/// the ones that carry `impersonated_by`.
pub async fn list_sessions(db: &PgPool, current: &AuthSession) -> sqlx::Result<Vec<Session>> {
    let now = JsDate::now();
    let sql =
        format!("SELECT {} FROM {} WHERE \"user_id\" = $1 AND \"expires_at\" > $2", Session::COLUMNS, Session::TABLE);
    let rows = sqlx::query_as::<_, Session>(&sql).bind(&current.user.id).bind(now).fetch_all(db).await?;
    Ok(rows
        .into_iter()
        .filter(|s| s.expires_at.millis() > now.millis())
        .filter(|s| s.impersonated_by.as_deref().is_none_or(str::is_empty))
        .collect())
}

/// `auth.api.revokeSession({ body: { token }, headers })` (`/revoke-session`):
/// deletes the session with that token only when it belongs to the caller;
/// answers `{status: true}` either way.
pub async fn revoke_session(db: &PgPool, current: &AuthSession, token: &str) -> sqlx::Result<()> {
    let owner: Option<(String,)> = sqlx::query_as("SELECT \"user_id\" FROM \"session\" WHERE \"token\" = $1 LIMIT 1")
        .bind(token)
        .fetch_optional(db)
        .await?;
    if owner.is_some_and(|(user_id,)| user_id == current.user.id) {
        sqlx::query("DELETE FROM \"session\" WHERE \"token\" = $1").bind(token).execute(db).await?;
    }
    Ok(())
}

/// `auth.api.revokeOtherSessions({ headers })` (`/revoke-other-sessions`):
/// every unexpired session of the caller except the current one (impersonated
/// ones included: no admin filter on this path).
pub async fn revoke_other_sessions(db: &PgPool, current: &AuthSession) -> sqlx::Result<()> {
    let now = JsDate::now();
    for s in list_user_session_rows(db, &current.user.id).await? {
        if s.expires_at.millis() > now.millis() && s.token != current.session.token {
            sqlx::query("DELETE FROM \"session\" WHERE \"token\" = $1").bind(&s.token).execute(db).await?;
        }
    }
    Ok(())
}

// ── auth.api.* through the endpoint pipeline ────────────────────────────────

use std::net::SocketAddr;

use axum::http::HeaderMap;
use serde_json::{Map, Value};

use super::endpoint::{EndpointCtx, Out, call};
use super::error::{ApiError, EndpointError};
use crate::http::set_cookie::SetCookie;
use crate::state::AppState;

/// What a server-side `auth.api.*` call produced: the endpoint's outcome and
/// the cookies it set, which lib/auth.ts's `nextCookies()` plugin copies onto
/// the action's response (one cookie per name, the last value winning).
#[derive(Debug)]
pub struct ServerCall {
    pub result: Result<Out, EndpointError>,
    pub cookies: Vec<SetCookie>,
}

impl ServerCall {
    /// `asResponse: false`: the JSON value, or the `APIError` thrown.
    pub fn json(&self) -> Result<Value, ServerCallError> {
        match &self.result {
            Ok(Out::Json(v)) => Ok(v.clone()),
            Ok(_) => Ok(Value::Null),
            Err(EndpointError::Api(e)) => Err(ServerCallError::Api(e.clone())),
            Err(EndpointError::Internal(e)) => Err(ServerCallError::Other(e.to_string())),
        }
    }

    /// The cookies as Next's cookie store would end up holding them:
    /// `parseSetCookieHeader` of better-call's serialisation (one entry per
    /// name, first position, last value; a `__Secure-`/`__Host-` name was
    /// written with `Secure`), then `cookies().set(name, value, options)`.
    pub fn next_cookies(&self) -> Vec<SetCookie> {
        let mut out: Vec<SetCookie> = Vec::new();
        for c in &self.cookies {
            let mut c = c.clone();
            if c.name.starts_with("__Secure-") || c.name.starts_with("__Host-") {
                c.secure = true;
            }
            if let Some(m) = c.max_age
                && m < 0
            {
                // better-call does not print a negative Max-Age.
                c.max_age = None;
            }
            match out.iter_mut().find(|x| x.name == c.name) {
                Some(slot) => *slot = c,
                None => out.push(c),
            }
        }
        out
    }

    /// lib/auth.ts's `nextCookies()` plugin: the call's cookies become the
    /// server action's own (which also marks the action as revalidating).
    pub fn apply_cookies(&self, ctx: &crate::rpc::ActionCtx) {
        for c in self.next_cookies() {
            ctx.set_cookie(c);
        }
    }

    /// `asResponse: true`: whether the Response would be `ok` (2xx).
    pub fn response_ok(&self) -> bool {
        self.result.is_ok()
    }

    /// The `code` of the APIError body, when the call failed with one.
    pub fn error_code(&self) -> Option<String> {
        match &self.result {
            Err(EndpointError::Api(e)) => e.code().map(str::to_string),
            _ => None,
        }
    }
}

/// A thrown error of a server-side call: an `APIError` (with its body) or
/// anything else (its message).
#[derive(Debug, Clone)]
pub enum ServerCallError {
    Api(ApiError),
    Other(String),
}

impl ServerCallError {
    /// `err.message` as JavaScript reads it.
    pub fn message(&self) -> String {
        match self {
            ServerCallError::Api(e) => e.message().unwrap_or_default().to_string(),
            ServerCallError::Other(m) => m.clone(),
        }
    }
}

/// `auth.api.<endpoint>({ body, query, headers })`: the endpoint at `path`
/// with its hooks, validation and middlewares, but none of the router's checks
/// (no rate limit, no origin check: there is no Request).
pub async fn call_endpoint(
    state: &AppState,
    headers: Option<&HeaderMap>,
    peer: Option<SocketAddr>,
    path: &str,
    body: Option<Value>,
    query: Option<Map<String, Value>>,
) -> ServerCall {
    let ep = super::routes::by_path(path);
    let method = ep.methods.first().cloned().unwrap_or(axum::http::Method::POST);
    let mut ctx = EndpointCtx::new(
        state.clone(),
        method,
        headers.cloned().unwrap_or_default(),
        headers.is_some(),
        false,
        String::new(),
        peer,
        body,
        query.unwrap_or_default(),
        Map::new(),
    );
    let result = call(&mut ctx, ep).await;
    ServerCall { result, cookies: ctx.cookies }
}

fn obj(pairs: &[(&str, Value)]) -> Value {
    Value::Object(pairs.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect())
}

/// `auth.api.signInEmail({ body: { email, password }, headers })`.
pub async fn sign_in_email(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
    email: &str,
    password: &str,
) -> ServerCall {
    let body = obj(&[("email", Value::String(email.into())), ("password", Value::String(password.into()))]);
    call_endpoint(state, Some(headers), peer, "/sign-in/email", Some(body), None).await
}

/// `auth.api.sendVerificationEmail({ body: { email, callbackURL }, headers })`.
pub async fn send_verification_email(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
    email: &str,
    callback_url: &str,
) -> ServerCall {
    let body = obj(&[("email", Value::String(email.into())), ("callbackURL", Value::String(callback_url.into()))]);
    call_endpoint(state, Some(headers), peer, "/send-verification-email", Some(body), None).await
}

/// `auth.api.createUser({ body, headers })` (admin plugin).
pub async fn create_user(state: &AppState, headers: &HeaderMap, peer: Option<SocketAddr>, body: Value) -> ServerCall {
    call_endpoint(state, Some(headers), peer, "/admin/create-user", Some(body), None).await
}

/// `auth.api.setRole({ body: { userId, role }, headers })`.
pub async fn set_role(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
    user_id: &str,
    role: &Value,
) -> ServerCall {
    let body = obj(&[("userId", Value::String(user_id.into())), ("role", role.clone())]);
    call_endpoint(state, Some(headers), peer, "/admin/set-role", Some(body), None).await
}

/// `auth.api.banUser({ body: { userId }, headers })`.
pub async fn ban_user(state: &AppState, headers: &HeaderMap, peer: Option<SocketAddr>, user_id: &str) -> ServerCall {
    let body = obj(&[("userId", Value::String(user_id.into()))]);
    call_endpoint(state, Some(headers), peer, "/admin/ban-user", Some(body), None).await
}

/// `auth.api.unbanUser({ body: { userId }, headers })`.
pub async fn unban_user(state: &AppState, headers: &HeaderMap, peer: Option<SocketAddr>, user_id: &str) -> ServerCall {
    let body = obj(&[("userId", Value::String(user_id.into()))]);
    call_endpoint(state, Some(headers), peer, "/admin/unban-user", Some(body), None).await
}

/// `auth.api.revokeUserSessions({ body: { userId }, headers })`.
pub async fn revoke_user_sessions(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
    user_id: &str,
) -> ServerCall {
    let body = obj(&[("userId", Value::String(user_id.into()))]);
    call_endpoint(state, Some(headers), peer, "/admin/revoke-user-sessions", Some(body), None).await
}

/// `auth.api.removeUser({ body: { userId }, headers })`.
pub async fn remove_user(state: &AppState, headers: &HeaderMap, peer: Option<SocketAddr>, user_id: &str) -> ServerCall {
    let body = obj(&[("userId", Value::String(user_id.into()))]);
    call_endpoint(state, Some(headers), peer, "/admin/remove-user", Some(body), None).await
}
