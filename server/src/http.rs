//! The HTTP edge: port of the request handling in `server.ts`, `proxy.ts` and
//! the `headers()` of `next.config.ts`, plus the router assembly.
//!
//! Order of a request, as upstream processes it:
//!
//! 1. **Raw upload paths** (`server.ts`): `/api/files*` (tus, any method) and
//!    `POST /upload` / `POST /api/upload` (ShareX) go straight to their
//!    handlers, before everything below: no path normalisation, no proxy, no
//!    config headers, no body limit, no timeout.
//! 2. **Path normalisation** (Next): repeated slashes or a backslash, then a
//!    trailing slash, are answered with a 308 to the clean path
//!    (`Location` + `Refresh: 0;url=...`, the location as body).
//! 3. **proxy.ts**: the shortener-host rewrite and the optimistic `/dashboard`
//!    gate (307 to `/login?next=...`).
//! 4. **Routing**: route handlers of the feature modules, `/_rpc`, `/_page`,
//!    every page pattern (SPA shell), static files, and the fallback (an
//!    unported route handler answers 501, anything else the 404 page).
//! 5. **next.config.ts headers** on the response, matched on the ORIGINAL raw
//!    path; config values win over the same header set by a handler.
//!
//! Plus what Node and Next did implicitly: `OPTIONS` on a route handler
//! answers 204 with `Allow`, an unsupported method 405 without one, and a
//! panic in a handler is a 500 instead of a dead connection.
//!
//! The server itself (`serve`): HTTP/1.1 on BIND_ADDR:PORT, no header, request
//! or idle timeouts (multi-hour uploads, `server.ts` sets them to 0), a
//! malformed request is answered 400 by hyper and only closes its connection,
//! and shutdown stops accepting, then waits briefly for in-flight requests.

pub mod cookies;
pub mod headers;
pub mod proxy;
pub mod set_cookie;
pub mod static_files;

use std::convert::Infallible;
use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderValue, Method, Request, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use futures_util::FutureExt;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tower::ServiceExt;

use crate::http::set_cookie::ResponseCookies;
use crate::state::AppState;

/// The complete application: what one accepted connection serves.
#[derive(Clone)]
pub struct App {
    pub state: AppState,
    inner: Router,
    raw_uploads: Router,
}

/// Every route of the application (except the raw upload paths).
pub fn router(state: &AppState) -> Router {
    Router::new()
        // ── Route handlers (app/**/route.ts): ONE LINE PER MODULE ──
        .merge(crate::routes::api::health::routes()) // app/api/health/route.ts
        .merge(crate::auth::api::routes()) // app/api/auth/[...all]/route.ts
        // ── Frameworks ──
        .merge(crate::rpc::routes())
        .merge(crate::pages::routes())
        .merge(static_files::routes(&state.statics))
        .fallback(crate::pages::fallback)
        .with_state(state.clone())
}

/// The raw upload paths of server.ts (`lib/upload/tus.ts`, `lib/upload/sharex.ts`).
/// ONE LINE PER MODULE: the uploads port replaces the placeholder with its
/// routers (they receive every `/api/files*` request and `POST /upload`,
/// `POST /api/upload`).
pub fn raw_upload_router(state: &AppState) -> Router {
    Router::new()
        .merge(crate::upload::sharex::routes()) // lib/upload/sharex.ts: POST /upload, POST /api/upload
        .fallback(raw_upload_not_ported)
        .with_state(state.clone())
}

async fn raw_upload_not_ported(uri: Uri) -> Response {
    let file = if uri.path().starts_with("/api/files") { "lib/upload/tus.ts" } else { "lib/upload/sharex.ts" };
    crate::api_json::json_response(
        StatusCode::NOT_IMPLEMENTED,
        &serde_json::json!({ "message": "Not implemented in the Rust port yet.", "file": file }),
        Some("no-store"),
    )
}

/// `pathname.startsWith("/api/files")` or `POST /upload | /api/upload`.
pub fn is_raw_upload(method: &Method, pathname: &str) -> bool {
    pathname.starts_with("/api/files")
        || (method == Method::POST && (pathname == "/upload" || pathname == "/api/upload"))
}

/// `new URL(req.url, "http://" + host).pathname`: dot segments resolved, the
/// way server.ts saw the path.
pub fn whatwg_pathname(uri: &Uri) -> String {
    let target = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
    url::Url::parse("http://localhost/")
        .and_then(|base| base.join(target))
        .map(|u| u.path().to_string())
        .unwrap_or_else(|_| uri.path().to_string())
}

