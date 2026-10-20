//! Port of `lib/media/poster.ts`: one still frame from a video, for use as a
//! tile, and the opening of a video for any of the frame grabbers.
//!
//! Why this exists: the desktop client draws its file grid from `thumbUrl`,
//! and that field was null for every video, so a library full of screen
//! recordings rendered as a wall of grey icons. The web panel gets away with
//! having no poster because a browser will draw one itself from `<video
//! preload="metadata">`; a desktop client cannot do that without downloading
//! the video.
//!
//! The reason this is affordable at all is that `-ss` BEFORE `-i` makes ffmpeg
//! seek rather than decode: a frame out of a 700 MB recording costs about what
//! reading a few megabytes costs, not what transcoding costs. Everything below
//! exists to keep it that way.

use bytes::Bytes;

use crate::db::models::Upload;
use crate::i18n::format::js_number_to_string;
use crate::media::abort::AbortSignal;
use crate::media::av_action::source_tag;
use crate::media::av_convert::frame_offset;
use crate::media::av_cost::{AV_MAX_INPUT_BYTES, AV_PROBE_TIMEOUT_MS};
use crate::media::av_probe::probe_av;
use crate::media::av_workspace::{AvWorkspace, OpenedWorkspace, open_av_workspace};
use crate::media::derived_cache::{derived_key, may_cache_derived, read_derived, write_derived};
use crate::media::ffmpeg::{AvTool, RunAvOptions, ffmpeg_available, input_args, run_av_tool};
use crate::state::AppState;

/// Wall clock for the extract. A seek-and-one-frame job that takes longer than
/// this is not slow, it is broken - a damaged index, or a file that is not
/// really seekable.
pub const POSTER_TIMEOUT_MS: u64 = 10_000;

/// The frame is captured at this width and then handed to the normal
/// thumbnail path, which does the real resizing. Larger than any tile we
/// serve, so one cached frame can answer every requested size.
pub const POSTER_CAPTURE_WIDTH: u32 = 1280;

/// The argument vector of the extract (`POSTER_INTERNALS` upstream pins the
/// numbers; the vector is what the parity check compares).
pub fn poster_args(input: &str, container: &str, at: f64, output: &str) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error"].map(String::from).to_vec();
    // `inputArgs` rather than a hand-written `-i`, and that is a security
    // fix rather than tidying: it forces the demuxer the probe approved (a
    // text file beginning `ffconcat version 1.0` would otherwise be read by the
    // concat demuxer) and brings the protocol whitelist. `-ss` goes BEFORE
    // `-i`: after it, ffmpeg decodes everything up to the offset.
    args.extend(input_args(input, Some(container), &["-ss".to_string(), js_number_to_string(at)]));
    args.extend(["-frames:v", "1"].map(String::from));
    // Even the seek can land past the end on a file whose duration is a lie;
    // -update lets the single-image output overwrite rather than fail.
    args.extend(["-update", "1", "-vf"].map(String::from));
    args.push(format!("scale={POSTER_CAPTURE_WIDTH}:-2:flags=bilinear"));
    args.extend(["-q:v", "4", "-y"].map(String::from));
    args.push(output.to_string());
    args
}

