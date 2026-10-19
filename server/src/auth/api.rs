//! `/api/auth/*`: port of `app/api/auth/[...all]/route.ts`
//! (`export const { GET, POST } = toNextJsHandler(auth)`), i.e. better-call
//! 1.3.5's router (`router.mjs`) with better-auth 1.6.13's router options
//! (`api/index.mjs`).
//!
//! A request, in upstream order:
//!
//! 1. `onRequest`: the rate limiter ([`super::rate_limit`]) on
//!    `<ip>|<path below /api/auth>`; a refusal is a 429 and is not counted.
//! 2. The path below the base path; empty or containing `//`: 404.
//! 3. Route lookup by method and path (static, then `:params`); a path
//!    whose trailing slash differs, or no route: 404 with no body. Only GET
//!    and POST reach here (Next answers other methods 405, OPTIONS 204), and
//!    a HEAD finds no route: 404.
//! 4. The body (POST): the content type must be one the endpoint allows
//!    (`application/json`, plus form-urlencoded for sign-in/sign-up), else
//!    `415 415`; JSON that does not parse is a 500 with no body.
//! 5. The router middleware ([`super::origin::origin_check_middleware`]).
//! 6. The endpoint ([`super::endpoint::call`]).
//! 7. `onResponse`: the request is counted against its rate-limit bucket.
//!
//! Responses: an `APIError` is its status with the status NAME as reason
//! phrase (`400 BAD_REQUEST`, `400 400` for validation errors), a JSON body
//! (or none) and `content-type: application/json`; anything unexpected is a
//! bare `500 Internal Server Error`. Cookies use better-call's format
//! (`Max-Age`, `Path`, `HttpOnly`, `Secure`, `SameSite=Lax`).

use std::net::SocketAddr;

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use bytes::Bytes;
use futures_util::StreamExt;
use serde_json::{Map, Value, json};

use super::endpoint::{EndpointCtx, Out, call};
use super::error::{ApiError, EndpointError};
use super::routes::find_route;
use crate::http::set_cookie::SetCookie;
use crate::state::AppState;

pub const BASE_PATH: &str = "/api/auth";

/// The largest body read (upstream's handler buffers whatever arrives; this is
/// a safety ceiling far above any real request).
const BODY_MAX: usize = 32 * 1024 * 1024;

/// Handlers for `/api/auth/{*all}`.
pub fn routes() -> Router<AppState> {
    Router::new().route("/api/auth/{*all}", get(handle).post(handle))
}

/// `normalizePathname(url, basePath)`: the rate-limit path.
pub fn normalize_pathname(pathname: &str) -> String {
    let trimmed = pathname.trim_end_matches('/');
    let pathname = if trimmed.is_empty() { "/" } else { trimmed };
    if pathname == BASE_PATH {
        return "/".to_string();
    }
    if let Some(rest) = pathname.strip_prefix(&format!("{BASE_PATH}/")) {
        let rest = format!("/{rest}");
        let rest = rest.trim_end_matches('/');
        return if rest.is_empty() { "/".to_string() } else { rest.to_string() };
    }
    pathname.to_string()
}

/// better-call's `pathname.split(basePath).reduce(...)`: the base path's
/// first occurrence dropped, any later one kept.
pub fn route_path(pathname: &str) -> String {
    let parts: Vec<&str> = pathname.split(BASE_PATH).collect();
    let mut out = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            continue;
        }
        if i > 1 {
            out.push_str(BASE_PATH);
        }
        out.push_str(part);
    }
    out
}

/// `url.searchParams` folded into an object (repeated keys become arrays).
pub fn query_object(query: Option<&str>) -> Map<String, Value> {
    let mut out = Map::new();
    for (k, v) in form_urlencoded::parse(query.unwrap_or("").as_bytes()).into_owned() {
        match out.get_mut(&k) {
            None => {
                out.insert(k, Value::String(v));
            }
            Some(Value::Array(items)) => items.push(Value::String(v)),
            Some(existing) => {
                let first = existing.take();
                *existing = Value::Array(vec![first, Value::String(v)]);
            }
        }
    }
    out
}

fn empty(status: StatusCode, reason: &'static str) -> Response {
    let mut res = status.into_response();
    res.extensions_mut().insert(hyper::ext::ReasonPhrase::from_static(reason.as_bytes()));
    res
}

