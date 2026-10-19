//! Port of `lib/avatar-store.ts`: persisting an avatar (limiter, encoder,
//! storage, row, and the orphan cleanup), shared by the raw-body route
//! (`POST /api/avatar`) and the server action (`setAvatarFromUpload`).
//!
//! Order matters: the object is written before the column points at it, so a
//! failure between the two leaves an orphan rather than a 404 on somebody's
//! face; the old object is deleted last and never fails the request.

use bytes::Bytes;
use rand::RngCore;
use sqlx::PgPool;

use crate::avatar::{avatar_key, avatar_url, is_avatar_key};
use crate::db::types::JsDate;
use crate::media::abort::AbortSignal;
use crate::media::avatar::{AvatarDegradation, AvatarEncodeResult, encode_avatar};
use crate::media::transform_cost::{THUMBNAIL_TIMEOUT_MS, THUMBNAIL_WEIGHT};
use crate::media::transform_queue::{AbortedError, BusyError, RunOptions, TimeoutError, image_transforms};
use crate::storage::{StorageDriver, bytes_stream};

/// `AvatarSaveError`: everything a caller has to be able to tell the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvatarSaveError {
    /// libvips cannot read these bytes, whatever the header claimed.
    Decode,
    /// A single frame beyond the decompression-bomb ceiling.
    TooManyPixels,
    /// No transform slot, or the encode outlived its deadline.
    Busy,
    /// The caller hung up mid-encode.
    Aborted,
}

impl AvatarSaveError {
    pub fn as_str(self) -> &'static str {
        match self {
            AvatarSaveError::Decode => "decode",
            AvatarSaveError::TooManyPixels => "too-many-pixels",
            AvatarSaveError::Busy => "busy",
            AvatarSaveError::Aborted => "aborted",
        }
    }
}

/// The `{ ok: true }` arm of `AvatarSaveResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedAvatar {
    pub url: Option<String>,
    pub bytes: usize,
    pub animated: bool,
    pub degraded: Option<AvatarDegradation>,
}

/// `randomBytes(8).toString("hex")`
fn random_token() -> String {
    let mut buf = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// `saveAvatar(userId, input, { signal })`: re-encode `input` (through the
/// shared limiter at thumbnail weight) and make it the account's avatar.
pub async fn save_avatar(
    db: &PgPool,
    storage: &dyn StorageDriver,
    user_id: &str,
    input: Bytes,
    signal: Option<AbortSignal>,
) -> anyhow::Result<Result<SavedAvatar, AvatarSaveError>> {
    let encoded = image_transforms()
        .run(
            move |_signal| async move { encode_avatar(input).await },
            RunOptions { weight: Some(THUMBNAIL_WEIGHT), timeout_ms: Some(THUMBNAIL_TIMEOUT_MS), signal },
        )
        .await;
    let (bytes, animated, degraded) = match encoded {
        Ok(AvatarEncodeResult::Ok { bytes, animated, degraded, .. }) => (bytes, animated, degraded),
        Ok(AvatarEncodeResult::Failed { error }) => {
            return Ok(Err(match error {
                crate::media::avatar::AvatarEncodeError::Decode => AvatarSaveError::Decode,
                crate::media::avatar::AvatarEncodeError::TooManyPixels => AvatarSaveError::TooManyPixels,
            }));
        }
        Err(err) if err.is::<BusyError>() || err.is::<TimeoutError>() => return Ok(Err(AvatarSaveError::Busy)),
        Err(err) if err.is::<AbortedError>() => return Ok(Err(AvatarSaveError::Aborted)),
        // Anything libvips throws from inside the encoder is "we cannot read
        // this", which is a fact the user can act on.
        Err(_) => return Ok(Err(AvatarSaveError::Decode)),
    };

    let key = avatar_key(user_id, &random_token());
    let size = bytes.len();
    storage.write(&key, bytes_stream(bytes)).await?;

    // `.returning({ old: users.image, ... })` returns the row AFTER the
    // update, so `old` is the new key and the discard below is a no-op: the
    // previous object is left behind (an upstream quirk, kept).
    let previous: Option<(Option<String>, Option<String>)> = sqlx::query_as(
        r#"update "user" set "image" = $1, "updated_at" = $2 where "user"."id" = $3 returning "image", "user_code""#,
    )
    .bind(&key)
    .bind(JsDate::now())
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    let (old, user_code) = previous.unwrap_or((None, None));
    discard(storage, old.as_deref(), Some(&key)).await;

    Ok(Ok(SavedAvatar { url: avatar_url(user_code.as_deref(), Some(&key)), bytes: size, animated, degraded }))
}

/// `clearAvatar(userId)`: back to initials.
pub async fn clear_avatar(db: &PgPool, storage: &dyn StorageDriver, user_id: &str) -> anyhow::Result<()> {
    let previous: Option<Option<String>> = sqlx::query_scalar(
        r#"update "user" set "image" = null, "updated_at" = $1 where "user"."id" = $2 returning "image""#,
    )
    .bind(JsDate::now())
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    discard(storage, previous.flatten().as_deref(), None).await;
    Ok(())
}

/// Delete the object a column used to point at. Never fails; only our own
/// avatar keys are ever handed to the driver.
async fn discard(storage: &dyn StorageDriver, old: Option<&str>, keep: Option<&str>) {
    if !is_avatar_key(old) || old == keep {
        return;
    }
    if let Some(old) = old {
        let _ = storage.delete(old).await;
    }
}
