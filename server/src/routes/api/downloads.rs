//! Not upstream: the HTTP side of Downloads (`crate::downloads`;
//! docs/SKELETON.md section 9, item 45; README "Downloads").
//!
//! | Route | Who | Answer |
//! |---|---|---|
//! | `GET /api/downloads` | signed in | `{platforms: [{platform, builds}], keep, admin, publishToken}` |
//! | `GET /downloads/{id}/{filename}` | signed in | the file: attachment, `Content-Length`, `ETag` (the quoted sha256), `Range` |
//! | `POST /api/downloads/upload` | publisher | JSON body -> `{upload, chunkSize}` |
//! | `PUT /api/downloads/upload/{upload}/{n}` | publisher | raw bytes of chunk `n` (0-based) -> `{upload, chunk, size, received}` |
//! | `POST /api/downloads/upload/{upload}/finish` | publisher | -> the `Build`; a wrong size or sha256 drops the upload (400) |
//! | `DELETE /api/downloads/{id}` | administrator | -> `{id, deleted: true}` |
//!
//! "Signed in" is a session cookie or an API token (`Authorization: Bearer`
//! or `x-api-key`) of any scope: the desktop app checks for updates with the
//! token it uses for everything else, whatever its scope, and the list says
//! nothing about any account. A publisher is `RELEASE_PUBLISH_TOKEN` as a
//! bearer token, an administrator's session, or an administrator's `full` API
//! token. The publish token opens the three publish routes and nothing else:
//! anywhere else it is an API token nobody issued (401).
//!
//! Every refusal is JSON `{"error", "code"}` with `Cache-Control: private,
//! no-store`: 401 `unauthorized` / `account_disabled` without valid
//! credentials, 403 `forbidden` for an account that may not do it (403
//! `insufficient_scope` / `invalid_scope` for an administrator's token that may
//! not), 400 for a malformed request, 404 `not_found`, 409 `busy`, 413
//! `too_large`, 415 `unsupported_media_type`, 429 `too_many_uploads`, 507
//! `insufficient_storage`, 500 `internal`. The credential is checked before a
//! byte of a body is read.
//!
//! `/downloads/{id}/{filename}` has the shape of upstream's media actions
//! (`/{userCode}/{mediaCode}/{action}`): a path that names no build (an
//! account tagged `downloads` before the tag was reserved) goes on to that
//! route unchanged.

use std::net::SocketAddr;
use std::time::SystemTime;

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use serde_json::{Value, json};

use crate::api_json::{JSON_BODY_MAX, PRIVATE_NO_STORE, json_private, read_json_object};
use crate::api_token::{TokenAuth, TokenCapability, authenticate_api_token, bearer_from_request, scope_refusal_body};
use crate::auth_helpers::{RequestSession, Sessions};
use crate::db::models::{JsDate, Release, User};
use crate::downloads::{
    self, Assembled, Build, CHUNK_SIZE, ChunkError, FinishGuard, KEEP, MAX_PENDING_UPLOADS, Refusal, StagedUpload,
    TOKEN_UPLOADER,
};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/downloads", get(list))
        .route("/api/downloads/upload", post(upload_start))
        .route("/api/downloads/upload/{upload}/finish", post(upload_finish))
        .route("/api/downloads/upload/{upload}/{n}", put(upload_chunk))
        .route("/api/downloads/{id}", delete(remove))
        .route("/downloads/{id}/{filename}", get(download))
}

// ── Refusals ────────────────────────────────────────────────────────────────

const SIGN_IN: &str = "Sign in, or send an API token (Authorization: Bearer <token>).";
const SIGN_IN_TO_PUBLISH: &str =
    "Send the publish token (Authorization: Bearer <RELEASE_PUBLISH_TOKEN>) or sign in as an administrator.";
const NOT_ADMIN_PUBLISH: &str = "Only administrators and build machines with the publish token may publish builds.";
const NOT_ADMIN_DELETE: &str = "Only administrators may delete builds.";
const NO_UPLOAD: &str = "No such upload: it was finished, dropped or has expired. Start it again.";

fn unauthorized(message: &str) -> Refusal {
    Refusal::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
}

fn disabled() -> Refusal {
    Refusal::new(StatusCode::UNAUTHORIZED, "account_disabled", "This account is disabled.")
}

