//! Port of `lib/media/convert-in-place.ts`: converting a file that already
//! exists - either rewriting its bytes in place, or producing a second file
//! beside it.
//!
//! Deliberately a separate module from the /convert route. That endpoint
//! READS: a public visitor asks for a copy in another format and the stored
//! file is never touched. This one WRITES, destructively in `replace` mode,
//! and the two must not share a code path where a later edit could confuse
//! them.
//!
//! ORDER OF OPERATIONS, and it is the whole design:
//!
//! ```text
//!   1. row, authorization, state gates, target admission   (no side effects)
//!   2. size ceiling, disk headroom                         (cheap refusals)
//!   3. limiter slot                                        (shared with /convert)
//!   4. produce the result into a buffer or a scratch file
//!   5. write it under a NEW storage key, and stat it back
//!   6. one transaction: lock the row, charge the quota, rewrite the row
//!   7. only then: drop the old bytes and the derived cache
//! ```
//!
//! No step can leave a row pointing at bytes that do not exist. The row moves
//! only onto a key that has already been written AND verified, and the old
//! object is removed only after the row has stopped referring to it. Every
//! failure in between leaves at most an orphaned object on disk, which the
//! admin's stray file scan already reports, and never a broken link.
//!
//! WHAT MAKES THIS SAFE FOR LINKS: `code` is not touched. The extension in a
//! public URL is decorative, so /{tag}/{code}.mp4 keeps resolving to the row
//! that now holds a GIF.

use std::path::{Path, PathBuf};

use bytes::Bytes;
use rand::RngCore;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::db::models::{Columns, Upload, User};
use crate::db::types::JsDate;
use crate::ids::secure_token;
use crate::media::abort::AbortSignal;
use crate::media::av_action::source_kind_of;
use crate::media::av_convert::{PlanInput, plan_av, run_av_plan};
use crate::media::av_cost::{
    AV_MAX_INPUT_BYTES, AV_PROBE_TIMEOUT_MS, AV_WEIGHT, AvBudget, AvBudgetInput, av_budget, av_timeout_for,
};
use crate::media::av_meta::total_source_pixels;
use crate::media::av_probe::probe_av;
use crate::media::av_targets::{AvCost, AvOption, AvSourceSize, VideoContainer, audio_copyable_to, av_option_for};
use crate::media::av_workspace::{AvWorkspace, OpenedWorkspace, open_av_workspace};
use crate::media::convert::{ConvertOptions, conversion_budget_for, convert_image_buffer};
use crate::media::convert_plan::{
    ConversionRowPatch, ConvertCandidate, ConvertFailure, ConvertMode, ConvertOutcome, ConvertPlan, ConvertedBytes,
    PatchSource, conversion_row_patch, plan_conversion, quota_delta_for,
};
use crate::media::convert_targets::ConvertTarget;
use crate::media::derived_cache::purge_derived_lazy;
use crate::media::download_menu::Engine;
use crate::media::ffmpeg::ffmpeg_available;
use crate::media::image::image_meta_from_buffer;
use crate::media::paths::{suffix_key, swap_filename_ext, swap_key_ext};
use crate::media::probe::probe_image;
use crate::media::transform_cost::{CONVERT_TIMEOUT_MS, CONVERT_WEIGHT, MAX_TRANSFORM_BYTES};
use crate::media::transform_queue::{AbortedError, BusyError, RunOptions, TimeoutError, image_transforms};
use crate::media::urls::storage_key_for;
use crate::state::AppState;
use crate::upload::disk::{fits_on_disk, free_bytes_for_uploads};
use crate::upload::finalize::allocate_code_for;

/// Who is asking. `role` comes from the session, never from the caller's input.
#[derive(Debug, Clone)]
pub struct ConvertActor {
    pub id: String,
    pub role: Option<String>,
}

/// `ConvertInPlaceInput`
pub struct ConvertInPlaceInput {
    pub upload_id: String,
    /// A target name from the same table the menu renders.
    pub to: String,
    pub mode: ConvertMode,
    pub actor: ConvertActor,
    pub signal: Option<AbortSignal>,
}

/// Largest result we will read back into memory to measure (dimensions,
/// frame count, blurhash). Above it the picture metadata is simply left
/// unknown, which the whole app already treats as normal.
const IMAGE_META_MAX_BYTES: u64 = 64 * 1024 * 1024;

