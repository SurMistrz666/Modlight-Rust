//! Port of `lib/upload/sharex.ts`: the ShareX / legacy single-POST upload
//! endpoint (`POST /upload`, `POST /api/upload`).
//!
//! Streams the file straight to a temp file (constant memory, no body cap),
//! hashing as it goes. Auth is resolved from the `x-api-key` /
//! `Authorization` header BEFORE a single byte is read. Response shape is the
//! one existing ShareX configs expect.
//!
//! SCOPE: `upload`, at every place a token can enter, and nothing wider.
//!
//! Security fix 2 (PORT.md section 12): upstream wrote up to ~512 MB of an
//! unauthenticated body to disk before the legacy `token` field was seen,
//! and skipped the free-space check for chunked bodies. Here:
//! - a header token is authenticated before the body is read (as upstream);
//! - without one, at most [`ANON_MAX_BYTES`] (8 MiB unless
//!   `LEGACY_UPLOAD_MAX_SIZE` says otherwise) are written before a valid token
//!   has been seen (ShareX sends its form arguments before the file, so a
//!   legacy config authenticates long before that);
//! - while the bytes stream, the account's remaining quota and the free space
//!   on the staging and storage filesystems are enforced, with the same
//!   status and message finalize would answer with at the end.

use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use bytes::Bytes;
use futures_util::StreamExt;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::api_scopes::TokenCapability;
use crate::api_token::{TokenAuth, UserRow, authenticate_api_token, token_refusal};
use crate::config::{Config, js_number};
use crate::state::AppState;
use crate::upload::busboy::{BodyType, PartKind, body_type, classify_part, decode_field};
use crate::upload::disk::{fits_on_disk, has_room_for};
use crate::upload::finalize::{FinalizeError, FinalizeInput, finalize_upload, remaining_quota};
use crate::upload::folder_target::{FOLDER_KEY, FolderTarget, resolve_folder_target};

const VERSION: &str = "0.1.0";

/// How much of the payload's header is kept for content sniffing.
const HEAD_BYTES: usize = 4096;

/// busboy's default `fieldSize` limit: longer values are truncated.
const FIELD_SIZE_LIMIT: usize = 1024 * 1024;

/// How often (in bytes written) free space is re-measured while streaming.
const SPACE_CHECK_EVERY: u64 = 8 * 1024 * 1024;

/// The ceiling for a request that has not (yet) authenticated: bytes written
/// before a valid token was seen. `LEGACY_UPLOAD_MAX_SIZE` overrides it, as
/// upstream; the default is the security fix's small bound (upstream: 512 MiB).
pub fn anon_max_bytes() -> u64 {
    let raw = std::env::var("LEGACY_UPLOAD_MAX_SIZE").map(|v| js_number(&v)).unwrap_or(f64::NAN);
    if raw.is_finite() && raw > 0.0 {
        return raw as u64;
    }
    ANON_MAX_BYTES
}

/// The default unauthenticated ceiling (security fix 2).
pub const ANON_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// The exact wording for "no such token", kept because ShareX users' configs
/// have been matching on it since v1.
const UNKNOWN_TOKEN: &str = "Token specified not found.";

/// `send(res, status, body)`: `{ version, ...body }`.
fn send(status: u16, body: Value) -> Response {
    let mut out = Map::new();
    out.insert("version".into(), json!(VERSION));
    if let Value::Object(m) = body {
        out.extend(m);
    }
    let bytes = serde_json::to_vec(&Value::Object(out)).unwrap_or_default();
    let mut res = (StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), bytes).into_response();
    res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    res
}

fn message(status: u16, text: &str) -> Response {
    send(status, json!({ "message": text }))
}

/// Answer a token refusal: 404 for an unknown token (what ShareX configs have
/// always received), the shared status otherwise.
fn refuse(auth: &TokenAuth) -> Response {
    let (status, body) = token_refusal(auth, UNKNOWN_TOKEN).unwrap_or((StatusCode::UNAUTHORIZED, json!({})));
    let status = if matches!(auth, TokenAuth::Unknown) { 404 } else { status.as_u16() };
    send(status, body)
}