fn not_found(message: &str) -> Refusal {
    Refusal::new(StatusCode::NOT_FOUND, "not_found", message)
}

fn too_large(message: String) -> Refusal {
    Refusal::new(StatusCode::PAYLOAD_TOO_LARGE, "too_large", message)
}

/// A token that resolved to no usable caller.
fn token_refusal(auth: &TokenAuth, unknown: &str) -> Refusal {
    match auth {
        TokenAuth::Banned { .. } => disabled(),
        TokenAuth::InvalidScope => Refusal::new(
            StatusCode::FORBIDDEN,
            "invalid_scope",
            "This API token carries a scope this server does not recognise. Create a new token in Integrations.",
        ),
        TokenAuth::Scope { scope, required, .. } => Refusal::new(
            StatusCode::FORBIDDEN,
            "insufficient_scope",
            scope_refusal_body(*scope, *required)["message"].as_str().unwrap_or("Insufficient scope.").to_string(),
        ),
        TokenAuth::Ok { .. } | TokenAuth::Unknown => unauthorized(unknown),
    }
}

/// Anything unexpected: logged, answered in the same shape as a refusal.
fn respond(result: anyhow::Result<Response>) -> Response {
    result.unwrap_or_else(|err| {
        tracing::error!(error = ?err, "[downloads] request failed");
        Refusal::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "Something went wrong on the server; try again.")
            .into_response()
    })
}

// ── Who is calling ──────────────────────────────────────────────────────────

fn is_admin(user: &User) -> bool {
    user.role.as_deref() == Some("admin")
}

/// A signed-in account: an API token of any scope (it decides alone when one
/// is offered), else the session.
async fn reader(
    state: &AppState,
    session: &RequestSession,
    headers: &HeaderMap,
) -> anyhow::Result<Result<User, Refusal>> {
    if let Some(token) = bearer_from_request(headers) {
        return Ok(match authenticate_api_token(&state.db, Some(&token), TokenCapability::Identify).await? {
            TokenAuth::Ok { user, .. } => Ok(user),
            other => Err(token_refusal(&other, SIGN_IN)),
        });
    }
    Ok(match session.get().await? {
        Some(s) if s.user.banned == Some(true) => Err(disabled()),
        Some(s) => Ok(s.user),
        None => Err(unauthorized(SIGN_IN)),
    })
}

/// Who publishes (or, without the token, deletes).
enum Publisher {
    /// `RELEASE_PUBLISH_TOKEN`.
    Token,
    /// An administrator, by session or by a `full` API token.
    Admin { user: Box<User>, via: &'static str },
}

impl Publisher {
    /// The `uploader` a build records: the administrator's name (else tag).
    fn uploader(&self) -> String {
        match self {
            Publisher::Token => TOKEN_UPLOADER.to_string(),
            Publisher::Admin { user, .. } => {
                let name = crate::config::js_trim(&user.name);
                if !name.is_empty() {
                    name.to_string()
                } else {
                    user.user_code.clone().filter(|c| !c.is_empty()).unwrap_or_else(|| "admin".to_string())
                }
            }
        }
    }

    fn actor_id(&self) -> Option<&str> {
        match self {
            Publisher::Token => None,
            Publisher::Admin { user, .. } => Some(&user.id),
        }
    }

