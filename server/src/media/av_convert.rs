//! Port of `lib/media/av-convert.ts`: building the ffmpeg command for one
//! target, and running it.
//!
//! The argument LISTS are pure functions, exported and unit-tested, because an
//! argument list is exactly the kind of thing that looks obviously right and
//! is not: `-ss` after `-i` decodes the whole file instead of seeking to it,
//! `-vf` before `-i` is a fatal parse error, and a missing `-map` silently
//! picks a stream ffmpeg thought was best rather than the one we meant.
//!
//! Nothing here interpolates a caller's string into an argument. Numbers
//! arrive clamped from `av_cost`, the container comes from the allowlist in
//! `av_probe`, and paths come from a workspace the caller owns. There is no
//! shell anywhere on this path (`ffmpeg::run_av_tool` spawns an argument
//! vector), so even a filename full of shell metacharacters is only ever a
//! filename.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::format::js_to_fixed;
use crate::i18n::format::js_number_to_string;
use crate::media::abort::AbortSignal;
use crate::media::av_cost::{AV_ANIMATION_FPS, AV_STILL_FPS, animation_width, av_timeout_for};
use crate::media::av_probe::AvProbe;
use crate::media::av_targets::{
    AvOption, AvSourceKind, AvSourceSize, AvTarget, VideoContainer, WEB_MAX_EDGE, audio_copyable_to, av_output,
    container_muxer, container_verdict, is_iso_bmff, needs_web_rescue,
};
use crate::media::av_workspace::AvWorkspace;
use crate::media::ffmpeg::{AvTool, RunAvOptions, input_args, run_av_tool, thread_args};

/// A plan's view of its workspace: the input to read and where an output
/// named by a target table goes (`AvWorkspace.output`, which refuses unsafe
/// names). A trait so the argument lists can be tested without a directory.
pub trait PlanWorkspace {
    fn input_path(&self) -> String;
    fn output(&self, name: &str) -> anyhow::Result<PathBuf>;
}

impl PlanWorkspace for AvWorkspace {
    fn input_path(&self) -> String {
        self.input.to_string_lossy().into_owned()
    }

    fn output(&self, name: &str) -> anyhow::Result<PathBuf> {
        AvWorkspace::output(self, name)
    }
}

/// `AvPlan`
#[derive(Debug, Clone, PartialEq)]
pub struct AvPlan {
    /// The full argument list, ready for spawn.
    pub args: Vec<String>,
    /// File the job writes, inside the workspace.
    pub output_path: PathBuf,
    pub mime: &'static str,
    pub ext: &'static str,
    /// Wall clock this job gets, and the deadline its process group dies on.
    pub timeout_ms: u64,
    /// Something true about the result the caller did not ask for and would
    /// otherwise not discover - "the audio track was dropped", "this is 6
    /// seconds of a 4 minute video". Surfaced as a response header.
    pub note: Option<&'static str>,
    /// The picture was RE-ENCODED, so the source's dimensions no longer
    /// describe the result and a caller storing them would be recording a lie
    /// (`convert_in_place` re-probes the output when this is set).
    pub remeasure: bool,
}

/// `RefuseReason`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefuseReason {
    VideoCodec,
    AudioCodec,
    NoVideo,
    NoAudio,
}

impl RefuseReason {
    pub fn as_str(self) -> &'static str {
        match self {
            RefuseReason::VideoCodec => "video-codec",
            RefuseReason::AudioCodec => "audio-codec",
            RefuseReason::NoVideo => "no-video",
            RefuseReason::NoAudio => "no-audio",
        }
    }
}

static AUDIO_COVER: LazyLock<String> = LazyLock::new(|| match std::env::var("AUDIO_COVER_PATH") {
    Ok(v) if !v.is_empty() => v,
    _ => crate::paths::Paths::resolve().public.join("brand").join("audio-cover.png").to_string_lossy().into_owned(),
});

/// `audioCoverPath()`: the still picture that plays behind an audio track.
///
/// A committed asset (rendered once from app/icon.svg), not something drawn
/// per request: it is the same 1280x720 frame for every file on the server,
/// so building it on the request path would be work with no variable in it.
/// `AUDIO_COVER_PATH`, or `public/brand/audio-cover.png` under the server's
/// root (upstream: under the working directory, which is the same place in
/// its container).
pub fn audio_cover_path() -> &'static str {
    &AUDIO_COVER
}

