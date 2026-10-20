//! Server actions as RPC (PORT.md section 7).
//!
//! ```text
//! POST /_rpc/<module>/<exportName>
//! Content-Type: application/json          (or multipart/form-data)
//! X-Modlight-Rpc: 1                       (or "form" for a FormData action)
//! Origin: <must match the request's own host, else 403>
//!
//! <tagged JSON array of the arguments>
//! ```
//!
//! Always HTTP 200 with an envelope once the request reached an action:
//! `{"ok": v}`, `{"redirect": url}`, `{"notFound": true}` or
//! `{"error": {"message": ...}}`, plus `"revalidate": true` when the action
//! called `revalidatePath`/`revalidateTag` or the response sets or deletes any
//! cookie (Next 16 refreshes the page for both). Cookies set by the action
//! (and by a session refresh) travel as `Set-Cookie`.
//!
//! Thrown errors: Next replaces the message of an error thrown by a server
//! action with a generic one in production (React Flight's `digest` error),
//! so that is what this does when NODE_ENV=production; in development the real
//! message is sent, as `next dev` does.
//!
//! Registration: each `src/actions/<module>.rs` exposes
//! `pub fn register(reg: &mut RpcRegistry)`; `src/actions/mod.rs` lists them.

pub mod tagged;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::auth::session::{AuthSession, RefreshPolicy};
use crate::auth_helpers::{AuthRefusal, RequestSession};
use crate::http::cookies::parse_cookies_next;
use crate::http::set_cookie::{ResponseCookies, SetCookie};
use crate::i18n::Locale;
use crate::state::AppState;

/// Next's default `serverActions.bodySizeLimit` (upstream never raised it).
pub const RPC_BODY_LIMIT: usize = 1024 * 1024;

/// The message React Flight substitutes for a thrown error in production.
pub const PRODUCTION_ERROR_MESSAGE: &str = "An error occurred in the Server Components render. The specific message is omitted in production builds to avoid leaking sensitive details. A digest property is included on this error instance which may provide additional details about the nature of the error.";

/// How an action ended other than by returning.
#[derive(Debug)]
pub enum ActionError {
    /// `redirect(url)`
    Redirect(String),
    /// `notFound()`
    NotFound,
    /// `throw new Error(message)`
    Thrown(String),
    /// Anything unexpected (also a throw upstream; logged).
    Internal(anyhow::Error),
}

impl ActionError {
    pub fn thrown(message: impl Into<String>) -> Self {
        ActionError::Thrown(message.into())
    }
}

impl From<anyhow::Error> for ActionError {
    fn from(e: anyhow::Error) -> Self {
        ActionError::Internal(e)
    }
}

impl From<sqlx::Error> for ActionError {
    fn from(e: sqlx::Error) -> Self {
        ActionError::Internal(e.into())
    }
}

impl From<serde_json::Error> for ActionError {
    fn from(e: serde_json::Error) -> Self {
        ActionError::Internal(e.into())
    }
}

impl From<AuthRefusal> for ActionError {
    fn from(r: AuthRefusal) -> Self {
        match r {
            AuthRefusal::Redirect(to) => ActionError::Redirect(to.to_string()),
            AuthRefusal::Failed(e) => ActionError::Internal(e.into()),
        }
    }
}

pub type ActionResult = Result<Value, ActionError>;

/// `return value`: tagged-encode any serializable value.
pub fn ok<T: Serialize + ?Sized>(value: &T) -> ActionResult {
    Ok(tagged::to_tagged_value(value)?)
}

/// `return;` (resolves to `undefined`).
pub fn void() -> ActionResult {
    Ok(tagged::undefined())
}

/// The positional arguments, as sent (tagged).
#[derive(Debug, Clone, Default)]
pub struct ActionArgs(pub Vec<Value>);

impl ActionArgs {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The raw tagged value of argument `i`.
    pub fn raw(&self, i: usize) -> Option<&Value> {
        self.0.get(i)
    }

    /// Missing or `undefined`.
    pub fn is_undefined(&self, i: usize) -> bool {
        self.0.get(i).is_none_or(tagged::is_undefined)
    }

    /// Argument `i` decoded with serde from its normalized form (a missing or
    /// `undefined` argument reads as `null`, so `Option<T>` is `None`).
    /// A value of the wrong shape is a thrown error, which is what upstream
    /// code would eventually have hit on it.
    pub fn get<T: DeserializeOwned>(&self, i: usize) -> Result<T, ActionError> {
        let v = self.0.get(i).cloned().map(tagged::normalize).unwrap_or(Value::Null);
        serde_json::from_value(v).map_err(|e| ActionError::Thrown(format!("Invalid argument {i}: {e}")))
    }