fn not_found() -> Response {
    empty(StatusCode::NOT_FOUND, "Not Found")
}

fn server_error() -> Response {
    empty(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
}

fn set_reason(res: &mut Response, text: &str) {
    if let Ok(phrase) = hyper::ext::ReasonPhrase::try_from(text.as_bytes().to_vec()) {
        res.extensions_mut().insert(phrase);
    }
}

fn write_cookies(headers: &mut HeaderMap, cookies: &[SetCookie]) {
    for c in cookies {
        if let Ok(v) = HeaderValue::from_str(&c.to_better_call_string()) {
            headers.append(header::SET_COOKIE, v);
        }
    }
}

/// `toResponse(...)` for an endpoint's outcome.
pub fn to_response(result: Result<Out, EndpointError>, cookies: &[SetCookie], location: Option<&str>) -> Response {
    let mut res = match result {
        Ok(Out::Json(v)) => {
            let body = serde_json::to_string(&v).unwrap_or_else(|_| "null".into());
            let mut res = (StatusCode::OK, body).into_response();
            res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
            res
        }
        Ok(Out::Redirect { status, location: to }) => {
            let mut res = StatusCode::from_u16(status).unwrap_or(StatusCode::FOUND).into_response();
            if let Ok(v) = HeaderValue::from_str(&to) {
                res.headers_mut().insert(header::LOCATION, v);
            }
            res
        }
        Ok(Out::Html(html)) => {
            let mut res = (StatusCode::OK, html).into_response();
            res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html"));
            res
        }
        Err(EndpointError::Api(e)) => api_error_response(&e),
        Err(EndpointError::Internal(err)) => {
            tracing::error!(error = ?err, "# SERVER_ERROR");
            return server_error();
        }
    };
    write_cookies(res.headers_mut(), cookies);
    if let Some(loc) = location
        && let Ok(v) = HeaderValue::from_str(loc)
    {
        res.headers_mut().insert(header::LOCATION, v);
    }
    res
}

/// An `APIError` as better-call answers it.
pub fn api_error_response(e: &ApiError) -> Response {
    let body = match &e.body {
        Some(v) => serde_json::to_string(v).unwrap_or_default(),
        None => String::new(),
    };
    let mut res = (e.status, body).into_response();
    res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    set_reason(&mut res, &e.status_text);
    if let Some(loc) = &e.location
        && let Ok(v) = HeaderValue::from_str(loc)
    {
        res.headers_mut().insert(header::LOCATION, v);
    }
    res
}

async fn read_body(body: Body) -> Result<Bytes, Response> {
    let mut buf = Vec::new();
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| server_error())?;
        buf.extend_from_slice(&chunk);
        if buf.len() > BODY_MAX {
            return Err(empty(StatusCode::PAYLOAD_TOO_LARGE, "Payload Too Large"));
        }
    }
    Ok(Bytes::from(buf))
}

