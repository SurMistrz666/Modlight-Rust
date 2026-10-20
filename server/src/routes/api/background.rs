//! Port of `app/api/background/route.ts`: set (POST, the raw image as the
//! body) or remove (DELETE) the signed-in account's wallpaper.
//!
//! The uploaded bytes are never stored: libvips re-encodes them to WebP
//! (EXIF and trailing payloads dropped, the served type matches the bytes),
//! through the shared limiter at thumbnail weight.

use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::post;
use bytes::Bytes;
use rand::RngCore;
use serde_json::json;

use super::avatar::{client_closed, error, json, read_image_body};
use crate::auth_helpers::MaybeUser;
use crate::background::{
    BACKGROUND_MAX_BYTES, BACKGROUND_MAX_EDGE, BACKGROUND_PREFIX, background_key, is_accepted_background_type,
};
use crate::db::types::JsDate;
use crate::error::{AppError, AppResult};
use crate::media::abort::AbortController;
use crate::media::transform_cost::{THUMBNAIL_TIMEOUT_MS, THUMBNAIL_WEIGHT};
use crate::media::transform_queue::{AbortedError, BusyError, RunOptions, TimeoutError, image_transforms};
use crate::media::vips::{Fit, Resize, Sharp, WebpOptions};
use crate::state::AppState;
use crate::storage::{StorageDriver, bytes_stream};

/// Decompression-bomb guard: a 40 kB PNG can declare 100k x 100k pixels.
const MAX_INPUT_PIXELS: u64 = 64_000_000;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/background", post(set).delete(remove))
}

/// `sharp(input, { limitInputPixels, animated: false }).timeout().rotate()
/// .resize(2560, 2560, inside, withoutEnlargement).webp({ quality: 82 })`
pub async fn encode_background(input: Bytes, controller: &AbortController) -> anyhow::Result<Vec<u8>> {
    image_transforms()
        .run(
            move |_signal| async move {
                Ok(Sharp::new(input)
                    .limit_input_pixels(Some(MAX_INPUT_PIXELS))
                    .animated(false)
                    .timeout(THUMBNAIL_TIMEOUT_MS.div_ceil(1000) as u32)
                    // Before resize, always: the orientation tag is dropped on encode.
                    .rotate()
                    .resize(
                        Resize::to(Some(BACKGROUND_MAX_EDGE), Some(BACKGROUND_MAX_EDGE))
                            .fit(Fit::Inside)
                            .without_enlargement(true),
                    )
                    .webp(WebpOptions { quality: 82, ..WebpOptions::default() })
                    .to_buffer()
                    .await?)
            },
            RunOptions {
                weight: Some(THUMBNAIL_WEIGHT),
                timeout_ms: Some(THUMBNAIL_TIMEOUT_MS),
                signal: Some(controller.signal()),
            },
        )
        .await
}

fn random_token() -> String {
    let mut buf = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

async fn set(State(state): State<AppState>, MaybeUser(session): MaybeUser, req: Request) -> AppResult<Response> {
    let Some(session) = session else {
        return Ok(error(StatusCode::UNAUTHORIZED, "unauthorized"));
    };
    let (parts, body) = req.into_parts();
    let input =
        match read_image_body(&parts.headers, body, is_accepted_background_type, BACKGROUND_MAX_BYTES as u64).await {
            Ok(b) => b,
            Err(res) => return Ok(*res),
        };

    let controller = AbortController::default();
    let _guard = controller.abort_on_drop();
    let output = match encode_background(input, &controller).await {
        Ok(out) => out,
        Err(err) if err.is::<BusyError>() || err.is::<TimeoutError>() => {
            return Ok(error(StatusCode::SERVICE_UNAVAILABLE, "busy"));
        }
        Err(err) if err.is::<AbortedError>() => return Ok(client_closed()),
        // Anything libvips cannot decode is not an image, whatever the header claimed.
        Err(_) => return Ok(error(StatusCode::BAD_REQUEST, "decode")),
    };

    let storage = state.storage.get().map_err(AppError::Internal)?;
    let key = background_key(&session.user.id, &random_token());
    let size = output.len();
    storage.write(&key, bytes_stream(Bytes::from(output))).await?;

    // Fix (SKELETON.md 9): upstream's RETURNING read the row AFTER the
    // update, so the previous wallpaper was never deleted. The previous key is
    // read under the row lock and its object deleted once the new one is set.
    let old = set_background_key(&state.db, &session.user.id, Some(&key)).await?;
    discard(storage.as_ref(), &session.user.id, old.flatten().as_deref(), Some(&key)).await;

    Ok(json(StatusCode::OK, json!({ "ok": true, "bytes": size })))
}

async fn remove(State(state): State<AppState>, MaybeUser(session): MaybeUser) -> AppResult<Response> {
    let Some(session) = session else {
        return Ok(error(StatusCode::UNAUTHORIZED, "unauthorized"));
    };
    let old = set_background_key(&state.db, &session.user.id, None).await?;
    let storage = state.storage.get().map_err(AppError::Internal)?;
    discard(storage.as_ref(), &session.user.id, old.flatten().as_deref(), None).await;
    Ok(json(StatusCode::OK, json!({ "ok": true })))
}

/// Point `user.background_key` at `key` (or clear it) and return the value it
/// held BEFORE (`None`: no such user). Read under the row lock the statement
/// takes, so concurrent replacements each get the key they replaced.
pub async fn set_background_key(
    db: &sqlx::PgPool,
    user_id: &str,
    key: Option<&str>,
) -> anyhow::Result<Option<Option<String>>> {
    Ok(sqlx::query_scalar(
        r#"update "user" as u set "background_key" = $1, "updated_at" = $2
           from (select "id", "background_key" from "user" where "id" = $3 for update) as prev
           where u."id" = prev."id"
           returning prev."background_key""#,
    )
    .bind(key)
    .bind(JsDate::now())
    .bind(user_id)
    .fetch_optional(db)
    .await?)
}

/// Delete the file a key used to point at; never fails the request. Only
/// this account's own wallpaper keys (`backgrounds/<userId>-...`) are ever
/// handed to the driver.
pub async fn discard(storage: &dyn StorageDriver, user_id: &str, old: Option<&str>, keep: Option<&str>) {
    let own = format!("{BACKGROUND_PREFIX}/{user_id}-");
    let Some(old) = old.filter(|o| o.starts_with(&own)) else {
        return;
    };
    if Some(old) == keep {
        return;
    }
    let _ = storage.delete(old).await;
}