    /// Who, for an audit entry.
    fn by(&self) -> Value {
        match self {
            Publisher::Token => json!({ "by": TOKEN_UPLOADER, "via": "publish-token" }),
            Publisher::Admin { user, via } => json!({ "by": user.id, "byEmail": user.email, "via": via }),
        }
    }
}

/// The publish routes' gate (`with_token`), or the delete route's (without).
/// A bearer token decides alone when one is offered: the publish token, else
/// an administrator's API token with the `admin` capability.
async fn publisher(
    state: &AppState,
    session: &RequestSession,
    headers: &HeaderMap,
    with_token: bool,
) -> anyhow::Result<Result<Publisher, Refusal>> {
    let (sign_in, not_admin) =
        if with_token { (SIGN_IN_TO_PUBLISH, NOT_ADMIN_PUBLISH) } else { (SIGN_IN, NOT_ADMIN_DELETE) };
    let forbidden = || Refusal::new(StatusCode::FORBIDDEN, "forbidden", not_admin);
    if let Some(token) = bearer_from_request(headers) {
        if with_token && state.config.release_publish_token.matches(&token) {
            return Ok(Ok(Publisher::Token));
        }
        return Ok(match authenticate_api_token(&state.db, Some(&token), TokenCapability::Admin).await? {
            TokenAuth::Ok { user, .. } if is_admin(&user) => {
                Ok(Publisher::Admin { user: Box::new(user), via: "api-token" })
            }
            TokenAuth::Ok { .. } => Err(forbidden()),
            // A narrower token of a regular account is refused for the account, not the scope.
            TokenAuth::Scope { user, .. } if !is_admin(&user) => Err(forbidden()),
            other => Err(token_refusal(&other, sign_in)),
        });
    }
    Ok(match session.get().await? {
        Some(s) if s.user.banned == Some(true) => Err(disabled()),
        Some(s) if s.is_admin() => Ok(Publisher::Admin { user: Box::new(s.user), via: "session" }),
        Some(_) => Err(forbidden()),
        None => Err(unauthorized(sign_in)),
    })
}

/// The audit entry's address: `password::client_ip`, as everywhere.
fn client_ip(state: &AppState, parts: &Parts) -> String {
    let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
    crate::password::client_ip(&parts.headers, peer, state.config.trust_proxy)
}

/// What an audit entry says about a build.
fn subject(id: &str, platform: &str, arch: &str, version: &str, build: &str, filename: &str) -> Value {
    json!({ "id": id, "platform": platform, "arch": arch, "version": version, "build": build, "filename": filename })
}

fn release_subject(r: &Release) -> Value {
    subject(&r.id, &r.platform, &r.arch, &r.version, &r.build, &r.filename)
}

/// One `audit_log` entry: who, about what, and the action's own details.
async fn audit(state: &AppState, who: &Publisher, action: &str, about: Value, extra: Value, ip: &str) {
    let mut detail = who.by();
    if let Some(map) = detail.as_object_mut() {
        for part in [about, extra] {
            if let Value::Object(fields) = part {
                map.extend(fields);
            }
        }
    }
    crate::audit::record_audit_by(&state.db, who.actor_id(), action, detail, ip).await;
}

// ── Reading ─────────────────────────────────────────────────────────────────

async fn list(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> Response {
    respond(list_builds(&state, &session, req).await)
}

async fn list_builds(state: &AppState, session: &RequestSession, req: Request) -> anyhow::Result<Response> {
    let (parts, _) = req.into_parts();
    let user = match reader(state, session, &parts.headers).await? {
        Ok(user) => user,
        Err(refusal) => return Ok(refusal.into_response()),
    };
    let body = downloads::list(&state.db, is_admin(&user), state.config.release_publish_token.is_set()).await?;
    Ok(json_private(StatusCode::OK, &body))
}

async fn download(
    State(state): State<AppState>,
    Path((id, filename)): Path<(String, String)>,
    Sessions(session): Sessions,
    req: Request,
) -> Response {
    let release = match downloads::find_release(&state.db, &id).await {
        Ok(release) => release.filter(|r| r.filename == filename),
        Err(err) => return respond(Err(err.into())),
    };
    let Some(release) = release else {
        // Not a build: upstream's media action of an account tagged `downloads`.
        return crate::routes::media_action::handle(
            State(state),
            Path(("downloads".to_string(), id, filename)),
            Sessions(session),
            req,
        )
        .await
        .into_response();
    };
    respond(serve(&state, &session, release, req).await)
}

/// Whether an `If-None-Match` list names `etag` (weak or strong, or `*`).
fn none_match(header_value: &str, etag: &str) -> bool {
    header_value.split(',').map(str::trim).any(|t| t == "*" || t.strip_prefix("W/").unwrap_or(t) == etag)
}

fn value(s: &str) -> HeaderValue {
    HeaderValue::from_str(s).unwrap_or_else(|_| HeaderValue::from_static(""))
}

async fn serve(state: &AppState, session: &RequestSession, release: Release, req: Request) -> anyhow::Result<Response> {
    let (parts, _) = req.into_parts();
    if let Err(refusal) = reader(state, session, &parts.headers).await? {
        return Ok(refusal.into_response());
    }
    let storage = state.storage.get()?;
    let key = downloads::storage_key(&release.id, &release.filename);
    let Some(stat) = storage.stat(&key).await? else {
        tracing::warn!("[downloads] build {} has no file at {key}", release.id);
        return Ok(not_found("The file of this build is missing on the server.").into_response());
    };
    let size = stat.size;
    let etag = format!("\"{}\"", release.sha256);
    let header_str = |name: header::HeaderName| parts.headers.get(name).and_then(|v| v.to_str().ok());

    let mut res = if header_str(header::IF_NONE_MATCH).is_some_and(|v| none_match(v, &etag)) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        // A Range meant for another version of the file is no range at all.
        let range = match header_str(header::IF_RANGE) {
            Some(v) if v.trim() != etag => None,
            _ => crate::media::serve::parse_range(header_str(header::RANGE), size),
        };
        let stream = storage.read_stream(&key, range).await?;
        let mut res = Response::new(Body::from_stream(stream));
        let length = match range {
            Some(r) => {
                *res.status_mut() = StatusCode::PARTIAL_CONTENT;
                res.headers_mut().insert(header::CONTENT_RANGE, value(&format!("bytes {}-{}/{size}", r.start, r.end)));
                r.end - r.start + 1
            }
            None => size,
        };
        let h = res.headers_mut();
        h.insert(header::CONTENT_LENGTH, value(&length.to_string()));
        h.insert(header::CONTENT_TYPE, HeaderValue::from_static(downloads::content_type_for(&release.filename)));
        // The name is plain ASCII by construction (downloads::valid_filename).
        h.insert(header::CONTENT_DISPOSITION, value(&format!("attachment; filename=\"{}\"", release.filename)));
        h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
        res
    };
    let h = res.headers_mut();
    h.insert(header::ETAG, value(&etag));
    // Behind a sign-in: no shared cache may keep it (Cloudflare caches .zip by default).
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(PRIVATE_NO_STORE));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    Ok(res)
}

