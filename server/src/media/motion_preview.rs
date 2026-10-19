//! Port of `lib/media/motion-preview.ts`: the policy half of the moving
//! thumbnail - who gets one, which stretch of the file it is made of, and how
//! fast it plays. Pure (no libvips, no ffmpeg, no storage), so every decision
//! here is a unit test, and so `GET /api/uploads` and `/api/link-info` can
//! publish `motionThumbUrl` without reading or decoding anything. The engine
//! that acts on these answers is `media::motion`.
//!
//! WHY A CLIP RATHER THAN A SPREAD. The first version of this sampled eight
//! frames across the whole file, and next to the real video playing in the
//! panel it read as a slideshow. Raising the count was not the fix: on that
//! shape the cost is the SEEK, measured at about 0.85 s each on a 4K source,
//! so sixteen frames would have been fourteen seconds of ffmpeg for a tile.
//! One seek followed by a forward decode buys thirty frames for less than
//! eight seeks cost, and thirty contiguous frames at 12 fps look like video.

use crate::media::mime::{MediaKind, media_kind};

/// `MAX_TRANSFORM_BYTES` of `lib/media/transform-cost.ts` (32 MiB): the read
/// budget of an image transform.
pub const MAX_TRANSFORM_BYTES: i64 = 32 * 1024 * 1024;

/// `AV_MAX_INPUT_BYTES` of `lib/media/av-cost.ts` (512 MiB): the seek budget
/// of the audio/video path.
pub const AV_MAX_INPUT_BYTES: i64 = 512 * 1024 * 1024;

/// Query flag that asks for a preview that moves. Only `1` counts.
pub const MOTION_PARAM: &str = "motion";

/// Tile size used when a caller asks for `motion=1` and names no size.
pub const MOTION_DEFAULT_SIZE: u32 = 480;

/// Largest edge a moving tile is ever produced at.
pub const MOTION_CAPTURE_WIDTH: u32 = 480;

/// Why a motion preview cannot be produced. One vocabulary, server and client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

/// Milliseconds each frame of a VIDEO clip is shown: the reciprocal of the
/// capture rate, so the preview plays at REAL TIME (`Math.round(1000 / 12)`).
pub const MOTION_FRAME_DELAY_MS: u32 = 83;

/// Bounds on a per-frame delay, whatever the source claims.
pub const MOTION_MIN_DELAY_MS: f64 = 40.0;
pub const MOTION_MAX_DELAY_MS: f64 = 2_000.0;

/// Where in a video the clip starts, as a fraction of its length, and the cap
/// on that fraction. NOT frame zero: videos open on black, on a fade-in, on a
/// title card or on a player's splash screen far more often than on their
/// content. The cap stops an hour-long recording from starting six minutes in.
pub const MOTION_CLIP_START_FRACTION: f64 = 0.1;
pub const MOTION_CLIP_START_MAX_SECONDS: f64 = 10.0;

/// Kept clear of the very last picture: a container's duration is a claim,
/// and a decode that runs past the end of the pictures produces nothing.
pub const MOTION_CLIP_TAIL_GUARD: f64 = 0.05;

/// `clampMotionEdge(edge)`
pub fn clamp_motion_edge(edge: Option<u32>) -> Option<u32> {
    edge.map(|e| e.min(MOTION_CAPTURE_WIDTH))
}

/// `MotionCandidate`: what the row says about itself.
#[derive(Debug, Clone, Copy)]
pub struct MotionCandidate<'a> {
    pub mime_type: Option<&'a str>,
    pub ext: Option<&'a str>,
    pub size: i64,
    /// Frames the decoder reported at ingest; `None` = unknown.
    pub frames: Option<i32>,
}

/// `canMotionPreview(row)`: a video within the seek budget, or an image with
/// a RECORDED frame count above one within the read budget.
pub fn can_motion_preview(row: MotionCandidate<'_>) -> bool {
    match media_kind(row.mime_type, row.ext) {
        MediaKind::Video => row.size > 0 && row.size <= AV_MAX_INPUT_BYTES,
        MediaKind::Image => row.frames.is_some_and(|f| f > 1) && row.size <= MAX_TRANSFORM_BYTES,
        _ => false,
    }
}

