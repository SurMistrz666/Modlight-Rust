//! Port of `lib/media/av-action.ts`: the /convert handler for audio and video.
//!
//! Split out of the route because it is a different shape of work from the
//! image conversion beside it: it spawns processes, it streams outputs too
//! large to hold, and it is the only place in the app where a public request
//! can start a program.
//!
//! Order of operations, and every step is here because skipping it costs
//! something real:
//!
//! ```text
//!   1. is this target offered for this KIND of file?       (free, static)
//!   2. may this caller ask for it?                          (free, session)
//!   3. is the file small enough?                            (one stat)
//!   4. is it cached?                                        (one stat)
//!   5. take a limiter slot
//!   6. ffprobe: what is it really, and how long?            (one cheap process)
//!   7. does the budget allow it?                            (arithmetic)
//!   8. can these codecs actually do it?                     (arithmetic)
//!   9. run ffmpeg
//! ```
//!
//! Steps 1-4 all precede the slot, so the cheap refusals stay cheap under
//! load, and step 6 precedes step 9 so nothing expensive starts before the
//! file has been measured.

use std::future::Future;
use std::pin::Pin;

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;

use crate::db::models::Upload;
use crate::http::cookies::encode_uri_component;
use crate::media::abort::AbortSignal;
use crate::media::av_convert::{PlanInput, av_variant, plan_av, run_av_plan};
use crate::media::av_cost::{
    AV_MAX_INPUT_BYTES, AV_PROBE_TIMEOUT_MS, AV_WEIGHT, AvBudget, AvBudgetInput, av_budget, av_timeout_for,
};
use crate::media::av_meta::total_source_pixels;
use crate::media::av_probe::probe_av;
use crate::media::av_targets::{
    AvCost, AvOption, AvSourceKind, AvSourceSize, VideoContainer, audio_copyable_to, av_option_for, av_output,
};
use crate::media::av_workspace::{OpenedWorkspace, open_av_workspace, read_and_dispose};
use crate::media::convert_verdict::{ConvertVerdict, VerdictKeyParts, read_verdict, remember_verdict, verdict_key};
use crate::media::derived_cache::{
    derived_key, may_cache_derived, read_derived, read_derived_stream, write_derived, write_derived_file,
};
use crate::media::ffmpeg::ffmpeg_available;
use crate::media::mime::{MediaKind, media_kind};
use crate::media::transform_queue::{RunOptions, image_transforms};
use crate::state::AppState;
use crate::storage::ByteStream;

/// A stable cache identity for the source bytes: the row's own checksum when
/// it has one and `s<size>` otherwise. Hashing a 400 MB video to decide
/// whether to convert it would cost more than the conversion.
pub fn source_tag(media: &Upload) -> String {
    media.checksum.clone().unwrap_or_else(|| format!("s{}", media.size))
}

/// The view-budget hook (`admit`): spends the file's view budget, answering
/// false when it is exhausted.
pub type AdmitHook = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send>> + Send>;

/// `AvConvertRequest`
pub struct AvConvertRequest {
    pub media: Upload,
    /// Owner or admin. Decides whether an ENCODER may be run for this caller.
    pub privileged: bool,
    pub to: Option<String>,
    /// `?t=` seconds, for a clip start.
    pub offset_seconds: Option<f64>,
    /// `?d=` seconds, for an animation clip length.
    pub clip_seconds: Option<f64>,
    pub signal: Option<AbortSignal>,
    /// Spend the file's view budget, returning false when it is exhausted.
    ///
    /// A hook rather than something the caller does around this function,
    /// because only this function knows the right MOMENT: after the target and
    /// permission checks (a request about to be answered 401 must not burn a
    /// one-time link) and before the cache lookup (a cache hit is still a
    /// download and still counts).
    pub admit: Option<AdmitHook>,
    /// Answer whether this conversion WOULD work, without producing it
    /// (`?check=1`). Every gate runs; the encoder does not. The view budget is
    /// not spent and nothing is written to the derived cache.
    pub dry_run: bool,
}

