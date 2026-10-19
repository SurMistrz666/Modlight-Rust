//! The calls the serving half makes into the PROCESSING half of `lib/media`,
//! each under its upstream name.
//!
//! The media serving port (`media::serve`, `routes::media_action`, the public
//! page loaders) was written in parallel with the processing port, against the
//! upstream signatures, and calls the other half only through this module.
//! Since both landed it is a thin adapter: every function here delegates to
//! the processing module named beside it (SKELETON section 12), converting
//! only the shapes the two ports chose differently.
//!
//! | here | processing module |
//! |---|---|
//! | [`docx_support`], [`MAX_DOCX_BYTES`] | `lib/media/docx-support.ts` (kept here until the docx port lands) |
//! | [`shows_source`], [`inline_text_plan`] | `media::source_preview` |
//! | [`can_timeline_preview`], `TIMELINE_*_ACTION`, [`TimelineRefusal`] | `media::timeline_preview` |
//! | [`diagnose_playback`], [`playback_source`] | `media::playback_health` |
//! | [`conversion_budget`], [`convert_image_buffer`] | `media::convert` |
//! | [`read_derived`], [`write_derived`] | `media::derived_cache` |
//! | [`video_poster`] | `media::poster` |
//! | [`motion_preview`], [`MotionRefusal`] | `media::motion`, `media::motion_preview` |
//! | [`timeline_track`], [`timeline_sheet`] | `media::timeline` |
//! | [`av_convert`] | `media::av_action` |
//! | [`docx_to_html`] | `lib/media/docx.ts` (the docx port; until it lands, an error) |
//! | [`render_card`] | `media::card_render` |

use axum::response::Response;
use bytes::Bytes;
use futures_util::future::BoxFuture;

use crate::db::models::Upload;
use crate::media::abort::AbortSignal;
use crate::media::card::CardChoice;
use crate::media::convert_targets::ConvertTarget;
use crate::media::playback_health::PlaybackSource;
use crate::media::source_preview::SourceSubject;
use crate::state::AppState;

pub use crate::media::convert::{BudgetVerdict, conversion_budget};
pub use crate::media::motion_preview::MotionRefusal;
pub use crate::media::playback_health::diagnose_playback;
pub use crate::media::timeline_preview::{TIMELINE_SHEET_ACTION, TIMELINE_TRACK_ACTION, TimelineRefusal};

// ── docx-support.ts ─────────────────────────────────────────────────────────

/// Largest .docx offered a preview (it is a ZIP that expands).
pub const MAX_DOCX_BYTES: i64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocxKind {
    Docx,
    LegacyDoc,
}

/// `docxSupport(mime, ext)`
pub fn docx_support(mime: Option<&str>, ext: Option<&str>) -> Option<DocxKind> {
    let e = ext.unwrap_or("");
    let e = e.strip_prefix('.').unwrap_or(e).to_lowercase();
    let m = crate::config::js_trim(mime.unwrap_or("").split(';').next().unwrap_or("")).to_lowercase();
    if e == "docx" || m == "application/vnd.openxmlformats-officedocument.wordprocessingml.document" {
        return Some(DocxKind::Docx);
    }
    if e == "doc" || m == "application/msword" {
        return Some(DocxKind::LegacyDoc);
    }
    None
}

// ── source-preview.ts ───────────────────────────────────────────────────────

pub use crate::media::source_preview::MAX_INLINE_TEXT;

/// `showsSource(subject)` for a row.
pub fn shows_source(media: &Upload) -> bool {
    crate::media::source_preview::shows_source(&SourceSubject {
        mime_type: &media.mime_type,
        ext: Some(&media.ext),
        kind: Some(&media.kind),
        language: media.language.as_deref(),
    })
}

/// `inlineTextPlan(size)`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineTextPlan {
    Read,
    TooLarge,
}

pub fn inline_text_plan(size: i64) -> InlineTextPlan {
    match crate::media::source_preview::inline_text_plan(size) {
        "too-large" => InlineTextPlan::TooLarge,
        _ => InlineTextPlan::Read,
    }
}

