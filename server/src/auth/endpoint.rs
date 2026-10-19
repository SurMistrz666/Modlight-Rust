//! One Better Auth endpoint call: the context the handler sees and the
//! pipeline every call goes through, whether it came over HTTP
//! (`/api/auth/*`, [`super::api`]) or from server code (`auth.api.*`,
//! [`super::server`]).
//!
//! The pipeline, as better-auth 1.6.13 `api/to-auth-endpoints.mjs` and
//! better-call 1.3.5 `endpoint.mjs` / `context.mjs` run it:
//!
//! 1. before hooks: lib/auth.ts's registration gate on `/sign-up/email`
//!    (it reads the RAW body, before validation); an error here is answered
//!    as is;
//! 2. validation of the body and the query (better-call `runValidation`,
//!    [`super::validate`]) -> `400 400 VALIDATION_ERROR`;
//! 3. the endpoint's `use` middlewares in order (form CSRF, origin checks,
//!    the session middlewares);
//! 4. the handler;
//! 5. after hooks, which run even when 2-4 threw an `APIError`: the admin
//!    plugin's `/list-sessions` filter and the twoFactor plugin's sign-in
//!    interception.
//!
//! Cookies a handler sets accumulate in order (better-call
//! `headers.append("set-cookie")`); `expireCookie` first removes earlier
//! entries of the same name. An `APIError` carries the cookies set before it
//! was thrown (`attachResponseHeadersToAPIError`).
//!
//! Sessions inside an endpoint (`getSessionFromCtx`): refreshed in the
//! database like any `getSession`, but the refresh cookie never reaches the
//! response (better-auth overwrites `ctx.context.responseHeaders` after the
//! handler), and a lookup failure counts as "no session".

use std::net::SocketAddr;

use axum::http::{HeaderMap, Method};
use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use super::cookies::{AuthCookie, SignedCookie, verify_signed_cookie};
use super::error::{ApiError, EndpointError};
use super::session::{AuthSession, RefreshPolicy, get_session};
use super::validate::{Schema, format_issues, parse};
use crate::http::cookies::parse_cookies_better_call;
use crate::http::set_cookie::SetCookie;
use crate::state::AppState;

/// What a handler produced.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    /// `ctx.json(value)` or a plain returned value.
    Json(Value),
    /// `new Response(null, { status, headers: { Location } })`.
    Redirect { status: u16, location: String },
    /// `new Response(html, { headers: { "Content-Type": "text/html" } })`.
    Html(String),
}

pub type HandlerResult = Result<Out, EndpointError>;
pub type Handler = for<'a> fn(&'a mut EndpointCtx) -> BoxFuture<'a, HandlerResult>;

/// An endpoint's `use` middlewares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Middleware {
    /// `formCsrfMiddleware` (sign-in, sign-up).
    FormCsrf,
    /// `sessionMiddleware`: 401 `{message: "Unauthorized", code: "UNAUTHORIZED"}`.
    Session,
    /// `sensitiveSessionMiddleware` (same answer; cookie cache bypassed).
    SensitiveSession,
    /// `freshSessionMiddleware`: also 403 SESSION_NOT_FRESH after a day.
    FreshSession,
    /// The admin plugin's `adminMiddleware`: 401 without a body.
    Admin,
    /// `originCheck((ctx) => ctx.query.callbackURL)`.
    OriginCheckQueryCallback,
    /// `originCheck((ctx) => ctx.body.redirectTo)`.
    OriginCheckBodyRedirectTo,
}

/// One endpoint's declaration (`createAuthEndpoint(path, options, handler)`).
pub struct Endpoint {
    pub path: &'static str,
    pub methods: &'static [Method],
    pub body: Option<fn() -> Schema>,
    pub query: Option<fn() -> Schema>,
    /// `metadata.allowedMediaTypes`; empty = the router's `["application/json"]`.
    pub media_types: &'static [&'static str],
    pub middleware: &'static [Middleware],
    pub handler: Handler,
}