    /// A number exactly as sent (NaN and infinities included).
    pub fn num(&self, i: usize) -> Option<f64> {
        self.0.get(i).and_then(tagged::raw_num)
    }
}

/// A `FormData` argument.
#[derive(Debug, Clone, Default)]
pub struct FormData {
    pub fields: Vec<(String, FormValue)>,
}

#[derive(Debug, Clone)]
pub enum FormValue {
    Text(String),
    File { filename: String, content_type: String, data: Bytes },
}

impl FormData {
    /// `formData.get(name)`: the first value.
    pub fn get(&self, name: &str) -> Option<&FormValue> {
        self.fields.iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }

    pub fn get_text(&self, name: &str) -> Option<&str> {
        match self.get(name) {
            Some(FormValue::Text(t)) => Some(t),
            _ => None,
        }
    }

    pub fn get_all(&self, name: &str) -> Vec<&FormValue> {
        self.fields.iter().filter(|(k, _)| k == name).map(|(_, v)| v).collect()
    }
}

/// Everything an action can see and do.
pub struct ActionCtx {
    pub state: AppState,
    pub headers: Arc<HeaderMap>,
    pub peer: Option<SocketAddr>,
    pub args: ActionArgs,
    /// Set for `X-Modlight-Rpc: form` calls (the single FormData argument).
    pub form: Option<FormData>,
    /// `getSession()` with the server-action policy (refresh + cookie).
    pub session: RequestSession,
    cookies: ResponseCookies,
    revalidate: Arc<AtomicBool>,
}

impl ActionCtx {
    /// `(await cookies()).set(...)`
    pub fn set_cookie(&self, cookie: SetCookie) {
        self.cookies.push(cookie);
    }

    /// `(await cookies()).delete(name)`: Next writes an empty value that
    /// expired at the epoch, `name=; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT`
    /// (no `Max-Age`; verified on the reference).
    pub fn delete_cookie(&self, name: &str) {
        let mut cookie = SetCookie::new(name, "").path("/");
        cookie.expires = Some(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH);
        self.cookies.push(cookie);
    }

    /// `(await cookies()).get(name)?.value` for every request cookie.
    pub fn request_cookies(&self) -> IndexMap<String, String> {
        parse_cookies_next(&self.headers)
    }

    /// `revalidatePath(path)`: the SPA refetches the current page's data.
    pub fn revalidate_path(&self, _path: &str) {
        self.revalidate.store(true, Ordering::Relaxed);
    }

    /// `revalidateTag(tag)`
    pub fn revalidate_tag(&self, _tag: &str) {
        self.revalidate.store(true, Ordering::Relaxed);
    }

    /// `clientIp(await headers())`
    pub fn client_ip(&self) -> String {
        crate::password::client_ip(&self.headers, self.peer, self.state.config.trust_proxy)
    }

    /// The request's locale (i18n/request.ts).
    pub fn locale(&self) -> Locale {
        crate::i18n::request::request_locale(&self.headers)
    }

    pub async fn get_session(&self) -> Result<Option<AuthSession>, ActionError> {
        self.session.get().await.map_err(|e| ActionError::Internal(e.into()))
    }

    pub async fn require_user(&self) -> Result<AuthSession, ActionError> {
        Ok(self.session.require_user().await?)
    }

    pub async fn require_admin(&self) -> Result<AuthSession, ActionError> {
        Ok(self.session.require_admin().await?)
    }
}

/// One server action.
pub trait Action: Send + Sync + 'static {
    fn call(&self, ctx: ActionCtx) -> BoxFuture<'static, ActionResult>;
}

impl<F, Fut> Action for F
where
    F: Fn(ActionCtx) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ActionResult> + Send + 'static,
{
    fn call(&self, ctx: ActionCtx) -> BoxFuture<'static, ActionResult> {
        Box::pin(self(ctx))
    }
}

/// Every action, by `<module>/<exportName>`.
#[derive(Default)]
pub struct RpcRegistry {
    actions: HashMap<String, Arc<dyn Action>>,
}

impl RpcRegistry {
    /// Register `module.exportName`. Registering the same name twice is a bug
    /// and panics at startup.
    pub fn register(&mut self, module: &str, export: &str, action: impl Action) {
        let key = format!("{module}/{export}");
        if self.actions.insert(key.clone(), Arc::new(action)).is_some() {
            panic!("server action {key} registered twice");
        }
    }

