//! The calls the serving half makes into the PROCESSING half of `lib/media`
//! (branch `srv-media-a`), each under its upstream name.
//!
//! The media serving port (`media::serve`, `routes::media_action`, the public
//! page loaders) was written in parallel with the processing port, against the
//! upstream signatures. Everything it needs from the other half goes through
//! this one module, so wiring the real implementations after both land in main
//! is a change here and nowhere else:
//!
//! | here | upstream | state |
//! |---|---|---|
//! | [`docx_support`], [`MAX_DOCX_BYTES`] | `lib/media/docx-support.ts` | pure, complete |
//! | [`shows_source`], [`inline_text_plan`] | `lib/media/source-preview.ts` | pure, complete |
//! | [`can_timeline_preview`], `TIMELINE_*_ACTION` | `lib/media/timeline-preview.ts` | pure, complete |
//! | [`diagnose_playback`] | `lib/media/playback-health.ts` | pure, complete |
//! | [`conversion_budget`] | `lib/media/convert.ts` | pure, complete |
//! | [`read_derived`], [`write_derived`] | `lib/media/derived-cache.ts` | complete |
//! | [`video_poster`] | `lib/media/poster.ts` | waits for part A (no poster) |
//! | [`motion_preview`] | `lib/media/motion.ts` | waits for part A (`no-engine`) |
//! | [`timeline_track`], [`timeline_sheet`] | `lib/media/timeline.ts` | waits for part A (`no-engine`) |
//! | [`av_convert`] | `lib/media/av-action.ts` | waits for part A (not claimed) |
//! | [`convert_image_buffer`] | `lib/media/convert.ts` | waits for part A (error) |
//! | [`docx_to_html`] | `lib/media/docx.ts` | waits for part A (error) |
//! | [`render_card`] | `lib/media/card-render.ts` | waits for part A (error) |

use axum::response::Response;
use bytes::Bytes;
use futures_util::future::BoxFuture;

use crate::db::models::Upload;
use crate::media::abort::AbortSignal;
use crate::media::av_cost::AV_MAX_INPUT_BYTES;
use crate::media::av_targets::{AvSourceSize, WEB_MAX_EDGE, needs_web_rescue};
use crate::media::card::CardChoice;
use crate::media::convert_targets::{ConvertTarget, keeps_animation};
use crate::media::mime::{MediaKind, is_text_payload, media_kind};
use crate::media::paste_type::row_mime_type;
use crate::media::playback_health::{
    HARDWARE_DECODER_MAX_EDGE, PlaybackDiagnosis, PlaybackFault, PlaybackSource, STARVED_BITS_PER_PIXEL,
    TYPICAL_BITS_PER_PIXEL, bits_per_pixel, longest_edge,
};
use crate::media::transform_cost::{max_pixels_for, peak_bytes_for};
use crate::state::AppState;

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

/// Largest text inlined into the preview page.
pub const MAX_INLINE_TEXT: i64 = 512 * 1024;

/// `showsSource(subject)`: anything /raw would hand over as text, plus every
/// in-app paste (whose type is declared by its language).
pub fn shows_source(media: &Upload) -> bool {
    let mime = row_mime_type(Some(&media.kind), media.language.as_deref(), &media.mime_type);
    is_text_payload(Some(&mime), Some(&media.ext))
}

/// `inlineTextPlan(size)`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineTextPlan {
    Read,
    TooLarge,
}

pub fn inline_text_plan(size: i64) -> InlineTextPlan {
    if size > MAX_INLINE_TEXT { InlineTextPlan::TooLarge } else { InlineTextPlan::Read }
}

// ── timeline-preview.ts ─────────────────────────────────────────────────────

pub const TIMELINE_TRACK_ACTION: &str = "timeline.vtt";
pub const TIMELINE_SHEET_ACTION: &str = "timeline.jpg";

/// `canTimelinePreview(row)`: offered on what the ROW can settle - kind and
/// size; duration is decided by the endpoint after its probe.
pub fn can_timeline_preview(mime_type: &str, ext: &str, size: i64) -> bool {
    media_kind(Some(mime_type), Some(ext)) == MediaKind::Video && size > 0 && size as u64 <= AV_MAX_INPUT_BYTES
}