/// `bearerFromHeaders(req)`: `x-api-key` when non-empty (untrimmed), else a
/// `Bearer` authorization header (trimmed).
pub fn bearer_from_headers(headers: &HeaderMap) -> Option<String> {
    // Node joins repeated headers with ", "; values are latin1.
    let joined = |name: &str| -> Option<String> {
        let values: Vec<String> =
            headers.get_all(name).iter().map(|v| v.as_bytes().iter().map(|b| *b as char).collect()).collect();
        if values.is_empty() { None } else { Some(values.join(", ")) }
    };
    if let Some(x) = joined("x-api-key").filter(|v| !v.is_empty()) {
        return Some(x);
    }
    let auth =
        headers.get(header::AUTHORIZATION).map(|v| v.as_bytes().iter().map(|b| *b as char).collect::<String>())?;
    if auth.to_lowercase().starts_with("bearer ") {
        let token = auth.chars().skip(7).collect::<String>();
        let token = crate::config::js_trim(&token).to_string();
        return if token.is_empty() { None } else { Some(token) };
    }
    None
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/upload", post(handle_sharex_upload)).route("/api/upload", post(handle_sharex_upload))
}

/// Removes the scratch file however the request ends.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// How the body stage ended early.
enum Abort {
    /// A response to send now (limits, quota, space).
    Respond(Box<Response>),
    /// The stream broke (malformed body, client gone).
    StreamFailed,
}

/// State of one upload while its body streams past.
struct Progress {
    owner: Option<UserRow>,
    pending_auth: Option<TokenAuth>,
    fields: Map<String, Value>,
    filename: Option<String>,
    mime: String,
    size: u64,
    got_file: bool,
    head: Vec<u8>,
    over_hard_limit: bool,
    hasher: Sha256,
}

pub async fn handle_sharex_upload(State(state): State<AppState>, req: axum::extract::Request) -> Response {
    match upload(&state, req).await {
        Ok(res) => res,
        Err(err) => {
            tracing::error!("[upload] handler error: {err:#}");
            crate::error::internal_error_response()
        }
    }
}