/// `frameOffset(requested, duration)`: where in the file to start a clip.
///
/// Clamped into the file rather than refused: `?t=` past the end is a request
/// that has an obviously right answer. Defaults to one second in, not zero -
/// the first frame of a video is very often black.
pub fn frame_offset(requested: Option<f64>, duration: Option<f64>) -> f64 {
    let asked = match requested {
        Some(r) if r.is_finite() && r >= 0.0 => r,
        _ => 1.0,
    };
    match duration {
        Some(d) if d > 0.0 => asked.min((d - 0.05).max(0.0)),
        _ => asked,
    }
}

/// `animationFilters({ resampleFps, width, paletted })`.
///
/// `fps` is applied only for VIDEO sources: an animated image already carries
/// its own frame timing. GIF holds 256 colours, so it needs a palette computed
/// from the clip itself; ffmpeg's default 8-bit palette turns anything with a
/// gradient into mud.
pub fn animation_filters(resample_fps: bool, width: f64, paletted: bool) -> String {
    let mut chain: Vec<String> = Vec::new();
    if resample_fps {
        chain.push(format!("fps={AV_ANIMATION_FPS}"));
    }
    chain.push(format!("scale={}:-2:flags=lanczos", js_number_to_string(width)));
    let chain = chain.join(",");
    if paletted {
        format!("{chain},split[a][b];[a]palettegen=max_colors=128[p];[b][p]paletteuse=dither=bayer")
    } else {
        chain
    }
}

/// `webScaleFilter(width, height)`: the one filter a web-playable copy needs.
///
/// Written as a filter EXPRESSION rather than two numbers computed here:
/// ffmpeg auto-rotates on the way in, so literal output dimensions derived
/// from ffprobe's CODED width and height would squash a phone recording with a
/// 90-degree display matrix; `iw`/`ih` and a square bounding box are evaluated
/// after the rotation. `force_divisible_by=2` because H.264 in 4:2:0 needs even
/// edges; `lanczos` because these are desktop captures full of text.
pub fn web_scale_filter(width: Option<f64>, height: Option<f64>) -> String {
    if needs_web_rescue(Some(&AvSourceSize { width, height })) {
        let edge = js_number_to_string(WEB_MAX_EDGE);
        format!("scale=w={edge}:h={edge}:force_original_aspect_ratio=decrease:force_divisible_by=2:flags=lanczos")
    } else {
        "scale=trunc(iw/2)*2:trunc(ih/2)*2".to_string()
    }
}

/// `PlanInput`
pub struct PlanInput<'a> {
    pub option: AvOption,
    pub probe: &'a AvProbe,
    pub source_kind: AvSourceKind,
    pub workspace: &'a dyn PlanWorkspace,
    /// From `av_budget` - already clamped. `None` for targets that take no clip.
    pub clip_seconds: Option<f64>,
    /// `?t=` for a clip start, already a number or `None`.
    pub offset_seconds: Option<f64>,
}

struct Common {
    output_path: PathBuf,
    output: String,
    mime: &'static str,
    ext: &'static str,
    timeout_ms: u64,
}