/// `TimelineRefusal`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineRefusal {
    NotVideo,
    TooLarge,
    TooShort,
    TooManyPixels,
    UnknownDuration,
    Unreadable,
    NoEngine,
}

impl TimelineRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            TimelineRefusal::NotVideo => "not-video",
            TimelineRefusal::TooLarge => "too-large",
            TimelineRefusal::TooShort => "too-short",
            TimelineRefusal::TooManyPixels => "too-many-pixels",
            TimelineRefusal::UnknownDuration => "unknown-duration",
            TimelineRefusal::Unreadable => "unreadable",
            TimelineRefusal::NoEngine => "no-engine",
        }
    }
}

// ── playback-health.ts ──────────────────────────────────────────────────────

/// `diagnosePlayback(source)`: what is wrong with this video's shape, or None
/// when nothing is (and for everything that is not a measured video).
pub fn diagnose_playback(source: &PlaybackSource) -> Option<PlaybackDiagnosis> {
    if media_kind(source.mime_type.as_deref(), source.ext.as_deref()) != MediaKind::Video {
        return None;
    }
    let size = AvSourceSize { width: source.width, height: source.height };
    let edge = longest_edge(&size);
    if edge <= 0.0 {
        return None;
    }
    let mut faults = Vec::new();
    if edge > HARDWARE_DECODER_MAX_EDGE {
        faults.push(PlaybackFault::Oversized);
    }
    let bpp = bits_per_pixel(source);
    let starved = bpp.is_some_and(|b| b > 0.0 && b < STARVED_BITS_PER_PIXEL);
    if starved {
        faults.push(PlaybackFault::Starved);
    }
    if faults.is_empty() {
        return None;
    }
    Some(PlaybackDiagnosis {
        faults,
        edge,
        bits_per_pixel: bpp,
        starvation_factor: if starved {
            bpp.map(|b| crate::config::js_round(TYPICAL_BITS_PER_PIXEL / b))
        } else {
            None
        },
        repairable: needs_web_rescue(Some(&size)),
        repair_edge: WEB_MAX_EDGE,
    })
}

/// The row's [`PlaybackSource`].
pub fn playback_source(media: &Upload) -> PlaybackSource {
    PlaybackSource {
        mime_type: Some(media.mime_type.clone()),
        ext: Some(media.ext.clone()),
        size: Some(media.size as f64),
        width: media.width.map(f64::from),
        height: media.height.map(f64::from),
        frames: media.frames.map(f64::from),
        duration_seconds: media.duration_seconds,
        fps: media.fps,
    }
}

// ── convert.ts ──────────────────────────────────────────────────────────────

/// `BudgetVerdict`
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BudgetVerdict {
    pub ok: bool,
    pub animated: bool,
    pub pixels: u64,
    pub limit: u64,
    pub peak_bytes: f64,
}

/// `conversionBudget(probe, to)`: counts every frame when the animation is
/// kept, and measures the ceiling against the TARGET.
pub fn conversion_budget(animated: bool, frame_pixels: u64, total_pixels: u64, to: ConvertTarget) -> BudgetVerdict {
    let animated = animated && keeps_animation(to);
    let pixels = if animated { total_pixels } else { frame_pixels };
    let limit = max_pixels_for(to, animated);
    let peak_bytes = peak_bytes_for(to, animated, pixels);
    BudgetVerdict { ok: pixels <= limit, animated, pixels, limit, peak_bytes }
}

/// `Converted`
#[derive(Debug, Clone)]
pub struct Converted {
    pub buffer: Vec<u8>,
    pub mime: String,
    pub ext: String,
}

/// `convertImageBuffer(input, to, { animated, timeoutMs, signal })`.
/// WAITS FOR PART A.
pub async fn convert_image_buffer(
    _input: Bytes,
    _to: ConvertTarget,
    _animated: bool,
    _timeout_ms: u64,
    _signal: AbortSignal,
) -> anyhow::Result<Converted> {
    anyhow::bail!("convertImageBuffer is not wired yet (srv-media-a)")
}

