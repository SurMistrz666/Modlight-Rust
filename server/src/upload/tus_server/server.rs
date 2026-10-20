//! Port of `@tus/server` 2.4.1: `Server.handler`, the method handlers
//! (`OptionsHandler`, `PostHandler`, `HeadHandler`, `PatchHandler`,
//! `DeleteHandler`, `GetHandler`), `BaseHandler` (URLs, ids, body limits,
//! `writeToStore`), the header validators and `MemoryLocker`.
//!
//! Wire details kept from the library as the reference build sends them:
//! header names lower case and sorted (fetch `Headers` iteration), every
//! response but a 204 carries `content-type: text/plain;charset=UTF-8`
//! (a string body), `Access-Control-Expose-Headers` ends in "Forwarded,"
//! (an empty `exposedHeaders` array is joined in), errors after the request
//! was aborted carry `connection: close`, and the error bodies of `ERRORS`.
//! Hyper writes an empty body with `content-length: 0` where Node streamed a
//! zero-length chunked body, and the status line carries a reason phrase.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use tokio_util::sync::CancellationToken;

use super::store::{
    DataStore, EXT_CREATION_DEFER_LENGTH, EXT_EXPIRATION, Metadata, StoreError, TusError, Upload, errors,
    parse_metadata, stringify_metadata,
};
use crate::storage::ByteStream;

/// `TUS_RESUMABLE`
pub const TUS_RESUMABLE: &str = "1.0.0";

/// `HEADERS` of `@tus/utils/constants`.
pub const HEADERS: &[&str] = &[
    "Authorization",
    "Content-Type",
    "Location",
    "Tus-Extension",
    "Tus-Max-Size",
    "Tus-Resumable",
    "Tus-Version",
    "Upload-Concat",
    "Upload-Defer-Length",
    "Upload-Length",
    "Upload-Metadata",
    "Upload-Offset",
    "X-HTTP-Method-Override",
    "X-Requested-With",
    "X-Forwarded-Host",
    "X-Forwarded-Proto",
    "Forwarded",
];

const ALLOWED_METHODS: &str = "POST, HEAD, PATCH, OPTIONS, DELETE";
const MAX_AGE: u64 = 86_400;

/// What a request looks like to the hooks.
pub struct TusRequest {
    pub method: Method,
    /// The absolute URL srvx builds: `http://<host><path?query>`.
    pub url: String,
    pub headers: HeaderMap,
}

impl TusRequest {
    pub fn header(&self, name: &str) -> Option<String> {
        header_value(&self.headers, name)
    }
}

/// `headers.get(name)`: repeated headers joined with ", ", latin1.
pub fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let values: Vec<String> =
        headers.get_all(name).iter().map(|v| v.as_bytes().iter().map(|b| *b as char).collect()).collect();
    if values.is_empty() { None } else { Some(values.join(", ")) }
}

/// What `onUploadFinish` may change about the response.
#[derive(Debug, Default)]
pub struct FinishPatch {
    pub status_code: Option<u16>,
    pub body: Option<String>,
    pub headers: Vec<(String, String)>,
}

/// The `ServerOptions` callbacks `lib/upload/tus.ts` sets.
pub trait TusHooks: Send + Sync {
    /// `maxSize(req, id)`: 0 means unlimited.
    fn max_size<'a>(&'a self, req: &'a TusRequest) -> BoxFuture<'a, u64>;
    /// `onUploadCreate(req, upload)`: the metadata to store, or a refusal.
    fn on_upload_create<'a>(
        &'a self,
        req: &'a TusRequest,
        upload: &'a Upload,
    ) -> BoxFuture<'a, Result<Option<Metadata>, StoreError>>;
    /// `onUploadFinish(req, upload)`
    fn on_upload_finish<'a>(
        &'a self,
        req: &'a TusRequest,
        upload: &'a Upload,
    ) -> BoxFuture<'a, Result<FinishPatch, StoreError>>;
}

/// `ServerOptions`
pub struct ServerOptions {
    pub path: String,
    pub respect_forwarded_headers: bool,
    /// `lockDrainTimeout` (3000 ms by default).
    pub lock_drain_timeout: Duration,
    /// `MemoryLocker` acquire timeout (30 s by default).
    pub lock_timeout: Duration,
    /// `disableTerminationForFinishedUploads` (false by default): DELETE on a
    /// finished upload answers 400 instead of removing it.
    pub disable_termination_for_finished_uploads: bool,
    /// Whether GET streams a finished upload back (`GetHandler`, always on in
    /// `@tus/server`). Off in Modlight: a finished upload is a media item and
    /// is served only by the media routes, with their privacy and view checks.
    pub serve_finished_uploads: bool,
}

/// `Server`
pub struct TusServer {
    pub datastore: Arc<dyn DataStore>,
    pub options: ServerOptions,
    hooks: Arc<dyn TusHooks>,
    locker: Arc<MemoryLocker>,
}

/// A request's abort state (`createContext()`): `abort()` now, or `cancel()`
/// = abort after the lock drain timeout.
#[derive(Clone)]
pub struct Context {
    token: CancellationToken,
    delayed: Arc<AtomicBool>,
    drain: Duration,
}