// ── timeline-preview.ts ─────────────────────────────────────────────────────

/// `canTimelinePreview(row)`
pub fn can_timeline_preview(mime_type: &str, ext: &str, size: i64) -> bool {
    crate::media::timeline_preview::can_timeline_preview(Some(mime_type), Some(ext), size)
}

// ── playback-health.ts ──────────────────────────────────────────────────────

/// The row's [`PlaybackSource`].
pub fn playback_source(media: &Upload) -> PlaybackSource {
    PlaybackSource::from(media)
}

// ── convert.ts ──────────────────────────────────────────────────────────────

/// `Converted`
#[derive(Debug, Clone)]
pub struct Converted {
    pub buffer: Vec<u8>,
    pub mime: String,
    pub ext: String,
}

/// `convertImageBuffer(input, to, { animated, timeoutMs, signal })`.
pub async fn convert_image_buffer(
    input: Bytes,
    to: ConvertTarget,
    animated: bool,
    timeout_ms: u64,
    signal: AbortSignal,
) -> anyhow::Result<Converted> {
    let out = crate::media::convert::convert_image_buffer(
        input,
        to,
        crate::media::convert::ConvertOptions { animated, timeout_ms: Some(timeout_ms), signal: Some(signal) },
    )
    .await?;
    Ok(Converted { buffer: out.buffer, mime: out.mime.to_string(), ext: out.ext.to_string() })
}

// ── derived-cache.ts (reads and writes) ─────────────────────────────────────

/// `readDerived(key)`
pub async fn read_derived(state: &AppState, key: &str) -> Option<Bytes> {
    crate::media::derived_cache::read_derived(&state.storage, key).await
}

/// `writeDerived(key, body)`
pub async fn write_derived(state: &AppState, key: &str, body: Bytes) {
    crate::media::derived_cache::write_derived(&state.storage, key, body).await
}

// ── poster.ts ───────────────────────────────────────────────────────────────

/// `videoPoster(media, signal)`: a still frame of a video (cached), or None.
pub async fn video_poster(state: &AppState, media: &Upload, signal: AbortSignal) -> anyhow::Result<Option<Bytes>> {
    Ok(crate::media::poster::video_poster(state, media, signal).await)
}

// ── motion.ts ───────────────────────────────────────────────────────────────

/// `MotionOptions`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MotionOptions {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub inside: bool,
}

/// `MotionResult`
#[derive(Debug, Clone)]
pub enum MotionResult {
    Ok { buffer: Vec<u8>, frames: u32 },
    Refused(MotionRefusal),
}

/// `motionPreview(media, opts, signal)`
pub async fn motion_preview(
    state: &AppState,
    media: &Upload,
    opts: MotionOptions,
    signal: AbortSignal,
) -> anyhow::Result<MotionResult> {
    use crate::media::motion::{MotionFit, MotionOptions as Opts};
    let fit = if opts.inside { MotionFit::Inside } else { MotionFit::Cover };
    let built = crate::media::motion::motion_preview(
        state,
        media,
        Opts { width: opts.width, height: opts.height, fit },
        signal,
    )
    .await?;
    Ok(match built {
        Ok((buffer, frames)) => MotionResult::Ok { buffer: buffer.to_vec(), frames },
        Err(refusal) => MotionResult::Refused(refusal),
    })
}

// ── timeline.ts ─────────────────────────────────────────────────────────────

/// `TimelineTrack`
#[derive(Debug, Clone)]
pub enum TimelineTrack {
    Ok { vtt: String },
    Refused(TimelineRefusal),
}

/// `TimelineSheet`
#[derive(Debug, Clone)]
pub enum TimelineSheet {
    Ok { buffer: Bytes },
    Refused(TimelineRefusal),
}

/// `timelineTrack(media, signal)`
pub async fn timeline_track(state: &AppState, media: &Upload, signal: AbortSignal) -> anyhow::Result<TimelineTrack> {
    Ok(match crate::media::timeline::timeline_track(state, media, signal).await? {
        Ok(vtt) => TimelineTrack::Ok { vtt },
        Err(refusal) => TimelineTrack::Refused(refusal),
    })
}