// ── Publishing ──────────────────────────────────────────────────────────────

/// The `code` of a body [`read_json_object`] refused.
fn body_code(status: StatusCode) -> &'static str {
    match status {
        StatusCode::UNSUPPORTED_MEDIA_TYPE => "unsupported_media_type",
        StatusCode::PAYLOAD_TOO_LARGE => "too_large",
        _ => "invalid_body",
    }
}

async fn upload_start(State(state): State<AppState>, Sessions(session): Sessions, req: Request) -> Response {
    respond(start_upload(&state, &session, req).await)
}

async fn start_upload(state: &AppState, session: &RequestSession, req: Request) -> anyhow::Result<Response> {
    let (parts, body) = req.into_parts();
    let who = match publisher(state, session, &parts.headers, true).await? {
        Ok(who) => who,
        Err(refusal) => return Ok(refusal.into_response()),
    };
    let input = match read_json_object(&parts.headers, body, JSON_BODY_MAX).await {
        Ok(input) => input,
        Err(e) => return Ok(Refusal::new(e.status, body_code(e.status), e.message).into_response()),
    };
    let start = match downloads::parse_start(&input, JsDate::now()) {
        Ok(start) => start,
        Err(refusal) => return Ok(refusal.into_response()),
    };
    let config = &state.config;
    let pending = downloads::sweep_staged_uploads(config, SystemTime::now()).await;
    if pending >= MAX_PENDING_UPLOADS {
        return Ok(Refusal::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_uploads",
            format!("{pending} uploads are unfinished; finish them, or wait a day until they expire."),
        )
        .into_response());
    }
    // Room for the chunks and for the joined copy `finish` writes beside them.
    let root = downloads::staging_root(config);
    tokio::fs::create_dir_all(&root).await?;
    let free = tokio::task::spawn_blocking(move || crate::upload::disk::filesystem_space_at(&root))
        .await?
        .map(|s| s.free_bytes as f64);
    if !crate::upload::disk::fits_on_disk(start.size as f64 * 2.0, free, config.uploads.min_free_bytes as f64) {
        return Ok(Refusal::new(
            StatusCode::INSUFFICIENT_STORAGE,
            "insufficient_storage",
            "The server does not have enough free disk space for this build.",
        )
        .into_response());
    }
    let upload = downloads::begin_upload(config, start, &who.uploader()).await?;
    Ok(json_private(StatusCode::OK, &json!({ "upload": upload, "chunkSize": CHUNK_SIZE })))
}

async fn upload_chunk(
    State(state): State<AppState>,
    Path((upload, n)): Path<(String, String)>,
    Sessions(session): Sessions,
    req: Request,
) -> Response {
    respond(receive_chunk(&state, &session, &upload, &n, req).await)
}