impl Common {
    fn plan(&self, args: Vec<String>, note: Option<&'static str>, remeasure: bool) -> AvPlan {
        AvPlan {
            args,
            output_path: self.output_path.clone(),
            mime: self.mime,
            ext: self.ext,
            timeout_ms: self.timeout_ms,
            note,
            remeasure,
        }
    }
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

/// `planAv(input)`: turn a target into a command, or the reason this file
/// cannot do it after all (the dynamic half of the split in av-targets; the
/// caller turns it into a 409 carrying the sentence). The outer error is a
/// workspace that has already been disposed.
pub fn plan_av(input: &PlanInput<'_>) -> anyhow::Result<Result<AvPlan, RefuseReason>> {
    let (mime, ext) = av_output(input.option.to);
    let output_path = input.workspace.output(&format!("out.{ext}"))?;
    // One deadline, decided once (av_cost): a stream copy gets the short one,
    // an ordinary encode the medium one, and the full picture re-encode the
    // long one.
    let common = Common {
        output: output_path.to_string_lossy().into_owned(),
        output_path,
        mime,
        ext,
        timeout_ms: av_timeout_for(input.option.to, input.option.cost),
    };
    let ws = input.workspace;
    let probe = input.probe;

    // Before the container branch, because this target writes an .mp4 too -
    // it is not a container choice at all, it is the only entry here that
    // re-encodes a picture.
    if input.option.to == AvTarget::Webmp4 {
        return Ok(plan_web_video(probe, ws, &common));
    }

    Ok(match input.option.to {
        // MP4 is reachable from three kinds of source and means something
        // different in each: move a video into another box, give an audio
        // track a picture, squeeze an animation into a real video. Dispatching
        // on what the ROW says the source is keeps them apart.
        AvTarget::Container(to) => match input.source_kind {
            AvSourceKind::Audio => plan_audio_video(probe, ws, &common),
            AvSourceKind::ImageSequence => plan_animation_video(probe, ws, &common),
            AvSourceKind::Video => plan_container(to, probe, ws, &common),
        },
        AvTarget::Mp3 | AvTarget::Ogg | AvTarget::Wav => plan_audio_encode(input.option.to, probe, ws, &common),
        AvTarget::Gif | AvTarget::Webp => plan_animation(
            input.option.to,
            probe,
            input.source_kind,
            ws,
            input.clip_seconds,
            input.offset_seconds,
            &common,
        ),
        AvTarget::Webmp4 => unreachable!("handled above"),
    })
}

/// Move the streams into another container without touching a sample.
fn plan_container(
    to: VideoContainer,
    probe: &AvProbe,
    ws: &dyn PlanWorkspace,
    common: &Common,
) -> Result<AvPlan, RefuseReason> {
    // The codec tables live in av_targets, next to the menu that depends on them.
    if let Err(refusal) = container_verdict(to, probe.video.as_ref().map(|v| v.codec.as_str())) {
        return Err(match refusal {
            crate::media::av_targets::ContainerRefusal::VideoCodec => RefuseReason::VideoCodec,
            crate::media::av_targets::ContainerRefusal::NoVideo => RefuseReason::NoVideo,
        });
    }
    let keep_audio = audio_copyable_to(to, probe.audio.as_ref().map(|a| a.codec.as_str()));

    let mut args = s(&["-v", "error"]);
    args.extend(input_args(&ws.input_path(), Some(&probe.container), &[]));
    args.extend(s(&["-map", "0:v:0"]));
    if keep_audio {
        args.extend(s(&["-map", "0:a:0"]));
    }
    // Subtitles and data streams have no home in MP4 or MOV and abort the mux.
    args.extend(s(&["-dn", "-sn", "-c", "copy"]));
    // Container metadata can carry the recording device, the software, and on
    // phone footage sometimes a location. Rotation survives: it lives in the
    // track matrix, which `-c copy` keeps.
    args.extend(s(&["-map_metadata", "-1"]));
    // Move the index to the front so the result can start playing before it
    // has finished downloading. Matroska rejects the flag.
    if is_iso_bmff(to) {
        args.extend(s(&["-movflags", "+faststart"]));
    }
    args.extend(s(&["-f", container_muxer(to), "-y"]));
    args.push(common.output.clone());
    let note = (probe.audio.is_some() && !keep_audio).then_some("audio-dropped");
    Ok(common.plan(args, note, false))
}

/// Re-encode a recording into something a browser can actually play.
///
/// WHY THESE SETTINGS, all measured on the production container (two cores,
/// 2 GiB, ffmpeg 5.1.9): CRF, never a bitrate (the input is starved; copying
/// its bitrate would copy the fault); CRF 21 rather than 23, because these are
/// screen recordings with text; preset `veryfast` (`ultrafast` silently drops
/// `-profile:v high`); profile high; no explicit `-g`; `+faststart`.
fn plan_web_video(probe: &AvProbe, ws: &dyn PlanWorkspace, common: &Common) -> Result<AvPlan, RefuseReason> {
    let Some(video) = probe.video.as_ref() else {
        return Err(RefuseReason::NoVideo);
    };
    let audio = probe.audio.as_ref();
    let copy_audio = audio.is_some_and(|a| audio_copyable_to(VideoContainer::Mp4, Some(&a.codec)));

    let mut args = s(&["-v", "error"]);
    args.extend(input_args(&ws.input_path(), Some(&probe.container), &[]));
    args.extend(thread_args());
    args.extend(s(&["-map", "0:v:0"]));
    if audio.is_some() {
        args.extend(s(&["-map", "0:a:0"]));
    }
    args.extend(s(&["-dn", "-sn", "-vf"]));
    args.push(web_scale_filter(video.width, video.height));
    args.extend(s(&["-c:v", "libx264", "-preset", "veryfast", "-crf", "21", "-profile:v", "high"]));
    // Every browser decodes 4:2:0 planar YUV; Safari refuses 4:4:4 outright.
    args.extend(s(&["-pix_fmt", "yuv420p"]));
    if audio.is_some() {
        if copy_audio {
            args.extend(s(&["-c:a", "copy"]));
        } else {
            args.extend(s(&["-c:a", "aac", "-b:a", "192k"]));
        }
    }
    args.extend(s(&["-map_metadata", "-1", "-movflags", "+faststart", "-f", "mp4", "-y"]));
    args.push(common.output.clone());
    let note = (audio.is_some() && !copy_audio).then_some("audio-reencoded");
    Ok(common.plan(args, note, true))
}

/// Wrap an audio track in a video, on one unchanging frame.
///
/// ONE FRAME PER SECOND (every frame is identical, so the rate buys nothing),
/// and the sound is copied when MP4 will take it. `-t` pins the length to the
/// audio: at 1 fps `-shortest` can only stop on a frame boundary.
fn plan_audio_video(probe: &AvProbe, ws: &dyn PlanWorkspace, common: &Common) -> Result<AvPlan, RefuseReason> {
    let Some(audio) = probe.audio.as_ref() else {
        return Err(RefuseReason::NoAudio);
    };
    let copy_audio = audio_copyable_to(VideoContainer::Mp4, Some(&audio.codec));
    let still = AV_STILL_FPS.to_string();

    let mut args = s(&["-v", "error"]);
    // The cover is our own file, so forcing `image2` here is simply the demuxer
    // that knows how to hold one picture on screen for the length of another
    // stream.
    args.extend(input_args(audio_cover_path(), Some("image2"), &s(&["-loop", "1", "-framerate", &still])));
    args.extend(input_args(&ws.input_path(), Some(&probe.container), &[]));
    args.extend(thread_args());
    args.extend(s(&["-map", "0:v:0", "-map", "1:a:0", "-c:v", "libx264", "-preset", "veryfast"]));
    // Tells x264 the picture does not move, which is exactly true here.
    args.extend(s(&["-tune", "stillimage", "-pix_fmt", "yuv420p", "-r", &still]));
    // A keyframe a minute. Seeking in a static picture is free either way.
    args.extend(s(&["-g", &(AV_STILL_FPS * 60).to_string()]));
    if copy_audio {
        args.extend(s(&["-c:a", "copy"]));
    } else {
        args.extend(s(&["-c:a", "aac", "-b:a", "192k"]));
    }
    if let Some(d) = probe.duration.filter(|d| *d > 0.0) {
        args.extend(s(&["-t", &js_to_fixed(d, 3)]));
    }
    args.extend(s(&["-shortest", "-fflags", "+shortest"]));
    // Without this the muxer can stall waiting to interleave a stream that
    // produces one packet a second against one producing forty.
    args.extend(s(&["-max_interleave_delta", "100M"]));
    args.extend(s(&["-map_metadata", "-1", "-movflags", "+faststart", "-f", "mp4", "-y"]));
    args.push(common.output.clone());
    Ok(common.plan(args, (!copy_audio).then_some("audio-reencoded"), false))
}

/// Re-encode a whole animation as an MP4. Nothing is trimmed or resampled;
/// the only filter is the even-dimension fix h264 requires.
fn plan_animation_video(probe: &AvProbe, ws: &dyn PlanWorkspace, common: &Common) -> Result<AvPlan, RefuseReason> {
    if probe.video.is_none() {
        return Err(RefuseReason::NoVideo);
    }
    let mut args = s(&["-v", "error"]);
    args.extend(input_args(&ws.input_path(), Some(&probe.container), &[]));
    args.extend(thread_args());
    args.extend(s(&[
        "-map",
        "0:v:0",
        "-an",
        "-dn",
        "-sn",
        "-vf",
        "scale=trunc(iw/2)*2:trunc(ih/2)*2",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "23",
        // GIF is palettised RGB; every player expects 4:2:0 planar YUV.
        "-pix_fmt",
        "yuv420p",
        "-map_metadata",
        "-1",
        "-movflags",
        "+faststart",
        "-f",
        "mp4",
        "-y",
    ]));
    args.push(common.output.clone());
    Ok(common.plan(args, None, false))
}

fn plan_audio_encode(
    to: AvTarget,
    probe: &AvProbe,
    ws: &dyn PlanWorkspace,
    common: &Common,
) -> Result<AvPlan, RefuseReason> {
    if probe.audio.is_none() {
        return Err(RefuseReason::NoAudio);
    }
    let codec = match to {
        AvTarget::Mp3 => s(&["-c:a", "libmp3lame", "-q:a", "4"]),
        AvTarget::Ogg => s(&["-c:a", "libvorbis", "-q:a", "4"]),
        // 16-bit PCM at the source rate. WAV carries no compression, which is
        // why av_cost gives it a much shorter ceiling than the other two.
        _ => s(&["-c:a", "pcm_s16le"]),
    };
    let mut args = s(&["-v", "error"]);
    args.extend(input_args(&ws.input_path(), Some(&probe.container), &[]));
    args.extend(thread_args());
    args.extend(s(&["-map", "0:a:0", "-vn", "-dn", "-sn"]));
    args.extend(codec);
    args.extend(s(&["-map_metadata", "-1", "-f", to.as_str(), "-y"]));
    args.push(common.output.clone());
    Ok(common.plan(args, None, false))
}

fn plan_animation(
    to: AvTarget,
    probe: &AvProbe,
    source_kind: AvSourceKind,
    ws: &dyn PlanWorkspace,
    clip_seconds: Option<f64>,
    offset: Option<f64>,
    common: &Common,
) -> Result<AvPlan, RefuseReason> {
    let Some(video) = probe.video.as_ref() else {
        return Err(RefuseReason::NoVideo);
    };
    // See AvSourceKind: only a real video is clipped and resampled.
    let resample = source_kind != AvSourceKind::ImageSequence;
    let width = animation_width(video.width);
    let at = if offset.is_some() { frame_offset(offset, probe.duration) } else { 0.0 };
    let clipped = resample && clip_seconds.is_some();

    let mut before = Vec::new();
    if at > 0.0 && resample {
        before.extend(s(&["-ss", &js_to_fixed(at, 3)]));
    }
    // -t BEFORE -i, so the DEMUXER stops after the clip: `palettegen` cannot
    // emit a palette until its input ends, so as an output option the filter
    // graph would pull the whole remaining stream anyway (3917 ms against
    // 995 ms for a 6 s GIF of a 32 s file).
    if clipped && let Some(c) = clip_seconds {
        before.extend(s(&["-t", &js_number_to_string(c)]));
    }

    let mut args = s(&["-v", "error"]);
    args.extend(input_args(&ws.input_path(), Some(&probe.container), &before));
    args.extend(thread_args());
    args.extend(s(&["-map", "0:v:0", "-an", "-dn", "-sn", "-vf"]));
    args.push(animation_filters(resample, width, to == AvTarget::Gif));
    if to == AvTarget::Webp {
        args.extend(s(&["-c:v", "libwebp", "-lossless", "0", "-q:v", "75"]));
    }
    // Loop forever. GIF defaults to this; libwebp defaults to playing once,
    // which makes a "converted to animated WebP" file look like a still.
    args.extend(s(&["-loop", "0", "-f", to.as_str(), "-y"]));
    args.push(common.output.clone());
    let note = (clipped && probe.duration.is_some_and(|d| d > clip_seconds.unwrap_or(0.0))).then_some("clipped");
    Ok(common.plan(args, note, false))
}

/// `runAvPlan(plan, signal)`: resolves when the output file is complete on disk.
pub async fn run_av_plan(plan: &AvPlan, signal: Option<AbortSignal>) -> anyhow::Result<()> {
    run_av_tool(
        AvTool::Ffmpeg,
        &plan.args,
        RunAvOptions { timeout_ms: plan.timeout_ms, signal, capture_stdout: false },
    )
    .await?;
    Ok(())
}

/// `avVariant(to, clip, offset, sourceKind)`: the variant tag a result caches
/// under. Only the things the CALLER chose, and only where the PLAN reads
/// them: a container copy moves the whole file, an audio encode takes the
/// whole track, and an animation is converted whole - so `?t=` and `?d=`
/// change nothing about those outputs, and folding them into the key anyway
/// turned one cacheable result into an unbounded family of identical ones.
pub fn av_variant(to: AvTarget, clip: Option<f64>, offset: Option<f64>, source_kind: AvSourceKind) -> String {
    let mut parts = vec![to.as_str().to_string()];
    let clips = matches!(to, AvTarget::Gif | AvTarget::Webp) && source_kind == AvSourceKind::Video;
    if clips {
        // Rounded, exactly like the offset: `?d=6.1` and `?d=6.1001119` cut the
        // same frames.
        if let Some(c) = clip {
            parts.push(format!("c{}", js_to_fixed(c, 2)));
        }
        if let Some(o) = offset.filter(|o| *o > 0.0) {
            parts.push(format!("t{}", js_to_fixed(o, 2)));
        }
    }
    parts.join("-")
}

/// Convenience for the engines: the workspace's own directory.
pub fn workspace_dir(ws: &AvWorkspace) -> Option<&Path> {
    ws.dir()
}

#[cfg(test)]
mod tests {
    //! tests/unit/av-plan.test.ts and the ffmpeg-command parts of
    //! tests/unit/web-playable.test.ts.
    use super::*;
    use crate::media::av_cost::AV_WEB_TIMEOUT_MS;
    use crate::media::av_probe::{AvStream, AvStreamKind};
    use crate::media::av_targets::{AvCost, AvYield};
    use crate::media::ffmpeg::probe_input_args;