fn failed(upload_id: &str, reason: ConvertFailure) -> ConvertOutcome {
    ConvertOutcome::Failed { upload_id: upload_id.to_string(), reason }
}

/// `convertUploadInPlace(input)`. Errors are what upstream let escape: a
/// database error reading the row, or allocating a code for a copy.
pub async fn convert_upload_in_place(state: &AppState, input: ConvertInPlaceInput) -> anyhow::Result<ConvertOutcome> {
    let ConvertInPlaceInput { upload_id, to, mode, actor, signal } = input;

    // `$1::uuid`: node-postgres sends the id untyped and Postgres infers the
    // column's type, so a malformed id is the same error ("invalid input
    // syntax for type uuid") on both sides.
    let sql = format!(r#"select {} from {} where "uploads"."id" = $1::uuid limit 1"#, Upload::COLUMNS, Upload::TABLE);
    let media: Option<Upload> = sqlx::query_as(&sql).bind(&upload_id).fetch_optional(&state.db).await?;
    let Some(media) = media else {
        return Ok(failed(&upload_id, ConvertFailure::NotFound));
    };
    let id = media.id.to_string();

    // Owner or admin, exactly as every other write action in the panel
    // decides it. An orphaned upload belongs to nobody, so only an admin may
    // touch it.
    let is_admin = actor.role.as_deref() == Some("admin");
    if !is_admin && media.owner_id.as_deref().is_none_or(|o| o.is_empty() || o != actor.id) {
        return Ok(failed(&id, ConvertFailure::Forbidden));
    }

    let candidate = ConvertCandidate::from(&media);
    let (engine, _ext, _mime) = match plan_conversion(&candidate, &to, mode, JsDate::now()) {
        ConvertPlan::Ok { engine, ext, mime } => (engine, ext, mime),
        ConvertPlan::Skip(reason) => return Ok(ConvertOutcome::Skipped { upload_id: id, reason }),
    };

    // A copy is a NEW file: it needs a code under an owner's tag and its bytes
    // are charged to an owner's quota. An orphan has neither.
    if mode == ConvertMode::Copy && media.owner_id.as_deref().is_none_or(str::is_empty) {
        return Ok(failed(&id, ConvertFailure::Forbidden));
    }

    let limit = if engine == Engine::Av { AV_MAX_INPUT_BYTES } else { MAX_TRANSFORM_BYTES };
    if media.size as f64 > limit as f64 {
        return Ok(failed(&id, ConvertFailure::TooLarge));
    }

    // The result's size is not knowable before it exists, so the source's
    // size is the estimate; the real size is checked again before the write.
    if !has_headroom(state, media.size as f64).await {
        return Ok(failed(&id, ConvertFailure::NoSpace));
    }

    let option = if engine == Engine::Av {
        let size = AvSourceSize { width: media.width.map(f64::from), height: media.height.map(f64::from) };
        av_option_for(Some(&media.mime_type), Some(&media.ext), media.frames.map(f64::from), Some(&to), Some(&size))
    } else {
        None
    };
    if engine == Engine::Av && option.is_none() {
        return Ok(failed(&id, ConvertFailure::UnsupportedTarget));
    }
    let image_target = ConvertTarget::parse(Some(&to));
    if engine == Engine::Image && image_target.is_none() {
        return Ok(failed(&id, ConvertFailure::UnsupportedTarget));
    }

    let job_state = state.clone();
    let job_media = media.clone();
    let run = image_transforms()
        .run(
            move |signal| async move {
                Ok(match option {
                    Some(option) => produce_av(&job_state, &job_media, option, signal).await?,
                    None => produce_image(&job_state, &job_media, image_target.expect("checked above"), signal).await?,
                })
            },
            RunOptions {
                weight: Some(if option.is_some() { AV_WEIGHT } else { CONVERT_WEIGHT }),
                timeout_ms: Some(match option {
                    Some(o) => av_timeout_for(o.to, o.cost),
                    None => CONVERT_TIMEOUT_MS,
                }),
                signal,
            },
        )
        .await;
    let produced = match run {
        Ok(Ok(p)) => p,
        Ok(Err(reason)) => return Ok(failed(&id, reason)),
        Err(err) => return Ok(failed(&id, limiter_failure(&err))),
    };

    let outcome = if mode == ConvertMode::Copy {
        commit_copy(state, &media, &produced).await
    } else {
        commit_replace(state, &media, &produced).await
    };
    produced.dispose().await;
    outcome
}

// ---------------------------------------------------------------------------
// Producing the bytes
// ---------------------------------------------------------------------------

/// A finished result, not yet in storage: a buffer (libvips, bounded by
/// MAX_TRANSFORM_BYTES on the way in) or a scratch file (ffmpeg, whose outputs
/// are as large as their sources and must never be held in memory).
struct Produced {
    buffer: Option<Bytes>,
    path: Option<PathBuf>,
    size: u64,
    checksum: String,
    mime: String,
    ext: String,
    width: Option<i32>,
    height: Option<i32>,
    frames: Option<i32>,
    duration_seconds: Option<f64>,
    fps: Option<f64>,
    blurhash: Option<String>,
    /// Something true about the result that was not asked for.
    note: Option<String>,
    workspace: Option<AvWorkspace>,
}

impl Produced {
    async fn dispose(self) {
        if let Some(ws) = self.workspace {
            ws.dispose().await;
        }
    }
}

fn to_i32(v: Option<u32>) -> Option<i32> {
    v.and_then(|n| i32::try_from(n).ok())
}

fn f64_to_i32(v: Option<f64>) -> Option<i32> {
    v.filter(|n| n.is_finite()).map(|n| n as i32)
}

async fn produce_image(
    state: &AppState,
    media: &Upload,
    to: ConvertTarget,
    signal: AbortSignal,
) -> anyhow::Result<Result<Produced, ConvertFailure>> {
    let src = state.storage.get()?.read_buffer(&media.storage_key).await?;

    // Ask libvips whether it can open this at all before committing to the
    // work, so a format we merely classify as an image fails as "this cannot
    // be converted" rather than as a thrown encoder.
    let Some(probe) = probe_image(src).await else {
        return Ok(Err(ConvertFailure::Encoder));
    };
    let budget = conversion_budget_for(&probe, to);
    if !budget.ok {
        return Ok(Err(ConvertFailure::TooLarge));
    }
    let converted = convert_image_buffer(
        probe.input.clone(),
        to,
        ConvertOptions { animated: budget.animated, timeout_ms: Some(CONVERT_TIMEOUT_MS), signal: Some(signal) },
    )
    .await?;
    let buffer = Bytes::from(converted.buffer);
    let meta =
        if converted.mime.starts_with("image/") { Some(image_meta_from_buffer(buffer.clone()).await) } else { None };
    Ok(Ok(Produced {
        size: buffer.len() as u64,
        checksum: hex::encode(Sha256::digest(&buffer)),
        mime: converted.mime.to_string(),
        ext: converted.ext.to_string(),
        width: to_i32(meta.as_ref().and_then(|m| m.width)),
        height: to_i32(meta.as_ref().and_then(|m| m.height)),
        frames: to_i32(meta.as_ref().and_then(|m| m.frames)),
        // A still or an animation has no playing time and no frame rate in
        // the sense these columns mean, and the source's would be a leftover.
        duration_seconds: None,
        fps: None,
        blurhash: meta.and_then(|m| m.blurhash),
        note: None,
        buffer: Some(buffer),
        path: None,
        workspace: None,
    }))
}

async fn produce_av(
    state: &AppState,
    media: &Upload,
    option: AvOption,
    signal: AbortSignal,
) -> anyhow::Result<Result<Produced, ConvertFailure>> {
    if !ffmpeg_available().await {
        return Ok(Err(ConvertFailure::Encoder));
    }
    let storage = state.storage.get()?;
    let ws = match open_av_workspace(storage.as_ref(), &state.config, &media.storage_key, AV_MAX_INPUT_BYTES).await? {
        OpenedWorkspace::Ready(ws) => ws,
        OpenedWorkspace::Missing => return Ok(Err(ConvertFailure::Storage)),
        OpenedWorkspace::TooLarge { .. } => return Ok(Err(ConvertFailure::TooLarge)),
    };
    // Every early return below drops `ws`, which removes the scratch directory.

    let input = ws.input.to_string_lossy().into_owned();
    let Some(probe) = probe_av(&input, AV_PROBE_TIMEOUT_MS, Some(signal.clone())).await? else {
        return Ok(Err(ConvertFailure::Encoder));
    };

    let source_kind = source_kind_of(media);
    let budget = av_budget(
        option.to,
        option.cost,
        &AvBudgetInput {
            bytes: media.size.max(0) as u64,
            duration: probe.duration,
            source_kind: Some(source_kind),
            audio_copyable: audio_copyable_to(VideoContainer::Mp4, probe.audio.as_ref().map(|a| a.codec.as_str())),
            pixels: total_source_pixels(&probe),
        },
        // No caller-chosen clip: an in-place conversion has no query string
        // to take one from, so the defaults in av_cost apply.
        None,
    );
    let clip = match budget {
        AvBudget::Ok { clip_seconds } => clip_seconds,
        AvBudget::Refused { .. } => return Ok(Err(ConvertFailure::TooLarge)),
    };

    let plan = match plan_av(&PlanInput {
        option,
        probe: &probe,
        source_kind,
        workspace: &ws,
        clip_seconds: clip,
        offset_seconds: None,
    })? {
        Ok(plan) => plan,
        Err(_) => return Ok(Err(ConvertFailure::Encoder)),
    };

    run_av_plan(&plan, Some(signal.clone())).await?;

    let size = match tokio::fs::metadata(&plan.output_path).await {
        Ok(m) if m.is_file() && m.len() > 0 => m.len(),
        _ => return Ok(Err(ConvertFailure::Encoder)),
    };
    let checksum = hash_file(&plan.output_path).await?;

    // When the picture was re-encoded the source probe has stopped describing
    // the result: one more ffprobe of what was actually written.
    let remeasured = if plan.remeasure {
        probe_av(&plan.output_path.to_string_lossy(), AV_PROBE_TIMEOUT_MS, Some(signal)).await.ok().flatten()
    } else {
        None
    };

    // A GIF or an animated WebP is a picture, and the dashboard wants a
    // picture's metadata for it. Anything else keeps the source's frame size
    // where that is still true (a container copy) and nothing where it is not.
    let (mut width, mut height, mut frames, mut duration_seconds, mut fps, mut blurhash) =
        (None, None, None, None, None, None);
    if plan.mime.starts_with("image/") && size <= IMAGE_META_MAX_BYTES {
        // Metadata is a nicety; a result we cannot measure is still a result.
        if let Ok(buf) = tokio::fs::read(&plan.output_path).await {
            let meta = image_meta_from_buffer(Bytes::from(buf)).await;
            width = to_i32(meta.width);
            height = to_i32(meta.height);
            frames = to_i32(meta.frames);
            blurhash = meta.blurhash;
        }
    } else if plan.mime.starts_with("video/") {
        let shape = remeasured.as_ref().and_then(|r| r.video.as_ref()).or(probe.video.as_ref());
        width = f64_to_i32(shape.and_then(|s| s.width));
        height = f64_to_i32(shape.and_then(|s| s.height));
    }

    // Playing time survives every audio/video target on the menu; the only
    // targets that SHORTEN anything produce an image (the branch above).
    if plan.mime.starts_with("video/") || plan.mime.starts_with("audio/") {
        duration_seconds = remeasured.as_ref().and_then(|r| r.duration).or(probe.duration);
        // Frame rate and count are carried over only when not one sample is
        // touched, unless the result was measured.
        let counted = remeasured.as_ref().and_then(|r| r.video.as_ref()).or(if option.cost == AvCost::Copy {
            probe.video.as_ref()
        } else {
            None
        });
        if let Some(c) = counted {
            fps = c.fps;
            frames = f64_to_i32(c.frames);
        }
    }

    Ok(Ok(Produced {
        buffer: None,
        path: Some(plan.output_path.clone()),
        size,
        checksum,
        mime: plan.mime.to_string(),
        ext: plan.ext.to_string(),
        width,
        height,
        frames,
        duration_seconds,
        fps,
        blurhash,
        note: plan.note.map(str::to_string),
        workspace: Some(ws),
    }))
}

// ---------------------------------------------------------------------------
// Committing
// ---------------------------------------------------------------------------

/// Aborts a transaction with a reason the caller can report.
#[derive(Debug, thiserror::Error)]
#[error("convert aborted: {}", .0.as_str())]
struct ConvertAbort(ConvertFailure);

fn abort_reason(err: &anyhow::Error, otherwise: ConvertFailure) -> ConvertFailure {
    err.downcast_ref::<ConvertAbort>().map(|a| a.0).unwrap_or(otherwise)
}

fn to_bytes(produced: &Produced, storage_key: String) -> ConvertedBytes {
    ConvertedBytes {
        storage_key,
        size: produced.size as i64,
        checksum: produced.checksum.clone(),
        mime: produced.mime.clone(),
        ext: produced.ext.clone(),
        width: produced.width,
        height: produced.height,
        frames: produced.frames,
        duration_seconds: produced.duration_seconds,
        fps: produced.fps,
        blurhash: produced.blurhash.clone(),
    }
}

async fn commit_replace(state: &AppState, media: &Upload, produced: &Produced) -> anyhow::Result<ConvertOutcome> {
    let id = media.id.to_string();
    let stored = match store_produced(state, produced, &swap_key_ext(&media.storage_key, &produced.ext)).await {
        None => return Ok(failed(&id, ConvertFailure::NoSpace)),
        Some(Err(reason)) => return Ok(failed(&id, reason)),
        Some(Ok(key)) => key,
    };
    let patch = conversion_row_patch(
        &PatchSource { ext: Some(&media.ext), filename: &media.filename, original_ext: media.original_ext.as_deref() },
        &to_bytes(produced, stored.clone()),
        JsDate::now(),
    );

    let tx_result: anyhow::Result<i64> = async {
        let mut tx = state.db.begin().await?;
        let locked: Option<(String, i64, Option<String>)> = sqlx::query_as(
            r#"select "storage_key", "size", "owner_id" from "uploads" where "uploads"."id" = $1 limit 1 for update"#,
        )
        .bind(media.id)
        .fetch_optional(&mut *tx)
        .await?;
        // Somebody else changed the bytes while we were encoding (another
        // conversion, a delete): ours describe a file that is no longer current.
        let Some((storage_key, size, owner_id)) = locked else {
            return Err(ConvertAbort(ConvertFailure::Conflict).into());
        };
        if storage_key != media.storage_key {
            return Err(ConvertAbort(ConvertFailure::Conflict).into());
        }
        // Read UNDER the lock, never from the snapshot taken before the
        // encode: two conversions of the same file would otherwise both
        // compute their delta from the same "before" size.
        let previous = size;
        let delta = quota_delta_for(ConvertMode::Replace, previous, produced.size as i64);
        if let Some(owner) = owner_id.as_deref().filter(|o| !o.is_empty()) {
            charge_quota(&mut tx, owner, delta).await?;
        }
        let updated = apply_patch(&mut tx, media, &patch).await?;
        if updated == 0 {
            return Err(ConvertAbort(ConvertFailure::Conflict).into());
        }
        tx.commit().await?;
        Ok(previous)
    }
    .await;
    let previous_size = match tx_result {
        Ok(previous) => previous,
        Err(err) => {
            // Ours are the only bytes nobody is pointing at, so ours are the ones to go.
            if let Ok(driver) = state.storage.get() {
                let _ = driver.delete(&stored).await;
            }
            return Ok(failed(&id, abort_reason(&err, ConvertFailure::Storage)));
        }
    };

    // Past the commit. Everything below is cleanup: it may fail without making
    // the conversion any less complete, and none of it may throw.
    purge_derived_lazy(&state.storage, &id).await;
    if let Ok(driver) = state.storage.get() {
        if stored != media.storage_key
            && let Err(err) = driver.delete(&media.storage_key).await
        {
            tracing::error!("[convert] old object left behind: {} {err}", media.storage_key);
        }
        if let Some(thumb) = media.thumb_key.as_deref().filter(|t| !t.is_empty()) {
            let _ = driver.delete(thumb).await;
        }
    }

    Ok(ConvertOutcome::Converted {
        upload_id: id,
        code: media.code.clone(),
        ext: produced.ext.clone(),
        mime_type: produced.mime.clone(),
        size: produced.size as i64,
        // `Number(row.size) || 0`
        previous_size,
        previous_ext: media.ext.clone(),
        note: produced.note.clone(),
    })
}

/// The `UPDATE uploads SET <patch> WHERE id = $ AND storage_key = $ RETURNING id`.
async fn apply_patch(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    media: &Upload,
    p: &ConversionRowPatch,
) -> anyhow::Result<u64> {
    let rows = sqlx::query(
        r#"update "uploads" set "ext" = $1, "mime_type" = $2, "size" = $3, "storage_key" = $4, "checksum" = $5,
             "width" = $6, "height" = $7, "frames" = $8, "duration_seconds" = $9, "fps" = $10, "blurhash" = $11,
             "thumb_key" = $12, "filename" = $13, "converted_at" = $14, "original_ext" = $15, "updated_at" = $16
           where ("uploads"."id" = $17 and "uploads"."storage_key" = $18) returning "id""#,
    )
    .bind(&p.ext)
    .bind(&p.mime_type)
    .bind(p.size)
    .bind(&p.storage_key)
    .bind(&p.checksum)
    .bind(p.width)
    .bind(p.height)
    .bind(p.frames)
    .bind(p.duration_seconds)
    .bind(p.fps)
    .bind(&p.blurhash)
    .bind(&p.thumb_key)
    .bind(&p.filename)
    .bind(p.converted_at)
    .bind(&p.original_ext)
    .bind(p.updated_at)
    .bind(media.id)
    .bind(&media.storage_key)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.len() as u64)
}