/// `videoPoster(media, signal)`: a still frame for this row, or `None` when
/// there is not going to be one.
///
/// `None` means "no poster", never "serve the original instead" - the caller
/// must not fall through to streaming the file, because answering a request
/// for a 4 kB tile with a 700 MB video is precisely the failure this is meant
/// to prevent.
pub async fn video_poster(state: &AppState, media: &Upload, signal: AbortSignal) -> Option<Bytes> {
    // Keyed on the row's own checksum when it has one, and on its size
    // otherwise - the same identity /convert uses.
    let cacheable = may_cache_derived(media.into());
    let key = derived_key(&media.id.to_string(), &source_tag(media), "poster", "jpg");

    if cacheable && let Some(hit) = read_derived(&state.storage, &key).await {
        return Some(hit);
    }

    // A poster is decoration. Nothing here is worth turning a tile request
    // into a 500, and the caller treats None as "no poster".
    let ws = open_video_source(state, media.size, &media.storage_key).await.ok()?;
    let result = async {
        // The probe is what makes the offset honest: without a duration the
        // obvious guess - frame zero - is very often a black frame.
        let probe = probe_av(&ws.input.to_string_lossy(), AV_PROBE_TIMEOUT_MS, Some(signal.clone())).await.ok()??;
        probe.video.as_ref()?;
        let at = frame_offset(None, probe.duration);
        let output = ws.output("poster.jpg").ok()?;
        let args = poster_args(&ws.input.to_string_lossy(), &probe.container, at, &output.to_string_lossy());
        run_av_tool(
            AvTool::Ffmpeg,
            &args,
            RunAvOptions { timeout_ms: POSTER_TIMEOUT_MS, signal: Some(signal.clone()), capture_stdout: false },
        )
        .await
        .ok()?;
        let frame = tokio::fs::read(&output).await.ok().filter(|f| !f.is_empty())?;
        Some(Bytes::from(frame))
    }
    .await;
    ws.dispose().await;

    let frame = result?;
    if cacheable {
        write_derived(&state.storage, &key, frame.clone()).await;
    }
    Some(frame)
}

// ---------------------------------------------------------------------------
// Opening a video for ANY of the frame grabbers
// ---------------------------------------------------------------------------

/// Why a video cannot be opened at all, in a vocabulary both frame grabbers
/// share. The timeline sheet turns these into `X-Timeline-Refused`; the
/// poster turns all three into "no poster" and lets the route answer 415.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoSourceError {
    TooLarge,
    NoEngine,
    Unreadable,
}

impl VideoSourceError {
    pub fn as_str(self) -> &'static str {
        match self {
            VideoSourceError::TooLarge => "too-large",
            VideoSourceError::NoEngine => "no-engine",
            VideoSourceError::Unreadable => "unreadable",
        }
    }
}

/// `openVideoSource(media)`: a video ready for ffmpeg to read, or a reason
/// there is not one. The three checks in front of every frame grab, cheapest
/// first: the ceiling the audio/video path budgets for, an engine at all, and
/// somewhere ffmpeg can open the bytes. Nothing reads the file into memory:
/// on the local driver the workspace hands over the stored path itself.
pub async fn open_video_source(
    state: &AppState,
    size: i64,
    storage_key: &str,
) -> Result<AvWorkspace, VideoSourceError> {
    if size > AV_MAX_INPUT_BYTES as i64 {
        return Err(VideoSourceError::TooLarge);
    }
    if !ffmpeg_available().await {
        return Err(VideoSourceError::NoEngine);
    }
    let storage = state.storage.get().map_err(|_| VideoSourceError::Unreadable)?;
    match open_av_workspace(storage.as_ref(), &state.config, storage_key, AV_MAX_INPUT_BYTES).await {
        Ok(OpenedWorkspace::Ready(ws)) => Ok(ws),
        Ok(OpenedWorkspace::TooLarge { .. }) => Err(VideoSourceError::TooLarge),
        // Upstream: any other workspace error ("missing") is unreadable; a
        // thrown error escaped `openVideoSource` and was caught by the
        // poster's own try (None) or reached the timeline route (500).
        Ok(OpenedWorkspace::Missing) | Err(_) => Err(VideoSourceError::Unreadable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeks_before_the_input_and_takes_one_frame() {
        let args = poster_args("/s/v.mp4", "mov", 1.0, "/tmp/av-x/poster.jpg");
        let at = |f: &str| args.iter().position(|a| a == f).unwrap();
        assert!(at("-ss") < at("-i"));
        assert_eq!(args[at("-ss") + 1], "1");
        assert_eq!(args[at("-frames:v") + 1], "1");
        assert_eq!(args[at("-vf") + 1], "scale=1280:-2:flags=bilinear");
        assert_eq!(args[at("-f") + 1], "mov");
        assert_eq!(args.last().unwrap(), "/tmp/av-x/poster.jpg");
        let other = poster_args("/i", "mov", 32.18, "/o");
        assert_eq!(other[other.iter().position(|a| a == "-ss").unwrap() + 1], "32.18");
    }
}