/// `timelineSheet(media, signal)`
pub async fn timeline_sheet(state: &AppState, media: &Upload, signal: AbortSignal) -> anyhow::Result<TimelineSheet> {
    Ok(match crate::media::timeline::timeline_sheet(state, media, signal).await? {
        Ok(buffer) => TimelineSheet::Ok { buffer },
        Err(refusal) => TimelineSheet::Refused(refusal),
    })
}

// ── av-action.ts ────────────────────────────────────────────────────────────

/// `AvConvertRequest`: `admit` spends the view budget (never on a dry run).
pub struct AvConvertRequest<'a> {
    pub media: &'a Upload,
    pub privileged: bool,
    pub to: Option<String>,
    pub offset_seconds: Option<f64>,
    pub clip_seconds: Option<f64>,
    pub signal: AbortSignal,
    pub dry_run: bool,
    pub admit: Option<Box<dyn FnOnce() -> BoxFuture<'static, bool> + Send>>,
}

/// `avConvert(req)`: the response for an audio/video conversion, or None when
/// ffmpeg does not claim this target.
pub async fn av_convert(state: &AppState, req: AvConvertRequest<'_>) -> anyhow::Result<Option<Response>> {
    let admit = req.admit.map(|admit| -> crate::media::av_action::AdmitHook {
        Box::new(move || Box::pin(async move { Ok(admit().await) }))
    });
    crate::media::av_action::av_convert(
        state,
        crate::media::av_action::AvConvertRequest {
            media: req.media.clone(),
            privileged: req.privileged,
            to: req.to,
            offset_seconds: req.offset_seconds,
            clip_seconds: req.clip_seconds,
            signal: Some(req.signal),
            admit,
            dry_run: req.dry_run,
        },
    )
    .await
}

// ── docx.ts ─────────────────────────────────────────────────────────────────

/// `DocxResult`
#[derive(Debug, Clone)]
pub enum DocxResult {
    Ok {
        html: String,
    },
    /// `too-large`, `legacy-doc`, `not-a-document` or a zip-guard refusal.
    Refused {
        reason: String,
    },
}

/// `docxToHtml(buf)`. The docx port has not landed on this branch yet.
pub async fn docx_to_html(_buf: Bytes) -> anyhow::Result<DocxResult> {
    anyhow::bail!("docxToHtml is not wired yet (srv-media-a)")
}

// ── card-render.ts ──────────────────────────────────────────────────────────