async fn receive_chunk(
    state: &AppState,
    session: &RequestSession,
    upload: &str,
    n: &str,
    req: Request,
) -> anyhow::Result<Response> {
    let (parts, body) = req.into_parts();
    if let Err(refusal) = publisher(state, session, &parts.headers, true).await? {
        return Ok(refusal.into_response());
    }
    let Some(index) = downloads::parse_chunk_index(n) else {
        let message = format!("Chunks are numbered 0, 1, 2, ... up to {}.", downloads::MAX_CHUNK_INDEX);
        return Ok(Refusal::bad("invalid_chunk", message).into_response());
    };
    let Some(staged) = downloads::staged_upload(&state.config, upload).await? else {
        return Ok(not_found(NO_UPLOAD).into_response());
    };
    let chunk_too_large = || too_large(format!("A chunk may be at most {CHUNK_SIZE} bytes (chunkSize)."));
    let declared = parts
        .headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    if declared.is_some_and(|len| len > CHUNK_SIZE) {
        return Ok(chunk_too_large().into_response());
    }
    let dir = downloads::upload_dir(&state.config, upload);
    let others: u64 =
        downloads::chunk_sizes(&dir).await?.into_iter().filter(|(i, _)| *i != index).map(|(_, size)| size).sum();
    let room = staged.request.size.saturating_sub(others);
    let over_declared = || {
        too_large(format!(
            "The chunks would add up to more than the {} bytes this upload declared.",
            staged.request.size
        ))
    };
    if room == 0 {
        return Ok(over_declared().into_response());
    }
    match downloads::write_chunk(&dir, index, body, room.min(CHUNK_SIZE)).await {
        Ok(size) => Ok(json_private(
            StatusCode::OK,
            &json!({ "upload": upload, "chunk": index, "size": size, "received": others + size }),
        )),
        Err(ChunkError::TooLarge) if room < CHUNK_SIZE => Ok(over_declared().into_response()),
        Err(ChunkError::TooLarge) => Ok(chunk_too_large().into_response()),
        Err(ChunkError::Empty) => Ok(Refusal::bad("empty_chunk", "A chunk carries at least one byte.").into_response()),
        Err(ChunkError::Body) => {
            Ok(Refusal::bad("invalid_body", "The chunk did not arrive whole; send it again.").into_response())
        }
        // The upload was finished, dropped or swept while this chunk streamed.
        Err(ChunkError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(not_found(NO_UPLOAD).into_response()),
        Err(ChunkError::Io(e)) => Err(e.into()),
    }
}

async fn upload_finish(
    State(state): State<AppState>,
    Path(upload): Path<String>,
    Sessions(session): Sessions,
    req: Request,
) -> Response {
    respond(finish_upload(&state, &session, &upload, req).await)
}

async fn finish_upload(
    state: &AppState,
    session: &RequestSession,
    upload: &str,
    req: Request,
) -> anyhow::Result<Response> {
    let (parts, _) = req.into_parts();
    let who = match publisher(state, session, &parts.headers, true).await? {
        Ok(who) => who,
        Err(refusal) => return Ok(refusal.into_response()),
    };
    if !downloads::is_id(upload) {
        return Ok(not_found(NO_UPLOAD).into_response());
    }
    let Some(_guard) = FinishGuard::acquire(upload) else {
        return Ok(Refusal::new(StatusCode::CONFLICT, "busy", "This upload is being finished already.").into_response());
    };
    let config = &state.config;
    // A repeated finish (its first answer got lost) gets the same build.
    if let Some(id) = downloads::finished_build_id(config, upload).await {
        return Ok(match downloads::find_release(&state.db, &id).await? {
            Some(release) => json_private(StatusCode::OK, &Build::from(release)),
            None => not_found("This upload was published, and that build has been deleted since.").into_response(),
        });
    }
    let Some(staged) = downloads::staged_upload(config, upload).await? else {
        return Ok(not_found(NO_UPLOAD).into_response());
    };
    let ip = client_ip(state, &parts);
    let request = &staged.request;
    let (code, reason) = match downloads::assemble(&downloads::upload_dir(config, upload)).await? {
        Err(reason) => ("size_mismatch", reason),
        Ok(joined) if joined.size != request.size => {
            ("size_mismatch", format!("{} bytes arrived, the upload declared {}", joined.size, request.size))
        }
        Ok(joined) if joined.sha256 != request.sha256 => {
            ("checksum_mismatch", "the SHA-256 of the bytes that arrived is not the one declared".to_string())
        }
        Ok(joined) => return publish(state, &who, upload, &staged, joined, &ip).await,
    };
    downloads::drop_upload(config, upload).await;
    let about = subject(upload, &request.platform, &request.arch, &request.version, &request.build, &request.filename);
    let extra =
        json!({ "upload": upload, "code": code, "reason": reason, "size": request.size, "sha256": request.sha256 });
    audit(state, &who, "downloads.rejected", about, extra, &ip).await;
    Ok(Refusal::bad(code, format!("{reason}: the upload was dropped; publish it again.")).into_response())
}

/// The checked file becomes a build: stored, recorded, the oldest beyond
/// [`KEEP`] dropped, every step audited.
async fn publish(
    state: &AppState,
    who: &Publisher,
    upload: &str,
    staged: &StagedUpload,
    joined: Assembled,
    ip: &str,
) -> anyhow::Result<Response> {
    let storage = state.storage.get()?;
    let request = &staged.request;
    let release = Release {
        id: downloads::new_id(),
        platform: request.platform.clone(),
        arch: request.arch.clone(),
        version: request.version.clone(),
        build: request.build.clone(),
        built: request.built,
        published: downloads::publish_time(),
        size: joined.size as i64,
        sha256: joined.sha256,
        filename: request.filename.clone(),
        uploader: staged.uploader.clone(),
        note: request.note.clone(),
    };
    storage.ingest_local_file(&joined.path, &downloads::storage_key(&release.id, &release.filename)).await?;
    let pruned = match downloads::insert_release(&state.db, &release).await {
        Ok(pruned) => pruned,
        Err(err) => {
            downloads::delete_files(storage.as_ref(), &release.id).await;
            return Err(err.into());
        }
    };
    if let Err(err) = downloads::mark_finished(&state.config, upload, &release.id).await {
        tracing::warn!("[downloads] could not mark upload {upload} as finished: {err}");
    }
    let extra = json!({ "upload": upload, "size": release.size, "sha256": release.sha256 });
    audit(state, who, "downloads.published", release_subject(&release), extra, ip).await;
    for old in &pruned {
        downloads::delete_files(storage.as_ref(), &old.id).await;
        let extra = json!({ "keep": KEEP, "replacedBy": release.id });
        audit(state, who, "downloads.pruned", release_subject(old), extra, ip).await;
    }
    Ok(json_private(StatusCode::OK, &Build::from(release)))
}

async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Sessions(session): Sessions,
    req: Request,
) -> Response {
    respond(remove_build(&state, &session, &id, req).await)
}

