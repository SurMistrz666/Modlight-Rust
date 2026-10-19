//! Pages as SPA routes with a page-data endpoint (PORT.md section 8).
//!
//! ```text
//! GET /_page?path=<pathname>&<the page's own query string>
//! ```
//!
//! answers one of (tagged JSON):
//!
//! | Outcome | HTTP | Body |
//! |---|---|---|
//! | page | 200 | `{"route", "params", "layouts": {"root": .., "(app)": ..}, "props", "meta"}` |
//! | `redirect()` | 200 | `{"redirect": "/login", "status": 307}` |
//! | `notFound()` / no route | 200 | `{"notFound": true, "layouts": {"root": ..}, "meta"}` |
//! | URL is a route handler or a static file | 200 | `{"reload": true}` (load it as a document) |
//! | loader not ported yet | 501 | `{"notImplemented": true, "route", "file", "layouts"}` |
//! | loader failed | 500 | `{"error": {"message"}, "layouts"}` |
//!
//! A page URL requested directly gets the SPA shell ([`shell`]) with the same
//! payload inlined, the head from [`meta`], and the status the outcome implies
//! (307 + `Location` for a redirect, 404 for not found).
//!
//! Loaders: one per upstream page (`pub async fn load(ctx: PageCtx) ->
//! PageResult`), registered in [`register_all`] by app path. Layout loaders
//! work the same way and run first, outside in; a layout that redirects
//! (e.g. `(app)`'s `requireUser()`) stops the page from loading. A page without
//! a registered loader answers 501 so gaps stay visible.
//!
//! Session policy (see [`crate::auth::session`]): a direct page load refreshes
//! the session row without re-issuing the cookie (a Server Component cannot
//! write cookies); a `/_page` fetch is a client-side navigation (`RSC: 1`
//! upstream) and does not refresh at all.

pub mod layout_auth;
pub mod meta;
pub mod root_layout;
pub mod shell;
pub mod table;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodFilter, get, on};
use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use serde_json::{Value, json};

use crate::auth::session::{AuthSession, RefreshPolicy};
use crate::auth_helpers::{AuthRefusal, RequestSession};
use crate::background::BackgroundStyle;
use crate::http::cookies::parse_cookies_next;
use crate::http::set_cookie::ResponseCookies;
use crate::i18n::{Locale, Translator};
use crate::rpc::tagged;
use crate::state::AppState;
use meta::{PageMeta, ResolvedHead};
pub use table::{AppRoute, Layout, ParamValue, RouteKind};

/// Where a page load comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageMode {
    /// The browser asked for the URL itself (first paint, link preview bot).
    Document,
    /// The SPA fetched `/_page` for a client-side navigation.
    Navigation,
}

/// A page's `searchParams`, in request order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchParams(pub Vec<(String, String)>);

impl SearchParams {
    /// `URLSearchParams` parsing (`+` is a space, percent-decoding is lossy).
    pub fn parse(query: &str) -> SearchParams {
        SearchParams(form_urlencoded::parse(query.as_bytes()).into_owned().collect())
    }

    /// The first value (`Array.isArray(v) ? v[0] : v`).
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn get_all(&self, key: &str) -> Vec<&str> {
        self.0.iter().filter(|(k, _)| k == key).map(|(_, v)| v.as_str()).collect()
    }

    /// Next's `searchParams` object: a string, or an array for repeated keys.
    pub fn to_value(&self) -> Value {
        let mut map = serde_json::Map::new();
        for (k, v) in &self.0 {
            match map.get_mut(k) {
                None => {
                    map.insert(k.clone(), Value::String(v.clone()));
                }
                Some(Value::Array(items)) => items.push(Value::String(v.clone())),
                Some(existing) => {
                    let first = existing.take();
                    *existing = Value::Array(vec![first, Value::String(v.clone())]);
                }
            }
        }
        Value::Object(map)
    }