/// `avConvert(req)`: answer a /convert request for a media file, or `None`
/// when this file is not audio/video at all and the image path should have
/// it.
///
/// Errors are the limiter's (`BusyError`, `TimeoutError`, `AbortedError`) and
/// ffprobe's (`AvTimeoutError`, `AvAbortedError`), which the route turns into
/// 503/499 exactly as for the image path; anything else is a 500.
pub async fn av_convert(state: &AppState, req: AvConvertRequest) -> anyhow::Result<Option<Response>> {
    let media = &req.media;
    // The row's own dimensions are part of the question: the web-playable
    // target is offered only to files the numbers say are too big for a
    // hardware decoder, and this is the same call the menu renders from.
    let size = AvSourceSize { width: media.width.map(f64::from), height: media.height.map(f64::from) };
    let Some(option) = av_option_for(
        Some(&media.mime_type),
        Some(&media.ext),
        media.frames.map(f64::from),
        req.to.as_deref(),
        Some(&size),
    ) else {
        return Ok(None);
    };

    let media_id = media.id.to_string();
    let tag = source_tag(media);
    let memo_key = verdict_key(&VerdictKeyParts {
        media_id: &media_id,
        source_tag: &tag,
        to: option.to.as_str(),
        privileged: req.privileged,
        clip_seconds: req.clip_seconds,
        offset_seconds: req.offset_seconds,
    });

    // A refusal we have already established is served from memory for BOTH
    // kinds of request: no probe and, more importantly, no limiter slot. A
    // remembered `ok` only ever answers a dry run.
    if let Some(known) = read_verdict(&memo_key)
        && (!known.ok() || req.dry_run)
    {
        return Ok(Some(verdict_response(&known)));
    }

    // Every encoder is owner-only: a stream copy costs about what serving the
    // file costs, while an encode lets the CALLER choose the workload. 403
    // rather than 401: `privileged` is false both for a visitor who is not
    // signed in and for one signed in as somebody else.
    if option.cost == AvCost::Encode && !req.privileged {
        return Ok(Some(decided(&memo_key, ConvertVerdict::refused(403, "encode-requires-owner"))));
    }

    if !ffmpeg_available().await {
        // Deliberately NOT remembered: a property of the server, not of these bytes.
        return Ok(Some(refusal(501, "ffmpeg-unavailable", None)));
    }

    // The row already knows the size, so the cheapest refusal of all needs no
    // filesystem call and happens before the cache lookup, let alone a slot.
    if media.size as f64 > AV_MAX_INPUT_BYTES as f64 {
        return Ok(Some(decided(
            &memo_key,
            ConvertVerdict::Refused {
                status: 413,
                reason: "too-large".into(),
                limit: Some(AV_MAX_INPUT_BYTES.to_string()),
            },
        )));
    }

    // Never on a dry run: `admit` spends the file's view budget.
    let AvConvertRequest { media, admit, dry_run, signal, clip_seconds, offset_seconds, .. } = req;
    if !dry_run
        && let Some(admit) = admit
        && !admit().await?
    {
        return Ok(Some(plain(StatusCode::NOT_FOUND, "Not found")));
    }

    let (out_mime, out_ext) = av_output(option.to);
    let cacheable = may_cache_derived((&media).into());
    let key = derived_key(
        &media_id,
        &tag,
        &av_variant(option.to, clip_seconds, offset_seconds, source_kind_of(&media)),
        out_ext,
    );
    let base = base_name(&media.filename, &media.code);
    let note_key = format!("{key}.note");

    // Before the slot: a hit costs one stat and no process at all. It is also
    // the cheapest possible YES for a dry run.
    if cacheable && let Some((stream, size)) = read_derived_stream(&state.storage, &key).await {
        // The note is read back from beside the cached bytes rather than
        // recomputed: on a cache hit there is no plan to ask. Without it, only
        // the FIRST person to convert a file was told that its sound could not
        // come along.
        let note = read_cached_note(state, &note_key).await;
        if dry_run {
            drop(stream);
            return Ok(Some(decided(&memo_key, ConvertVerdict::Ok { note })));
        }
        return Ok(Some(streamed(stream, size, out_mime, out_ext, &base, note.as_deref())));
    }

    let job =
        RunOne { state: state.clone(), media, option, key, base, memo_key, dry_run, clip_seconds, offset_seconds };
    // Limiter refusals (busy, deadline, caller gone) propagate to the route.
    let response = image_transforms()
        .run(
            move |signal| job.run(signal),
            RunOptions {
                weight: Some(AV_WEIGHT),
                // A dry run stops after ffprobe, so holding it to the encode
                // deadline would let a slot sit occupied far longer than the
                // work can possibly take.
                timeout_ms: Some(if dry_run {
                    AV_PROBE_TIMEOUT_MS * 2
                } else {
                    av_timeout_for(option.to, option.cost)
                }),
                signal,
            },
        )
        .await?;
    Ok(Some(response))
}

struct RunOne {
    state: AppState,
    media: Upload,
    option: AvOption,
    key: String,
    base: String,
    memo_key: String,
    dry_run: bool,
    clip_seconds: Option<f64>,
    offset_seconds: Option<f64>,
}

