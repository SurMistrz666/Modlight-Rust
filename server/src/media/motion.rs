//! Port of `lib/media/motion.ts`: a thumbnail that MOVES - one contiguous clip
//! of a file, returned as a single animated WebP.
//!
//! The policy - which rows qualify, which stretch is taken, how fast it
//! plays, what it is allowed to cost - is in `motion_preview` and
//! `transform_cost`. This module is only the two engines that act on it, and
//! the cache in front of them.
//!
//! WHY A CLIP AND NOT A SPREAD. Eight frames sampled from across the whole
//! file read as a slideshow, and on that shape the cost is the SEEK (about
//! 0.85 s each on the production 4K .mov). One seek followed by a forward
//! decode is the cheap direction, and it buys thirty frames for less than
//! eight seeks cost.
//!
//! THE PIPELINE THAT ACTUALLY HOLDS. Two facts constrain it:
//!
//!  1. libvips writes an animated WebP only from an image it knows is
//!     animated. The ANIMATION path gets that for free - a run of pages loads
//!     as one multi-page image, the resize on it IS page-aware, and the WebP
//!     writer writes the pages straight out. The VIDEO path has no such image:
//!     its frames arrive as separate files, so they are stacked with an
//!     animated join. Two traps there: the join does not accept raw pixel
//!     buffers, so the intermediates are an encoded format (PNG); and a resize
//!     AFTER a join is not page-aware, so every frame is resized to its final
//!     size BEFORE it is joined.
//!  2. ffmpeg 5.1 cannot demux an animated WebP, so it cannot be the engine
//!     for animated images, and libvips cannot seek a video, so it cannot be
//!     the engine for video. Each kind therefore has its own sampler, and they
//!     meet at "an animated WebP whose frames we count".

use bytes::Bytes;

use crate::db::models::Upload;
use crate::format::js_to_fixed;
use crate::media::abort::AbortSignal;
use crate::media::av_action::source_tag;
use crate::media::av_cost::{AV_MAX_INPUT_BYTES, AV_PROBE_TIMEOUT_MS};
use crate::media::av_probe::probe_av;
use crate::media::av_workspace::{OpenedWorkspace, open_av_workspace};
use crate::media::derived_cache::{derived_key, may_cache_derived, read_derived, write_derived};
use crate::media::ffmpeg::{AvTool, RunAvOptions, ffmpeg_available, input_args, run_av_tool, thread_args};
use crate::media::mime::{MediaKind, media_kind};
use crate::media::motion_preview::{
    MOTION_CAPTURE_WIDTH, MOTION_FRAME_DELAY_MS, MotionRefusal, clamp_motion_edge, clip_delays, clip_pages,
    count_animated_webp_frames, video_clip,
};
use crate::media::probe::probe_image;
use crate::media::transform_cost::{
    MAX_TRANSFORM_BYTES, MOTION_CLIP_FPS, MOTION_FRAMES, motion_clip_budget, motion_page_budget,
};
use crate::media::vips::{self, FailOn, Fit, PngOptions, Position, Resize, Sharp, VipsError, WebpOptions};
use crate::state::AppState;

/// Wall clock for the one ffmpeg invocation: the heaviest clip measured on
/// production is 4.3 s; fifteen seconds sits inside MOTION_TIMEOUT_MS so the
/// process is killed before the response gives up.
pub const MOTION_FFMPEG_TIMEOUT_MS: u64 = 15_000;

/// Quality of the finished preview: a tile, not a download. At thirty frames
/// the count is what the bytes are made of, so the quality dropped one notch
/// (55 -> 45, a measured 14 %) and stopped there.
pub const MOTION_WEBP_QUALITY: u32 = 45;
/// Effort 4 was measured at 411 kB against effort 3's 410 kB. Not worth it.
pub const MOTION_WEBP_EFFORT: u32 = 3;

/// Filename pattern ffmpeg writes its frames to: a `%04d` sequence, joined to
/// the workspace's own private directory (never through `output()`, which
/// rightly refuses a `%`). A literal in this file; nothing a request carries
/// reaches it.
const MOTION_FRAME_PREFIX: &str = "mf";
const MOTION_FRAME_PATTERN: &str = "mf%04d.jpg";

/// `MotionOptions`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MotionOptions {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fit: MotionFit,
}

/// `fit: "cover" | "inside"`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionFit {
    Cover,
    Inside,
}

impl MotionFit {
    pub fn as_str(self) -> &'static str {
        match self {
            MotionFit::Cover => "cover",
            MotionFit::Inside => "inside",
        }
    }

    fn fit(self) -> Fit {
        match self {
            MotionFit::Cover => Fit::Cover,
            MotionFit::Inside => Fit::Inside,
        }
    }
}