async fn upload(state: &AppState, req: axum::extract::Request) -> anyhow::Result<Response> {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let config: &Config = &state.config;

    // ── auth before any disk IO ──
    let mut progress = Progress {
        owner: None,
        pending_auth: None,
        fields: Map::new(),
        filename: None,
        mime: "application/octet-stream".to_string(),
        size: 0,
        got_file: false,
        head: Vec::with_capacity(HEAD_BYTES),
        over_hard_limit: false,
        hasher: Sha256::new(),
    };
    if let Some(token) = bearer_from_headers(&headers) {
        let auth = authenticate_api_token(&state.db, Some(&token), TokenCapability::Upload).await?;
        match auth {
            TokenAuth::Ok { user, .. } => progress.owner = Some(user),
            other => return Ok(refuse(&other)),
        }
    }

    // ── refuse an oversized body BEFORE a byte touches the disk ──
    let declared =
        headers.get(header::CONTENT_LENGTH).map(|v| js_number(&String::from_utf8_lossy(v.as_bytes()))).unwrap_or(0.0);
    if declared.is_finite() && declared > 0.0 && !has_room_for(config, declared, &config.uploads.tmp_path).await {
        return Ok(message(507, "Server is low on disk space; upload rejected."));
    }

    let content_type =
        headers.get(header::CONTENT_TYPE).map(|v| v.as_bytes().iter().map(|b| *b as char).collect::<String>());
    let Ok(kind) = body_type(content_type.as_deref()) else {
        return Ok(message(400, "Invalid multipart request."));
    };

    tokio::fs::create_dir_all(&config.uploads.tmp_path).await?;
    let tmp_path = PathBuf::from(&config.uploads.tmp_path).join(uuid::Uuid::new_v4().to_string());
    let _guard = TempFile(tmp_path.clone());

    let streamed = match kind {
        BodyType::UrlEncoded => {
            // busboy parses the fields; there can be no file, so only the
            // "no file" answer remains. Drain without storing.
            let mut stream = body.into_data_stream();
            let mut failed = false;
            while let Some(chunk) = stream.next().await {
                if chunk.is_err() {
                    failed = true;
                    break;
                }
            }
            if failed { Err(Abort::StreamFailed) } else { Ok(()) }
        }
        BodyType::Multipart { boundary } => stream_multipart(state, body, boundary, &tmp_path, &mut progress).await,
    };
    match streamed {
        Ok(()) => {}
        Err(Abort::Respond(res)) => return Ok(*res),
        Err(Abort::StreamFailed) => {
            if progress.over_hard_limit {
                return Ok(message(413, "File too large."));
            }
            return Ok(message(400, "Upload stream failed."));
        }
    }

    // ── validate ──
    if progress.over_hard_limit {
        return Ok(message(413, "File too large."));
    }
    if !progress.got_file {
        return Ok(message(400, "Request without file attached."));
    }
    let owner = match progress.owner.take() {
        Some(o) => o,
        None => {
            let body_token = progress.fields.get("token").and_then(Value::as_str).unwrap_or("").to_string();
            if body_token.is_empty() {
                return Ok(message(400, "Token not specified."));
            }
            // The field may have arrived after the file: verify it now.
            let auth = match progress.pending_auth.take() {
                Some(a) => a,
                None => authenticate_api_token(&state.db, Some(&body_token), TokenCapability::Upload).await?,
            };
            match auth {
                TokenAuth::Ok { user, .. } => user,
                other => return Ok(refuse(&other)),
            }
        }
    };

    // ── where should it land? ── An id this account does not own does NOT
    // fail the upload: the bytes are here, the account default applies and
    // the client is told.
    let folder_raw = progress.fields.get(FOLDER_KEY).and_then(Value::as_str).map(str::to_string);
    let (folder_id, folder_warning) = match resolve_folder_target(&state.db, &owner.id, folder_raw.as_deref()).await {
        Ok(target) => (target.explicit(), None),
        Err(_) => (FolderTarget::Default.explicit(), Some("unknown-folder")),
    };

    // ── had we already got these exact bytes? ── A REPORT, never a refusal,
    // asked BEFORE the insert (afterwards the new row would match itself).
    let checksum = hex::encode(progress.hasher.finalize_reset());
    let existing = match crate::upload::duplicate_lookup::first_match_for(&state.db, &owner.id, &checksum).await {
        Ok(e) => e,
        Err(err) => {
            tracing::error!("[sharex] duplicate lookup failed: {err}");
            None
        }
    };

    // ── finalize ──
    let storage = state.storage.get()?;
    let custom_code = progress.fields.get("code").and_then(Value::as_str).filter(|c| !c.is_empty()).map(str::to_string);
    let filename = progress.filename.clone().unwrap_or_else(|| "file".to_string());
    let result = finalize_upload(
        &state.db,
        config,
        storage.as_ref(),
        FinalizeInput {
            owner: &owner,
            filename: &filename,
            mime: &progress.mime,
            size: progress.size as i64,
            head: Some(Bytes::from(std::mem::take(&mut progress.head))),
            local_path: Some(&tmp_path),
            existing_key: None,
            checksum: Some(checksum),
            custom_code: custom_code.as_deref(),
            folder_id,
        },
    )
    .await;
    match result {
        Ok(result) => {
            let mut body = Map::new();
            body.insert("message".into(), json!("OK"));
            body.insert("url".into(), json!(result.url));
            body.insert("raw_url".into(), json!(result.raw_url));
            // The link to put on the clipboard, chosen by the ACCOUNT's `copy_raw`.
            body.insert("copy_url".into(), json!(result.copy_url));
            body.insert("deletionUrl".into(), json!(result.deletion_url));
            body.insert("folder".into(), json!(result.folder_id));
            if let Some(w) = folder_warning {
                body.insert("folder_warning".into(), json!(w));
            }
            // Absent unless there really is an older copy.
            if let Some(existing) = existing {
                let dup = crate::media::urls::media_urls(
                    config,
                    &crate::media::urls::LinkOwner::of_user(&owner),
                    &existing.code,
                    &existing.ext,
                );
                let copy = if owner.copy_raw == Some(true) { dup.raw_url.clone() } else { dup.url.clone() };
                body.insert(
                    "duplicate_of".into(),
                    json!({
                        "code": existing.code,
                        "filename": existing.filename,
                        "size": existing.size,
                        "created_at": existing.created_at.to_iso_string(),
                        "folder": existing.folder_id,
                        "url": dup.url,
                        "raw_url": dup.raw_url,
                        "copy_url": copy,
                    }),
                );
            }
            Ok(send(201, Value::Object(body)))
        }
        Err(err @ (FinalizeError::QuotaExceeded | FinalizeError::OutOfSpace)) => Ok(message(507, &err.to_string())),
        Err(err @ FinalizeError::CodeTaken) => Ok(message(409, &err.to_string())),
        Err(FinalizeError::Other(err)) => {
            tracing::error!("[sharex] finalize failed: {err:#}");
            Ok(message(500, "Upload failed."))
        }
    }
}