impl RunOne {
    async fn run(self, signal: AbortSignal) -> anyhow::Result<Response> {
        let RunOne { state, media, option, key, base, memo_key, dry_run, clip_seconds, offset_seconds } = self;
        let (out_mime, out_ext) = av_output(option.to);
        let cacheable = may_cache_derived((&media).into());

        let storage = state.storage.get()?;
        let opened = open_av_workspace(storage.as_ref(), &state.config, &media.storage_key, AV_MAX_INPUT_BYTES).await?;
        let ws = match opened {
            OpenedWorkspace::Ready(ws) => ws,
            OpenedWorkspace::Missing => return Ok(refusal(404, "missing", None)),
            OpenedWorkspace::TooLarge { .. } => {
                return Ok(refusal(413, "too-large", Some(&AV_MAX_INPUT_BYTES.to_string())));
            }
        };
        // From here every early return drops `ws`, which removes the scratch
        // directory (upstream's `dispose()` on each path).

        let Some(probe) = probe_av(&ws.input.to_string_lossy(), AV_PROBE_TIMEOUT_MS, Some(signal.clone())).await?
        else {
            ws.dispose().await;
            return Ok(decided(&memo_key, ConvertVerdict::refused(415, "unreadable")));
        };

        let source_kind = source_kind_of(&media);
        let budget = av_budget(
            option.to,
            option.cost,
            &AvBudgetInput {
                bytes: media.size.max(0) as u64,
                duration: probe.duration,
                source_kind: Some(source_kind),
                // Whether MP4 will take the sound untouched moves the length
                // ceiling for "audio as video" by a factor of five.
                audio_copyable: audio_copyable_to(VideoContainer::Mp4, probe.audio.as_ref().map(|a| a.codec.as_str())),
                // Picture pixels in the whole file: the meter a full
                // re-encode is budgeted against.
                pixels: total_source_pixels(&probe),
            },
            clip_seconds,
        );
        let clip = match budget {
            AvBudget::Ok { clip_seconds } => clip_seconds,
            AvBudget::Refused { reason, limit } => {
                ws.dispose().await;
                return Ok(decided(
                    &memo_key,
                    ConvertVerdict::Refused {
                        status: 413,
                        reason: reason.as_str().into(),
                        limit: Some(limit.to_string()),
                    },
                ));
            }
        };

        let plan = plan_av(&PlanInput {
            option,
            probe: &probe,
            source_kind,
            workspace: &ws,
            clip_seconds: clip,
            offset_seconds,
        })?;
        let plan = match plan {
            Ok(plan) => plan,
            Err(refuse) => {
                ws.dispose().await;
                // 409, not 415: the FORMAT is convertible in general, this
                // particular file's codecs are what decline.
                return Ok(decided(&memo_key, ConvertVerdict::refused(409, refuse.as_str())));
            }
        };

        // Everything that can refuse has now spoken: the end of a dry run.
        if dry_run {
            ws.dispose().await;
            let note = plan.note.map(str::to_string);
            remember_verdict(&memo_key, ConvertVerdict::Ok { note: note.clone() });
            return Ok(no_content(note.as_deref()));
        }

        run_av_plan(&plan, Some(signal)).await?;

        let produced = match tokio::fs::metadata(&plan.output_path).await {
            Ok(m) if m.is_file() && m.len() > 0 => m.len(),
            _ => {
                ws.dispose().await;
                return Ok(refusal(500, "empty-output", None));
            }
        };

        // Written to the cache BEFORE the response streams, because the
        // response stream owns the workspace and deletes it on close.
        if cacheable {
            write_derived_file(&state.storage, &key, &plan.output_path).await;
            write_cached_note(&state, &format!("{key}.note"), plan.note).await;
        }

        // read_and_dispose hands ownership of the workspace to the stream, so
        // the scratch directory is removed when the client finishes or hangs up.
        let body = read_and_dispose(&plan.output_path, ws).await?;
        Ok(streamed(body, produced, out_mime, out_ext, &base, plan.note))
    }
}

/// What the ROW says the source is - never re-derived from the probe.
pub fn source_kind_of(media: &Upload) -> AvSourceKind {
    match media_kind(Some(&media.mime_type), Some(&media.ext)) {
        MediaKind::Audio => AvSourceKind::Audio,
        MediaKind::Image => AvSourceKind::ImageSequence,
        _ => AvSourceKind::Video,
    }
}

/// `media.filename.replace(/\.[^.]+$/, "") || media.code`
pub fn base_name(filename: &str, code: &str) -> String {
    let stem = match filename.rfind('.') {
        Some(i) if i + 1 < filename.len() => &filename[..i],
        _ => filename,
    };
    if stem.is_empty() { code.to_string() } else { stem.to_string() }
}