/// Next's path normalisation: where a path must be redirected (308) to, if
/// anywhere. Repeated slashes and backslashes first, then one trailing slash.
pub fn normalized_location(path: &str) -> Option<String> {
    if path.contains('\\') || path.contains("//") {
        let mut out = String::with_capacity(path.len());
        let mut last_slash = false;
        for c in path.chars() {
            let c = if c == '\\' { '/' } else { c };
            if c == '/' && last_slash {
                continue;
            }
            last_slash = c == '/';
            out.push(c);
        }
        return Some(out);
    }
    if path.len() > 1 && path.ends_with('/') {
        return Some(path[..path.len() - 1].to_string());
    }
    None
}

fn permanent_redirect(location: &str) -> Response {
    let mut res = (StatusCode::PERMANENT_REDIRECT, location.to_string()).into_response();
    res.headers_mut().remove(header::CONTENT_TYPE);
    if let Ok(v) = HeaderValue::from_str(location) {
        res.headers_mut().insert(header::LOCATION, v.clone());
    }
    if let Ok(v) = HeaderValue::from_str(&format!("0;url={location}")) {
        res.headers_mut().insert("refresh", v);
    }
    res
}

/// Next's order for the `Allow` header of a route handler.
const METHOD_ORDER: &[&str] = &["GET", "HEAD", "OPTIONS", "POST", "PUT", "DELETE", "PATCH"];

/// 405 handling as Next's app routes do it.
fn fix_method_not_allowed(method: &Method, mut res: Response) -> Response {
    if res.status() != StatusCode::METHOD_NOT_ALLOWED {
        return res;
    }
    let allow = res.headers_mut().remove(header::ALLOW);
    if method != Method::OPTIONS {
        return res;
    }
    let mut methods: Vec<String> = allow
        .and_then(|v| v.to_str().ok().map(str::to_string))
        .unwrap_or_default()
        .split(',')
        .map(|m| m.trim().to_ascii_uppercase())
        .filter(|m| !m.is_empty())
        .collect();
    methods.push("OPTIONS".into());
    methods.sort_by_key(|m| METHOD_ORDER.iter().position(|o| o == m).unwrap_or(usize::MAX));
    methods.dedup();
    let mut ok = StatusCode::NO_CONTENT.into_response();
    if let Ok(v) = HeaderValue::from_str(&methods.join(", ")) {
        ok.headers_mut().insert(header::ALLOW, v);
    }
    ok
}

impl App {
    pub fn new(state: AppState) -> App {
        App { inner: router(&state), raw_uploads: raw_upload_router(&state), state }
    }

    /// Answer one request (steps 1-5 of the module docs).
    pub async fn handle(&self, req: Request<Body>) -> Response {
        let fut = AssertUnwindSafe(self.handle_inner(req)).catch_unwind();
        match fut.await {
            Ok(res) => res,
            Err(_) => {
                tracing::error!("a request handler panicked");
                crate::error::internal_error_response()
            }
        }
    }

    async fn handle_inner(&self, mut req: Request<Body>) -> Response {
        // 1. Raw upload paths.
        let pathname = whatwg_pathname(req.uri());
        if is_raw_upload(req.method(), &pathname) {
            return self.raw_uploads.clone().oneshot(req).await.unwrap_or_else(|e| match e {});
        }

        // 2. Path normalisation.
        let raw_path = req.uri().path().to_string();
        if let Some(clean) = normalized_location(&raw_path) {
            let location = match req.uri().query() {
                Some(q) => format!("{clean}?{q}"),
                None => clean,
            };
            return permanent_redirect(&location);
        }

        // 3. proxy.ts
        let host = req.headers().get(header::HOST).and_then(|v| v.to_str().ok()).map(str::to_string);
        let rewritten = self.state.proxy.rewrite(host.as_deref(), &raw_path);
        let response = if let Some(new_path) = rewritten {
            let pq = match req.uri().query() {
                Some(q) => format!("{new_path}?{q}"),
                None => new_path,
            };
            let mut parts = req.uri().clone().into_parts();
            parts.path_and_query = pq.parse().ok();
            if let Ok(uri) = Uri::from_parts(parts) {
                *req.uri_mut() = uri;
            }
            self.route(req).await
        } else if let Some(location) = self.state.proxy.dashboard_gate(&raw_path, req.headers()) {
            proxy::redirect_response(&location)
        } else {
            self.route(req).await
        };

        // 5. next.config.ts headers, on the original path.
        let mut response = response;
        self.state.security.apply(&raw_path, response.headers_mut());
        response
    }