/// The call's view of the request and what it has written so far.
pub struct EndpointCtx {
    pub state: AppState,
    /// The endpoint path (`/sign-in/email`): what hook matchers see.
    pub path: &'static str,
    pub method: Method,
    pub headers: HeaderMap,
    /// `ctx.headers` was passed (always over HTTP).
    pub has_headers: bool,
    /// The call came through the HTTP router (`ctx.request` exists).
    pub is_request: bool,
    /// The full request URL (for `/error`).
    pub request_url: String,
    pub peer: Option<SocketAddr>,
    /// `ctx.body` (`None` = undefined).
    pub body: Option<Value>,
    pub query: Map<String, Value>,
    pub params: Map<String, Value>,
    /// Response `Set-Cookie`s in order.
    pub cookies: Vec<SetCookie>,
    /// A `Location` response header (`ctx.setHeader("Location", ..)`, redirects).
    pub location: Option<String>,
    /// `ctx.context.session` (set by the session middlewares).
    pub session: Option<AuthSession>,
    /// `ctx.context.newSession` (`setSessionCookie` sets it).
    pub new_session: Option<AuthSession>,
    request_cookies: IndexMap<String, String>,
}

impl EndpointCtx {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: AppState,
        method: Method,
        headers: HeaderMap,
        has_headers: bool,
        is_request: bool,
        request_url: String,
        peer: Option<SocketAddr>,
        body: Option<Value>,
        query: Map<String, Value>,
        params: Map<String, Value>,
    ) -> EndpointCtx {
        let request_cookies = parse_cookies_better_call(&headers);
        EndpointCtx {
            state,
            path: "",
            method,
            headers,
            has_headers,
            is_request,
            request_url,
            peer,
            body,
            query,
            params,
            cookies: Vec::new(),
            location: None,
            session: None,
            new_session: None,
            request_cookies,
        }
    }

    pub fn db(&self) -> &sqlx::PgPool {
        &self.state.db
    }

    pub fn secret(&self) -> &str {
        &self.state.auth_cookies.secret
    }

    /// `ctx.context.baseURL`: APP_URL + `/api/auth`.
    pub fn base_url(&self) -> String {
        format!("{}/api/auth", self.state.config.app_url)
    }

    /// A body field (`ctx.body.x`), `None` for undefined.
    pub fn body_field(&self, key: &str) -> Option<&Value> {
        self.body.as_ref().and_then(|b| b.get(key))
    }

    pub fn body_str(&self, key: &str) -> Option<&str> {
        self.body_field(key).and_then(Value::as_str)
    }

    pub fn query_str(&self, key: &str) -> Option<&str> {
        self.query.get(key).and_then(Value::as_str)
    }

    /// `ctx.getCookie(name)`.
    pub fn get_cookie(&self, name: &str) -> Option<&str> {
        self.request_cookies.get(name).map(String::as_str).filter(|v| !v.is_empty())
    }

    /// `ctx.getSignedCookie(name, secret)`.
    pub fn get_signed_cookie(&self, name: &str) -> SignedCookie {
        verify_signed_cookie(self.request_cookies.get(name).map(String::as_str), self.secret())
    }

    /// The signed cookie's value, or `None` for absent/invalid.
    pub fn signed_cookie_value(&self, name: &str) -> Option<String> {
        match self.get_signed_cookie(name) {
            SignedCookie::Valid(v) if !v.is_empty() => Some(v),
            _ => None,
        }
    }

    /// `ctx.setCookie(...)` / `ctx.setSignedCookie(...)` (sign it before).
    pub fn set_cookie(&mut self, cookie: SetCookie) {
        self.cookies.push(cookie);
    }

    /// `expireCookie(ctx, cookie)`: drop earlier `Set-Cookie`s of that name
    /// (and its chunks), then send it empty with `Max-Age=0`.
    pub fn expire_cookie(&mut self, cookie: &AuthCookie) {
        let chunk_prefix = format!("{}.", cookie.name);
        self.cookies.retain(|c| c.name != cookie.name && !c.name.starts_with(&chunk_prefix));
        self.cookies.push(cookie.expire());
    }

    /// The client IP Better Auth sees (`getIp`, PORT.md 12.7 applied).
    pub fn ip(&self) -> Option<String> {
        if !self.has_headers {
            return None;
        }
        super::ip::get_ip(&self.headers, self.peer, self.state.config.trust_proxy, self.state.config.is_prod)
    }

    /// `headers.get("user-agent") || ""`.
    pub fn user_agent(&self) -> String {
        self.headers.get("user-agent").and_then(|v| v.to_str().ok()).unwrap_or("").to_string()
    }

    /// `getSessionFromCtx(ctx)`: the caller's session, refreshed in the
    /// database (the cookie is not re-issued, see the module docs). Errors read
    /// as "no session"; only a found session is remembered.
    pub async fn session_from_ctx(&mut self) -> Option<AuthSession> {
        if let Some(s) = &self.session {
            return Some(s.clone());
        }
        let lookup =
            get_session(&self.state.db, &self.state.auth_cookies, &self.headers, RefreshPolicy::DbOnly).await.ok()?;
        let session = lookup.session?;
        self.session = Some(session.clone());
        Some(session)
    }

    /// `ctx.redirect(url)`: a `FOUND` APIError carrying the cookies so far.
    pub fn redirect(&mut self, url: impl Into<String>) -> EndpointError {
        let url = url.into();
        self.location = Some(url.clone());
        EndpointError::Api(ApiError::redirect(url))
    }

    /// The JSON a handler answers with (`ctx.json(v)`).
    pub fn json(&self, v: Value) -> HandlerResult {
        Ok(Out::Json(v))
    }
}

