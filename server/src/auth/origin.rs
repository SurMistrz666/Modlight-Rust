//! Better Auth's CSRF and redirect-target checks
//! (`api/middlewares/origin-check.mjs`, `auth/trusted-origins.mjs`).
//!
//! The trusted origins are APP_URL's origin alone (lib/auth.ts sets no
//! `trustedOrigins`). Three checks, all skipped for server-side calls:
//!
//! - the router middleware on every non-GET request: when the request carries
//!   a `Cookie` header, its `Origin` (else `Referer`) must be a trusted origin
//!   (403 MISSING_OR_NULL_ORIGIN / INVALID_ORIGIN); then `callbackURL` (body,
//!   else query), `redirectTo`, `errorCallbackURL` and `newUserCallbackURL`
//!   must be trusted or a safe relative path (403 INVALID_CALLBACK_URL ...);
//! - `formCsrfMiddleware` (sign-in, sign-up): with cookies, the same origin
//!   check; without cookies, a request that has fetch metadata must not be a
//!   cross-site navigation and must then pass the origin check;
//! - `originCheck(getter)` on single endpoints (`/verify-email`,
//!   `/reset-password/:token`, `/request-password-reset`).
//!
//! A truthy value that is not a string (a number, an object) crashes
//! upstream's `url.startsWith` with a TypeError: an internal error (500 with
//! no body), reproduced as such.

use axum::http::{HeaderMap, Method};
use serde_json::Value;

use super::endpoint::EndpointCtx;
use super::error::{ApiError, EndpointError, base};
use super::validate::js_truthy;

/// `getOrigin(url)`: the serialised origin, `None` for opaque origins and
/// unparseable input.
pub fn get_origin(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    match parsed.origin() {
        url::Origin::Tuple(..) => Some(parsed.origin().ascii_serialization()),
        url::Origin::Opaque(_) => None,
    }
}

/// `getProtocol(url)`, e.g. `"https:"`.
fn get_protocol(url: &str) -> Option<String> {
    url::Url::parse(url).ok().map(|u| format!("{}:", u.scheme()))
}

/// `^\/(?!\/|\\|%2f|%5c)[\w\-.\+/@]*(?:\?[\w\-.\+/=&%@]*)?$`
fn is_safe_relative_path(url: &str) -> bool {
    let Some(rest) = url.strip_prefix('/') else {
        return false;
    };
    if rest.starts_with('/') || rest.starts_with('\\') || rest.starts_with("%2f") || rest.starts_with("%5c") {
        return false;
    }
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let (path, query) = match rest.find('?') {
        Some(i) => (&rest[..i], Some(&rest[i + 1..])),
        None => (rest, None),
    };
    let path_ok = path.chars().all(|c| word(c) || matches!(c, '-' | '.' | '+' | '/' | '@'));
    let query_ok =
        query.is_none_or(|q| q.chars().all(|c| word(c) || matches!(c, '-' | '.' | '+' | '/' | '=' | '&' | '%' | '@')));
    path_ok && query_ok
}

/// `matchesOriginPattern(url, pattern, { allowRelativePaths })` for a
/// pattern without wildcards (APP_URL's origin never has any).
pub fn matches_origin_pattern(url: &str, pattern: &str, allow_relative: bool) -> bool {
    if url.starts_with('/') {
        return allow_relative && is_safe_relative_path(url);
    }
    match get_protocol(url).as_deref() {
        Some("http:") | Some("https:") | None => get_origin(url).as_deref() == Some(pattern),
        Some(_) => url.starts_with(pattern),
    }
}

/// The trusted origins (`ctx.context.trustedOrigins`).
pub fn trusted_origins(ctx: &EndpointCtx) -> Vec<String> {
    get_origin(&ctx.state.config.app_url).into_iter().collect()
}

/// `ctx.context.isTrustedOrigin(url, { allowRelativePaths })`.
pub fn is_trusted_origin(ctx: &EndpointCtx, url: &str, allow_relative: bool) -> bool {
    trusted_origins(ctx).iter().any(|o| matches_origin_pattern(url, o, allow_relative))
}

/// `validateOrigin(ctx, forceValidate)`.
fn validate_origin(ctx: &EndpointCtx, force: bool) -> Result<(), EndpointError> {
    if !ctx.is_request {
        return Ok(());
    }
    let headers: &HeaderMap = &ctx.headers;
    let origin_header = header_text(headers, "origin")
        .filter(|v| !v.is_empty())
        .or_else(|| header_text(headers, "referer").filter(|v| !v.is_empty()))
        .unwrap_or_default();
    let use_cookies = headers.contains_key("cookie");
    if !(force || use_cookies) {
        return Ok(());
    }
    if origin_header.is_empty() || origin_header == "null" {
        return Err(ApiError::from("FORBIDDEN", base::MISSING_OR_NULL_ORIGIN).into());
    }
    if !trusted_origins(ctx).iter().any(|o| matches_origin_pattern(&origin_header, o, false)) {
        tracing::error!("Invalid origin: {origin_header}");
        return Err(ApiError::from("FORBIDDEN", base::INVALID_ORIGIN).into());
    }
    Ok(())
}

fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_string)
}

/// `validateURL(url, label)` inside both origin middlewares.
fn validate_url(ctx: &EndpointCtx, url: Option<&Value>, label: &str) -> Result<(), EndpointError> {
    if !js_truthy(url) {
        return Ok(());
    }
    let Some(Value::String(url)) = url else {
        // `url.startsWith` on a non-string: TypeError.
        return Err(EndpointError::Internal(anyhow::anyhow!("{label} is not a string")));
    };
    if is_trusted_origin(ctx, url, label != "origin") {
        return Ok(());
    }
    tracing::error!("Invalid {label}: {url}");
    let code = match label {
        "callbackURL" => base::INVALID_CALLBACK_URL,
        "redirectURL" => base::INVALID_REDIRECT_URL,
        "errorCallbackURL" => base::INVALID_ERROR_CALLBACK_URL,
        "newUserCallbackURL" => base::INVALID_NEW_USER_CALLBACK_URL,
        _ => base::INVALID_ORIGIN,
    };
    Err(ApiError::from("FORBIDDEN", code).into())
}

/// `originCheckMiddleware`: the router middleware (every path, before the
/// endpoint's own validation; `body` and `query` are the raw ones).
pub fn origin_check_middleware(ctx: &EndpointCtx) -> Result<(), EndpointError> {
    if ctx.method == Method::GET || ctx.method == Method::OPTIONS || ctx.method == Method::HEAD || !ctx.is_request {
        return Ok(());
    }
    validate_origin(ctx, false)?;
    let body_field = |k: &str| ctx.body.as_ref().and_then(|b| b.as_object()).and_then(|m| m.get(k)).cloned();
    // `body?.callbackURL || query?.callbackURL`
    let callback =
        body_field("callbackURL").filter(|v| js_truthy(Some(v))).or_else(|| ctx.query.get("callbackURL").cloned());
    validate_url(ctx, callback.as_ref(), "callbackURL")?;
    validate_url(ctx, body_field("redirectTo").as_ref(), "redirectURL")?;
    validate_url(ctx, body_field("errorCallbackURL").as_ref(), "errorCallbackURL")?;
    validate_url(ctx, body_field("newUserCallbackURL").as_ref(), "newUserCallbackURL")?;
    Ok(())
}

/// `originCheck((ctx) => value)`: one or several callback URLs.
pub fn origin_check_value(ctx: &EndpointCtx, value: Option<&Value>) -> Result<(), EndpointError> {
    if !ctx.is_request {
        return Ok(());
    }
    match value {
        Some(Value::Array(items)) => {
            for item in items {
                validate_url(ctx, Some(item), "callbackURL")?;
            }
            Ok(())
        }
        other => validate_url(ctx, other, "callbackURL"),
    }
}

/// `formCsrfMiddleware` / `validateFormCsrf`.
pub fn form_csrf(ctx: &EndpointCtx) -> Result<(), EndpointError> {
    if !ctx.is_request {
        return Ok(());
    }
    if ctx.headers.contains_key("cookie") {
        return validate_origin(ctx, false);
    }
    let site = header_text(&ctx.headers, "sec-fetch-site");
    let mode = header_text(&ctx.headers, "sec-fetch-mode");
    let dest = header_text(&ctx.headers, "sec-fetch-dest");
    let present = |v: &Option<String>| v.as_deref().is_some_and(|s| !crate::config::js_trim(s).is_empty());
    if present(&site) || present(&mode) || present(&dest) {
        if site.as_deref() == Some("cross-site") && mode.as_deref() == Some("navigate") {
            tracing::error!("Blocked cross-site navigation login attempt (CSRF protection)");
            return Err(ApiError::from("FORBIDDEN", base::CROSS_SITE_NAVIGATION_LOGIN_BLOCKED).into());
        }
        return validate_origin(ctx, true);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_and_origins() {
        let o = "http://localhost:8424";
        assert!(matches_origin_pattern("/dashboard", o, true));
        assert!(matches_origin_pattern("/reset?token=a%20b", o, true));
        assert!(!matches_origin_pattern("/dashboard", o, false));
        assert!(!matches_origin_pattern("//evil.com", o, true));
        assert!(!matches_origin_pattern("/\\evil.com", o, true));
        assert!(!matches_origin_pattern("/%2fevil.com", o, true));
        // `%` is not in the path class: rejected whatever its case.
        assert!(!matches_origin_pattern("/%2Fevil.com", o, true));
        assert!(!matches_origin_pattern("/a b", o, true));
        assert!(matches_origin_pattern("http://localhost:8424/x?y=1", o, true));
        assert!(matches_origin_pattern("HTTP://LOCALHOST:8424", o, false));
        assert!(!matches_origin_pattern("http://localhost:8425", o, false));
        assert!(!matches_origin_pattern("not a url", o, true));
        assert!(!matches_origin_pattern("javascript:alert(1)", o, true));
    }
}