/// Free space is still above the reserve on the staging and storage
/// filesystems (`fitsOnDisk(0, free, reserve)` on each).
async fn still_has_room(config: &Config) -> bool {
    if config.storage.driver != "local" || config.uploads.min_free_bytes <= 0 {
        // Without a reserve, only a completely full disk refuses (the write
        // fails by itself then).
        return true;
    }
    let reserve = config.uploads.min_free_bytes as f64;
    for path in [&config.uploads.tmp_path, &config.storage.local_path] {
        let free = crate::upload::disk::filesystem_space_at(std::path::Path::new(path)).map(|s| s.free_bytes as f64);
        if !fits_on_disk(0.0, free, reserve) {
            return false;
        }
    }
    true
}

async fn stream_multipart(
    state: &AppState,
    body: Body,
    boundary: String,
    tmp_path: &std::path::Path,
    p: &mut Progress,
) -> Result<(), Abort> {
    let config = &state.config;
    let max_size = if config.uploads.max_size > 0 { Some(config.uploads.max_size as u64) } else { None };
    let anon_max = anon_max_bytes();
    let stream = body.into_data_stream();
    let mut multipart = multer::Multipart::new(stream, boundary);
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(_) => return Err(Abort::StreamFailed),
        };
        let disposition = field.headers().get(header::CONTENT_DISPOSITION).map(|v| v.as_bytes().to_vec());
        let part_type = field.headers().get(header::CONTENT_TYPE).map(|v| v.as_bytes().to_vec());
        match classify_part(disposition.as_deref(), part_type.as_deref()) {
            PartKind::Skip => drain(field).await?,
            PartKind::Field { name, charset } => {
                let mut field = field;
                let mut bytes: Vec<u8> = Vec::new();
                while let Some(chunk) = field.chunk().await.map_err(|_| Abort::StreamFailed)? {
                    let room = FIELD_SIZE_LIMIT.saturating_sub(bytes.len());
                    bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
                }
                let value = decode_field(&bytes, &charset);
                // `fields[name] = val` with an undefined name is the key "undefined".
                let name = name.unwrap_or_else(|| "undefined".to_string());
                // Legacy config: the token rides along as a form field. When it
                // arrives before the file, authenticate now and lift the
                // anonymous cap. Only an ALLOWED token lifts it.
                if name == "token" && p.owner.is_none() && p.pending_auth.is_none() {
                    let auth = authenticate_api_token(&state.db, Some(&value), TokenCapability::Upload).await.map_err(
                        |e| {
                            tracing::error!("[sharex] token check failed: {e:#}");
                            Abort::Respond(Box::new(crate::error::internal_error_response()))
                        },
                    )?;
                    if let TokenAuth::Ok { user, .. } = &auth {
                        p.owner = Some(user.clone());
                    }
                    p.pending_auth = Some(auth);
                }
                p.fields.insert(name, Value::String(value));
            }
            PartKind::File { filename, mime_type, .. } => {
                if p.got_file {
                    // busboy's files:1 limit skips these.
                    drain(field).await?;
                    continue;
                }
                p.got_file = true;
                p.filename = Some(filename.filter(|f| !f.is_empty()).unwrap_or_else(|| "file".to_string()));
                if !mime_type.is_empty() {
                    p.mime = mime_type;
                }
                let mut out = tokio::fs::File::create(tmp_path).await.map_err(|_| Abort::StreamFailed)?;
                let mut field = field;
                let mut since_check: u64 = 0;
                let mut first_check_done = false;
                while let Some(chunk) = field.chunk().await.map_err(|_| Abort::StreamFailed)? {
                    let mut chunk = chunk;
                    if let Some(limit) = max_size {
                        if p.over_hard_limit {
                            continue; // truncated: the rest is discarded
                        }
                        if p.size + chunk.len() as u64 > limit {
                            chunk = chunk.slice(..(limit - p.size) as usize);
                            p.over_hard_limit = true;
                        }
                    }
                    p.hasher.update(&chunk);
                    if p.head.len() < HEAD_BYTES {
                        let take = (HEAD_BYTES - p.head.len()).min(chunk.len());
                        p.head.extend_from_slice(&chunk[..take]);
                    }
                    p.size += chunk.len() as u64;
                    match &p.owner {
                        None if p.size > anon_max => {
                            return Err(Abort::Respond(Box::new(message(
                                413,
                                "File too large for an unauthenticated upload. Send your token in the x-api-key header (regenerate your ShareX config) to upload files of any size.",
                            ))));
                        }
                        Some(owner) => {
                            // Security fix 2: the quota, as the bytes arrive.
                            if let Some(left) = remaining_quota(owner.used_quota, owner.max_quota)
                                && p.size as i64 > left
                            {
                                return Err(Abort::Respond(Box::new(message(507, "User disk quota exceeded."))));
                            }
                        }
                        None => {}
                    }
                    // Security fix 2: free space, as the bytes arrive.
                    since_check += chunk.len() as u64;
                    if !first_check_done || since_check >= SPACE_CHECK_EVERY {
                        first_check_done = true;
                        since_check = 0;
                        if !still_has_room(config).await {
                            return Err(Abort::Respond(Box::new(message(
                                507,
                                "Server is low on disk space; upload rejected.",
                            ))));
                        }
                    }
                    out.write_all(&chunk).await.map_err(|_| Abort::StreamFailed)?;
                }
                out.flush().await.map_err(|_| Abort::StreamFailed)?;
            }
        }
    }
    Ok(())
}