impl Context {
    fn new(drain: Duration) -> Context {
        Context { token: CancellationToken::new(), delayed: Arc::new(AtomicBool::new(false)), drain }
    }

    pub fn aborted(&self) -> bool {
        self.token.is_cancelled()
    }

    pub fn abort(&self) {
        self.token.cancel();
    }

    /// Initiates the delayed abort sequence unless it is already in progress.
    pub fn cancel(&self) {
        if !self.delayed.swap(true, Ordering::SeqCst) {
            let token = self.token.clone();
            let drain = self.drain;
            tokio::spawn(async move {
                tokio::time::sleep(drain).await;
                token.cancel();
            });
        }
    }
}

// ── MemoryLocker ────────────────────────────────────────────────────────────

struct LockEntry {
    holder: u64,
    request_release: Context,
}

/// `MemoryLocker`: one lock per upload id; a request waiting for a lock asks
/// the holder to release it (the holder's context is cancelled, i.e. aborted
/// after the drain timeout), then polls until it is free or 30 s passed.
pub struct MemoryLocker {
    locks: Mutex<HashMap<String, LockEntry>>,
    next: AtomicU64,
    timeout: Duration,
}

pub struct Lock {
    locker: Arc<MemoryLocker>,
    id: String,
    holder: u64,
}

impl MemoryLocker {
    pub fn new(timeout: Duration) -> MemoryLocker {
        MemoryLocker { locks: Mutex::new(HashMap::new()), next: AtomicU64::new(1), timeout }
    }