/// `/^application\/([a-z0-9.+-]*\+)?json/i` (no end anchor).
fn is_json_content_type(normalized: &str) -> bool {
    let Some(rest) = normalized.strip_prefix("application/") else {
        return false;
    };
    if rest.starts_with("json") {
        return true;
    }
    // ([a-z0-9.+-]*\+)json: some "+json" preceded only by those characters.
    rest.match_indices("+json")
        .any(|(i, _)| rest[..i].chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-')))
}

/// better-call `getBody(request, allowedMediaTypes)` for a request that has a
/// body (every POST through Next). `Err` is the response to send.
fn get_body(headers: &HeaderMap, bytes: &[u8], allowed: &[&str]) -> Result<Option<Value>, Response> {
    let content_type = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let normalized = content_type.to_lowercase();
    let base = crate::config::js_trim(normalized.split(';').next().unwrap_or("")).to_string();
    if !allowed.iter().any(|a| {
        let a = a.to_lowercase();
        base == a || base.contains(&a)
    }) {
        let message = if normalized.is_empty() {
            format!("Content-Type is required. Allowed types: {}", allowed.join(", "))
        } else {
            format!("Content-Type \"{content_type}\" is not allowed. Allowed types: {}", allowed.join(", "))
        };
        let e = ApiError::numeric(415, json!({ "message": message, "code": "UNSUPPORTED_MEDIA_TYPE" }));
        return Err(api_error_response(&e));
    }
    let text = || {
        let s = String::from_utf8_lossy(bytes).into_owned();
        s.strip_prefix('\u{feff}').map(str::to_string).unwrap_or(s)
    };
    if is_json_content_type(&normalized) {
        return match serde_json::from_str::<Value>(&text()) {
            Ok(v) => Ok(Some(v)),
            Err(_) => Err(server_error()),
        };
    }
    if normalized.contains("application/x-www-form-urlencoded") {
        let mut out = Map::new();
        for (k, v) in form_urlencoded::parse(bytes).into_owned() {
            out.insert(k, Value::String(v));
        }
        return Ok(Some(Value::Object(out)));
    }
    // Any other allowed type is read as text.
    Ok(Some(Value::String(text())))
}

async fn handle(State(state): State<AppState>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let method = parts.method.clone();
    let headers = parts.headers.clone();
    let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
    let pathname = parts.uri.path().to_string();
    let query = parts.uri.query().map(str::to_string);

    // 1. onRequest: rate limit.
    let ip = super::ip::get_ip(&headers, peer, state.config.trust_proxy, state.config.is_prod);
    let rl_path = normalize_pathname(&pathname);
    if let Some(retry_after) = super::rate_limit::check(ip.as_deref(), &rl_path) {
        return super::rate_limit::too_many_requests(retry_after);
    }
    let res = process(&state, method, headers, peer, &pathname, query, body).await;
    // 7. onResponse
    super::rate_limit::record(ip.as_deref(), &rl_path);
    res
}

async fn process(
    state: &AppState,
    method: Method,
    headers: HeaderMap,
    peer: Option<SocketAddr>,
    pathname: &str,
    query: Option<String>,
    body: Body,
) -> Response {
    // 2.
    let path = route_path(pathname);
    if path.is_empty() || path.contains("//") {
        return not_found();
    }
    // 3.
    let Some((ep, params)) = find_route(&method, &path) else {
        return not_found();
    };
    if path.ends_with('/') != ep.path.ends_with('/') {
        return not_found();
    }
    // 4.
    let body_value = if method == Method::POST {
        let bytes = match read_body(body).await {
            Ok(b) => b,
            Err(res) => return res,
        };
        let allowed: &[&str] = if ep.media_types.is_empty() { &["application/json"] } else { ep.media_types };
        match get_body(&headers, &bytes, allowed) {
            Ok(v) => v,
            Err(res) => return res,
        }
    } else {
        None
    };
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("localhost").to_string();
    let request_url = match &query {
        Some(q) => format!("http://{host}{pathname}?{q}"),
        None => format!("http://{host}{pathname}"),
    };
    let mut ctx = EndpointCtx::new(
        state.clone(),
        method,
        headers,
        true,
        true,
        request_url,
        peer,
        body_value,
        query_object(query.as_deref()),
        params,
    );
    // 5. the router middleware
    if let Err(e) = super::origin::origin_check_middleware(&ctx) {
        return to_response(Err(e), &[], None);
    }
    // 6.
    let result = call(&mut ctx, ep).await;
    to_response(result, &ctx.cookies, ctx.location.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_like_better_call() {
        assert_eq!(route_path("/api/auth/sign-in/email"), "/sign-in/email");
        assert_eq!(route_path("/api/auth/x/api/auth/y"), "/x/api/auth/y");
        assert_eq!(route_path("/api/auth/api/auth/ok"), "/api/auth/ok");
        assert_eq!(normalize_pathname("/api/auth/sign-in/email/"), "/sign-in/email");
        assert_eq!(normalize_pathname("/api/auth"), "/");
        assert_eq!(normalize_pathname("/api/auth/"), "/");
    }

    #[test]
    fn json_content_types() {
        assert!(is_json_content_type("application/json"));
        assert!(is_json_content_type("application/json; charset=utf-8"));
        assert!(is_json_content_type("application/jsonx"));
        assert!(is_json_content_type("application/vnd.api+json"));
        assert!(!is_json_content_type("text/json"));
    }

    #[test]
    fn query_arrays() {
        let q = query_object(Some("a=1&a=2&b=x+y"));
        assert_eq!(Value::Object(q), json!({"a": ["1", "2"], "b": "x y"}));
    }
}
