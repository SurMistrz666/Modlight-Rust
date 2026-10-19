//! Port of `lib/media/timeline.ts`: the scrub-bar preview - one sprite sheet
//! of a video plus the WebVTT that says which rectangle belongs to which
//! moment.
//!
//! The policy is in `timeline_preview`, the budget in `transform_cost`. This
//! module is the one ffmpeg invocation that acts on them, and the cache in
//! front of it.
//!
//! WHY ONE PASS AND NOT MANY SEEKS. The poster and the motion clip want
//! pictures from ONE place; a timeline wants pictures from EVERYWHERE. A
//! hundred seeks is a hundred walks into the file (13.6 s against 1.2 s for a
//! 100 tile sheet of a 35 s 1080p file), and a hundred seeks batched into one
//! process is a memory bomb (40 inputs peaked at 1.93 GB in a 2 GiB
//! container).
//!
//! WHAT MAKES THE ONE PASS AFFORDABLE. `-discard nokey` tells the DEMUXER to
//! throw away every packet that is not a keyframe, so the decoder is handed 13
//! to 157 pictures for a normal clip instead of thousands (1.2 s against 19.5 s
//! for a full decode). The preview is therefore made of keyframes, the nearest
//! one at or before each moment - what every storyboard on the web is made of.
//!
//! LENGTH IS NOT THE COST: a two hour recording costs 3.2 s and 80 MB, fifteen
//! seconds of 4K costs 1.8 s and 260 MB. So the ceilings are on BYTES and on
//! the size of one frame, never on duration.

use bytes::Bytes;

use crate::db::models::Upload;
use crate::format::js_to_fixed;
use crate::media::abort::AbortSignal;
use crate::media::av_action::source_tag;
use crate::media::av_cost::{AV_MAX_INPUT_BYTES, AV_PROBE_TIMEOUT_MS};
use crate::media::av_probe::probe_av;
use crate::media::av_workspace::AvWorkspace;
use crate::media::derived_cache::{derived_key, may_cache_derived, read_derived, write_derived};
use crate::media::ffmpeg::{AvTool, RunAvOptions, input_args, run_av_tool, thread_args};
use crate::media::poster::{VideoSourceError, open_video_source};
use crate::media::timeline_preview::{
    TIMELINE_SHEET_ACTION, TimelinePlan, TimelineRefusal, can_timeline_preview, timeline_plan, timeline_vtt,
};
use crate::media::transform_cost::max_timeline_frame_pixels;
use crate::state::AppState;

/// Cache variant, and the number on the end is load-bearing: a VTT written by
/// one plan and a sheet written by another produce a preview whose rectangles
/// are off by a tile. Both files carry this one tag, so bumping it retires
/// them together. BUMP IT whenever anything in `timeline_preview` changes what
/// a plan comes out as.
pub const TIMELINE_VARIANT: &str = "timeline1";

/// Wall clock for the one ffmpeg invocation (the heaviest job measured on
/// production is 4.0 s). Inside TIMELINE_TIMEOUT_MS, so the process is killed
/// before the response gives up.
pub const TIMELINE_FFMPEG_TIMEOUT_MS: u64 = 15_000;

/// JPEG quality of the sheet (ffmpeg's mjpeg scale, 2 best to 31 worst). Six:
/// the picture is displayed at 160 pixels across inside a tooltip.
pub const TIMELINE_JPEG_QUALITY: u32 = 6;

/// `TimelineSheet`
pub type TimelineSheet = Result<Bytes, TimelineRefusal>;

/// `TimelineTrack`
pub type TimelineTrack = Result<String, TimelineRefusal>;

