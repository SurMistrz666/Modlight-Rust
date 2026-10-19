//! Port of `app/api/avatar/route.ts`: set (POST, the raw image as the body)
//! or remove (DELETE) the signed-in account's avatar.
//!
//! A route handler rather than an action because images exceed the 1 MB
//! action body cap. Picking one of the account's own uploads goes through
//! `actions/avatar.rs` instead.

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use bytes::Bytes;
use futures_util::StreamExt;
use serde_json::json;

use crate::api_json::json_response;
use crate::auth_helpers::MaybeUser;
use crate::avatar::{AVATAR_MAX_BYTES, is_accepted_avatar_type};
use crate::avatar_store::{AvatarSaveError, clear_avatar, save_avatar};
use crate::config::js_number;
use crate::error::{AppError, AppResult};
use crate::media::abort::AbortController;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/avatar", post(set).delete(remove))
}

/// `json(body, status)`: `Content-Type: application/json`, nothing else.
pub(crate) fn json(status: StatusCode, body: serde_json::Value) -> Response {
    json_response(status, &body, None)
}

pub(crate) fn error(status: StatusCode, code: &str) -> Response {
    json(status, json!({ "error": code }))
}

/// The raw-body checks both image routes share, in upstream's order: the
/// type, the declared length, then the real length once read. Reading stops
/// one byte past the ceiling (upstream buffers everything and then refuses:
/// same answer, without the buffer).
pub(crate) async fn read_image_body(
    headers: &HeaderMap,
    body: Body,
    accepted: fn(Option<&str>) -> bool,
    max_bytes: u64,
) -> Result<Bytes, Box<Response>> {
    let kind = headers.get(header::CONTENT_TYPE).map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned());
    if !accepted(kind.as_deref()) {
        return Err(Box::new(error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "type")));
    }
    let declared = headers.get(header::CONTENT_LENGTH).map(|v| js_number(&String::from_utf8_lossy(v.as_bytes())));
    if let Some(n) = declared
        && n.is_finite()
        && n > max_bytes as f64
    {
        return Err(Box::new(error(StatusCode::PAYLOAD_TOO_LARGE, "too-large")));
    }
    let mut buf: Vec<u8> = Vec::new();
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        // A body that breaks mid-read: upstream's arrayBuffer() rejects (500).
        let chunk = chunk.map_err(|e| Box::new(AppError::internal(anyhow::anyhow!("{e}")).into_response()))?;
        buf.extend_from_slice(&chunk);
        if buf.len() as u64 > max_bytes {
            return Err(Box::new(error(StatusCode::PAYLOAD_TOO_LARGE, "too-large")));
        }
    }
    if buf.is_empty() {
        return Err(Box::new(error(StatusCode::BAD_REQUEST, "empty")));
    }
    Ok(Bytes::from(buf))
}

/// The bodiless answer to a caller that hung up.
pub(crate) fn client_closed() -> Response {
    StatusCode::from_u16(499).map(|s| s.into_response()).unwrap_or_else(|_| StatusCode::BAD_REQUEST.into_response())
}

async fn set(State(state): State<AppState>, MaybeUser(session): MaybeUser, req: Request) -> AppResult<Response> {
    let Some(session) = session else {
        return Ok(error(StatusCode::UNAUTHORIZED, "unauthorized"));
    };
    let (parts, body) = req.into_parts();
    let input = match read_image_body(&parts.headers, body, is_accepted_avatar_type, AVATAR_MAX_BYTES).await {
        Ok(b) => b,
        Err(res) => return Ok(*res),
    };

    // `req.signal`: dropping this handler (the client went away) aborts.
    let controller = AbortController::default();
    let _guard = controller.abort_on_drop();
    let storage = state.storage.get().map_err(AppError::Internal)?;
    let result = save_avatar(&state.db, storage.as_ref(), &session.user.id, input, Some(controller.signal())).await?;
    match result {
        Ok(saved) => Ok(json(
            StatusCode::OK,
            json!({
                "ok": true,
                "url": saved.url,
                "bytes": saved.bytes,
                "animated": saved.animated,
                "degraded": saved.degraded.map(|d| d.as_str()),
            }),
        )),
        Err(AvatarSaveError::Busy) => Ok(error(StatusCode::SERVICE_UNAVAILABLE, "busy")),
        Err(AvatarSaveError::Aborted) => Ok(client_closed()),
        Err(AvatarSaveError::TooManyPixels) => Ok(error(StatusCode::PAYLOAD_TOO_LARGE, "too-many-pixels")),
        Err(AvatarSaveError::Decode) => Ok(error(StatusCode::BAD_REQUEST, "decode")),
    }
}

async fn remove(State(state): State<AppState>, MaybeUser(session): MaybeUser) -> AppResult<Response> {
    let Some(session) = session else {
        return Ok(error(StatusCode::UNAUTHORIZED, "unauthorized"));
    };
    let storage = state.storage.get().map_err(AppError::Internal)?;
    clear_avatar(&state.db, storage.as_ref(), &session.user.id).await?;
    Ok(json(StatusCode::OK, json!({ "ok": true })))
}