/// `wantsMotion(params)`: the FIRST `motion` value is exactly `1`.
pub fn wants_motion(params: &[(String, String)]) -> bool {
    params.iter().find(|(k, _)| k == MOTION_PARAM).is_some_and(|(_, v)| v == "1")
}

/// One stretch of a video: where to seek to, and how much to decode from there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionClip {
    pub start: f64,
    pub seconds: f64,
}

/// `Math.round(x)` for the non-negative values used here.
fn js_round(x: f64) -> f64 {
    let r = x.round();
    if x < 0.0 && r - x == -0.5 { r + 1.0 } else { r }
}

fn round3(n: f64) -> f64 {
    js_round(n * 1000.0) / 1000.0
}

/// `videoClip(duration, seconds)`: which stretch of a video to take, in
/// seconds. `seconds` is what the BUDGET allows; what this adds is what the
/// FILE allows. Both numbers are finite and non-negative, and `start +
/// seconds` never reaches the last picture, because `start` becomes an ffmpeg
/// `-ss` and `seconds` becomes a `-t`. A file shorter than the clip is a
/// shorter clip, taken from the beginning.
pub fn video_clip(duration: f64, seconds: f64) -> Option<MotionClip> {
    if !duration.is_finite() || duration <= 0.0 {
        return None;
    }
    if !seconds.is_finite() || seconds <= 0.0 {
        return None;
    }
    let usable = duration - MOTION_CLIP_TAIL_GUARD;
    if usable <= 0.0 {
        return None;
    }
    let want = seconds.min(usable);
    let preferred = (duration * MOTION_CLIP_START_FRACTION).min(MOTION_CLIP_START_MAX_SECONDS);
    // Never so late that the clip would run off the end: the start yields to
    // the length.
    let start = preferred.min((usable - want).max(0.0));
    Some(MotionClip { start: round3(start), seconds: round3(want.min(usable - start)) })
}

/// One stretch of an animation: which page to start at, and how many to take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MotionPageRun {
    pub start: u32,
    pub count: u32,
}

/// `clipPages(pages, count)`: a run from page ZERO - the frame the still
/// thumbnail already shows, and the cheapest run there is (reading page p
/// costs decoding pages 0..p). An animation with no more pages than `count`
/// is taken whole.
pub fn clip_pages(pages: f64, count: f64) -> Option<MotionPageRun> {
    if pages.fract() != 0.0 || !pages.is_finite() || pages < 2.0 {
        return None;
    }
    if !count.is_finite() || count < 2.0 {
        return None;
    }
    Some(MotionPageRun { start: 0, count: pages.min(count.floor()) as u32 })
}

fn clamp_delay(ms: f64) -> u32 {
    js_round(ms.clamp(MOTION_MIN_DELAY_MS, MOTION_MAX_DELAY_MS)) as u32
}

/// `clipDelays(source, start, count, fallbackMs)`: per-frame delays for the
/// finished preview, in milliseconds. `source` is the decoder's own delay
/// array when there is one (an animation, never a video clip): a GIF that
/// holds its first frame for a second and then runs at 20 fps is extremely
/// common, and a uniform delay plays nothing like it. Every value is clamped,
/// because a delay comes out of a file.
pub fn clip_delays(source: Option<&[f64]>, start: usize, count: usize, fallback_ms: f64) -> Vec<u32> {
    let fallback = clamp_delay(fallback_ms);
    (0..count)
        .map(|i| match source.and_then(|s| s.get(start + i)) {
            Some(raw) if raw.is_finite() && *raw > 0.0 => clamp_delay(*raw),
            _ => fallback,
        })
        .collect()
}