/// `MotionResult`: the animated WebP and the frames it really holds.
pub type MotionResult = Result<(Bytes, u32), MotionRefusal>;

/// `motionPreview(media, requested, signal)`: a preview that moves, or a
/// reason there is none. Never returns the original bytes and never falls
/// back to a still. The outer error is the engines' own failure (ffprobe's
/// timeout, the caller going away, a storage read), which the route maps.
pub async fn motion_preview(
    state: &AppState,
    media: &Upload,
    requested: MotionOptions,
    signal: AbortSignal,
) -> anyhow::Result<MotionResult> {
    // Clamped BEFORE the cache key is built, so `w=1200` and `w=480` are one
    // entry and not two names for the same bytes.
    let opts = MotionOptions {
        width: clamp_motion_edge(requested.width),
        height: clamp_motion_edge(requested.height),
        fit: requested.fit,
    };
    let cacheable = may_cache_derived(media.into());
    // `motion2` rather than `motion`: the generator changed from eight seeks
    // to one continuous clip, and a cached artefact outlives the code that
    // made it. Deliberately NOT a bump of CACHE_EPOCH, which every thumbnail
    // and derivative shares.
    let key = derived_key(
        &media.id.to_string(),
        &source_tag(media),
        &format!("motion2-{}x{}-{}", opts.width.unwrap_or(0), opts.height.unwrap_or(0), opts.fit.as_str()),
        "webp",
    );

    if cacheable && let Some(hit) = read_derived(&state.storage, &key).await {
        // The frame count is read back out of the bytes rather than remembered
        // alongside them: it is a property of the artefact.
        let frames = count_animated_webp_frames(&hit);
        return Ok(Ok((hit, frames)));
    }

    let built = match media_kind(Some(&media.mime_type), Some(&media.ext)) {
        MediaKind::Video => video_motion(state, media, opts, &signal).await?,
        MediaKind::Image => animation_motion(state, media, opts, &signal).await?,
        _ => Err(MotionRefusal::NotAnimated),
    };

    if let Ok((buffer, _)) = &built
        && cacheable
    {
        write_derived(&state.storage, &key, buffer.clone()).await;
    }
    Ok(built)
}

// ---------------------------------------------------------------------------
// Video: one seek, then decode forward
// ---------------------------------------------------------------------------

/// The one ffmpeg invocation of the video sampler.
pub fn video_motion_args(
    input: &str,
    container: &str,
    start: f64,
    seconds: f64,
    width: f64,
    pattern: &str,
) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-v", "error"].map(String::from).to_vec();
    // ONE input with ONE `-ss` in front of its `-i`.
    args.extend(input_args(input, Some(container), &["-ss".to_string(), js_to_fixed(start, 3)]));
    // After `-i`, so it bounds the OUTPUT: decode this many seconds forward.
    args.extend(["-t".to_string(), js_to_fixed(seconds, 3)]);
    // Nothing but the picture.
    args.extend(["-an", "-sn"].map(String::from));
    args.extend(thread_args());
    args.push("-vf".into());
    // A plain integer, never an ffmpeg expression.
    args.push(format!(
        "fps={},scale={}:-2:flags=bilinear",
        crate::i18n::format::js_number_to_string(MOTION_CLIP_FPS),
        crate::i18n::format::js_number_to_string(width)
    ));
    // A hard cap on top of `-t`: a container whose duration is a lie cannot
    // turn a tile into an unbounded write into the workspace.
    args.extend(["-frames:v".to_string(), MOTION_FRAMES.to_string(), "-q:v".into(), "4".into(), "-y".into()]);
    args.push(pattern.to_string());
    args
}