    /// The query string again (`a=1&b=2`).
    pub fn to_query(&self) -> String {
        form_urlencoded::Serializer::new(String::new()).extend_pairs(self.0.iter()).finish()
    }
}

/// Everything a page or layout loader can see.
#[derive(Clone)]
pub struct PageCtx {
    pub state: AppState,
    pub route: &'static AppRoute,
    /// The pathname being rendered (after the shortener rewrite).
    pub path: String,
    pub params: IndexMap<String, ParamValue>,
    pub search: SearchParams,
    pub headers: Arc<HeaderMap>,
    pub peer: Option<SocketAddr>,
    pub mode: PageMode,
    /// `getSession()` with this load's refresh policy, shared by all loaders.
    pub session: RequestSession,
}

impl PageCtx {
    /// A single-segment param (`params.userCode`).
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.get(name).and_then(ParamValue::as_str)
    }

    /// The request's locale (i18n/request.ts).
    pub fn locale(&self) -> Locale {
        crate::i18n::request::request_locale(&self.headers)
    }

    /// `getTranslations(namespace)`
    pub fn t(&self, namespace: &'static str) -> Translator<'_> {
        self.state.messages.translator(self.locale(), namespace)
    }

    /// `(await cookies())` as Next reads them.
    pub fn cookies(&self) -> IndexMap<String, String> {
        parse_cookies_next(&self.headers)
    }

    /// `clientIp(await headers())`
    pub fn client_ip(&self) -> String {
        crate::password::client_ip(&self.headers, self.peer, self.state.config.trust_proxy)
    }

    pub async fn get_session(&self) -> Result<Option<AuthSession>, PageError> {
        self.session.get().await.map_err(|e| PageError::Internal(e.into()))
    }

    pub async fn require_user(&self) -> Result<AuthSession, PageError> {
        Ok(self.session.require_user().await?)
    }

    pub async fn require_admin(&self) -> Result<AuthSession, PageError> {
        Ok(self.session.require_admin().await?)
    }
}

/// How a loader ended other than with data.
#[derive(Debug)]
pub enum PageError {
    /// `redirect(to)` (307) or `permanentRedirect(to)` (308).
    Redirect { to: String, status: StatusCode },
    /// `notFound()`
    NotFound,
    /// No loader registered yet (the default).
    NotImplemented,
    /// Anything else (500).
    Internal(anyhow::Error),
}

impl PageError {
    pub fn redirect(to: impl Into<String>) -> PageError {
        PageError::Redirect { to: to.into(), status: StatusCode::TEMPORARY_REDIRECT }
    }

    pub fn permanent_redirect(to: impl Into<String>) -> PageError {
        PageError::Redirect { to: to.into(), status: StatusCode::PERMANENT_REDIRECT }
    }
}

impl From<AuthRefusal> for PageError {
    fn from(r: AuthRefusal) -> Self {
        match r {
            AuthRefusal::Redirect(to) => PageError::redirect(to),
            AuthRefusal::Failed(e) => PageError::Internal(e.into()),
        }
    }
}

impl From<anyhow::Error> for PageError {
    fn from(e: anyhow::Error) -> Self {
        PageError::Internal(e)
    }
}

impl From<sqlx::Error> for PageError {
    fn from(e: sqlx::Error) -> Self {
        PageError::Internal(e.into())
    }
}

impl From<serde_json::Error> for PageError {
    fn from(e: serde_json::Error) -> Self {
        PageError::Internal(e.into())
    }
}

/// What a page loader produces: the props upstream passed to its client
/// components (build them with [`tagged::to_tagged_value`] or [`PageData::new`])
/// and the page's metadata.
#[derive(Debug, Clone, Default)]
pub struct PageData {
    pub props: Value,
    pub meta: PageMeta,
    /// The skin and wallpaper the page's own `<ThemeApplier>` paints before the
    /// first frame (public pages show the OWNER's), for the document only
    /// (CONTRACT 2.2). Pages under the `(app)` layout get theirs from the layout.
    pub skin: Option<DocumentSkin>,
}