/// `countAnimatedWebpFrames(buf)`: how many frames are really in this WebP,
/// counted from its bytes - a RIFF walk looking for ANMF chunks, the
/// container's own record of an animation frame. Deliberately not the
/// decoder's page count: the question is "does the artefact we are about to
/// serve actually move", and asking the library that wrote it would be asking
/// it to confirm its own intention.
pub fn count_animated_webp_frames(buf: &[u8]) -> u32 {
    if buf.len() < 12 || &buf[0..4] != b"RIFF" || &buf[8..12] != b"WEBP" {
        return 0;
    }
    let mut offset: u64 = 12;
    let mut frames = 0;
    let len = buf.len() as u64;
    while offset + 8 <= len {
        let o = offset as usize;
        let size = u32::from_le_bytes([buf[o + 4], buf[o + 5], buf[o + 6], buf[o + 7]]) as u64;
        if &buf[o..o + 4] == b"ANMF" {
            frames += 1;
        }
        // Chunks are padded to an even length; ignoring that walks into the
        // middle of the next header and the count silently becomes nonsense.
        offset += 8 + size + (size % 2);
    }
    frames
}

#[cfg(test)]
mod tests {
    //! tests/unit/motion-preview.test.ts (who is offered a moving tile, the
    //! query flag, the edge)
    use super::*;

    fn row<'a>(mime: &'a str, ext: &'a str, size: i64, frames: Option<i32>) -> MotionCandidate<'a> {
        MotionCandidate { mime_type: Some(mime), ext: Some(ext), size, frames }
    }

    #[test]
    fn offers_one_for_a_video_on_the_seek_budget() {
        let video = row("video/mp4", "mp4", 400_000_000, None);
        assert!(can_motion_preview(video));
        assert!(video.size > MAX_TRANSFORM_BYTES);
        assert!(!can_motion_preview(MotionCandidate { size: AV_MAX_INPUT_BYTES + 1, ..video }));
    }

    #[test]
    fn offers_one_for_an_animation_on_the_read_budget() {
        let gif = row("image/gif", "gif", 2_000_000, Some(43));
        assert!(can_motion_preview(gif));
        assert!(!can_motion_preview(MotionCandidate { size: MAX_TRANSFORM_BYTES + 1, ..gif }));
        assert!(!can_motion_preview(MotionCandidate { frames: Some(1), ..gif }));
        assert!(!can_motion_preview(MotionCandidate { frames: None, ..gif }));
    }

    #[test]
    fn stays_silent_about_everything_else() {
        assert!(!can_motion_preview(row("audio/mpeg", "mp3", 5_000, None)));
        assert!(!can_motion_preview(row("application/pdf", "pdf", 5_000, Some(9))));
        assert!(!can_motion_preview(row("image/svg+xml", "svg", 5_000, Some(9))));
    }

    #[test]
    fn the_flag_only_answers_to_an_exact_1() {
        let p = |q: &str| -> Vec<(String, String)> { form_urlencoded::parse(q.as_bytes()).into_owned().collect() };
        assert!(wants_motion(&p("motion=1")));
        assert!(!wants_motion(&p("motion=true")));
        assert!(!wants_motion(&p("motion=0")));
        assert!(!wants_motion(&p("motion=")));
        assert!(!wants_motion(&p("w=480&h=480")));
    }

    use crate::media::transform_cost::{
        MOTION_CLIP_FPS, MOTION_CLIP_MIN_SECONDS, MOTION_CLIP_SECONDS, MOTION_FRAMES, max_motion_decode_pixels,
        max_motion_source_pixels, motion_clip_budget, motion_page_budget,
    };

    #[test]
    fn a_preview_that_reads_as_motion() {
        const { assert!(MOTION_FRAMES >= 20 && MOTION_FRAMES <= 36) };
        assert_eq!(f64::from(MOTION_FRAMES), (MOTION_CLIP_SECONDS * MOTION_CLIP_FPS).round());
        assert_eq!(f64::from(MOTION_FRAME_DELAY_MS), (1000.0 / MOTION_CLIP_FPS).round());
        assert!(1000.0 / f64::from(MOTION_FRAME_DELAY_MS) >= 10.0);
    }