async fn video_motion(
    state: &AppState,
    media: &Upload,
    opts: MotionOptions,
    signal: &AbortSignal,
) -> anyhow::Result<MotionResult> {
    if media.size > AV_MAX_INPUT_BYTES as i64 {
        return Ok(Err(MotionRefusal::TooLarge));
    }
    if !ffmpeg_available().await {
        return Ok(Err(MotionRefusal::NoEngine));
    }
    let storage = state.storage.get()?;
    let ws = match open_av_workspace(storage.as_ref(), &state.config, &media.storage_key, AV_MAX_INPUT_BYTES).await? {
        OpenedWorkspace::Ready(ws) => ws,
        OpenedWorkspace::TooLarge { .. } => return Ok(Err(MotionRefusal::TooLarge)),
        OpenedWorkspace::Missing => return Ok(Err(MotionRefusal::Unreadable)),
    };

    let result = async {
        let Some(probe) = probe_av(&ws.input.to_string_lossy(), AV_PROBE_TIMEOUT_MS, Some(signal.clone())).await?
        else {
            return Ok(Err(MotionRefusal::Unreadable));
        };
        let Some(video) = probe.video.as_ref() else {
            return Ok(Err(MotionRefusal::Unreadable));
        };
        // Without a duration there is nothing to place the clip inside.
        let Some(duration) = probe.duration else {
            return Ok(Err(MotionRefusal::UnknownDuration));
        };
        // A stream that will not say how big its picture is cannot be charged for.
        let (Some(src_w), Some(src_h)) = (video.width.filter(|w| *w != 0.0), video.height.filter(|h| *h != 0.0)) else {
            return Ok(Err(MotionRefusal::Unreadable));
        };

        // `nb_frames / duration` rather than a rate field; a missing count
        // falls back to the assumed (expensive) rate.
        let source_fps = video.frames.map(|f| f / duration);
        let budget = motion_clip_budget((src_w * src_h) as u64, source_fps);
        if !budget.ok {
            return Ok(Err(MotionRefusal::TooManyPixels));
        }
        let Some(clip) = video_clip(duration, budget.seconds) else {
            return Ok(Err(MotionRefusal::UnknownDuration));
        };
        let width = 2f64.max(f64::from(MOTION_CAPTURE_WIDTH).min(src_w));

        let dir = ws.dir().ok_or_else(|| anyhow::anyhow!("workspace already disposed"))?.to_path_buf();
        let pattern = dir.join(MOTION_FRAME_PATTERN);
        run_av_tool(
            AvTool::Ffmpeg,
            &video_motion_args(
                &ws.input.to_string_lossy(),
                &probe.container,
                clip.start,
                clip.seconds,
                width,
                &pattern.to_string_lossy(),
            ),
            RunAvOptions { timeout_ms: MOTION_FFMPEG_TIMEOUT_MS, signal: Some(signal.clone()), capture_stdout: false },
        )
        .await?;

        // Read back what was written rather than what was asked for: a clip
        // that ran into the end of the file has fewer frames. Sorted by name,
        // which is why the pattern is zero-padded.
        let mut names: Vec<String> = Vec::new();
        if let Ok(mut rd) = tokio::fs::read_dir(&dir).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with(MOTION_FRAME_PREFIX) && name.ends_with(".jpg") {
                    names.push(name);
                }
            }
        }
        names.sort();
        if names.is_empty() {
            return Ok(Err(MotionRefusal::Unreadable));
        }
        let mut grabbed = Vec::new();
        for name in names.iter().take(MOTION_FRAMES as usize) {
            match tokio::fs::read(dir.join(name)).await {
                Ok(buf) if !buf.is_empty() => grabbed.push(Bytes::from(buf)),
                _ => return Ok(Err(MotionRefusal::Unreadable)),
            }
        }
        // No source delay array for a video: the clip plays back at the rate
        // it was captured at.
        let delays = clip_delays(None, 0, grabbed.len(), f64::from(MOTION_FRAME_DELAY_MS));
        Ok(assemble(grabbed, delays, opts).await?)
    }
    .await;
    ws.dispose().await;
    result
}

// ---------------------------------------------------------------------------
// Animated image: one run of pages, one load
// ---------------------------------------------------------------------------

async fn animation_motion(
    state: &AppState,
    media: &Upload,
    opts: MotionOptions,
    signal: &AbortSignal,
) -> anyhow::Result<MotionResult> {
    // The image engine reads the whole file into memory, so the ceiling that
    // guards every other libvips read applies unchanged.
    if media.size > MAX_TRANSFORM_BYTES as i64 {
        return Ok(Err(MotionRefusal::TooLarge));
    }
    let src = state.storage.get()?.read_buffer(&media.storage_key).await?;
    if signal.aborted() {
        return Ok(Err(MotionRefusal::Unreadable));
    }

    // probe_image rather than a bare metadata(): it carries the `avis` brand
    // workaround and reports the height of ONE FRAME.
    let Some(probe) = probe_image(src.clone()).await else {
        return Ok(Err(MotionRefusal::Unreadable));
    };
    // `animated` is false for a still and for the AVIF sequence whose frames
    // libvips will not enumerate: no motion this engine can reach.
    if !probe.animated {
        return Ok(Err(MotionRefusal::NotAnimated));
    }
    let budget = motion_page_budget(probe.frame_pixels, u64::from(probe.pages));
    if !budget.ok {
        return Ok(Err(MotionRefusal::TooManyPixels));
    }
    let Some(run) = clip_pages(f64::from(probe.pages), budget.frames as f64) else {
        return Ok(Err(MotionRefusal::NotAnimated));
    };

    let source = source_delays(src).await;
    let delays =
        clip_delays(source.as_deref(), run.start as usize, run.count as usize, f64::from(MOTION_FRAME_DELAY_MS));
    if signal.aborted() {
        return Ok(Err(MotionRefusal::Unreadable));
    }

    // ONE load of the whole run. `limitInputPixels: false` because the budget
    // above is the ceiling here, and the tighter of the two.
    let built = Sharp::new(probe.input)
        .fail_on(FailOn::None)
        .page(run.start)
        .pages(run.count as i32)
        .limit_input_pixels(None)
        // Page-aware, unlike a resize after a join. `position: centre`, NOT
        // the `attention` the still thumbnail uses: attention picks the crop
        // from the content, so a run of frames would jitter as it plays.
        .resize(
            Resize::to(opts.width, opts.height)
                .fit(opts.fit.fit())
                .position(Position::Centre)
                .without_enlargement(true),
        )
        .webp(WebpOptions {
            quality: MOTION_WEBP_QUALITY,
            effort: MOTION_WEBP_EFFORT,
            loop_count: Some(0),
            // Per page. A scalar only sets the first frame's delay.
            delay: Some(delays),
            ..Default::default()
        })
        .to_buffer()
        .await;
    Ok(match built {
        Ok(buffer) => finish(Bytes::from(buffer)),
        Err(_) => Err(MotionRefusal::Unreadable),
    })
}