async fn commit_copy(state: &AppState, media: &Upload, produced: &Produced) -> anyhow::Result<ConvertOutcome> {
    let source_id = media.id.to_string();
    let Some(owner_id) = media.owner_id.clone().filter(|o| !o.is_empty()) else {
        return Ok(failed(&source_id, ConvertFailure::Forbidden));
    };
    let sql = format!(r#"select {} from {} where "user"."id" = $1 limit 1"#, User::COLUMNS, User::TABLE);
    let owner: Option<User> = sqlx::query_as(&sql).bind(&owner_id).fetch_optional(&state.db).await?;
    let Some(owner) = owner else {
        return Ok(failed(&source_id, ConvertFailure::Forbidden));
    };

    let filename = swap_filename_ext(&media.filename, &produced.ext);
    let code = allocate_code_for(&state.db, &owner, &filename).await?;
    let key = storage_key_for(owner.user_code.as_deref().unwrap_or("u"), &code, &produced.ext);

    let stored = match store_produced(state, produced, &key).await {
        None => return Ok(failed(&source_id, ConvertFailure::NoSpace)),
        Some(Err(reason)) => return Ok(failed(&source_id, reason)),
        Some(Ok(key)) => key,
    };

    let created: anyhow::Result<String> = async {
        let mut tx = state.db.begin().await?;
        // A copy is a whole new file, so it is charged in full - the same
        // conditional UPDATE the upload path uses.
        charge_quota(&mut tx, &owner_id, produced.size as i64).await?;
        let id: uuid::Uuid = sqlx::query_scalar(
            r#"insert into "uploads" ("owner_id", "code", "filename", "ext", "kind", "language", "mime_type", "size",
                 "storage_key", "checksum", "width", "height", "blurhash", "published", "delete_token", "expires_at",
                 "max_views", "password_hash", "folder_id", "password_version", "frames", "converted_at", "original_ext")
               values ($1, $2, $3, $4, 'file', null, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18,
                 $19, $20, $21)
               returning "id""#,
        )
        .bind(&owner_id)
        .bind(&code)
        .bind(&filename)
        .bind(&produced.ext)
        .bind(&produced.mime)
        .bind(produced.size as i64)
        .bind(&stored)
        .bind(&produced.checksum)
        .bind(produced.width)
        .bind(produced.height)
        .bind(&produced.blurhash)
        // Inherited from the SOURCE ROW, not from the account defaults: the
        // only direction that leaks is a copy of a private file coming out
        // public. The password hash travels WITH its generation; policy
        // travels, history (the view count) does not.
        .bind(media.published)
        .bind(secure_token(16))
        .bind(media.expires_at)
        .bind(media.max_views)
        .bind(&media.password_hash)
        .bind(media.folder_id)
        .bind(media.password_version)
        .bind(produced.frames)
        .bind(JsDate::now())
        .bind(&media.ext)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(id.to_string())
    }
    .await;

    match created {
        Ok(upload_id) => Ok(ConvertOutcome::Created {
            upload_id,
            source_id,
            code,
            ext: produced.ext.clone(),
            mime_type: produced.mime.clone(),
            size: produced.size as i64,
            note: produced.note.clone(),
        }),
        Err(err) => {
            if let Ok(driver) = state.storage.get() {
                let _ = driver.delete(&stored).await;
            }
            Ok(failed(&source_id, abort_reason(&err, ConvertFailure::Conflict)))
        }
    }
}