/// `renderCard(choice)`: the 1200x630 PNG of a type card.
pub async fn render_card(choice: &CardChoice) -> anyhow::Result<Vec<u8>> {
    crate::media::card_render::render_card(choice).await.map(|b| b.to_vec()).map_err(anyhow::Error::msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::av_cost::AV_MAX_INPUT_BYTES;
    use crate::media::playback_health::PlaybackFault;

    #[test]
    fn docx_support_by_type_or_extension() {
        assert_eq!(docx_support(Some("application/octet-stream"), Some(".DOCX")), Some(DocxKind::Docx));
        assert_eq!(
            docx_support(Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document; x=1"), None),
            Some(DocxKind::Docx)
        );
        assert_eq!(docx_support(Some("application/msword"), None), Some(DocxKind::LegacyDoc));
        assert_eq!(docx_support(None, Some("doc")), Some(DocxKind::LegacyDoc));
        assert_eq!(docx_support(Some("text/plain"), Some("txt")), None);
    }

    fn row(mime: &str, ext: &str, kind: &str, language: Option<&str>) -> Upload {
        serde_json::from_value(serde_json::json!({
            "id": "9d2e1a54-0000-4000-8000-000000000001", "ownerId": "u1", "code": "abc", "filename": "f",
            "ext": ext, "kind": kind, "language": language, "mimeType": mime, "size": 10,
            "storageKey": "k", "checksum": null, "width": null, "height": null, "blurhash": null,
            "thumbKey": null, "published": true, "deleteToken": "d", "views": 0, "downloads": 0,
            "lastAccessedAt": null, "expiresAt": null, "maxViews": null, "passwordHash": null, "folderId": null,
            "deletedAt": null, "createdAt": "2026-01-01T00:00:00.000Z", "updatedAt": "2026-01-01T00:00:00.000Z",
            "passwordVersion": null, "frames": null, "convertedAt": null, "originalExt": null,
            "durationSeconds": null, "fps": null
        }))
        .unwrap()
    }

    #[test]
    fn the_page_reads_the_source_of_everything_raw_serves_as_text() {
        // tests/unit/viewer-stage.test.ts, the server half: the page decides with
        // the same `showsSource` the stage draws from, so code that mediaKind calls
        // "other" still arrives as text, and nothing else is read off disk.
        for (mime, ext) in [
            ("text/x-python", "py"),
            ("application/json", "json"),
            ("application/json", "sxcu"),
            ("application/x-sh", "sh"),
            ("application/sql", "sql"),
            ("application/xml", "xml"),
            ("image/svg+xml", "svg"),
            ("text/html", "html"),
        ] {
            assert!(shows_source(&row(mime, ext, "file", None)), "{ext}");
        }
        for (mime, ext) in [
            ("application/octet-stream", "gr2"),
            ("application/zip", "zip"),
            ("image/png", "png"),
            ("video/mp4", "mp4"),
            ("application/vnd.openxmlformats-officedocument.wordprocessingml.document", "docx"),
        ] {
            assert!(!shows_source(&row(mime, ext, "file", None)), "{ext}");
        }
        // A paste is text by its language, whatever was stored.
        assert!(shows_source(&row("application/octet-stream", "py", "paste", Some("python"))));
    }

    #[test]
    fn inline_text_ceiling() {
        assert_eq!(inline_text_plan(MAX_INLINE_TEXT), InlineTextPlan::Read);
        assert_eq!(inline_text_plan(MAX_INLINE_TEXT + 1), InlineTextPlan::TooLarge);
    }

    #[test]
    fn timeline_is_offered_on_kind_and_size() {
        assert!(can_timeline_preview("video/mp4", "mp4", 1));
        assert!(can_timeline_preview("application/octet-stream", "webm", 10));
        assert!(!can_timeline_preview("video/mp4", "mp4", 0));
        assert!(!can_timeline_preview("video/mp4", "mp4", AV_MAX_INPUT_BYTES as i64 + 1));
        assert!(!can_timeline_preview("image/gif", "gif", 10));
    }

    #[test]
    fn diagnoses_the_reported_recording() {
        // tests/unit/playback-health.test.ts: the reported recording, both faults.
        let source = PlaybackSource {
            mime_type: Some("video/mp4".into()),
            ext: Some("mp4".into()),
            size: Some(2_181_784.0),
            width: Some(5056.0),
            height: Some(1924.0),
            frames: Some(403.0),
            duration_seconds: Some(13.433),
            fps: Some(30.0),
        };
        let d = diagnose_playback(&source).unwrap();
        assert_eq!(d.faults, vec![PlaybackFault::Oversized, PlaybackFault::Starved]);
        assert_eq!(d.edge, 5056.0);
        assert!(d.repairable);
        assert_eq!(d.repair_edge, 1920.0);
        assert!(d.starvation_factor.unwrap() > 1.0);
        let fine = PlaybackSource { width: Some(1280.0), height: Some(720.0), size: Some(50_000_000.0), ..source };
        assert!(diagnose_playback(&fine).is_none());
        let picture = PlaybackSource { mime_type: Some("image/png".into()), ext: Some("png".into()), ..fine };
        assert!(diagnose_playback(&picture).is_none());
    }

    #[test]
    fn budget_counts_frames_only_when_the_animation_is_kept() {
        let kept = conversion_budget(true, 100, 1000, ConvertTarget::Webp);
        assert!(kept.animated);
        assert_eq!(kept.pixels, 1000);
        let flat = conversion_budget(true, 100, 1000, ConvertTarget::Png);
        assert!(!flat.animated);
        assert_eq!(flat.pixels, 100);
        assert!(flat.ok);
    }
}