// ── derived-cache.ts (reads and writes) ─────────────────────────────────────

/// `readDerived(key)`: cached bytes, or None on a miss (or on any storage
/// trouble - a miss is safe).
pub async fn read_derived(state: &AppState, key: &str) -> Option<Bytes> {
    let storage = state.storage.get().ok()?;
    storage.read_buffer(key).await.ok()
}

/// `writeDerived(key, body)`: best effort; a cache that cannot be written is
/// still a working conversion.
pub async fn write_derived(state: &AppState, key: &str, body: Bytes) {
    if let Ok(storage) = state.storage.get() {
        let _ = storage.write(key, crate::storage::bytes_stream(body)).await;
    }
}

// ── poster.ts ───────────────────────────────────────────────────────────────

/// `videoPoster(media, signal)`: a still frame of a video (cached), or None.
/// WAITS FOR PART A: no poster yet, so `/raw?w=` on a video answers 415.
pub async fn video_poster(_state: &AppState, _media: &Upload, _signal: AbortSignal) -> anyhow::Result<Option<Bytes>> {
    Ok(None)
}

// ── motion.ts ───────────────────────────────────────────────────────────────

/// `MotionRefusal`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionRefusal {
    NotAnimated,
    TooLarge,
    TooManyPixels,
    UnknownDuration,
    Unreadable,
    NoEngine,
}

impl MotionRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            MotionRefusal::NotAnimated => "not-animated",
            MotionRefusal::TooLarge => "too-large",
            MotionRefusal::TooManyPixels => "too-many-pixels",
            MotionRefusal::UnknownDuration => "unknown-duration",
            MotionRefusal::Unreadable => "unreadable",
            MotionRefusal::NoEngine => "no-engine",
        }
    }
}

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

/// `motionPreview(media, opts, signal)`. WAITS FOR PART A (`no-engine`).
pub async fn motion_preview(
    _state: &AppState,
    _media: &Upload,
    _opts: MotionOptions,
    _signal: AbortSignal,
) -> anyhow::Result<MotionResult> {
    Ok(MotionResult::Refused(MotionRefusal::NoEngine))
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

/// `timelineTrack(media, signal)`. WAITS FOR PART A (`no-engine`).
pub async fn timeline_track(_state: &AppState, _media: &Upload, _signal: AbortSignal) -> anyhow::Result<TimelineTrack> {
    Ok(TimelineTrack::Refused(TimelineRefusal::NoEngine))
}

/// `timelineSheet(media, signal)`. WAITS FOR PART A (`no-engine`).
pub async fn timeline_sheet(_state: &AppState, _media: &Upload, _signal: AbortSignal) -> anyhow::Result<TimelineSheet> {
    Ok(TimelineSheet::Refused(TimelineRefusal::NoEngine))
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
    pub admit: Option<Box<dyn FnOnce() -> BoxFuture<'a, bool> + Send + 'a>>,
}

/// `avConvert(req)`: the response for an audio/video conversion, or None when
/// ffmpeg does not claim this target. WAITS FOR PART A (claims nothing yet).
pub async fn av_convert(_state: &AppState, _req: AvConvertRequest<'_>) -> anyhow::Result<Option<Response>> {
    Ok(None)
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

/// `docxToHtml(buf)`. WAITS FOR PART A.
pub async fn docx_to_html(_buf: Bytes) -> anyhow::Result<DocxResult> {
    anyhow::bail!("docxToHtml is not wired yet (srv-media-a)")
}

// ── card-render.ts ──────────────────────────────────────────────────────────

/// `renderCard(choice)`: the 1200x630 PNG of a type card. WAITS FOR PART A.
pub async fn render_card(_choice: &CardChoice) -> anyhow::Result<Vec<u8>> {
    anyhow::bail!("renderCard is not wired yet (srv-media-a)")
}

#[cfg(test)]
mod tests {
    use super::*;

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