/// `new Response(text, { status })`: the Fetch API's default type for a
/// string body.
pub(crate) fn plain(status: StatusCode, text: &str) -> Response {
    let mut res = Response::new(Body::from(text.to_string()));
    *res.status_mut() = status;
    res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain;charset=UTF-8"));
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

fn no_content(note: Option<&str>) -> Response {
    let mut res = Response::new(Body::empty());
    *res.status_mut() = StatusCode::NO_CONTENT;
    if let Some(n) = note.and_then(|n| HeaderValue::from_str(n).ok()) {
        res.headers_mut().insert("x-convert-note", n);
    }
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

fn streamed(body: ByteStream, size: u64, mime: &str, ext: &str, base: &str, note: Option<&str>) -> Response {
    let mut res = Response::new(Body::from_stream(body));
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_str(mime).expect("static mime"));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(size));
    let disposition = format!("attachment; filename*=UTF-8''{}", encode_uri_component(&format!("{base}.{ext}")));
    if let Ok(v) = HeaderValue::from_str(&disposition) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    // Something true about the result that was not asked for. A header rather
    // than a silent difference between what was requested and what arrived.
    if let Some(n) = note.and_then(|n| HeaderValue::from_str(n).ok()) {
        h.insert("x-convert-note", n);
    }
    // Our own disk caches the bytes, but the response must not enter a shared
    // cache: the file behind it can be unpublished or deleted at any moment.
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

/// The note that belongs to a cached conversion, stored beside its bytes. A
/// property of the RESULT, so it has to outlive the run.
async fn read_cached_note(state: &AppState, key: &str) -> Option<String> {
    let buf = read_derived(&state.storage, key).await?;
    let text = String::from_utf8_lossy(&buf);
    let text = crate::config::js_trim(&text);
    (!text.is_empty()).then(|| text.to_string())
}

async fn write_cached_note(state: &AppState, key: &str, note: Option<&str>) {
    // A missing note costs a missing warning, not a missing file.
    if let Some(note) = note {
        write_derived(&state.storage, key, bytes::Bytes::from(note.to_string())).await;
    }
}

/// Remember a verdict and answer with it in one step.
fn decided(key: &str, verdict: ConvertVerdict) -> Response {
    remember_verdict(key, verdict.clone());
    verdict_response(&verdict)
}

/// `verdictResponse(verdict)`: the response for a verdict, whether it was
/// just decided or recalled. The note is carried on EVERY yes.
pub fn verdict_response(verdict: &ConvertVerdict) -> Response {
    match verdict {
        ConvertVerdict::Ok { note } => no_content(note.as_deref()),
        ConvertVerdict::Refused { status, reason, limit } => refusal(*status, reason, limit.as_deref()),
    }
}

/// `refusal(status, reason, extra)`: a refusal the UI can translate. The
/// reason travels as a header token; the body stays a short English fallback
/// for anything reading the endpoint directly (curl, ShareX, a script).
pub fn refusal(status: u16, reason: &str, limit: Option<&str>) -> Response {
    let text = REASON_TEXT.iter().find(|(k, _)| *k == reason).map(|(_, t)| *t).unwrap_or(reason);
    let mut res = Response::new(Body::from(text.to_string()));
    *res.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    if let Ok(v) = HeaderValue::from_str(reason) {
        h.insert("x-convert-refused", v);
    }
    if let Some(l) = limit.and_then(|l| HeaderValue::from_str(l).ok()) {
        h.insert("limit", l);
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

/// `REASON_TEXT`
pub const REASON_TEXT: [(&str, &str); 12] = [
    (
        "encode-requires-owner",
        "Sign in as the owner of this file to convert it. Re-encoding is not available on public links.",
    ),
    ("ffmpeg-unavailable", "Audio and video conversion is not available on this server."),
    (
        "video-codec",
        "This file's video cannot be placed in that format without re-encoding, which is too expensive to do on demand. Try MKV, which accepts it as it is.",
    ),
    ("audio-codec", "This file's audio track cannot be copied into the requested container."),
    ("no-video", "This file has no video track."),
    ("no-audio", "This file has no audio track."),
    ("unreadable", "This file could not be read as audio or video."),
    ("too-large", "This file is too large to convert."),
    ("too-long", "This file is too long to convert."),
    ("too-many-pixels", "This recording holds too many frames of too many pixels to re-encode on demand."),
    ("empty-output", "The conversion produced no output."),
    ("missing", "Not found"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_name_strips_one_extension() {
        assert_eq!(base_name("clip.final.mp4", "abc"), "clip.final");
        assert_eq!(base_name("noext", "abc"), "noext");
        assert_eq!(base_name(".mp4", "abc"), "abc");
        assert_eq!(base_name("name.", "abc"), "name.");
        assert_eq!(base_name("", "abc"), "abc");
    }

    #[test]
    fn refusals_carry_the_token_and_the_limit() {
        let r = refusal(413, "too-large", Some("536870912"));
        assert_eq!(r.status(), 413);
        assert_eq!(r.headers()["x-convert-refused"], "too-large");
        assert_eq!(r.headers()["limit"], "536870912");
        assert_eq!(r.headers()["content-type"], "text/plain; charset=utf-8");
        let unknown = refusal(409, "something-else", None);
        assert_eq!(unknown.headers()["x-convert-refused"], "something-else");
    }
}