    pub fn get(&self, module: &str, export: &str) -> Option<Arc<dyn Action>> {
        self.actions.get(&format!("{module}/{export}")).cloned()
    }

    /// Every registered `module/export`, sorted.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.actions.keys().cloned().collect();
        names.sort();
        names
    }
}

/// `POST /_rpc/{module}/{export}`
pub fn routes() -> Router<AppState> {
    Router::new().route("/_rpc/{module}/{export}", post(handle))
}

fn envelope_response(status: StatusCode, body: Value) -> Response {
    let mut res = crate::api_json::json_response(status, &body, None);
    res.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-cache, no-store, max-age=0, must-revalidate"),
    );
    // A server action's answer is an RSC payload upstream: compressed by Next.
    crate::http::compression::mark(res, crate::http::compression::Compress::Rendered)
}

fn refusal(status: StatusCode, message: &str) -> Response {
    envelope_response(status, json!({ "error": { "message": message } }))
}

/// Does the `Origin` header name the host this request was sent to? (Next's
/// server-action CSRF check compares hosts, which survives TLS termination.)
pub fn origin_matches(headers: &HeaderMap, trust_proxy: bool) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let Ok(url) = url::Url::parse(origin) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let origin_host = match url.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_string(),
    };
    let forwarded = if trust_proxy {
        headers
            .get("x-forwarded-host")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    } else {
        None
    };
    let Some(request_host) =
        forwarded.or_else(|| headers.get(header::HOST).and_then(|v| v.to_str().ok()).map(str::to_string))
    else {
        return false;
    };
    // A default port in Host is the same host the browser put in Origin.
    let default_port = match url.scheme() {
        "https" => ":443",
        "http" => ":80",
        _ => "",
    };
    let request_host = request_host.to_ascii_lowercase();
    let request_host = if !default_port.is_empty() {
        request_host.strip_suffix(default_port).unwrap_or(&request_host)
    } else {
        &request_host
    };
    request_host == origin_host.to_ascii_lowercase()
}

async fn read_limited(body: Body, limit: usize) -> Result<Bytes, Box<Response>> {
    let mut buf = Vec::new();
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| Box::new(refusal(StatusCode::BAD_REQUEST, "Body could not be read.")))?;
        buf.extend_from_slice(&chunk);
        if buf.len() > limit {
            return Err(Box::new(refusal(StatusCode::PAYLOAD_TOO_LARGE, "Body exceeded 1 MB limit.")));
        }
    }
    Ok(Bytes::from(buf))
}

fn invalid() -> Box<Response> {
    Box::new(refusal(StatusCode::BAD_REQUEST, "Invalid RPC request."))
}

async fn read_form(headers: &HeaderMap, body: Body) -> Result<FormData, Box<Response>> {
    let content_type = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");
    let boundary = multer::parse_boundary(content_type).map_err(|_| invalid())?;
    let bytes = read_limited(body, RPC_BODY_LIMIT).await?;
    let stream = futures_util::stream::once(async move { Ok::<Bytes, std::io::Error>(bytes) });
    let mut multipart = multer::Multipart::new(stream, boundary);
    let mut form = FormData::default();
    while let Some(field) = multipart.next_field().await.map_err(|_| invalid())? {
        let name = field.name().unwrap_or_default().to_string();
        let file_name = field.file_name().map(str::to_string);
        let content_type = field.content_type().map(|m| m.to_string());
        let data = field.bytes().await.map_err(|_| invalid())?;
        let value = match file_name {
            Some(filename) => FormValue::File {
                filename,
                content_type: content_type.unwrap_or_else(|| "application/octet-stream".to_string()),
                data,
            },
            None => FormValue::Text(String::from_utf8_lossy(&data).into_owned()),
        };
        form.fields.push((name, value));
    }
    Ok(form)
}