    #[test]
    fn where_in_a_video_the_clip_is_taken_from() {
        let clip = video_clip(60.0, MOTION_CLIP_SECONDS).unwrap();
        assert!(clip.start > 0.0);
        assert_eq!(clip.seconds, MOTION_CLIP_SECONDS);
        let hour = video_clip(3600.0, MOTION_CLIP_SECONDS).unwrap();
        assert!(hour.start <= MOTION_CLIP_START_MAX_SECONDS && hour.start > 0.0);
        for duration in [0.2, 0.5, 1.0, 2.5, 3.0, 15.0, 54.6, 3600.0] {
            let clip = video_clip(duration, MOTION_CLIP_SECONDS).unwrap_or_else(|| panic!("{duration}"));
            assert!(clip.start >= 0.0 && clip.seconds > 0.0);
            assert!(clip.start + clip.seconds <= duration);
        }
        let short = video_clip(1.2, MOTION_CLIP_SECONDS).unwrap();
        assert_eq!(short.start, 0.0);
        assert!(short.seconds > 1.0);
        let trimmed = video_clip(15.05, 1.808).unwrap();
        assert_eq!(trimmed.seconds, 1.808);
        assert!((trimmed.start - 1.505).abs() < 0.0005);
        assert_eq!(video_clip(0.0, MOTION_CLIP_SECONDS), None);
        assert_eq!(video_clip(f64::NAN, MOTION_CLIP_SECONDS), None);
        assert_eq!(video_clip(f64::INFINITY, MOTION_CLIP_SECONDS), None);
        assert_eq!(video_clip(-5.0, MOTION_CLIP_SECONDS), None);
        assert_eq!(video_clip(10.0, 0.0), None);
    }

    #[test]
    fn how_much_of_a_video_may_be_decoded() {
        const FOUR_K: u64 = 3840 * 2160;
        const FULL_HD: u64 = 1920 * 1080;
        assert_eq!(motion_clip_budget(FULL_HD, Some(60.0)).seconds, MOTION_CLIP_SECONDS);
        assert_eq!(motion_clip_budget(1280 * 720, Some(30.0)).seconds, MOTION_CLIP_SECONDS);
        let v = motion_clip_budget(FOUR_K, Some(60.0));
        assert!(v.ok && v.seconds < MOTION_CLIP_SECONDS && v.seconds >= MOTION_CLIP_MIN_SECONDS);
        assert!(v.pixels <= max_motion_decode_pixels());
        assert!(v.frames as f64 >= MOTION_CLIP_MIN_SECONDS * MOTION_CLIP_FPS);
        assert_eq!(motion_clip_budget(FOUR_K, Some(30.0)).seconds, MOTION_CLIP_SECONDS);
        for fps in [24.0, 30.0, 50.0, 60.0, 120.0] {
            assert!(motion_clip_budget(FOUR_K, Some(fps)).pixels <= max_motion_decode_pixels());
        }
        assert!(!motion_clip_budget(7680 * 4320, Some(60.0)).ok);
        assert_eq!(motion_clip_budget(7680 * 4320, Some(60.0)).frames, 0);
        assert_eq!(motion_clip_budget(FOUR_K, None).seconds, motion_clip_budget(FOUR_K, Some(60.0)).seconds);
        assert_eq!(motion_clip_budget(FULL_HD, Some(0.001)).seconds, MOTION_CLIP_SECONDS);
        assert_eq!(motion_clip_budget(FULL_HD, Some(100_000.0)).ok, motion_clip_budget(FULL_HD, Some(240.0)).ok);
        assert!(!motion_clip_budget(0, Some(30.0)).ok);
    }

