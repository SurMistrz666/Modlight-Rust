//! Port of the row-level half of `lib/media/motion-preview.ts`: who is offered
//! a moving tile, the query flag that asks for one, and the edge it is capped
//! at. `GET /api/uploads` and `/api/link-info` publish `motionThumbUrl` from
//! these answers without reading or decoding anything.
//!
//! The clip planning (`videoClip`, `clipPages`, `clipDelays`,
//! `countAnimatedWebpFrames`) belongs to the serving route's engine
//! (`lib/media/motion.ts`) and is ported with it.

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

    #[test]
    fn caps_the_edge() {
        assert_eq!(clamp_motion_edge(Some(1200)), Some(MOTION_CAPTURE_WIDTH));
        assert_eq!(clamp_motion_edge(Some(256)), Some(256));
        assert_eq!(clamp_motion_edge(None), None);
        assert_eq!(MOTION_CAPTURE_WIDTH, 480);
    }
}