async fn handle(
    State(state): State<AppState>,
    Path((module, export)): Path<(String, String)>,
    request: Request<Body>,
) -> Response {
    let (parts, body) = request.into_parts();
    let headers = parts.headers;
    let kind = headers.get("x-modlight-rpc").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    if !(kind == "1" || kind == "form") || !origin_matches(&headers, state.config.trust_proxy) {
        return refusal(StatusCode::FORBIDDEN, "Invalid Server Actions request.");
    }
    let Some(action) = state.rpc.get(&module, &export) else {
        return refusal(StatusCode::NOT_FOUND, "Server action not found.");
    };

    let (args, form) = if kind == "form" {
        match read_form(&headers, body).await {
            Ok(form) => (ActionArgs::default(), Some(form)),
            Err(res) => return *res,
        }
    } else {
        let bytes = match read_limited(body, RPC_BODY_LIMIT).await {
            Ok(b) => b,
            Err(res) => return *res,
        };
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Array(items)) => (ActionArgs(items), None),
            _ => return refusal(StatusCode::BAD_REQUEST, "Invalid RPC request."),
        }
    };

    let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
    let cookies = parts.extensions.get::<ResponseCookies>().cloned().unwrap_or_default();
    let headers = Arc::new(headers);
    let session = RequestSession::new(state.clone(), headers.clone(), RefreshPolicy::DbAndCookie, cookies.clone());
    let revalidate = Arc::new(AtomicBool::new(false));
    let ctx = ActionCtx {
        state: state.clone(),
        headers,
        peer,
        args,
        form,
        session,
        cookies: cookies.clone(),
        revalidate: revalidate.clone(),
    };

    let result = action.call(ctx).await;
    let mut body = match result {
        Ok(v) => json!({ "ok": v }),
        Err(ActionError::Redirect(url)) => json!({ "redirect": url }),
        Err(ActionError::NotFound) => json!({ "notFound": true }),
        Err(ActionError::Thrown(message)) => error_envelope(&state, &module, &export, message),
        Err(ActionError::Internal(err)) => {
            tracing::error!(error = ?err, "server action {module}.{export} failed");
            error_envelope(&state, &module, &export, err.to_string())
        }
    };
    // Next 16 counts a modified cookie as a revalidation (`addRevalidationHeader`
    // in action-handler.js) and its client refreshes the page: the envelope says
    // so whenever the response sets or deletes ANY cookie, the session-cookie
    // refresh or expiry included (CONTRACT 7.1).
    if (revalidate.load(Ordering::Relaxed) || !cookies.is_empty())
        && let Value::Object(map) = &mut body
    {
        map.insert("revalidate".into(), Value::Bool(true));
    }
    let mut res = envelope_response(StatusCode::OK, body);
    // Without the front door (tests), write the cookies here.
    if parts.extensions.get::<ResponseCookies>().is_none() {
        cookies.write_to(res.headers_mut());
    }
    res
}

fn error_envelope(state: &AppState, module: &str, export: &str, message: String) -> Value {
    if state.config.is_prod {
        let digest = rand::random::<u32>().to_string();
        tracing::warn!("server action {module}.{export} threw (digest {digest}): {message}");
        json!({ "error": { "message": PRODUCTION_ERROR_MESSAGE, "digest": digest } })
    } else {
        json!({ "error": { "message": message } })
    }
}

impl IntoResponse for ActionError {
    fn into_response(self) -> Response {
        refusal(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut m = HeaderMap::new();
        for (k, v) in pairs {
            m.insert(axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
        }
        m
    }

    #[test]
    fn origin_must_name_the_requested_host() {
        assert!(origin_matches(&h(&[("origin", "http://127.0.0.1:8421"), ("host", "127.0.0.1:8421")]), false));
        assert!(origin_matches(&h(&[("origin", "https://Host.example.com"), ("host", "host.example.com:443")]), false));
        assert!(!origin_matches(&h(&[("origin", "http://evil.test"), ("host", "127.0.0.1:8421")]), false));
        assert!(!origin_matches(&h(&[("host", "127.0.0.1:8421")]), false));
        assert!(!origin_matches(&h(&[("origin", "null"), ("host", "127.0.0.1:8421")]), false));
        let proxied = h(&[
            ("origin", "https://files.example.com"),
            ("host", "app:3000"),
            ("x-forwarded-host", "files.example.com"),
        ]);
        assert!(origin_matches(&proxied, true));
        assert!(!origin_matches(&proxied, false));
    }

    #[test]
    fn args_decode_like_javascript_values() {
        let args =
            ActionArgs(vec![json!("name"), json!({"$undefined": 1}), json!({"$date": "2026-10-18T12:00:00.000Z"})]);
        assert_eq!(args.get::<String>(0).unwrap(), "name");
        assert_eq!(args.get::<Option<String>>(1).unwrap(), None);
        assert!(args.is_undefined(1));
        assert!(args.is_undefined(7));
        assert_eq!(args.get::<Option<String>>(7).unwrap(), None);
        let d: crate::db::types::JsDate = args.get(2).unwrap();
        assert_eq!(d.to_iso_string(), "2026-10-18T12:00:00.000Z");
        assert!(matches!(args.get::<i64>(0), Err(ActionError::Thrown(_))));
    }
}