async fn remove_build(state: &AppState, session: &RequestSession, id: &str, req: Request) -> anyhow::Result<Response> {
    let (parts, _) = req.into_parts();
    let who = match publisher(state, session, &parts.headers, false).await? {
        Ok(who) => who,
        Err(refusal) => return Ok(refusal.into_response()),
    };
    let Some(release) = downloads::delete_release(&state.db, id).await? else {
        return Ok(not_found("No such build.").into_response());
    };
    let storage = state.storage.get()?;
    downloads::delete_files(storage.as_ref(), &release.id).await;
    let ip = client_ip(state, &parts);
    audit(state, &who, "downloads.deleted", release_subject(&release), json!({ "size": release.size }), &ip).await;
    Ok(json_private(StatusCode::OK, &json!({ "id": release.id, "deleted": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn if_none_match_lists() {
        let etag = "\"abc\"";
        assert!(none_match("\"abc\"", etag));
        assert!(none_match("W/\"abc\"", etag));
        assert!(none_match("\"x\", \"abc\"", etag));
        assert!(none_match("*", etag));
        assert!(!none_match("\"abd\"", etag));
        assert!(!none_match("abc", etag));
    }

    #[test]
    fn body_refusals_have_codes() {
        assert_eq!(body_code(StatusCode::UNSUPPORTED_MEDIA_TYPE), "unsupported_media_type");
        assert_eq!(body_code(StatusCode::PAYLOAD_TOO_LARGE), "too_large");
        assert_eq!(body_code(StatusCode::BAD_REQUEST), "invalid_body");
    }
}