    struct FakeWs;
    impl PlanWorkspace for FakeWs {
        fn input_path(&self) -> String {
            "/data/storage/abc/xyz.mkv".into()
        }
        fn output(&self, name: &str) -> anyhow::Result<PathBuf> {
            Ok(PathBuf::from(format!("/tmp/av-1/{name}")))
        }
    }

    fn stream(kind: AvStreamKind, codec: &str) -> AvStream {
        AvStream {
            kind,
            codec: codec.into(),
            width: None,
            height: None,
            frames: None,
            fps: None,
            attached_picture: false,
        }
    }

    fn video(codec: &str, w: f64, h: f64) -> AvStream {
        AvStream { width: Some(w), height: Some(h), ..stream(AvStreamKind::Video, codec) }
    }

    fn probe() -> AvProbe {
        AvProbe {
            container: "mov".into(),
            format_name: "mov,mp4,m4a,3gp,3g2,mj2".into(),
            duration: Some(32.23),
            video: Some(AvStream { frames: Some(968.0), ..video("h264", 1280.0, 720.0) }),
            audio: Some(stream(AvStreamKind::Audio, "aac")),
            streams: vec![],
        }
    }

    fn opt(to: AvTarget, cost: AvCost) -> AvOption {
        AvOption { to, cost, yields: AvYield::Video }
    }