/// `sourceDelays(buf)`: the per-frame delays the source animation carries, or
/// `None` when it carries none (a header read).
async fn source_delays(buf: Bytes) -> Option<Vec<f64>> {
    let meta = Sharp::new(buf).fail_on(FailOn::None).animated(true).metadata().await.ok()?;
    meta.delay.filter(|d| !d.is_empty()).map(|d| d.into_iter().map(f64::from).collect())
}

// ---------------------------------------------------------------------------
// Shared: frames -> animated WebP
// ---------------------------------------------------------------------------

/// `tile(buf, opts)`: one frame, resized to its final tile size. PNG for the
/// intermediate: a lossy one would quantise every frame twice.
fn tile(buf: Bytes, opts: MotionOptions) -> Result<Vec<u8>, VipsError> {
    Sharp::new(buf)
        .fail_on(FailOn::None)
        .resize(
            Resize::to(opts.width, opts.height)
                .fit(opts.fit.fit())
                .position(Position::Centre)
                .without_enlargement(true),
        )
        .png(PngOptions { compression_level: 1, ..Default::default() })
        .to_buffer_blocking()
}

/// `assemble(frames, delays, opts)`: stack tile-sized frames into one animated
/// WebP (the VIDEO path only). The animated join is what makes the result
/// multi-page; nothing is resized after it.
async fn assemble(frames: Vec<Bytes>, delays: Vec<u32>, opts: MotionOptions) -> Result<MotionResult, VipsError> {
    if frames.len() < 2 {
        return Ok(Err(MotionRefusal::NotAnimated));
    }
    let buffer = vips::run_blocking(move || {
        let sized = frames.into_iter().map(|f| tile(f, opts).map(Bytes::from)).collect::<Result<Vec<_>, _>>()?;
        let count = sized.len();
        Sharp::join(sized, 1, true)
            .fail_on(FailOn::None)
            .webp(WebpOptions {
                quality: MOTION_WEBP_QUALITY,
                effort: MOTION_WEBP_EFFORT,
                loop_count: Some(0),
                delay: Some(delays.into_iter().take(count).collect()),
                ..Default::default()
            })
            .to_buffer_blocking()
    })
    .await?;
    Ok(finish(Bytes::from(buffer)))
}

/// `finish(buffer)`: what came out, not what went in. libwebp collapses
/// frames that are byte for byte identical, and when they collapse to ONE the
/// file did not visibly change across the clip: there is no motion to preview.
fn finish(buffer: Bytes) -> MotionResult {
    let written = count_animated_webp_frames(&buffer);
    if written < 2 {
        return Err(MotionRefusal::NotAnimated);
    }
    Ok((buffer, written))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_video_sampler_seeks_once_and_caps_the_frames() {
        let args = video_motion_args("/s/v.mp4", "mov", 1.5, 2.5, 480.0, "/tmp/av-1/mf%04d.jpg");
        let at = |f: &str| args.iter().position(|a| a == f).unwrap();
        assert!(at("-ss") < at("-i") && at("-t") > at("-i"));
        assert_eq!(args[at("-ss") + 1], "1.500");
        assert_eq!(args[at("-t") + 1], "2.500");
        assert_eq!(args[at("-vf") + 1], "fps=12,scale=480:-2:flags=bilinear");
        assert_eq!(args[at("-frames:v") + 1], "30");
        assert_eq!(args.iter().filter(|a| *a == "-i").count(), 1);
        assert_eq!(args.last().unwrap(), "/tmp/av-1/mf%04d.jpg");
    }
}