    /// 4. Routing, with the per-request cookie store.
    async fn route(&self, mut req: Request<Body>) -> Response {
        let method = req.method().clone();
        let cookies = ResponseCookies::default();
        req.extensions_mut().insert(cookies.clone());
        let res = self.inner.clone().oneshot(req).await.unwrap_or_else(|e| match e {});
        let mut res = fix_method_not_allowed(&method, res);
        cookies.write_to(res.headers_mut());
        res
    }
}

/// Serve `app` on `listener` until `shutdown` resolves.
pub async fn serve(
    listener: TcpListener,
    app: App,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let graceful = hyper_util::server::graceful::GracefulShutdown::new();
    let mut shutdown = std::pin::pin!(shutdown);
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, peer) = match accepted {
                    Ok(pair) => pair,
                    Err(err) => {
                        // EMFILE and friends: never fatal, back off a little.
                        tracing::warn!("accept failed: {err}");
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                };
                let _ = stream.set_nodelay(true);
                let app = app.clone();
                let service = hyper::service::service_fn(move |req: Request<hyper::body::Incoming>| {
                    let app = app.clone();
                    async move {
                        let mut req = req.map(Body::new);
                        req.extensions_mut().insert(ConnectInfo(peer));
                        Ok::<_, Infallible>(app.handle(req).await)
                    }
                });
                // No timer: no header-read, request or keep-alive timeouts.
                let conn = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service);
                let conn = graceful.watch(conn);
                tokio::spawn(async move {
                    if let Err(err) = conn.await {
                        tracing::debug!("connection from {peer} ended: {err}");
                    }
                });
            }
            _ = &mut shutdown => break,
        }
    }
    drop(listener);
    tokio::select! {
        _ = graceful.shutdown() => {}
        _ = tokio::time::sleep(Duration::from_secs(10)) => {
            tracing::warn!("closing with requests still in flight");
        }
    }
    Ok(())
}

/// The address to listen on.
pub fn listen_addr(state: &AppState) -> anyhow::Result<SocketAddr> {
    let port = state.config.listen_port()?;
    let ip: std::net::IpAddr = state
        .config
        .bind_addr
        .parse()
        .map_err(|_| anyhow::anyhow!("BIND_ADDR is not an IP address: {}", state.config.bind_addr))?;
    Ok(SocketAddr::new(ip, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisation_matches_next() {
        assert_eq!(normalized_location("/login/"), Some("/login".into()));
        assert_eq!(normalized_location("//login"), Some("/login".into()));
        assert_eq!(normalized_location("/login//"), Some("/login/".into()));
        assert_eq!(normalized_location("///a//b"), Some("/a/b".into()));
        assert_eq!(normalized_location("/a\\b"), Some("/a/b".into()));
        assert_eq!(normalized_location("/"), None);
        assert_eq!(normalized_location("/login"), None);
    }

    #[test]
    fn raw_upload_paths() {
        assert!(is_raw_upload(&Method::PATCH, "/api/files/abc"));
        assert!(is_raw_upload(&Method::GET, "/api/filesx"));
        assert!(is_raw_upload(&Method::POST, "/upload"));
        assert!(!is_raw_upload(&Method::GET, "/upload"));
        assert!(is_raw_upload(&Method::POST, "/api/upload"));
        assert!(!is_raw_upload(&Method::POST, "/upload/"));
        let uri: Uri = "/x/../api/files/1?a=b".parse().unwrap();
        assert_eq!(whatwg_pathname(&uri), "/api/files/1");
    }

    #[test]
    fn options_and_405_like_next() {
        let mut r = StatusCode::METHOD_NOT_ALLOWED.into_response();
        r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("GET,HEAD"));
        let fixed = fix_method_not_allowed(&Method::OPTIONS, r);
        assert_eq!(fixed.status(), StatusCode::NO_CONTENT);
        assert_eq!(fixed.headers()[header::ALLOW], "GET, HEAD, OPTIONS");
        let mut r = StatusCode::METHOD_NOT_ALLOWED.into_response();
        r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("GET,HEAD"));
        let fixed = fix_method_not_allowed(&Method::POST, r);
        assert_eq!(fixed.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert!(fixed.headers().get(header::ALLOW).is_none());
    }
}