    fn copy(c: VideoContainer) -> AvOption {
        opt(AvTarget::Container(c), AvCost::Copy)
    }

    fn try_plan(option: AvOption, p: AvProbe, kind: AvSourceKind, clip: Option<f64>) -> Result<AvPlan, RefuseReason> {
        plan_av(&PlanInput {
            option,
            probe: &p,
            source_kind: kind,
            workspace: &FakeWs,
            clip_seconds: clip,
            offset_seconds: None,
        })
        .unwrap()
    }

    fn plan(option: AvOption, p: AvProbe) -> AvPlan {
        try_plan(option, p, AvSourceKind::Video, None).unwrap()
    }

    fn at(args: &[String], flag: &str) -> usize {
        args.iter().position(|a| a == flag).unwrap_or(usize::MAX)
    }

    fn after<'a>(args: &'a [String], flag: &str) -> &'a str {
        &args[at(args, flag) + 1]
    }

    fn count(args: &[String], flag: &str) -> usize {
        args.iter().filter(|a| *a == flag).count()
    }

    #[test]
    fn the_guards_every_invocation_carries() {
        let args = input_args("/x.mp4", Some("mov"), &[]);
        assert_eq!(after(&args, "-protocol_whitelist"), "file,pipe");
        assert!(probe_input_args("/x.mp4").contains(&"-protocol_whitelist".to_string()));
        let mkv = input_args("/x.mkv", Some("matroska"), &[]);
        assert_eq!(after(&mkv, "-f"), "matroska");
        assert!(at(&mkv, "-f") < at(&mkv, "-i"));
        assert!(input_args("/x.mp4", Some("mov"), &[]).contains(&"-use_absolute_path".to_string()));
        for c in ["matroska", "mp3", "wav", "avi", "gif", "ogg", "image2"] {
            assert!(!input_args("/x", Some(c), &[]).contains(&"-use_absolute_path".to_string()), "{c}");
        }
        let seek = input_args("/x.mp4", Some("mov"), &s(&["-ss", "5.000"]));
        assert!(at(&seek, "-ss") < at(&seek, "-i"));
        let nasty = "/data/storage/a b/;rm -rf $(echo x).mkv";
        let args = input_args(nasty, Some("matroska"), &[]);
        assert_eq!(args.last().unwrap(), nasty);
        assert_eq!(count(&args, nasty), 1);
    }

    #[test]
    fn moving_a_video_into_another_container() {
        use VideoContainer::*;
        for c in [Mp4, Mov, Mkv] {
            let p = plan(copy(c), probe());
            assert_eq!(after(&p.args, "-c"), "copy");
            assert!(!p.args.contains(&"libx264".to_string()));
        }
        for (c, muxer, ext) in [(Mp4, "mp4", "mp4"), (Mov, "mov", "mov"), (Mkv, "matroska", "mkv")] {
            let p = plan(copy(c), probe());
            let last_f = p.args.iter().rposition(|a| a == "-f").unwrap();
            assert_eq!(p.args[last_f + 1], muxer);
            assert_eq!(p.ext, ext);
            assert!(p.output_path.to_string_lossy().ends_with(&format!(".{ext}")));
        }
        let mp4 = plan(copy(Mp4), probe()).args;
        assert_eq!(after(&mp4, "-movflags"), "+faststart");
        assert!(plan(copy(Mov), probe()).args.contains(&"-movflags".to_string()));
        assert!(!plan(copy(Mkv), probe()).args.contains(&"-movflags".to_string()));
        assert!(mp4.contains(&"-dn".to_string()) && mp4.contains(&"-sn".to_string()));
        assert_eq!(after(&mp4, "-map_metadata"), "-1");
        let p = plan(copy(Mp4), probe());
        assert_eq!(count(&p.args, "-map"), 2);
        assert_eq!(p.note, None);
        let opus = AvProbe { audio: Some(stream(AvStreamKind::Audio, "opus")), ..probe() };
        let p = plan(copy(Mp4), opus.clone());
        assert_eq!(count(&p.args, "-map"), 1);
        assert_eq!(p.note, Some("audio-dropped"));
        assert_eq!(plan(copy(Mkv), opus).note, None);
        let vp9 = AvProbe { video: Some(video("vp9", 640.0, 480.0)), ..probe() };
        assert_eq!(try_plan(copy(Mp4), vp9.clone(), AvSourceKind::Video, None), Err(RefuseReason::VideoCodec));
        assert!(plan(copy(Mkv), vp9).args.contains(&"copy".to_string()));
    }

    fn audio_plan(audio: Option<AvStream>) -> Result<AvPlan, RefuseReason> {
        let p = AvProbe { video: None, audio, ..probe() };
        try_plan(opt(AvTarget::Container(VideoContainer::Mp4), AvCost::Encode), p, AvSourceKind::Audio, None)
    }

    #[test]
    fn an_audio_file_wrapped_in_a_video() {
        let p = audio_plan(Some(stream(AvStreamKind::Audio, "aac"))).unwrap();
        let args = &p.args;
        assert!(args.contains(&audio_cover_path().to_string()));
        assert_eq!(after(args, "-loop"), "1");
        assert_eq!(after(args, "-tune"), "stillimage");
        assert_eq!(count(args, "-i"), 2);
        assert_eq!(after(args, "-map"), "0:v:0");
        let last_map = args.iter().rposition(|a| a == "-map").unwrap();
        assert_eq!(args[last_map + 1], "1:a:0");
        assert!(args.contains(&"-shortest".to_string()));
        assert_eq!(after(args, "-fflags"), "+shortest");
        assert_eq!(after(args, "-t"), "32.230");
        let copied = audio_plan(Some(stream(AvStreamKind::Audio, "mp3"))).unwrap();
        assert_eq!(after(&copied.args, "-c:a"), "copy");
        assert_eq!(copied.note, None);
        let reencoded = audio_plan(Some(stream(AvStreamKind::Audio, "flac"))).unwrap();
        assert_eq!(after(&reencoded.args, "-c:a"), "aac");
        assert_eq!(reencoded.note, Some("audio-reencoded"));
        assert_eq!(audio_plan(None), Err(RefuseReason::NoAudio));
    }

    #[test]
    fn an_animation_reencoded_as_a_video() {
        let gif = AvProbe { container: "gif".into(), duration: Some(5.12), ..probe() };
        let p = try_plan(
            opt(AvTarget::Container(VideoContainer::Mp4), AvCost::Encode),
            gif,
            AvSourceKind::ImageSequence,
            None,
        )
        .unwrap();
        assert!(!p.args.contains(&"-t".to_string()) && !p.args.contains(&"-ss".to_string()));
        assert!(p.args.contains(&"-an".to_string()));
        assert_eq!(after(&p.args, "-vf"), "scale=trunc(iw/2)*2:trunc(ih/2)*2");
        assert_eq!(after(&p.args, "-pix_fmt"), "yuv420p");
    }

    #[test]
    fn clips_and_animated_pictures() {
        let f = animation_filters(true, 480.0, true);
        assert!(f.contains("palettegen") && f.contains("paletteuse") && f.contains("fps="));
        assert!(!animation_filters(true, 480.0, false).contains("palettegen"));
        let seq = AvProbe { duration: Some(2.15), audio: None, ..probe() };
        let sequence =
            try_plan(opt(AvTarget::Webp, AvCost::Encode), seq, AvSourceKind::ImageSequence, Some(6.0)).unwrap();
        assert!(!after(&sequence.args, "-vf").contains("fps="));
        assert!(!sequence.args.contains(&"-t".to_string()));
        let video = try_plan(opt(AvTarget::Gif, AvCost::Encode), probe(), AvSourceKind::Video, Some(6.0)).unwrap();
        assert!(after(&video.args, "-vf").contains("fps="));
        assert_eq!(after(&video.args, "-t"), "6");
        assert!(at(&video.args, "-t") < at(&video.args, "-i"));
        let webp = try_plan(opt(AvTarget::Webp, AvCost::Encode), probe(), AvSourceKind::Video, Some(6.0)).unwrap();
        assert_eq!(after(&webp.args, "-loop"), "0");
        assert!(webp.args.contains(&"libwebp".to_string()));
        let long = AvProbe { duration: Some(240.0), ..probe() };
        assert_eq!(
            try_plan(opt(AvTarget::Gif, AvCost::Encode), long, AvSourceKind::Video, Some(6.0)).unwrap().note,
            Some("clipped")
        );
        let short = AvProbe { duration: Some(4.0), ..probe() };
        assert_eq!(
            try_plan(opt(AvTarget::Gif, AvCost::Encode), short, AvSourceKind::Video, Some(4.0)).unwrap().note,
            None
        );
        assert_eq!(frame_offset(None, Some(60.0)), 1.0);
        assert!((frame_offset(Some(9999.0), Some(32.23)) - 32.18).abs() < 0.005);
        assert_eq!(frame_offset(Some(5.0), Some(32.23)), 5.0);
        assert_eq!(frame_offset(Some(-3.0), Some(60.0)), 1.0);
        assert_eq!(frame_offset(Some(10.0), None), 10.0);
    }

    #[test]
    fn cache_identity() {
        let v = AvSourceKind::Video;
        assert_eq!(av_variant(AvTarget::Gif, Some(6.0), Some(12.0), v), "gif-c6.00-t12.00");
        assert_eq!(
            av_variant(AvTarget::Gif, Some(6.1001119), Some(12.0), v),
            av_variant(AvTarget::Gif, Some(6.1), Some(12.0), v)
        );
        assert_eq!(av_variant(AvTarget::Container(VideoContainer::Mp4), None, None, v), "mp4");
        assert_eq!(av_variant(AvTarget::Container(VideoContainer::Mkv), Some(6.0), Some(4.0), v), "mkv");
        assert_eq!(av_variant(AvTarget::Gif, Some(6.0), Some(4.0), AvSourceKind::ImageSequence), "gif");
        assert_ne!(
            av_variant(AvTarget::Gif, Some(6.0), Some(0.0), v),
            av_variant(AvTarget::Gif, Some(15.0), Some(0.0), v)
        );
        assert_ne!(
            av_variant(AvTarget::Gif, Some(6.0), Some(1.0), v),
            av_variant(AvTarget::Gif, Some(6.0), Some(2.0), v)
        );
    }

    // tests/unit/web-playable.test.ts: "the scale filter", "the ffmpeg command"

    fn web(p: AvProbe) -> Result<AvPlan, RefuseReason> {
        try_plan(opt(AvTarget::Webmp4, AvCost::Encode), p, AvSourceKind::Video, None)
    }

    fn reported() -> AvProbe {
        AvProbe { video: Some(AvStream { frames: Some(403.0), ..video("h264", 5056.0, 1924.0) }), ..probe() }
    }

    #[test]
    fn the_scale_filter() {
        let f = web_scale_filter(Some(5056.0), Some(1924.0));
        assert_eq!(f, "scale=w=1920:h=1920:force_original_aspect_ratio=decrease:force_divisible_by=2:flags=lanczos");
        assert_eq!(web_scale_filter(Some(1924.0), Some(5056.0)), f);
        assert_eq!(web_scale_filter(Some(1280.0), Some(720.0)), "scale=trunc(iw/2)*2:trunc(ih/2)*2");
    }

    #[test]
    fn the_web_ffmpeg_command() {
        let p = web(reported()).unwrap();
        assert_eq!(after(&p.args, "-crf"), "21");
        assert!(!p.args.contains(&"-b:v".to_string()));
        assert_eq!(after(&p.args, "-profile:v"), "high");
        assert_eq!(after(&p.args, "-preset"), "veryfast");
        assert_eq!(after(&p.args, "-movflags"), "+faststart");
        assert_eq!(after(&p.args, "-map_metadata"), "-1");
        assert!(p.args.contains(&"-sn".to_string()) && p.args.contains(&"-dn".to_string()));
        assert!(p.remeasure);
        assert_eq!(p.timeout_ms, AV_WEB_TIMEOUT_MS);
        assert_eq!(p.ext, "mp4");
        let silent = web(AvProbe { audio: None, ..reported() }).unwrap();
        assert_eq!(count(&silent.args, "-map"), 1);
        assert!(!silent.args.contains(&"-c:a".to_string()));
        assert_eq!(after(&web(reported()).unwrap().args, "-c:a"), "copy");
        let opus = web(AvProbe { audio: Some(stream(AvStreamKind::Audio, "opus")), ..reported() }).unwrap();
        assert_eq!(after(&opus.args, "-c:a"), "aac");
        assert_eq!(opus.note, Some("audio-reencoded"));
        assert_eq!(web(AvProbe { video: None, ..reported() }), Err(RefuseReason::NoVideo));
    }
}