impl PageData {
    /// Props from anything serializable (tagged encoding applied).
    pub fn new<T: serde::Serialize + ?Sized>(props: &T, meta: PageMeta) -> Result<PageData, PageError> {
        Ok(PageData { props: tagged::to_tagged_value(props)?, meta, skin: None })
    }

    /// Paint the document with this skin and wallpaper (`<ThemeApplier theme
    /// background>`): `theme` is resolved here, as the applier does.
    pub fn with_skin(mut self, theme: Option<&str>, background: Option<BackgroundStyle>) -> PageData {
        self.skin = Some(DocumentSkin { theme: crate::themes::resolve_theme(theme), background });
        self
    }
}

/// `data-theme` and the wallpaper `<ThemeApplier>` sets on `<html>`.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentSkin {
    pub theme: &'static str,
    pub background: Option<BackgroundStyle>,
}

impl DocumentSkin {
    /// The `style` attribute value: `--ml-bg-image:url("<url>");--ml-bg-dim:<String(dim)>`.
    pub fn style(&self) -> Option<String> {
        self.background.as_ref().map(|bg| {
            format!(
                "--ml-bg-image:url(\"{}\");--ml-bg-dim:{}",
                bg.url,
                crate::i18n::format::js_number_to_string(bg.dim)
            )
        })
    }

    /// From the `(app)` layout payload (`theme`, `background`).
    pub fn from_app_layout(layout: &Value) -> DocumentSkin {
        let theme = crate::themes::resolve_theme(layout.get("theme").and_then(Value::as_str));
        let background = layout.get("background").and_then(|bg| {
            Some(BackgroundStyle { url: bg.get("url")?.as_str()?.to_string(), dim: bg.get("dim")?.as_f64()? })
        });
        DocumentSkin { theme, background }
    }
}

pub type PageResult = Result<PageData, PageError>;
pub type LayoutResult = Result<Value, PageError>;

/// A page's server side: `generateMetadata` + the server component's props.
pub trait PageLoader: Send + Sync + 'static {
    fn load(&self, ctx: PageCtx) -> BoxFuture<'static, PageResult>;
}

impl<F, Fut> PageLoader for F
where
    F: Fn(PageCtx) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = PageResult> + Send + 'static,
{
    fn load(&self, ctx: PageCtx) -> BoxFuture<'static, PageResult> {
        Box::pin(self(ctx))
    }
}

/// A layout's server side: the data it hands to its client components.
pub trait LayoutLoader: Send + Sync + 'static {
    fn load(&self, ctx: PageCtx) -> BoxFuture<'static, LayoutResult>;
}

impl<F, Fut> LayoutLoader for F
where
    F: Fn(PageCtx) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = LayoutResult> + Send + 'static,
{
    fn load(&self, ctx: PageCtx) -> BoxFuture<'static, LayoutResult> {
        Box::pin(self(ctx))
    }
}

/// Loaders by app path / layout.
#[derive(Default)]
pub struct PageRegistry {
    pages: HashMap<&'static str, Arc<dyn PageLoader>>,
    layouts: HashMap<Layout, Arc<dyn LayoutLoader>>,
}

impl PageRegistry {
    /// Register the loader of the page at `path` (an app path from
    /// [`table::APP_ROUTES`], e.g. `/dashboard/folders`, `/u/[userCode]`).
    pub fn page(&mut self, path: &'static str, loader: impl PageLoader) {
        let route = table::find_route(path).unwrap_or_else(|| panic!("no upstream page at {path}"));
        assert_eq!(route.kind, RouteKind::Page, "{path} is a route handler, not a page");
        if self.pages.insert(path, Arc::new(loader)).is_some() {
            panic!("page loader for {path} registered twice");
        }
    }