/// `timelineTrack(media, signal)`: the metadata track for this video, or a
/// reason there is none. Costs a probe and some string building, never a
/// decode - the player asks for this on every view, and the sheet only once
/// somebody reaches for the scrub bar.
///
/// The outer error is what must NOT become a refusal: ffprobe's own timeout
/// and the caller going away (`AvTimeoutError`, `AvAbortedError`), which the
/// route answers 503 and 499.
pub async fn timeline_track(state: &AppState, media: &Upload, signal: AbortSignal) -> anyhow::Result<TimelineTrack> {
    let cacheable = may_cache_derived(media.into());
    let key = derived_key(&media.id.to_string(), &source_tag(media), TIMELINE_VARIANT, "vtt");

    if cacheable && let Some(hit) = read_derived(&state.storage, &key).await {
        // A cache hit answers without a probe at all.
        return Ok(Ok(String::from_utf8_lossy(&hit).into_owned()));
    }

    let (source, _container, plan) = match open_timeline_source(state, media, &signal).await? {
        Ok(opened) => opened,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let vtt = timeline_vtt(&plan, TIMELINE_SHEET_ACTION);
    if cacheable {
        write_derived(&state.storage, &key, Bytes::from(vtt.clone())).await;
    }
    source.dispose().await;
    Ok(Ok(vtt))
}

/// `timelineSheet(media, signal)`: the sprite sheet for this video, or a
/// reason there is none. Never the original bytes and never a single frame.
pub async fn timeline_sheet(state: &AppState, media: &Upload, signal: AbortSignal) -> anyhow::Result<TimelineSheet> {
    let cacheable = may_cache_derived(media.into());
    let key = derived_key(&media.id.to_string(), &source_tag(media), TIMELINE_VARIANT, "jpg");

    if cacheable && let Some(hit) = read_derived(&state.storage, &key).await {
        return Ok(Ok(hit));
    }

    let (source, container, plan) = match open_timeline_source(state, media, &signal).await? {
        Ok(opened) => opened,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let output = source.output("sheet.jpg")?;
    run_av_tool(
        AvTool::Ffmpeg,
        &sheet_args(&source.input.to_string_lossy(), &container, &plan, &output.to_string_lossy()),
        RunAvOptions { timeout_ms: TIMELINE_FFMPEG_TIMEOUT_MS, signal: Some(signal), capture_stdout: false },
    )
    .await?;

    // Nothing written means the pass found no keyframe it could decode: no
    // preview, not a partial sheet whose empty cells the VTT would address.
    let sheet = tokio::fs::read(&output).await.ok().filter(|s| !s.is_empty());
    source.dispose().await;
    let Some(sheet) = sheet else {
        return Ok(Err(TimelineRefusal::Unreadable));
    };
    let sheet = Bytes::from(sheet);
    if cacheable {
        write_derived(&state.storage, &key, sheet.clone()).await;
    }
    Ok(Ok(sheet))
}

/// Open the file, ask what it is, and turn that into a plan - or refuse.
///
/// Asked from the ROW first, before anything is opened: on a non-local
/// storage driver opening stages a copy of the object, so a request for the
/// timeline of a PDF must not copy up to 512 MB to be told there is no video
/// in it. The workspace is removed on every path that does not hand it back.
async fn open_timeline_source(
    state: &AppState,
    media: &Upload,
    signal: &AbortSignal,
) -> anyhow::Result<Result<(AvWorkspace, String, TimelinePlan), TimelineRefusal>> {
    if !can_timeline_preview(Some(&media.mime_type), Some(&media.ext), media.size) {
        return Ok(Err(if media.size > AV_MAX_INPUT_BYTES as i64 {
            TimelineRefusal::TooLarge
        } else {
            TimelineRefusal::NotVideo
        }));
    }
    let source = match open_video_source(state, media.size, &media.storage_key).await {
        Ok(ws) => ws,
        Err(e) => {
            return Ok(Err(match e {
                VideoSourceError::TooLarge => TimelineRefusal::TooLarge,
                VideoSourceError::NoEngine => TimelineRefusal::NoEngine,
                VideoSourceError::Unreadable => TimelineRefusal::Unreadable,
            }));
        }
    };

    let refuse = |r| Ok(Err(r));
    let Some(probe) = probe_av(&source.input.to_string_lossy(), AV_PROBE_TIMEOUT_MS, Some(signal.clone())).await?
    else {
        return refuse(TimelineRefusal::Unreadable);
    };
    let Some(video) = probe.video.as_ref() else {
        return refuse(TimelineRefusal::NotVideo);
    };
    let Some(duration) = probe.duration else {
        return refuse(TimelineRefusal::UnknownDuration);
    };
    let (Some(width), Some(height)) = (video.width.filter(|w| *w != 0.0), video.height.filter(|h| *h != 0.0)) else {
        return refuse(TimelineRefusal::Unreadable);
    };
    // The one ceiling a sheet has of its own: a frame large enough to fill a
    // conversion slot's share of memory is refused rather than attempted.
    if width * height > max_timeline_frame_pixels() as f64 {
        return refuse(TimelineRefusal::TooManyPixels);
    }
    let plan = match timeline_plan(duration, width, height) {
        Ok(plan) => plan,
        Err(r) => return refuse(r),
    };
    Ok(Ok((source, probe.container, plan)))
}

/// `sheetArgs(input, container, plan, output)`: the one ffmpeg command, and
/// every part of it is load-bearing.
///
///  1. `fps` picks one picture per slice, starting HALF a slice in.
///  2. `tpad` clones the last picture forward: `fps` stops at the last
///     KEYFRAME, so without it the final cells come out black.
///  3. `scale` fits the picture inside the tile and `pad` centres it there,
///     so a phone video's rotation matrix letterboxes rather than squashes.
///  4. `tile` composes the grid; `-frames:v 1` ends the process at the sheet.
pub fn sheet_args(input: &str, container: &str, plan: &TimelinePlan, output: &str) -> Vec<String> {
    let n = crate::i18n::format::js_number_to_string;
    let rate = js_to_fixed(plan.count / plan.duration, 9);
    let start = js_to_fixed(plan.interval / 2.0, 3);
    let pad = js_to_fixed(plan.duration, 3);
    let (tw, th) = (n(plan.tile_width), n(plan.tile_height));

    let chain = [
        format!("fps=fps={rate}:start_time={start}"),
        format!("tpad=stop_mode=clone:stop_duration={pad}"),
        format!("scale={tw}:{th}:force_original_aspect_ratio=decrease:force_divisible_by=2:flags=bilinear"),
        format!("pad={tw}:{th}:(ow-iw)/2:(oh-ih)/2:color=black"),
        format!("tile={}x{}", n(plan.columns), n(plan.rows)),
    ]
    .join(",");

    let mut args: Vec<String> = ["-hide_banner", "-v", "error"].map(String::from).to_vec();
    args.extend(input_args(input, Some(container), &["-discard".to_string(), "nokey".to_string()]));
    // A sheet has no sound and no subtitles.
    args.extend(["-an", "-sn"].map(String::from));
    args.extend(thread_args());
    args.push("-vf".into());
    args.push(chain);
    args.extend(["-frames:v", "1", "-update", "1", "-q:v"].map(String::from));
    args.push(TIMELINE_JPEG_QUALITY.to_string());
    args.push("-y".into());
    args.push(output.to_string());
    args
}

#[cfg(test)]
mod tests {
    //! tests/unit/timeline-args.test.ts
    use super::*;

    fn plan_for(d: f64, w: f64, h: f64) -> TimelinePlan {
        timeline_plan(d, w, h).unwrap()
    }

    fn args_for(d: f64, w: f64, h: f64) -> Vec<String> {
        sheet_args("/scratch/av-1/input", "mov", &plan_for(d, w, h), "/scratch/av-1/sheet.jpg")
    }

    fn at(args: &[String], f: &str) -> usize {
        args.iter().position(|a| a == f).unwrap()
    }

    fn filter_of(args: &[String]) -> String {
        args[at(args, "-vf") + 1].clone()
    }

    #[test]
    fn the_sprite_pass() {
        let args = args_for(150.0, 1920.0, 1080.0);
        assert_eq!(args[at(&args, "-discard") + 1], "nokey");
        assert!(at(&args, "-discard") < at(&args, "-i"));
        assert_eq!(args[at(&args, "-f") + 1], "mov");
        assert_eq!(args[at(&args, "-protocol_whitelist") + 1], "file,pipe");
        assert_eq!(args.iter().filter(|a| *a == "-i").count(), 1);
        assert_eq!(args.last().unwrap(), "/scratch/av-1/sheet.jpg");
        assert!(!args.iter().any(|a| a.contains(';') || a.contains('|')));

        let plan = plan_for(30.0, 1920.0, 1080.0);
        let chain = filter_of(&args_for(30.0, 1920.0, 1080.0));
        assert!(chain.contains(&format!("start_time={}", js_to_fixed(plan.interval / 2.0, 3))));
        assert!(chain.contains(&format!("fps=fps={}", js_to_fixed(plan.count / plan.duration, 9))));
        let stages: Vec<&str> = chain.split(',').collect();
        let idx = |p: &str| stages.iter().position(|s| s.starts_with(p)).unwrap();
        assert!(idx("tpad=") > idx("fps=") && idx("scale=") > idx("tpad=") && idx("tile=") > idx("scale="));
        assert!(stages[idx("tpad=")].contains("stop_mode=clone"));
        assert!(chain.contains("force_original_aspect_ratio=decrease") && chain.contains("force_divisible_by=2"));
        assert!(chain.contains("pad=160:90:"));
        for d in [4.0, 30.0, 150.0, 7296.76] {
            let p = plan_for(d, 1920.0, 1080.0);
            assert!(filter_of(&args_for(d, 1920.0, 1080.0)).contains(&format!("tile={}x{}", p.columns, p.rows)));
        }
        assert!(filter_of(&args_for(30.0, 1080.0, 1920.0)).contains("scale=90:160:"));
        assert!(filter_of(&args_for(30.0, 1440.0, 1080.0)).contains("scale=160:120:"));

        let long = args_for(7296.76, 1920.0, 1080.0);
        assert_eq!(long[at(&long, "-frames:v") + 1], "1");
        assert!(long.contains(&"-an".to_string()) && long.contains(&"-sn".to_string()));
        assert_eq!(long[at(&long, "-q:v") + 1], TIMELINE_JPEG_QUALITY.to_string());

        let plan = plan_for(7296.76, 1920.0, 1080.0);
        let chain = filter_of(&long);
        let rate: f64 = chain.split("fps=fps=").nth(1).unwrap().split(':').next().unwrap().parse().unwrap();
        let last = plan.interval / 2.0 + (plan.count - 1.0) / rate;
        assert!(last < plan.duration);
        assert!((last - (plan.duration - plan.interval / 2.0)).abs() < 0.01);
    }

    #[test]
    fn the_cache_tag_carries_a_generation() {
        assert!(TIMELINE_VARIANT.ends_with(|c: char| c.is_ascii_digit()));
    }
}
