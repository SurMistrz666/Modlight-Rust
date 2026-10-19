//! The core endpoints of better-auth 1.6.13 (`api/routes/*.mjs`), one module
//! per upstream file, plus the endpoint table the router and the server-side
//! calls dispatch on. The twoFactor and admin plugins' endpoints live in
//! [`super::two_factor`] and [`super::admin`].

pub mod account;
pub mod email_verification;
pub mod misc;
pub mod password;
pub mod session;
pub mod sign_in;
pub mod sign_out;
pub mod sign_up;
pub mod update_session;
pub mod update_user;

use std::sync::OnceLock;

use axum::http::Method;
use serde_json::{Map, Value};

use super::cookies::AuthCookie;
use super::endpoint::{Endpoint, EndpointCtx};
use super::session::{AuthSession, session_token_cookie};
use crate::http::cookies::parse_cookies_better_auth;

/// Every endpoint reachable over HTTP, in better-auth's `getEndpoints` order
/// (core, then the plugins' in plugin order: admin, twoFactor).
pub fn endpoints() -> &'static [Endpoint] {
    static ALL: OnceLock<Vec<Endpoint>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut all = Vec::new();
        all.extend(account::social_endpoints());
        all.extend(session::endpoints());
        all.extend(sign_out::endpoints());
        all.extend(sign_up::endpoints());
        all.extend(sign_in::endpoints());
        all.extend(password::endpoints());
        all.extend(email_verification::endpoints());
        all.extend(update_user::endpoints());
        all.extend(update_session::endpoints());
        all.extend(account::endpoints());
        all.extend(super::admin::endpoints());
        all.extend(super::two_factor::endpoints());
        all.extend(misc::endpoints());
        all
    })
}

/// The endpoint registered at exactly this path (server-side calls).
pub fn by_path(path: &str) -> &'static Endpoint {
    endpoints().iter().find(|e| e.path == path).unwrap_or_else(|| panic!("no Better Auth endpoint {path}"))
}

/// rou3's `findRoute(method, path)`: a static path first, then one with
/// `:params` (each matching one non-empty segment); the method must be one
/// the endpoint declares.
pub fn find_route(method: &Method, path: &str) -> Option<(&'static Endpoint, Map<String, Value>)> {
    let all = endpoints();
    if let Some(ep) = all.iter().find(|e| e.path == path && e.methods.contains(method)) {
        return Some((ep, Map::new()));
    }
    let segments: Vec<&str> = path.split('/').collect();
    for ep in all.iter().filter(|e| e.path.contains(':') && e.methods.contains(method)) {
        let pattern: Vec<&str> = ep.path.split('/').collect();
        if pattern.len() != segments.len() {
            continue;
        }
        let mut params = Map::new();
        let matched = pattern.iter().zip(&segments).all(|(p, s)| match p.strip_prefix(':') {
            Some(name) => {
                params.insert(name.to_string(), Value::String((*s).to_string()));
                !s.is_empty()
            }
            None => p == s,
        });
        if matched {
            return Some((ep, params));
        }
    }
    None
}

/// `setSessionCookie(ctx, session, dontRememberMe, overrides)`.
pub fn set_session_cookie(
    ctx: &mut EndpointCtx,
    session: AuthSession,
    dont_remember: Option<bool>,
    max_age: Option<i64>,
) {
    let cookies = ctx.state.auth_cookies.clone();
    let dont_remember_cookie = ctx.signed_cookie_value(&cookies.dont_remember.name).is_some();
    let dont_remember = dont_remember.unwrap_or(dont_remember_cookie);
    let age = if dont_remember { None } else { Some(max_age.unwrap_or(super::cookies::SESSION_EXPIRES_IN)) };
    ctx.set_cookie(session_token_cookie(&cookies, &session.session.token, age));
    if dont_remember {
        ctx.set_cookie(cookies.dont_remember.set(super::cookies::signed_value("true", &cookies.secret)));
    }
    ctx.new_session = Some(session);
}

/// `deleteSessionCookie(ctx, skipDontRememberMe)`.
pub fn delete_session_cookie(ctx: &mut EndpointCtx, skip_dont_remember: bool) {
    let cookies = ctx.state.auth_cookies.clone();
    ctx.expire_cookie(&cookies.session_token);
    ctx.expire_cookie(&cookies.session_data);
    // `createSessionStore(...).clean()`: every request cookie that is a chunk
    // of session_data (`<name>.<n>`), set empty with Max-Age=0.
    let prefix = format!("{}.", cookies.session_data.name);
    let chunks: Vec<String> =
        parse_cookies_better_auth(&ctx.headers).keys().filter(|n| n.starts_with(&prefix)).cloned().collect();
    for name in chunks {
        let mut c = cookies.session_data.expire();
        c.name = name;
        ctx.set_cookie(c);
    }
    if !skip_dont_remember {
        ctx.expire_cookie(&cookies.dont_remember);
    }
}

/// `ctx.context.createAuthCookie(name, { maxAge })`.
pub fn auth_cookie(ctx: &EndpointCtx, name: &str, max_age: Option<i64>) -> AuthCookie {
    ctx.state.auth_cookies.create(name, max_age)
}