    pub fn layout(&mut self, layout: Layout, loader: impl LayoutLoader) {
        if self.layouts.insert(layout, Arc::new(loader)).is_some() {
            panic!("layout loader for {} registered twice", layout.key());
        }
    }

    pub fn has_page(&self, path: &str) -> bool {
        self.pages.contains_key(path)
    }
}

/// Every page and layout loader. ONE LINE PER MODULE: other agents add theirs
/// here (`reg.page("/dashboard/folders", dashboard_folders::load);`).
pub fn register_all(reg: &mut PageRegistry) {
    reg.layout(Layout::Root, root_layout::load);
    reg.layout(Layout::Auth, layout_auth::load);
}

// ── Pipeline ────────────────────────────────────────────────────────────────

/// The result of loading one page URL.
pub enum PageOutcome {
    Page {
        route: &'static AppRoute,
        params: IndexMap<String, ParamValue>,
        layouts: IndexMap<String, Value>,
        props: Value,
        head: ResolvedHead,
        skin: Option<DocumentSkin>,
    },
    Redirect {
        to: String,
        status: StatusCode,
    },
    NotFound {
        layouts: IndexMap<String, Value>,
        head: ResolvedHead,
    },
    NotImplemented {
        route: &'static AppRoute,
        missing: String,
        layouts: IndexMap<String, Value>,
    },
    Error {
        error: anyhow::Error,
        layouts: IndexMap<String, Value>,
    },
}