/// `chargeQuota(tx, ownerId, delta)`: add `delta` to the owner's usage,
/// atomically, refusing only when it would take them past their limit. The
/// predicate lives in the WHERE clause so Postgres evaluates it against the
/// row it is about to lock. A delta of zero or less is never refused.
async fn charge_quota(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    owner_id: &str,
    delta: i64,
) -> anyhow::Result<()> {
    if delta == 0 {
        return Ok(());
    }
    if delta < 0 {
        sqlx::query(r#"update "user" set "used_quota" = greatest(0, coalesce("user"."used_quota", 0) + $1) where "user"."id" = $2"#)
            .bind(delta)
            .bind(owner_id)
            .execute(&mut **tx)
            .await?;
        return Ok(());
    }
    let charged = sqlx::query(
        r#"update "user" set "used_quota" = coalesce("user"."used_quota", 0) + $1
           where ("user"."id" = $2 and (
             "user"."max_quota" is null
             or "user"."max_quota" < 0
             or coalesce("user"."used_quota", 0) + $3 <= "user"."max_quota"
           )) returning "id""#,
    )
    .bind(delta)
    .bind(owner_id)
    .bind(delta)
    .fetch_all(&mut **tx)
    .await?;
    if charged.is_empty() {
        return Err(ConvertAbort(ConvertFailure::Quota).into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

/// `storeProduced(produced, preferredKey)`: put the result in storage under
/// a key nothing else occupies, and prove it arrived whole. `None` is "no
/// headroom". The stat afterwards is not ceremony: a short write on a full
/// disk produces a truncated object that looks valid to every later reader.
async fn store_produced(
    state: &AppState,
    produced: &Produced,
    preferred_key: &str,
) -> Option<Result<String, ConvertFailure>> {
    // Re-checked with the REAL size now that it is known.
    if !has_headroom(state, produced.size as f64).await {
        return None;
    }
    let Ok(storage) = state.storage.get() else {
        return Some(Err(ConvertFailure::Storage));
    };
    let key = fresh_key(preferred_key);
    let written = match (&produced.path, &produced.buffer) {
        (Some(path), _) => storage.ingest_local_file(path, &key).await,
        (None, Some(buffer)) => storage.write(&key, crate::storage::bytes_stream(buffer.clone())).await,
        (None, None) => return Some(Err(ConvertFailure::Storage)),
    };
    let verified = match written {
        Ok(()) => matches!(storage.stat(&key).await, Ok(Some(info)) if info.size == produced.size),
        Err(_) => false,
    };
    if !verified {
        let _ = storage.delete(&key).await;
        return Some(Err(ConvertFailure::Storage));
    }
    Some(Ok(key))
}

/// `freshKey(preferred)`: a key nothing else can be using, by construction
/// rather than by checking (two concurrent conversions of the same file to
/// the same format would otherwise compute the same key, and the loser's
/// cleanup would delete the winner's bytes). 48 random bits, hex.
fn fresh_key(preferred: &str) -> String {
    let mut bytes = [0u8; 6];
    rand::thread_rng().fill_bytes(&mut bytes);
    suffix_key(preferred, &hex::encode(bytes))
}

/// `hasHeadroom(bytes)`: would `bytes` more leave the storage filesystem
/// below the operator's reserve?
async fn has_headroom(state: &AppState, bytes: f64) -> bool {
    if state.config.storage.driver != "local" {
        return true;
    }
    let floor = state.config.uploads.min_free_bytes as f64;
    if floor <= 0.0 {
        return true;
    }
    fits_on_disk(bytes, free_bytes_for_uploads(&state.config).await.map(|f| f as f64), floor)
}

async fn hash_file(path: &Path) -> anyhow::Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(hex::encode(hash.finalize()))
}

/// `limiterFailure(err)`: a limiter refusal as a reason; anything else is a
/// genuine failure.
fn limiter_failure(err: &anyhow::Error) -> ConvertFailure {
    if err.downcast_ref::<BusyError>().is_some() {
        ConvertFailure::Busy
    } else if err.downcast_ref::<TimeoutError>().is_some() {
        ConvertFailure::Timeout
    } else if err.downcast_ref::<AbortedError>().is_some() {
        ConvertFailure::Aborted
    } else {
        ConvertFailure::Encoder
    }
}