    async fn lock(self: &Arc<Self>, id: &str, ctx: &Context) -> Result<Lock, StoreError> {
        let holder = self.next.fetch_add(1, Ordering::SeqCst);
        let deadline = tokio::time::Instant::now() + self.timeout;
        loop {
            {
                let mut locks = self.locks.lock().unwrap_or_else(|e| e.into_inner());
                match locks.get(id) {
                    None => {
                        locks.insert(id.to_string(), LockEntry { holder, request_release: ctx.clone() });
                        return Ok(Lock { locker: self.clone(), id: id.to_string(), holder });
                    }
                    Some(entry) => entry.request_release.cancel(),
                }
            }
            if ctx.aborted() || tokio::time::Instant::now() >= deadline {
                return Err(errors::ERR_LOCK_TIMEOUT.into());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

impl Lock {
    fn unlock(self) {
        let mut locks = self.locker.locks.lock().unwrap_or_else(|e| e.into_inner());
        if locks.get(&self.id).is_some_and(|e| e.holder == self.holder) {
            locks.remove(&self.id);
        }
    }
}

// ── Validators ──────────────────────────────────────────────────────────────

/// `Number(value)` is an integer, `String(n) === value` and `n >= 0`.
fn is_canonical_uint(value: &str) -> bool {
    let n = crate::config::js_number(value);
    n.is_finite() && n.fract() == 0.0 && n >= 0.0 && crate::i18n::format::js_number_to_string(n) == value
}

/// `validateHeader(name, value)`
pub fn validate_header(name: &str, value: &str) -> bool {
    match name {
        "upload-offset" | "upload-length" => is_canonical_uint(value),
        "upload-defer-length" => value == "1",
        "upload-metadata" => parse_metadata(value).is_ok(),
        "x-forwarded-proto" => value == "http" || value == "https",
        "tus-version" => value == "1.0.0",
        "tus-resumable" => value == TUS_RESUMABLE,
        "content-type" => value == "application/offset+octet-stream",
        "upload-concat" => !value.is_empty() && (value == "partial" || value.starts_with("final;")),
        _ => true,
    }
}

/// `Number.parseInt(value, 10)`: leading whitespace, sign, digits; NaN when
/// there are none.
pub fn parse_int(value: &str) -> f64 {
    let t = crate::config::js_trim(value);
    let (neg, rest) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return f64::NAN;
    }
    let n: f64 = digits.parse().unwrap_or(f64::NAN);
    if neg { -n } else { n }
}

// ── Responses ───────────────────────────────────────────────────────────────

/// A response under construction: headers in a fetch `Headers`-like list.
#[derive(Debug, Clone, Default)]
pub struct Headers(Vec<(String, String)>);

impl Headers {
    pub fn set(&mut self, name: &str, value: impl Into<String>) {
        let name = name.to_ascii_lowercase();
        let value = value.into();
        // fetch `Headers` normalises values: leading/trailing whitespace goes.
        let value = value.trim_matches(|c| c == ' ' || c == '\t' || c == '\r' || c == '\n').to_string();
        match self.0.iter_mut().find(|(k, _)| *k == name) {
            Some(entry) => entry.1 = value,
            None => self.0.push((name, value)),
        }
    }

    pub fn has(&self, name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        self.0.iter().any(|(k, _)| *k == name)
    }
}

/// `new Response(body, { status, headers })`: a `Some` body is a string (and
/// gets the text content type), `None` is a null body.
fn response(status: u16, mut headers: Headers, body: Option<String>, stream: Option<ByteStream>) -> Response {
    if body.is_some() && !headers.has("content-type") {
        headers.set("content-type", "text/plain;charset=UTF-8");
    }
    // Iteration order of a `Headers` object: sorted by name.
    headers.0.sort_by(|a, b| a.0.cmp(&b.0));
    let mut res = match stream {
        Some(s) => Body::from_stream(s).into_response(),
        None => body.unwrap_or_default().into_response(),
    };
    *res.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let map = res.headers_mut();
    map.remove(axum::http::header::CONTENT_TYPE);
    for (k, v) in headers.0 {
        if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(k.as_bytes()), HeaderValue::from_str(&v)) {
            map.append(name, value);
        }
    }
    res
}

/// `BaseHandler.write(status, headers, body)`: Content-Length only for a
/// non-empty body, no body at all for a 204.
fn base_write(status: u16, mut headers: Headers, body: &str) -> Response {
    if status != 204 && !body.is_empty() {
        headers.set("Content-Length", body.len().to_string());
    }
    response(status, headers, if status == 204 { None } else { Some(body.to_string()) }, None)
}

/// `Server.write(context, headers, status, body)`
fn server_write(ctx: &Context, mut headers: Headers, status: u16, body: &str) -> Response {
    if status != 204 {
        headers.set("Content-Length", body.len().to_string());
    }
    if ctx.aborted() {
        headers.set("Connection", "close");
    }
    response(status, headers, Some(body.to_string()), None)
}

/// `new Date(ms).toUTCString()`
fn utc_string(ms: i64) -> String {
    crate::db::types::JsDate::from_millis(ms).map(|d| crate::http::set_cookie::http_date(d.0)).unwrap_or_default()
}

// ── Server ──────────────────────────────────────────────────────────────────

impl TusServer {
    pub fn new(datastore: Arc<dyn DataStore>, options: ServerOptions, hooks: Arc<dyn TusHooks>) -> TusServer {
        let locker = Arc::new(MemoryLocker::new(options.lock_timeout));
        TusServer { datastore, options, hooks, locker }
    }

    /// `server.handle(req, res)`
    pub async fn handle(&self, req: Request<Body>) -> Response {
        let (parts, body) = req.into_parts();
        let host = parts.headers.get(axum::http::header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("localhost");
        let path_and_query = parts.uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
        let treq = TusRequest {
            method: parts.method.clone(),
            url: format!("http://{host}{path_and_query}"),
            headers: parts.headers,
        };
        let ctx = Context::new(self.options.lock_drain_timeout);
        let mut headers = Headers::default();

        if treq.method == Method::GET {
            // The error page is written before the context is aborted.
            let res = match self.get(&treq, &ctx, headers.clone()).await {
                Ok(res) => res,
                Err(e) => self.on_error(&ctx, headers, e),
            };
            ctx.abort();
            return res;
        }

        // Tus-Resumable on every request and response except OPTIONS.
        headers.set("Tus-Resumable", TUS_RESUMABLE);
        if treq.method != Method::OPTIONS && treq.header("tus-resumable").is_none() {
            return server_write(&ctx, headers, 412, "Tus-Resumable Required\n");
        }

        // Validate all required headers (content type only on PATCH).
        if treq.method != Method::OPTIONS {
            let mut names: Vec<String> = treq.headers.keys().map(|k| k.as_str().to_string()).collect();
            names.sort();
            names.dedup();
            let invalid: Vec<String> = names
                .into_iter()
                .filter(|n| !(n == "content-type" && treq.method != Method::PATCH))
                .filter(|n| !validate_header(n, &treq.header(n).unwrap_or_default()))
                .collect();
            if !invalid.is_empty() {
                return server_write(&ctx, headers, 400, &format!("Invalid {}\n", invalid.join(" ")));
            }
        }

        // CORS
        let origin = treq.header("origin").filter(|o| !o.is_empty());
        headers.set("Access-Control-Allow-Origin", origin.unwrap_or_else(|| "*".to_string()));
        // `[...HEADERS, exposedHeaders ?? []].join(", ")`: the empty array
        // joins in as a trailing ", ", trimmed to "," by Headers.
        headers.set("Access-Control-Expose-Headers", format!("{}, ", HEADERS.join(", ")));

        let result = match treq.method {
            Method::OPTIONS => self.options_handler(&treq, headers.clone()).await,
            Method::POST => self.post(&treq, body, &ctx, headers.clone()).await,
            Method::HEAD => self.head(&treq, &ctx, headers.clone()).await,
            Method::PATCH => self.patch(&treq, body, &ctx, headers.clone()).await,
            Method::DELETE => self.delete(&treq, &ctx, headers.clone()).await,
            _ => return server_write(&ctx, headers, 404, "Not found\n"),
        };
        match result {
            Ok(mut res) => {
                if ctx.aborted() {
                    res.headers_mut().insert("connection", HeaderValue::from_static("close"));
                }
                res
            }
            Err(e) => self.on_error(&ctx, headers, e),
        }
    }

    fn on_error(&self, ctx: &Context, headers: Headers, err: StoreError) -> Response {
        if let StoreError::Other(e) = &err {
            tracing::error!("[tus] handler error: {e:#}");
        }
        let (status, body) = err.status_and_body();
        server_write(ctx, headers, status, &body)
    }

    // ── BaseHandler ──

    /// `getFileIdFromRequest(req)`: the last path segment of the URL, unless
    /// it is part of the server path.
    fn file_id(&self, req: &TusRequest) -> Option<String> {
        let url = req.url.strip_suffix('/').unwrap_or(&req.url);
        let last = url.rsplit('/').next().filter(|s| !s.is_empty())?;
        if self.options.path.contains(last) {
            return None;
        }
        crate::http::cookies::decode_uri_component(last)
    }

    /// `generateUrl(req, id)`
    fn generate_url(&self, req: &TusRequest, id: &str) -> String {
        let path = if self.options.path == "/" { "" } else { &self.options.path };
        let (proto, host) = self.extract_host_and_proto(req);
        format!("{proto}://{host}{path}/{id}")
    }

    fn extract_host_and_proto(&self, req: &TusRequest) -> (String, String) {
        let mut proto: Option<String> = None;
        let mut host: Option<String> = None;
        if self.options.respect_forwarded_headers {
            if let Some(forwarded) = req.header("forwarded") {
                let re_host = regex::Regex::new(r#"host="?([^";]+)"#).expect("static regex");
                let re_proto = regex::Regex::new(r"proto=(https?)").expect("static regex");
                host = re_host.captures(&forwarded).map(|c| c[1].to_string());
                proto = re_proto.captures(&forwarded).map(|c| c[1].to_string());
            }
            if proto.is_none()
                && let Some(p) = req.header("x-forwarded-proto").filter(|p| p == "http" || p == "https")
            {
                proto = Some(p);
            }
            if host.is_none() {
                host = req.header("x-forwarded-host");
            }
        }
        let host = host.or_else(|| req.header("host")).unwrap_or_default();
        (proto.unwrap_or_else(|| "http".to_string()), host)
    }

    async fn max_size(&self, req: &TusRequest) -> u64 {
        self.hooks.max_size(req).await
    }

    /// `calculateMaxBodySize(req, file, configuredMaxSize)`
    fn max_body_size(req: &TusRequest, file: &Upload, configured: u64) -> Result<u64, StoreError> {
        let content_length = req.header("content-length");
        let length = parse_int(content_length.as_deref().unwrap_or("0"));
        let length = if length.is_nan() { 0.0 } else { length };
        let offset = file.offset as f64;
        if file.size_is_deferred() {
            if content_length.is_some() && configured > 0 && offset + length > configured as f64 {
                return Err(errors::ERR_SIZE_EXCEEDED.into());
            }
            if configured > 0 {
                return Ok(configured.saturating_sub(file.offset));
            }
            return Ok(9_007_199_254_740_991);
        }
        let size = file.size.unwrap_or(0) as f64;
        if offset + length > size {
            return Err(errors::ERR_SIZE_EXCEEDED.into());
        }
        if content_length.is_some() {
            return Ok(length.max(0.0) as u64);
        }
        Ok((size - offset).max(0.0) as u64)
    }

    /// `writeToStore(body, upload, maxFileSize, context)`: the request body,
    /// ended gracefully when the client goes away or the context is aborted,
    /// through a `StreamLimiter`.
    async fn write_to_store(&self, body: Body, upload: &Upload, max: u64, ctx: &Context) -> Result<u64, StoreError> {
        if ctx.aborted() {
            return Err(errors::ABORTED.into());
        }
        let exceeded = Arc::new(AtomicBool::new(false));
        let flag = exceeded.clone();
        let token = ctx.token.clone();
        let mut total: u64 = 0;
        let mut data = body.into_data_stream();
        let stream = async_stream_from(move |tx| async move {
            loop {
                let next = tokio::select! {
                    _ = token.cancelled() => None,
                    chunk = data.next() => chunk,
                };
                let Some(chunk) = next else {
                    break;
                };
                // A dropped connection ends the body; what arrived is kept.
                let Ok(chunk) = chunk else {
                    break;
                };
                total += chunk.len() as u64;
                if total > max {
                    flag.store(true, Ordering::SeqCst);
                    let _ = tx.send(Err(std::io::Error::other("Maximum size exceeded"))).await;
                    break;
                }
                if tx.send(Ok(chunk)).await.is_err() {
                    break;
                }
            }
        });
        let result = self.datastore.write(stream, &upload.id, upload.offset).await;
        if exceeded.load(Ordering::SeqCst) {
            return Err(errors::ERR_MAX_SIZE_EXCEEDED.into());
        }
        result
    }

    fn expiration_on(&self) -> bool {
        self.datastore.has_extension(EXT_EXPIRATION) && self.datastore.get_expiration() > 0
    }

    // ── Handlers ──

    async fn options_handler(&self, req: &TusRequest, mut headers: Headers) -> Result<Response, StoreError> {
        let max = self.max_size(req).await;
        headers.set("Tus-Version", "1.0.0");
        if !self.datastore.extensions().is_empty() {
            headers.set("Tus-Extension", self.datastore.extensions().join(","));
        }
        if max > 0 {
            headers.set("Tus-Max-Size", max.to_string());
        }
        headers.set("Access-Control-Allow-Methods", ALLOWED_METHODS);
        headers.set("Access-Control-Allow-Headers", HEADERS.join(", "));
        headers.set("Access-Control-Max-Age", MAX_AGE.to_string());
        Ok(base_write(204, headers, ""))
    }

    async fn post(
        &self,
        req: &TusRequest,
        body: Body,
        ctx: &Context,
        headers: Headers,
    ) -> Result<Response, StoreError> {
        // The extension name is misspelt upstream, so concatenation is never on.
        if req.header("upload-concat").is_some() {
            return Err(errors::UNSUPPORTED_CONCATENATION_EXTENSION.into());
        }
        let upload_length = req.header("upload-length");
        let upload_defer_length = req.header("upload-defer-length");
        let upload_metadata = req.header("upload-metadata");
        if upload_defer_length.is_some() && !self.datastore.has_extension(EXT_CREATION_DEFER_LENGTH) {
            return Err(errors::UNSUPPORTED_CREATION_DEFER_LENGTH_EXTENSION.into());
        }
        if upload_length.is_none() == upload_defer_length.is_none() {
            return Err(errors::INVALID_LENGTH.into());
        }
        let metadata = match upload_metadata.as_deref().filter(|m| !m.is_empty()) {
            Some(m) => Some(parse_metadata(m).map_err(|_| StoreError::Tus(errors::INVALID_METADATA))?),
            None => None,
        };
        // Uid.rand(): 16 random bytes, hex.
        let id = {
            use rand::RngCore;
            let mut b = [0u8; 16];
            rand::rngs::OsRng.fill_bytes(&mut b);
            hex::encode(b)
        };
        let max = self.max_size(req).await;
        let declared = upload_length.as_deref().map(parse_int);
        if let Some(n) = declared
            && max > 0
            && n > max as f64
        {
            return Err(errors::ERR_MAX_SIZE_EXCEEDED.into());
        }
        let size = declared.filter(|n| n.is_finite()).map(|n| n.max(0.0) as u64);
        let mut upload = Upload::new(id.clone(), size, 0, metadata);
        if let Some(meta) = self.hooks.on_upload_create(req, &upload).await? {
            upload.metadata = Some(meta);
        }

        let lock = self.locker.lock(&id, ctx).await?;
        let mut is_final = false;
        let mut response_headers = headers.clone();
        let mut status: u16 = 201;
        let mut response_body = String::new();
        let mut url = String::new();
        let outcome: Result<(), StoreError> = async {
            let created = self.datastore.create(upload.clone()).await?;
            upload.storage = created.storage;
            url = self.generate_url(req, &id);
            is_final = upload.size == Some(0);
            if validate_header("content-type", &req.header("content-type").unwrap_or_default())
                && req.header("content-type").is_some()
            {
                let body_max = Self::max_body_size(req, &upload, max)?;
                let new_offset = self.write_to_store(body, &upload, body_max, ctx).await?;
                response_headers.set("Upload-Offset", new_offset.to_string());
                is_final = declared.is_some_and(|d| d == new_offset as f64);
                upload.offset = new_offset;
            }
            Ok(())
        }
        .await;
        lock.unlock();
        if let Err(e) = outcome {
            ctx.abort();
            return Err(e);
        }

        if is_final {
            let patch = self.hooks.on_upload_finish(req, &upload).await?;
            if let Some(s) = patch.status_code {
                status = s;
            }
            if let Some(b) = patch.body {
                response_body = b;
            }
            // Object.assign(patch.headers, responseData.headers): ours win.
            let mut merged = Headers::default();
            for (k, v) in patch.headers {
                merged.set(&k, v);
            }
            for (k, v) in response_headers.0 {
                merged.set(&k, v);
            }
            response_headers = merged;
        }

        if self.expiration_on() && upload.creation_date.is_some() {
            let created = self.datastore.get_upload(&upload.id).await?;
            if declared.is_none_or(|d| created.offset as f64 != d)
                && let Some(ms) = upload.creation_millis()
            {
                response_headers.set("Upload-Expires", utc_string(ms + self.datastore.get_expiration() as i64));
            }
        }
        if status == 201 || (300..400).contains(&status) {
            response_headers.set("Location", url);
        }
        Ok(base_write(status, response_headers, &response_body))
    }

    async fn head(&self, req: &TusRequest, ctx: &Context, mut headers: Headers) -> Result<Response, StoreError> {
        let id = self.file_id(req).ok_or(StoreError::Tus(errors::FILE_NOT_FOUND))?;
        let lock = self.locker.lock(&id, ctx).await?;
        let file = self.datastore.get_upload(&id).await;
        lock.unlock();
        let file = file?;
        if self.expiration_on()
            && let Some(created) = file.creation_millis()
            && crate::db::types::JsDate::now().millis() > created + self.datastore.get_expiration() as i64
        {
            return Err(errors::FILE_NO_LONGER_EXISTS.into());
        }
        headers.set("Cache-Control", "no-store");
        headers.set("Upload-Offset", file.offset.to_string());
        match file.size {
            None => headers.set("Upload-Defer-Length", "1"),
            Some(size) => headers.set("Upload-Length", size.to_string()),
        }
        if let Some(meta) = &file.metadata {
            headers.set("Upload-Metadata", stringify_metadata(meta));
        }
        Ok(response(200, headers, Some(String::new()), None))
    }

    async fn patch(
        &self,
        req: &TusRequest,
        body: Body,
        ctx: &Context,
        headers: Headers,
    ) -> Result<Response, StoreError> {
        let result = self.patch_inner(req, body, ctx, headers).await;
        if result.is_err() && !ctx.aborted() {
            ctx.abort();
        }
        result
    }

    async fn patch_inner(
        &self,
        req: &TusRequest,
        body: Body,
        ctx: &Context,
        headers: Headers,
    ) -> Result<Response, StoreError> {
        let id = self.file_id(req).ok_or(StoreError::Tus(errors::FILE_NOT_FOUND))?;
        let Some(offset_header) = req.header("upload-offset") else {
            return Err(errors::MISSING_OFFSET.into());
        };
        let offset = parse_int(&offset_header);
        if req.header("content-type").is_none() {
            return Err(errors::INVALID_CONTENT_TYPE.into());
        }
        let max = self.max_size(req).await;
        let lock = self.locker.lock(&id, ctx).await?;
        let outcome: Result<(Upload, u64), StoreError> = async {
            let mut upload = self.datastore.get_upload(&id).await?;
            let now = crate::db::types::JsDate::now().millis();
            let creation = upload.creation_millis().unwrap_or(now);
            if self.expiration_on() && now > creation + self.datastore.get_expiration() as i64 {
                return Err(errors::FILE_NO_LONGER_EXISTS.into());
            }
            if upload.offset as f64 != offset {
                return Err(errors::INVALID_OFFSET.into());
            }
            if let Some(length) = req.header("upload-length") {
                let size = parse_int(&length);
                if !self.datastore.has_extension(EXT_CREATION_DEFER_LENGTH) {
                    return Err(errors::UNSUPPORTED_CREATION_DEFER_LENGTH_EXTENSION.into());
                }
                if upload.size.is_some() || size < upload.offset as f64 {
                    return Err(errors::INVALID_LENGTH.into());
                }
                if max > 0 && size > max as f64 {
                    return Err(errors::ERR_MAX_SIZE_EXCEEDED.into());
                }
                let size = size as u64;
                self.datastore.declare_upload_length(&id, size).await?;
                upload.size = Some(size);
            }
            let max_body = Self::max_body_size(req, &upload, max)?;
            let new_offset = self.write_to_store(body, &upload, max_body, ctx).await?;
            Ok((upload, new_offset))
        }
        .await;
        lock.unlock();
        let (mut upload, new_offset) = outcome?;
        upload.offset = new_offset;

        let mut status: u16 = 204;
        let mut response_body = String::new();
        let mut response_headers = headers;
        response_headers.set("Upload-Offset", new_offset.to_string());
        if Some(new_offset) == upload.size {
            let patch = self.hooks.on_upload_finish(req, &upload).await?;
            if let Some(s) = patch.status_code {
                status = s;
            }
            if let Some(b) = patch.body {
                response_body = b;
            }
            // Object.assign(responseData.headers, headers): the hook's win.
            for (k, v) in patch.headers {
                response_headers.set(&k, v);
            }
        }
        if self.expiration_on()
            && let Some(ms) = upload.creation_millis()
            && upload.size.is_none_or(|s| new_offset < s)
        {
            response_headers.set("Upload-Expires", utc_string(ms + self.datastore.get_expiration() as i64));
        }
        Ok(base_write(status, response_headers, &response_body))
    }

    async fn delete(&self, req: &TusRequest, ctx: &Context, headers: Headers) -> Result<Response, StoreError> {
        let id = self.file_id(req).ok_or(StoreError::Tus(errors::FILE_NOT_FOUND))?;
        let lock = self.locker.lock(&id, ctx).await?;
        let removed = async {
            if self.options.disable_termination_for_finished_uploads {
                let upload = self.datastore.get_upload(&id).await?;
                if Some(upload.offset) == upload.size {
                    return Err(StoreError::Tus(errors::INVALID_TERMINATION));
                }
            }
            self.datastore.remove(&id).await
        }
        .await;
        lock.unlock();
        removed?;
        Ok(base_write(204, headers, ""))
    }

    /// `GetHandler.send`: the bytes of a FINISHED upload still in the store,
    /// with a content type from its metadata (an inline allowlist).
    async fn get(&self, req: &TusRequest, ctx: &Context, mut headers: Headers) -> Result<Response, StoreError> {
        if !self.options.serve_finished_uploads || !self.datastore.supports_read() {
            return Err(errors::FILE_NOT_FOUND.into());
        }
        let id = self.file_id(req).ok_or(StoreError::Tus(errors::FILE_NOT_FOUND))?;
        let stats = self.datastore.get_upload(&id).await?;
        if Some(stats.offset) != stats.size {
            return Err(errors::FILE_NOT_FOUND.into());
        }
        let (content_type, disposition) = filter_content_type(stats.metadata.as_ref());
        let lock = self.locker.lock(&id, ctx).await?;
        let stream = self.datastore.read(&id).await;
        lock.unlock();
        let stream = stream?;
        headers.set("Content-Length", stats.offset.to_string());
        headers.set("Content-Type", content_type);
        headers.set("Content-Disposition", disposition);
        Ok(response(200, headers, None, Some(stream)))
    }
}

/// `GetHandler.filterContentType(stats)`
fn filter_content_type(metadata: Option<&Metadata>) -> (String, String) {
    const INLINE: &[&str] = &[
        "text/plain",
        "image/png",
        "image/jpeg",
        "image/gif",
        "image/bmp",
        "image/webp",
        "audio/wave",
        "audio/wav",
        "audio/x-wav",
        "audio/x-pn-wav",
        "audio/webm",
        "audio/ogg",
        "video/mp4",
        "video/webm",
        "video/ogg",
        "application/ogg",
    ];
    let re = regex::Regex::new(
        r#"^(?:application|audio|example|font|haptics|image|message|model|multipart|text|video|x-(?:[0-9A-Za-z!#$%&'*+.^_`|~-]+))/([0-9A-Za-z!#$%&'*+.^_`|~-]+)((?:[ \t]*;[ \t]*[0-9A-Za-z!#$%&'*+.^_`|~-]+=(?:[0-9A-Za-z!#$%&'*+.^_`|~-]+|"(?:[^"\\]|\.)*"))*)$"#,
    )
    .expect("static regex");
    let get = |k: &str| metadata.and_then(|m| m.get(k)).and_then(|v| v.clone()).filter(|v| !v.is_empty());
    let (mut content_type, mut disposition) = ("application/octet-stream".to_string(), "attachment".to_string());
    if let Some(filetype) = get("filetype")
        && re.is_match(&filetype)
    {
        disposition = if INLINE.contains(&filetype.as_str()) { "inline" } else { "attachment" }.to_string();
        content_type = filetype;
    }
    if let Some(filename) = get("filename") {
        disposition.push_str(&format!("; filename=\"{}\"", filename.replace('"', "\\\"")));
    }
    (content_type, disposition)
}

/// A `ByteStream` fed by a task (bounded channel, so the body is pulled only
/// as fast as the store writes it).
fn async_stream_from<F, Fut>(f: F) -> ByteStream
where
    F: FnOnce(tokio::sync::mpsc::Sender<std::io::Result<Bytes>>) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (tx, rx) = tokio::sync::mpsc::channel::<std::io::Result<Bytes>>(4);
    tokio::spawn(f(tx));
    Box::pin(tokio_stream_from_receiver(rx))
}

fn tokio_stream_from_receiver(
    mut rx: tokio::sync::mpsc::Receiver<std::io::Result<Bytes>>,
) -> impl futures_util::Stream<Item = std::io::Result<Bytes>> + Send {
    futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx))
}

#[allow(dead_code)]
fn tus_error(status: u16, body: &str) -> StoreError {
    StoreError::Tus(TusError::new(status, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validators_like_tus() {
        assert!(validate_header("upload-offset", "0"));
        assert!(validate_header("upload-offset", "123"));
        assert!(!validate_header("upload-offset", "01"));
        assert!(!validate_header("upload-offset", "-1"));
        assert!(!validate_header("upload-offset", "1.5"));
        assert!(!validate_header("upload-length", " 5"));
        assert!(validate_header("upload-defer-length", "1"));
        assert!(!validate_header("upload-defer-length", "0"));
        assert!(validate_header("tus-resumable", "1.0.0"));
        assert!(!validate_header("tus-resumable", "0.2"));
        assert!(validate_header("content-type", "application/offset+octet-stream"));
        assert!(!validate_header("content-type", "application/octet-stream"));
        assert!(validate_header("x-anything", "whatever"));
        assert!(!validate_header("upload-metadata", ""));
    }

    #[test]
    fn parse_int_like_js() {
        assert_eq!(parse_int("12abc"), 12.0);
        assert!(parse_int("abc").is_nan());
        assert_eq!(parse_int(" 7"), 7.0);
        assert_eq!(parse_int("-3"), -3.0);
    }

    struct NoHooks;

    impl TusHooks for NoHooks {
        fn max_size<'a>(&'a self, _req: &'a TusRequest) -> BoxFuture<'a, u64> {
            Box::pin(async { 0 })
        }
        fn on_upload_create<'a>(
            &'a self,
            _req: &'a TusRequest,
            _upload: &'a Upload,
        ) -> BoxFuture<'a, Result<Option<Metadata>, StoreError>> {
            Box::pin(async { Ok(None) })
        }
        fn on_upload_finish<'a>(
            &'a self,
            _req: &'a TusRequest,
            _upload: &'a Upload,
        ) -> BoxFuture<'a, Result<FinishPatch, StoreError>> {
            Box::pin(async { Ok(FinishPatch { status_code: None, body: None, headers: vec![] }) })
        }
    }

    fn server(dir: &std::path::Path, modlight: bool) -> TusServer {
        let store = super::super::file_store::FileStore::new(dir.to_str().unwrap(), 0);
        TusServer::new(
            Arc::new(store),
            ServerOptions {
                path: "/api/files".into(),
                respect_forwarded_headers: false,
                lock_drain_timeout: Duration::from_millis(100),
                lock_timeout: Duration::from_secs(5),
                disable_termination_for_finished_uploads: modlight,
                serve_finished_uploads: !modlight,
            },
            Arc::new(NoHooks),
        )
    }

    async fn send(server: &TusServer, req: Request<Body>) -> (u16, String) {
        let res = server.handle(req).await;
        let status = res.status().as_u16();
        let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    /// Creates an upload of `len` bytes, writes `written` of them; its path.
    async fn upload(server: &TusServer, len: usize, written: usize) -> String {
        let created = server
            .handle(
                Request::post("/api/files")
                    .header("host", "127.0.0.1")
                    .header("tus-resumable", "1.0.0")
                    .header("upload-length", len.to_string())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(created.status().as_u16(), 201);
        let location = created.headers().get("location").unwrap().to_str().unwrap().to_string();
        let path = url::Url::parse(&location).unwrap().path().to_string();
        if written > 0 {
            let res = server
                .handle(
                    Request::patch(path.as_str())
                        .header("tus-resumable", "1.0.0")
                        .header("upload-offset", "0")
                        .header("content-type", "application/offset+octet-stream")
                        .body(Body::from(vec![b'x'; written]))
                        .unwrap(),
                )
                .await;
            assert_eq!(res.status().as_u16(), 204);
        }
        path
    }

    /// Security fix 7a (SKELETON.md 9): a finished upload is never streamed back
    /// at its tus URL nor terminated there; an unfinished one still resumes and
    /// can still be terminated. Upstream's defaults are kept for comparison.
    #[tokio::test]
    async fn finished_uploads_are_not_served_or_terminated() {
        let dir = tempfile::tempdir().unwrap();
        let upstream = server(dir.path(), false);
        let done = upload(&upstream, 5, 5).await;
        assert_eq!(
            send(&upstream, Request::get(done.as_str()).body(Body::empty()).unwrap()).await,
            (200, "xxxxx".into())
        );

        let modlight = server(dir.path(), true);
        assert_eq!(
            send(&modlight, Request::get(done.as_str()).body(Body::empty()).unwrap()).await,
            (404, "The file for this url was not found\n".into())
        );
        let delete = || Request::delete(done.as_str()).header("tus-resumable", "1.0.0").body(Body::empty()).unwrap();
        assert_eq!(send(&modlight, delete()).await, (400, "Cannot terminate an already completed upload".into()));
        // Still there: the refusal removed nothing.
        let head = Request::head(done.as_str()).header("tus-resumable", "1.0.0").body(Body::empty()).unwrap();
        assert_eq!(send(&modlight, head).await.0, 200);

        // Unfinished: HEAD reports the offset to resume from, PATCH resumes,
        // GET serves nothing, DELETE terminates.
        let partial = upload(&modlight, 10, 4).await;
        let head = modlight
            .handle(Request::head(partial.as_str()).header("tus-resumable", "1.0.0").body(Body::empty()).unwrap())
            .await;
        assert_eq!(head.headers().get("upload-offset").unwrap(), "4");
        assert_eq!(send(&modlight, Request::get(partial.as_str()).body(Body::empty()).unwrap()).await.0, 404);
        let resume = Request::patch(partial.as_str())
            .header("tus-resumable", "1.0.0")
            .header("upload-offset", "4")
            .header("content-type", "application/offset+octet-stream")
            .body(Body::from(vec![b'y'; 3]))
            .unwrap();
        assert_eq!(send(&modlight, resume).await.0, 204);
        let terminate = Request::delete(partial.as_str()).header("tus-resumable", "1.0.0").body(Body::empty()).unwrap();
        assert_eq!(send(&modlight, terminate).await.0, 204);
        let head = Request::head(partial.as_str()).header("tus-resumable", "1.0.0").body(Body::empty()).unwrap();
        assert_eq!(send(&modlight, head).await.0, 404);
    }

    #[test]
    fn content_types_for_get() {
        let mut m = Metadata::new();
        m.insert("filetype".into(), Some("image/png".into()));
        m.insert("filename".into(), Some("a\"b.png".into()));
        assert_eq!(filter_content_type(Some(&m)), ("image/png".into(), "inline; filename=\"a\\\"b.png\"".into()));
        let mut m = Metadata::new();
        m.insert("filetype".into(), Some("text/html".into()));
        assert_eq!(filter_content_type(Some(&m)).1, "attachment");
        assert_eq!(filter_content_type(None), ("application/octet-stream".into(), "attachment".into()));
    }
}