    #[test]
    fn how_much_of_an_animation_may_be_decoded() {
        let frame_pixels = 1654 * 762;
        let v = motion_page_budget(frame_pixels, 256);
        assert!(v.ok);
        assert_eq!(v.frames, u64::from(MOTION_FRAMES));
        assert_eq!(v.pixels, frame_pixels * u64::from(MOTION_FRAMES));
        let big = motion_page_budget(4000 * 3000, u64::from(MOTION_FRAMES));
        assert!(big.ok && big.frames < u64::from(MOTION_FRAMES) && big.frames >= 2);
        assert!(big.pixels <= max_motion_source_pixels());
        assert_eq!(motion_page_budget(200 * 200, 6).frames, 6);
        assert!(!motion_page_budget(100_000_000, 2).ok);
        assert!(!motion_page_budget(1_000, 1).ok);
        assert!(!motion_page_budget(0, 8).ok);
        let largest = motion_page_budget(1195 * 1169, 47);
        assert!(largest.ok);
        assert_eq!(largest.frames, u64::from(MOTION_FRAMES));
    }

    #[test]
    fn which_pages_of_an_animation_are_taken() {
        let frames = f64::from(MOTION_FRAMES);
        assert_eq!(clip_pages(300.0, frames), Some(MotionPageRun { start: 0, count: MOTION_FRAMES }));
        assert_eq!(clip_pages(5.0, frames), Some(MotionPageRun { start: 0, count: 5 }));
        assert_eq!(clip_pages(1.0, frames), None);
        assert_eq!(clip_pages(0.0, frames), None);
        assert_eq!(clip_pages(300.0, 1.0), None);
    }

    #[test]
    fn how_fast_the_preview_plays() {
        let d = f64::from(MOTION_FRAME_DELAY_MS);
        let source = [1000.0, 50.0, 50.0, 50.0];
        assert_eq!(clip_delays(Some(&source), 0, 4, d), [1000, 50, 50, 50]);
        assert_eq!(clip_delays(Some(&source), 1, 3, d), [50, 50, 50]);
        assert_eq!(clip_delays(None, 0, 3, d), [MOTION_FRAME_DELAY_MS; 3]);
        assert_eq!(clip_delays(Some(&[0.0, 0.0]), 0, 2, d), [MOTION_FRAME_DELAY_MS; 2]);
        assert_eq!(clip_delays(Some(&[120.0]), 0, 3, d), [120, MOTION_FRAME_DELAY_MS, MOTION_FRAME_DELAY_MS]);
        assert_eq!(f64::from(clip_delays(Some(&[1.0]), 0, 1, d)[0]), MOTION_MIN_DELAY_MS);
        assert_eq!(f64::from(clip_delays(Some(&[10_000_000.0]), 0, 1, d)[0]), MOTION_MAX_DELAY_MS);
    }

    #[test]
    fn counting_frames_out_of_webp_bytes() {
        assert_eq!(count_animated_webp_frames(&[]), 0);
        assert_eq!(count_animated_webp_frames(b"not a riff file at all"), 0);
        let chunk = |fourcc: &[u8], body: usize| {
            let mut c = fourcc.to_vec();
            c.extend_from_slice(&(body as u32).to_le_bytes());
            c.extend(std::iter::repeat_n(0u8, body + body % 2));
            c
        };
        let mut body = b"WEBP".to_vec();
        for (f, n) in [(&b"VP8X"[..], 10), (b"ANMF", 3), (b"ANMF", 4), (b"ANMF", 5)] {
            body.extend(chunk(f, n));
        }
        let mut file = b"RIFF".to_vec();
        file.extend_from_slice(&(body.len() as u32).to_le_bytes());
        file.extend(body);
        assert_eq!(count_animated_webp_frames(&file), 3);
    }

    #[test]
    fn caps_the_edge() {
        assert_eq!(clamp_motion_edge(Some(1200)), Some(MOTION_CAPTURE_WIDTH));
        assert_eq!(clamp_motion_edge(Some(256)), Some(256));
        assert_eq!(clamp_motion_edge(None), None);
        assert_eq!(MOTION_CAPTURE_WIDTH, 480);
    }
}