fn unauthorized() -> EndpointError {
    EndpointError::Api(ApiError::with_message("UNAUTHORIZED", "UNAUTHORIZED", "Unauthorized"))
}

async fn run_middleware(ctx: &mut EndpointCtx, mw: Middleware) -> Result<(), EndpointError> {
    match mw {
        Middleware::FormCsrf => super::origin::form_csrf(ctx),
        Middleware::OriginCheckQueryCallback => {
            let v = ctx.query.get("callbackURL").cloned();
            super::origin::origin_check_value(ctx, v.as_ref())
        }
        Middleware::OriginCheckBodyRedirectTo => {
            let v = ctx.body_field("redirectTo").cloned();
            super::origin::origin_check_value(ctx, v.as_ref())
        }
        Middleware::Session | Middleware::SensitiveSession => {
            if ctx.session_from_ctx().await.is_none() {
                return Err(unauthorized());
            }
            Ok(())
        }
        Middleware::FreshSession => {
            let Some(session) = ctx.session_from_ctx().await else {
                return Err(unauthorized());
            };
            let fresh_age_ms = super::cookies::SESSION_FRESH_AGE * 1000;
            if crate::db::types::JsDate::now().millis() - session.session.created_at.millis() >= fresh_age_ms {
                return Err(ApiError::from("FORBIDDEN", super::error::base::SESSION_NOT_FRESH).into());
            }
            Ok(())
        }
        Middleware::Admin => {
            if ctx.session_from_ctx().await.is_none() {
                return Err(ApiError::named("UNAUTHORIZED", None).into());
            }
            Ok(())
        }
    }
}

/// Steps 2-4 of the module docs: validation, middlewares, handler.
async fn run_endpoint(ctx: &mut EndpointCtx, ep: &Endpoint) -> HandlerResult {
    if let Some(schema) = ep.body {
        let (value, issues) = parse(&schema(), ctx.body.as_ref());
        if !issues.is_empty() {
            return Err(validation_error(&format_issues(&issues, "body")));
        }
        ctx.body = value;
    }
    if let Some(schema) = ep.query {
        let input = if ctx.query.is_empty() && !ctx.is_request { None } else { Some(Value::Object(ctx.query.clone())) };
        let (value, issues) = parse(&schema(), input.as_ref());
        if !issues.is_empty() {
            return Err(validation_error(&format_issues(&issues, "query")));
        }
        ctx.query = match value {
            Some(Value::Object(m)) => m,
            _ => Map::new(),
        };
    }
    for mw in ep.middleware {
        run_middleware(ctx, *mw).await?;
    }
    (ep.handler)(ctx).await
}

/// `new APIError(400, { message, code: "VALIDATION_ERROR" })`.
pub fn validation_error(message: &str) -> EndpointError {
    EndpointError::Api(ApiError::numeric(400, json!({ "message": message, "code": "VALIDATION_ERROR" })))
}

/// The whole pipeline for one endpoint (module docs, steps 1-5).
pub async fn call(ctx: &mut EndpointCtx, ep: &Endpoint) -> HandlerResult {
    ctx.path = ep.path;
    // 1. before hooks (lib/auth.ts `hooks.before`).
    if ep.path == "/sign-up/email" {
        super::routes::sign_up::registration_gate(ctx).await?;
    }
    // 2-4.
    let result = run_endpoint(ctx, ep).await;
    // An unexpected error skips the after hooks (it propagates upstream).
    let result = match result {
        Err(EndpointError::Internal(e)) => return Err(EndpointError::Internal(e)),
        other => other,
    };
    // 5. after hooks: admin, then twoFactor (plugin order).
    let result = if ep.path == "/list-sessions" { super::admin::list_sessions_after_hook(result) } else { result };
    if ep.path == "/sign-in/email" {
        return super::two_factor::sign_in_after_hook(ctx, result).await;
    }
    result
}