impl PageOutcome {
    pub fn status(&self) -> StatusCode {
        match self {
            PageOutcome::Page { .. } => StatusCode::OK,
            PageOutcome::Redirect { status, .. } => *status,
            PageOutcome::NotFound { .. } => StatusCode::NOT_FOUND,
            PageOutcome::NotImplemented { .. } => StatusCode::NOT_IMPLEMENTED,
            PageOutcome::Error { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// The JSON payload (`/_page` body and the inlined `__MODLIGHT_PAGE__`).
    pub fn payload(&self, is_prod: bool) -> Value {
        match self {
            PageOutcome::Page { route, params, layouts, props, head, .. } => json!({
                "route": route.path,
                "params": params,
                "layouts": layouts,
                "props": props,
                "meta": head,
            }),
            PageOutcome::Redirect { to, status } => json!({ "redirect": to, "status": status.as_u16() }),
            PageOutcome::NotFound { layouts, head } => json!({ "notFound": true, "layouts": layouts, "meta": head }),
            PageOutcome::NotImplemented { route, missing, layouts } => json!({
                "notImplemented": true,
                "route": route.path,
                "file": missing,
                "layouts": layouts,
            }),
            PageOutcome::Error { error, layouts } => {
                let message =
                    if is_prod { crate::rpc::PRODUCTION_ERROR_MESSAGE.to_string() } else { error.to_string() };
                json!({ "error": { "message": message }, "layouts": layouts })
            }
        }
    }
}

/// Load the page `matched` for `ctx` (layouts first, outside in).
pub async fn load_page(ctx: PageCtx) -> PageOutcome {
    let state = ctx.state.clone();
    let route = ctx.route;
    let mut layouts: IndexMap<String, Value> = IndexMap::new();
    for layout in route.layouts {
        let Some(loader) = state.pages.layouts.get(layout) else {
            return PageOutcome::NotImplemented { route, missing: layout.file().to_string(), layouts };
        };
        match loader.load(ctx.clone()).await {
            Ok(v) => {
                layouts.insert(layout.key().to_string(), v);
            }
            Err(e) => return error_outcome(&state, e, layouts),
        }
    }
    let Some(loader) = state.pages.pages.get(route.path) else {
        return PageOutcome::NotImplemented { route, missing: route.file.to_string(), layouts };
    };
    match loader.load(ctx.clone()).await {
        Ok(data) => {
            let skin = match layouts.get(Layout::App.key()) {
                Some(app) => Some(DocumentSkin::from_app_layout(app)),
                None => data.skin,
            };
            PageOutcome::Page {
                route,
                params: ctx.params.clone(),
                layouts,
                props: data.props,
                head: meta::resolve(&state.config, &data.meta, false),
                skin,
            }
        }
        Err(e) => error_outcome(&state, e, layouts),
    }
}

fn error_outcome(state: &AppState, e: PageError, mut layouts: IndexMap<String, Value>) -> PageOutcome {
    match e {
        PageError::Redirect { to, status } => PageOutcome::Redirect { to, status },
        PageError::NotFound => {
            // The not-found boundary renders inside the root layout only.
            layouts.retain(|k, _| k == Layout::Root.key());
            PageOutcome::NotFound { layouts, head: meta::resolve(&state.config, &PageMeta::default(), true) }
        }
        PageError::NotImplemented => PageOutcome::Error { error: anyhow::anyhow!("not implemented"), layouts },
        PageError::Internal(error) => {
            tracing::error!(error = ?error, "page failed to load");
            PageOutcome::Error { error, layouts }
        }
    }
}

/// The root layout alone (not-found pages outside any route).
async fn root_only(state: &AppState, base: &PageCtx) -> IndexMap<String, Value> {
    let mut layouts = IndexMap::new();
    if let Some(loader) = state.pages.layouts.get(&Layout::Root)
        && let Ok(v) = loader.load(base.clone()).await
    {
        layouts.insert(Layout::Root.key().to_string(), v);
    }
    layouts
}

// ── HTTP ────────────────────────────────────────────────────────────────────

/// `Cache-Control` of every dynamic page in Next.
pub const PAGE_CACHE_CONTROL: &str = "private, no-cache, no-store, max-age=0, must-revalidate";

fn with_page_cache(mut res: Response) -> Response {
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(PAGE_CACHE_CONTROL));
    res
}

fn json_page(status: StatusCode, body: &Value) -> Response {
    with_page_cache(crate::api_json::json_response(status, body, None))
}

/// The placeholder for an upstream route handler nobody has ported yet.
pub fn not_ported_handler(route: &AppRoute) -> Response {
    crate::api_json::json_response(
        StatusCode::NOT_IMPLEMENTED,
        &json!({ "message": "Not implemented in the Rust port yet.", "route": route.path, "file": route.file }),
        Some("no-store"),
    )
}

fn malformed() -> Response {
    crate::api_json::json_message(StatusCode::BAD_REQUEST, "Bad Request", None)
}

/// The request side of a page load.
struct RequestInfo {
    headers: Arc<HeaderMap>,
    peer: Option<SocketAddr>,
    cookies: ResponseCookies,
    mode: PageMode,
}

fn base_ctx(
    state: &AppState,
    route: &'static AppRoute,
    path: &str,
    params: IndexMap<String, ParamValue>,
    search: SearchParams,
    req: RequestInfo,
) -> PageCtx {
    let policy = match req.mode {
        PageMode::Document => RefreshPolicy::DbOnly,
        PageMode::Navigation => RefreshPolicy::Skip,
    };
    PageCtx {
        session: RequestSession::new(state.clone(), req.headers.clone(), policy, req.cookies),
        state: state.clone(),
        route,
        path: path.to_string(),
        params,
        search,
        headers: req.headers,
        peer: req.peer,
        mode: req.mode,
    }
}

/// Render an outcome as a document (the SPA shell).
async fn document_response(state: &AppState, ctx: &PageCtx, outcome: PageOutcome) -> Response {
    if let PageOutcome::Redirect { to, status } = &outcome {
        let mut res = (*status).into_response();
        if let Ok(v) = HeaderValue::from_str(to) {
            res.headers_mut().insert(header::LOCATION, v);
        }
        return with_page_cache(res);
    }
    let root = root_layout::compute(ctx).await;
    let head = match &outcome {
        PageOutcome::Page { head, .. } | PageOutcome::NotFound { head, .. } => head.clone(),
        _ => meta::resolve(&state.config, &PageMeta::default(), false),
    };
    let skin = match &outcome {
        PageOutcome::Page { skin, .. } => skin.clone(),
        _ => None,
    };
    let background_style = skin.as_ref().and_then(DocumentSkin::style);
    let payload = outcome.payload(state.config.is_prod);
    let html = state.shell.render(&shell::ShellParts {
        lang: root.locale.as_str(),
        data_mode: root.mode,
        data_theme: skin.as_ref().map(|s| s.theme).unwrap_or(root.theme),
        background_style: background_style.as_deref(),
        head: &meta::render_head(&head),
        payload_json: &tagged::json_for_html(&payload),
    });
    let mut res = (outcome.status(), html).into_response();
    res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    with_page_cache(res)
}

/// The not-found document for a URL no route matches.
pub async fn not_found_document(state: &AppState, uri: &Uri, headers: HeaderMap, peer: Option<SocketAddr>) -> Response {
    let route = table::find_route("/").expect("the root page exists");
    let req =
        RequestInfo { headers: Arc::new(headers), peer, cookies: ResponseCookies::default(), mode: PageMode::Document };
    let ctx = base_ctx(state, route, uri.path(), IndexMap::new(), SearchParams::default(), req);
    let layouts = root_only(state, &ctx).await;
    let outcome = PageOutcome::NotFound { layouts, head: meta::resolve(&state.config, &PageMeta::default(), true) };
    document_response(state, &ctx, outcome).await
}

/// Serve a page URL as a document. Registered for every page pattern; it
/// re-resolves the URL with Next's matcher, so a URL that belongs to a route
/// handler nobody has ported yet answers the 501 placeholder instead of
/// rendering the page that axum would otherwise have picked.
async fn page_document(State(state): State<AppState>, req: axum::extract::Request) -> Response {
    let (parts, _body) = req.into_parts();
    let (uri, headers) = (parts.uri, parts.headers);
    let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
    let cookies = parts.extensions.get::<ResponseCookies>().cloned();
    let matched = match table::match_path(uri.path()) {
        Ok(m) => m,
        Err(_) => return malformed(),
    };
    let Some(matched) = matched else {
        return not_found_document(&state, &uri, headers, peer).await;
    };
    if matched.route.kind == RouteKind::Handler {
        return not_ported_handler(matched.route);
    }
    let req = RequestInfo {
        headers: Arc::new(headers),
        peer,
        cookies: cookies.unwrap_or_default(),
        mode: PageMode::Document,
    };
    let search = SearchParams::parse(uri.query().unwrap_or(""));
    let ctx = base_ctx(&state, matched.route, uri.path(), matched.params, search, req);
    let outcome = load_page(ctx.clone()).await;
    document_response(&state, &ctx, outcome).await
}

/// `GET /_page?path=...`
async fn page_data(State(state): State<AppState>, req: axum::extract::Request) -> Response {
    let (parts, _body) = req.into_parts();
    let (uri, headers) = (parts.uri, parts.headers);
    let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
    let cookies = parts.extensions.get::<ResponseCookies>().cloned();
    // The first `path` is the pathname; every other pair is the page's own.
    let mut path: Option<String> = None;
    let mut rest = Vec::new();
    for (k, v) in form_urlencoded::parse(uri.query().unwrap_or("").as_bytes()).into_owned() {
        if k == "path" && path.is_none() {
            path = Some(v);
        } else {
            rest.push((k, v));
        }
    }
    let Some(path) = path.filter(|p| p.starts_with('/')) else {
        return json_page(
            StatusCode::BAD_REQUEST,
            &json!({ "error": { "message": "path must be an absolute pathname" } }),
        );
    };
    let search = SearchParams(rest);

    // The same front door a document request goes through.
    if let Some(normalized) = crate::http::normalized_location(&path) {
        let to = if search.0.is_empty() { normalized } else { format!("{normalized}?{}", search.to_query()) };
        return json_page(StatusCode::OK, &json!({ "redirect": to, "status": 308 }));
    }
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    let rewritten = state.proxy.rewrite(host, &path);
    if rewritten.is_none()
        && let Some(location) = state.proxy.dashboard_gate(&path, &headers)
    {
        return json_page(StatusCode::OK, &json!({ "redirect": location, "status": 307 }));
    }
    let path = rewritten.unwrap_or(path);
    if crate::http::static_files::is_static_path(&state, &path) {
        return json_page(StatusCode::OK, &json!({ "reload": true }));
    }
    let matched = match table::match_path(&path) {
        Ok(m) => m,
        Err(_) => return malformed(),
    };
    let headers = Arc::new(headers);
    let cookies = cookies.unwrap_or_default();
    let Some(matched) = matched else {
        let route = table::find_route("/").expect("the root page exists");
        let req = RequestInfo { headers, peer, cookies, mode: PageMode::Navigation };
        let ctx = base_ctx(&state, route, &path, IndexMap::new(), search, req);
        let layouts = root_only(&state, &ctx).await;
        let outcome = PageOutcome::NotFound { layouts, head: meta::resolve(&state.config, &PageMeta::default(), true) };
        return json_page(StatusCode::OK, &outcome.payload(state.config.is_prod));
    };
    if matched.route.kind == RouteKind::Handler {
        return json_page(StatusCode::OK, &json!({ "reload": true }));
    }
    let req = RequestInfo { headers, peer, cookies, mode: PageMode::Navigation };
    let ctx = base_ctx(&state, matched.route, &path, matched.params, search, req);
    let outcome = load_page(ctx).await;
    let status = match &outcome {
        PageOutcome::NotImplemented { .. } => StatusCode::NOT_IMPLEMENTED,
        PageOutcome::Error { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::OK,
    };
    json_page(status, &outcome.payload(state.config.is_prod))
}

/// `/_page` plus a document route for every upstream page pattern, so axum's
/// matcher ranks pages against route handlers exactly like Next does.
pub fn routes() -> Router<AppState> {
    let mut router = Router::new().route("/_page", get(page_data));
    for route in table::APP_ROUTES.iter().filter(|r| r.kind == RouteKind::Page) {
        router = router.route(
            &table::axum_path(route.path),
            on(MethodFilter::GET.or(MethodFilter::HEAD).or(MethodFilter::POST), page_document),
        );
    }
    router
}

/// The fallback for URLs no axum route took: an unported route handler (501),
/// or the not-found document (404).
pub async fn fallback(State(state): State<AppState>, req: axum::extract::Request) -> Response {
    let (parts, _body) = req.into_parts();
    let (method, uri, headers) = (parts.method, parts.uri, parts.headers);
    let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
    match table::match_path(uri.path()) {
        Err(_) => malformed(),
        Ok(Some(m)) if m.route.kind == RouteKind::Handler => not_ported_handler(m.route),
        Ok(_) => {
            let res = not_found_document(&state, &uri, headers, peer).await;
            if method == Method::HEAD {
                let (parts, _) = res.into_parts();
                return Response::from_parts(parts, Body::empty());
            }
            res
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_params_like_next() {
        let sp = SearchParams::parse("a=1&a=2&b=x+y&c=%2F");
        assert_eq!(sp.get("a"), Some("1"));
        assert_eq!(sp.get_all("a"), vec!["1", "2"]);
        assert_eq!(sp.to_value(), json!({ "a": ["1", "2"], "b": "x y", "c": "/" }));
        assert_eq!(sp.to_query(), "a=1&a=2&b=x+y&c=%2F");
    }
}