async fn drain(mut field: multer::Field<'_>) -> Result<(), Abort> {
    while field.chunk().await.map_err(|_| Abort::StreamFailed)?.is_some() {}
    Ok(())
}

#[cfg(test)]
mod tests {
    //! tests/unit/upload-sharex.test.ts, tests/unit/sharex-scope.test.ts
    //! (the parts that need no database; the rest is the parity suite).
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn bearer_rules() {
        assert_eq!(bearer_from_headers(&headers(&[("x-api-key", "abc")])).as_deref(), Some("abc"));
        assert_eq!(bearer_from_headers(&headers(&[("x-api-key", " abc ")])).as_deref(), Some(" abc "));
        assert_eq!(bearer_from_headers(&headers(&[("authorization", "Bearer  tok ")])).as_deref(), Some("tok"));
        assert_eq!(bearer_from_headers(&headers(&[("authorization", "BEARER tok")])).as_deref(), Some("tok"));
        assert_eq!(bearer_from_headers(&headers(&[("authorization", "Basic tok")])), None);
        assert_eq!(
            bearer_from_headers(&headers(&[("x-api-key", ""), ("authorization", "Bearer t")])).as_deref(),
            Some("t")
        );
        assert_eq!(bearer_from_headers(&headers(&[("x-api-key", "a"), ("x-api-key", "b")])).as_deref(), Some("a, b"));
        assert_eq!(bearer_from_headers(&headers(&[])), None);
    }

    #[test]
    fn refusals_keep_the_sharex_wording() {
        let res = refuse(&TokenAuth::Unknown);
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(anon_max_bytes(), ANON_MAX_BYTES);
    }
}
