//! Port of `lib/media/playback-health.ts`: "Will this video actually play
//! well for the people I send it to?"
//!
//! Answered from the upload row alone - width, height, size, and a frame total
//! from `frames` or from duration and rate - so it costs nothing to ask and can
//! be asked at the moment the owner is looking at the file, which is the only
//! moment at which the answer is worth anything.
//!
//! WHY THIS EXISTS
//!
//! A user reported that his ShareX recordings "stutter". The file he pointed at
//! is structurally perfect: faststart, constant 30 fps, 33.3 ms between every
//! one of its 403 frames. Nothing in the container is wrong. Two things about
//! its SHAPE are, and the panel had every number needed to say so at upload time
//! and said nothing - so the owner found out from a complaint.
//!
//!   1. It is 5056 pixels across. Hardware H.264 decoders stop at 4096 wide
//!      (Intel QuickSync, NVDEC and AMD alike). Past that the browser decodes in
//!      software, and software decoding manages around 292 Mpx/s - while this
//!      file is 9.73 Mpx at 30 fps, which is 292 Mpx/s. There is no headroom at
//!      all, on one core, while the rest of the page is also being drawn.
//!   2. It holds 0.0044 bits per pixel. At that budget the encoder cannot
//!      refresh the picture; it can only afford to mark most of it unchanged,
//!      which is exactly what "frozen but the clock is running" looks like.
//!
//! Both faults are properties of the file's shape, not of its content, which is
//! what makes them answerable from five integers and what keeps this module pure.
//!
//! WHAT THIS DELIBERATELY IS NOT
//!
//! It is not a quality score. Nothing here opens the file, looks at a frame or
//! asks an encoder anything, so it cannot tell a beautiful low-bitrate clip from
//! an ugly one. It answers one narrow question - "is this file outside the shape
//! a browser plays well?" - and every threshold below is set so that answering
//! "yes" is rare. See the measurements on each constant.

use crate::media::av_targets::AvSourceSize;
use crate::media::transform_cost::js_round;

/// Widest H.264 picture a hardware decoder will take.
///
/// Not a guess and not a Modlight number: it is the limit the shipping
/// fixed-function H.264 decoders share - Intel QuickSync, NVIDIA NVDEC and AMD
/// VCN all refuse a picture wider than this and hand the stream back to the CPU.
/// Chrome, Firefox and Safari then decode it in software, which is where the
/// stutter comes from.
///
/// The row does not store a codec, so the rule cannot be narrowed to H.264 and
/// the sentence the reader sees says "most hardware video decoders" rather than
/// naming one. That hedge is doing real work: an HEVC decoder usually goes to
/// 8192, so a 5000-pixel HEVC file would be overstated. All 7 files past this
/// line on production are H.264, and a picture that size is a heavy decode
/// whatever the codec, so the note stays worth showing - but it must not claim
/// more than it knows.
///
/// Distinct from WEB_MAX_EDGE (1920), which is what the repair scales DOWN to.
/// The two numbers answer different questions and must not be merged: 4096 is
/// "where it breaks", 1920 is "where it is safe" - see the encode measurements on
/// WEB_MAX_EDGE for why the repair does not simply park the output at 4096.
pub const HARDWARE_DECODER_MAX_EDGE: f64 = 4096.0;

/// Pixels in one H.264 macroblock: 16 x 16. The unit an encoder actually spends in.
pub const MACROBLOCK_PIXELS: f64 = 256.0;

/// Bits per macroblock below which a picture cannot be refreshed, only marked.
///
/// This is the derivation that puts the bitrate line somewhere defensible instead
/// of somewhere that looks tidy. H.264 codes in 16x16 macroblocks. Signalling
/// "this block is unchanged" costs on the order of one bit; actually coding a
/// block with any detail in it costs tens. So the reported file:
///
/// ```text
///     9.7 Mpx / 256 = 38 000 macroblocks per frame
///     1296 kb/s / 30 fps = 43 kbit per frame
///     43 000 / 38 000 = 1.14 bits per macroblock
/// ```
///
/// There is no bitrate left over to refresh anything. The encoder can code a
/// sliver of the picture per frame and must declare the rest unchanged, and that
/// is why the recording looks frozen while its frame timing is perfect.
///
/// Two bits per macroblock is therefore the line: below it the encoder cannot
/// even afford one coded block per two, whatever is in the file. Measured against
/// all 464 production videos that could be probed (read-only, September 2026):
///
/// ```text
///     1   bit  / macroblock   0 videos  (0.0%)   - would miss the reported file
///     1.5 bits / macroblock   3 videos  (0.6%)
///     2   bits / macroblock   6 videos  (1.3%)   <- chosen
///     3   bits / macroblock  14 videos  (3.0%)
///     4   bits / macroblock  30 videos  (6.5%)
///     6   bits / macroblock  58 videos  (12.5%)
/// ```
///
/// The reported file sits at 1.14, the lowest of all 464. The lowest file the
/// line does NOT flag is at 2.06, and the median production video is at 16.6, so
/// there is a factor of eight between the line and a normal file. Combined with
/// the width rule the note appears on 12 of 464 rows - 2.6% - which is the number
/// that matters, because a warning shown on a large share of a library is a
/// warning nobody reads.
pub const MIN_BITS_PER_MACROBLOCK: f64 = 2.0;

/// The same line expressed per pixel, which is what the row's arithmetic yields.
pub const STARVED_BITS_PER_PIXEL: f64 = MIN_BITS_PER_MACROBLOCK / MACROBLOCK_PIXELS;

/// What a video that plays well on this server actually carries.
///
/// The median of the same 464 production videos: 0.0649 bits per pixel, or 16.6
/// bits per macroblock. It is used for ONE thing - turning "0.0044" into "about
/// fifteen times less than normal", because a reader cannot place a bare bits-per
/// -pixel figure and can place a ratio instantly. Nothing branches on it.
pub const TYPICAL_BITS_PER_PIXEL: f64 = 0.065;

/// The columns this reasons over.
///
/// Every one of them is optional and every one of them may be None, because that
/// is the truthful state of an upload that predates the column. None means
/// "nobody measured", never "small" and never "fine", and each rule below fails
/// CLOSED on it: an unmeasured file is never warned about. That is the only safe
/// direction for a notice whose entire value is that it is rare.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlaybackSource {
    pub mime_type: Option<String>,
    pub ext: Option<String>,
    pub size: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    /// Frames ffprobe counted. None on roughly one production video in twenty.
    pub frames: Option<f64>,
    pub duration_seconds: Option<f64>,
    pub fps: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackFault {
    /// Wider than any hardware decoder will take.
    Oversized,
    /// Too few bits for the number of pixels to be refreshed.
    Starved,
}

impl PlaybackFault {
    pub fn as_str(self) -> &'static str {
        match self {
            PlaybackFault::Oversized => "oversized",
            PlaybackFault::Starved => "starved",
        }
    }
}

/// Serialises as the object `diagnosePlayback` returns (the media page hands
/// it to the client as `playback`).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackDiagnosis {
    /// Ordered, never empty. `oversized` first: it is the one with a repair.
    pub faults: Vec<PlaybackFault>,
    /// Longest edge in pixels, for the sentence.
    pub edge: f64,
    /// Bits per pixel, when it could be computed at all.
    pub bits_per_pixel: Option<f64>,
    /// How much more a normal video here carries, rounded, for the sentence.
    ///
    /// Set only when `starved` is among the faults. A field that carried a number
    /// in states where the number means nothing is a field that eventually gets
    /// printed in one of them: an oversized but well encoded recording would be
    /// told it holds "about 1 times less" than normal.
    pub starvation_factor: Option<f64>,
    /// Does the repair this app already offers actually apply to this file?
    ///
    /// Deliberately `needsWebRescue` and not a rule of its own: that is the exact
    /// test av_targets uses to decide whether `webmp4` appears in the menu at
    /// all, so the button can never offer a conversion the route would refuse.
    pub repairable: bool,
    /// Longest edge the repair would produce. Only meaningful when repairable.
    pub repair_edge: f64,
}

/// How many picture frames the file holds, counted if possible, derived if not.
///
/// `uploads.frames` means "somebody counted" and is absent on about one video in
/// twenty - an MPEG-TS stream wearing an .mp4 name, a WebM, a damaged MP4. Those
/// still carry a duration and a rate, and for a TOTAL (which is all bits-per-
/// pixel needs) the product is as good as the count. The derived number is never
/// written back to the column; see the note on uploads.frames for why that
/// distinction is worth keeping.
///
/// This is the one implementation. av_meta exports `frame_count` as a thin
/// wrapper over it so the upload path, the backfill script and this module
/// cannot drift.
pub fn frame_total(source: &PlaybackSource) -> Option<f64> {
    if let Some(frames) = source.frames
        && frames.is_finite()
    {
        return if frames > 0.0 { Some(js_round(frames)) } else { None };
    }
    let (duration, fps) = (source.duration_seconds?, source.fps?);
    if !duration.is_finite() || !fps.is_finite() {
        return None;
    }
    let estimate = js_round(duration * fps);
    if estimate > 0.0 { Some(estimate) } else { None }
}

/// JavaScript truthiness of an optional number: present, non-zero, not NaN.
fn truthy(v: Option<f64>) -> Option<f64> {
    v.filter(|n| *n != 0.0 && !n.is_nan())
}

/// Bits of file per pixel of picture: size x 8 / (width x height x frames).
///
/// The whole file's size, including its container overhead and its audio track,
/// which overstates the picture's share slightly - and overstating is the right
/// direction here, because it can only ever make the verdict kinder.
pub fn bits_per_pixel(source: &PlaybackSource) -> Option<f64> {
    let (Some(width), Some(height), Some(size)) = (truthy(source.width), truthy(source.height), truthy(source.size))
    else {
        return None;
    };
    if size <= 0.0 {
        return None;
    }
    if !width.is_finite() || !height.is_finite() || !size.is_finite() {
        return None;
    }
    let frames = frame_total(source)?;
    let pixels = width * height * frames;
    if pixels <= 0.0 {
        return None;
    }
    Some((size * 8.0) / pixels)
}

/// Longest edge, so a portrait recording and a rotated one are judged alike.
pub fn longest_edge(size: &AvSourceSize) -> f64 {
    size.width.unwrap_or(0.0).max(size.height.unwrap_or(0.0))
}

impl From<&crate::db::models::Upload> for PlaybackSource {
    fn from(u: &crate::db::models::Upload) -> Self {
        PlaybackSource {
            mime_type: Some(u.mime_type.clone()),
            ext: Some(u.ext.clone()),
            size: Some(u.size as f64),
            width: u.width.map(f64::from),
            height: u.height.map(f64::from),
            frames: u.frames.map(f64::from),
            duration_seconds: u.duration_seconds,
            fps: u.fps,
        }
    }
}

/// What is wrong with this video's shape, or `None` when nothing is.
///
/// `None` is the answer for everything that is not a measured video, which is
/// the overwhelming majority of rows: a picture, an audio file, a paste, and
/// every video whose dimensions nobody ever filled in.
pub fn diagnose_playback(source: &PlaybackSource) -> Option<PlaybackDiagnosis> {
    use crate::media::mime::{MediaKind, media_kind};
    if media_kind(source.mime_type.as_deref(), source.ext.as_deref()) != MediaKind::Video {
        return None;
    }
    let size = AvSourceSize { width: source.width, height: source.height };
    let edge = longest_edge(&size);
    if edge <= 0.0 || edge.is_nan() {
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
        starvation_factor: if starved { bpp.map(|b| js_round(TYPICAL_BITS_PER_PIXEL / b)) } else { None },
        repairable: crate::media::av_targets::needs_web_rescue(Some(&size)),
        repair_edge: crate::media::av_targets::WEB_MAX_EDGE,
    })
}

#[cfg(test)]
mod tests {
    //! tests/unit/playback-health.test.ts, the parts that do not need
    //! `mediaKind` (thresholds, frameTotal, bitsPerPixel).
    use super::*;
    use crate::media::av_meta::{AvMeta, frame_count};
    use crate::media::av_targets::WEB_MAX_EDGE;

    /// The reported recording. Both faults at once, and the lowest bpp of all 464.
    fn reported() -> PlaybackSource {
        PlaybackSource {
            mime_type: Some("video/mp4".into()),
            ext: Some("mp4".into()),
            size: Some(2_181_784.0),
            width: Some(5056.0),
            height: Some(1924.0),
            frames: Some(403.0),
            duration_seconds: Some(13.433),
            fps: Some(30.0),
        }
    }

    /// The same owner, the same capture rig, a recording nobody has complained about.
    fn healthy_wide() -> PlaybackSource {
        PlaybackSource { size: Some(16_134_048.0), frames: Some(218.0), duration_seconds: Some(7.267), ..reported() }
    }

    /// An ordinary 1080p clip. Nothing about its shape is remarkable.
    fn ordinary() -> PlaybackSource {
        PlaybackSource {
            mime_type: Some("video/mp4".into()),
            ext: Some("mp4".into()),
            size: Some(2_061_014.0),
            width: Some(1280.0),
            height: Some(720.0),
            frames: Some(395.0),
            duration_seconds: Some(13.19),
            fps: Some(30.0),
        }
    }

    fn source(frames: Option<f64>, duration: Option<f64>, fps: Option<f64>) -> PlaybackSource {
        PlaybackSource { frames, duration_seconds: duration, fps, ..Default::default() }
    }

    // describe("the thresholds themselves")

    #[test]
    fn derives_the_bitrate_line_from_macroblocks_not_from_taste() {
        assert_eq!(STARVED_BITS_PER_PIXEL, MIN_BITS_PER_MACROBLOCK / MACROBLOCK_PIXELS);
        assert!((STARVED_BITS_PER_PIXEL - 0.0078125).abs() < 1e-10);
    }

    #[test]
    fn keeps_the_hardware_wall_and_the_repair_size_apart() {
        // 4096 is where playback breaks; 1920 is where the repair parks the output.
        assert_eq!(HARDWARE_DECODER_MAX_EDGE, 4096.0);
        const { assert!(WEB_MAX_EDGE < HARDWARE_DECODER_MAX_EDGE) };
    }

    #[test]
    fn compares_against_what_a_normal_video_here_really_carries() {
        const { assert!(TYPICAL_BITS_PER_PIXEL > 0.06) };
        const { assert!(TYPICAL_BITS_PER_PIXEL < 0.07) };
        const { assert!(TYPICAL_BITS_PER_PIXEL / STARVED_BITS_PER_PIXEL > 8.0) };
    }

    // describe("frameTotal")

    #[test]
    fn prefers_the_count_when_there_is_one() {
        assert_eq!(frame_total(&source(Some(403.0), Some(13.4), Some(30.0))), Some(403.0));
    }

    #[test]
    fn falls_back_to_duration_times_rate_which_is_one_video_in_twenty() {
        assert_eq!(frame_total(&source(None, Some(40.0), Some(29.97))), Some(1199.0));
    }

    #[test]
    fn answers_none_rather_than_zero_when_neither_is_available() {
        assert_eq!(frame_total(&PlaybackSource::default()), None);
        assert_eq!(frame_total(&source(None, Some(40.0), None)), None);
        assert_eq!(frame_total(&source(None, None, Some(30.0))), None);
        assert_eq!(frame_total(&source(Some(0.0), None, None)), None);
        assert_eq!(frame_total(&source(None, Some(0.001), Some(1.0))), None);
    }

    #[test]
    fn survives_the_nonsense_a_damaged_container_reports() {
        assert_eq!(frame_total(&source(None, Some(f64::NAN), Some(30.0))), None);
        assert_eq!(frame_total(&source(None, Some(f64::INFINITY), Some(30.0))), None);
        assert_eq!(frame_total(&source(None, Some(10.0), Some(f64::NAN))), None);
    }

    #[test]
    fn is_the_same_implementation_the_upload_path_and_the_backfill_use() {
        let meta = AvMeta {
            width: Some(1920),
            height: Some(1080),
            frames: None,
            duration_seconds: Some(40.0),
            fps: Some(29.97),
        };
        assert_eq!(frame_count(&meta), frame_total(&source(None, Some(40.0), Some(29.97))));
        assert_eq!(frame_count(&AvMeta { frames: Some(7), ..meta }), Some(7.0));
    }

    // describe("bitsPerPixel")

    #[test]
    fn reproduces_the_reported_recordings_measurement() {
        // 2.18 MB over 5056 x 1924 x 403 = 0.00445, the lowest of 464 production
        // videos, and 1.14 bits per 16x16 macroblock.
        let bpp = bits_per_pixel(&reported()).unwrap();
        assert!((bpp - 0.00445).abs() < 0.000005);
        assert!((bpp * MACROBLOCK_PIXELS - 1.14).abs() < 0.005);
    }

    #[test]
    fn puts_the_same_owners_healthy_recording_an_order_of_magnitude_higher() {
        assert!((bits_per_pixel(&healthy_wide()).unwrap() - 0.0609).abs() < 0.00005);
        assert!(bits_per_pixel(&healthy_wide()).unwrap() / bits_per_pixel(&reported()).unwrap() > 13.0);
    }

    #[test]
    fn is_none_not_zero_when_the_row_was_never_measured() {
        let s = |size: f64, w: Option<f64>, h: Option<f64>, frames: Option<f64>| PlaybackSource {
            size: Some(size),
            width: w,
            height: h,
            frames,
            ..Default::default()
        };
        assert_eq!(bits_per_pixel(&s(1000.0, None, None, Some(10.0))), None);
        assert_eq!(bits_per_pixel(&s(0.0, Some(100.0), Some(100.0), Some(10.0))), None);
        assert_eq!(bits_per_pixel(&s(1000.0, Some(100.0), Some(100.0), None)), None);
    }

    // describe("diagnosePlayback")

    fn video(size: f64, w: Option<f64>, h: Option<f64>, frames: Option<f64>) -> PlaybackSource {
        PlaybackSource {
            mime_type: Some("video/mp4".into()),
            ext: Some("mp4".into()),
            size: Some(size),
            width: w,
            height: h,
            frames,
            ..Default::default()
        }
    }

    #[test]
    fn names_both_faults_on_the_recording_that_started_this() {
        let d = diagnose_playback(&reported()).unwrap();
        assert_eq!(d.faults, [PlaybackFault::Oversized, PlaybackFault::Starved]);
        assert_eq!(d.edge, 5056.0);
        assert!(d.repairable);
        assert_eq!(d.repair_edge, WEB_MAX_EDGE);
        assert_eq!(d.starvation_factor, Some(15.0));
        let d = diagnose_playback(&healthy_wide()).unwrap();
        assert_eq!(d.faults, [PlaybackFault::Oversized]);
        assert!(d.repairable);
        assert_eq!(d.starvation_factor, None);
        assert_eq!(diagnose_playback(&ordinary()), None);
    }

    #[test]
    fn leaves_the_file_just_above_the_line_alone() {
        // 0.00805 bits per pixel, 2.06 bits per macroblock: the margin.
        let just_above = PlaybackSource {
            size: Some(7_549_000.0),
            width: Some(1080.0),
            height: Some(1920.0),
            frames: Some(3619.0),
            duration_seconds: Some(120.6),
            fps: Some(30.0),
            ..ordinary()
        };
        assert!(bits_per_pixel(&just_above).unwrap() * MACROBLOCK_PIXELS > MIN_BITS_PER_MACROBLOCK);
        assert_eq!(diagnose_playback(&just_above), None);
    }

    #[test]
    fn fires_exactly_at_the_wall_and_judges_the_longest_edge() {
        let at = |w: f64| PlaybackSource {
            width: Some(w),
            height: Some(1000.0),
            frames: Some(1.0),
            size: Some(50_000_000.0),
            ..ordinary()
        };
        assert_eq!(diagnose_playback(&at(HARDWARE_DECODER_MAX_EDGE)), None);
        assert_eq!(diagnose_playback(&at(HARDWARE_DECODER_MAX_EDGE + 1.0)).unwrap().faults, [PlaybackFault::Oversized]);
        let tall = PlaybackSource {
            width: Some(2160.0),
            height: Some(4320.0),
            size: Some(400_000_000.0),
            frames: Some(100.0),
            ..ordinary()
        };
        let d = diagnose_playback(&tall).unwrap();
        assert_eq!(d.faults, [PlaybackFault::Oversized]);
        assert_eq!(d.edge, 4320.0);
    }

    #[test]
    fn fails_closed_on_a_row_nobody_has_measured() {
        assert_eq!(diagnose_playback(&video(900.0, None, None, None)), None);
        let no_frames = diagnose_playback(&video(900.0, Some(5056.0), Some(1924.0), None)).unwrap();
        assert_eq!(no_frames.faults, [PlaybackFault::Oversized]);
        assert_eq!(no_frames.bits_per_pixel, None);
        assert_eq!(no_frames.starvation_factor, None);
    }

    #[test]
    fn is_about_video_and_nothing_else() {
        for (mime, ext) in
            [("image/png", "png"), ("image/gif", "gif"), ("audio/mpeg", "mp3"), ("application/pdf", "pdf")]
        {
            let s = PlaybackSource {
                mime_type: Some(mime.into()),
                ext: Some(ext.into()),
                ..video(400.0, Some(8000.0), Some(8000.0), Some(1.0))
            };
            assert_eq!(diagnose_playback(&s), None, "{mime}");
        }
        let octet = PlaybackSource { mime_type: Some("application/octet-stream".into()), ..reported() };
        assert_eq!(diagnose_playback(&octet).unwrap().faults, [PlaybackFault::Oversized, PlaybackFault::Starved]);
    }

    #[test]
    fn offers_no_repair_for_a_starved_file_that_is_already_small() {
        let small = PlaybackSource {
            size: Some(460_000.0),
            width: Some(1624.0),
            height: Some(916.0),
            frames: Some(480.0),
            duration_seconds: Some(16.0),
            fps: Some(30.0),
            ..ordinary()
        };
        let d = diagnose_playback(&small).unwrap();
        assert_eq!(d.faults, [PlaybackFault::Starved]);
        assert!(!d.repairable);
        for edge in [WEB_MAX_EDGE - 1.0, WEB_MAX_EDGE, WEB_MAX_EDGE + 1.0, 5056.0] {
            if let Some(d) = diagnose_playback(&video(1000.0, Some(edge), Some(400.0), Some(500.0))) {
                assert_eq!(d.repairable, edge > WEB_MAX_EDGE);
            }
        }
    }

    #[test]
    fn serialises_like_the_upstream_object() {
        let v = serde_json::to_value(diagnose_playback(&healthy_wide()).unwrap()).unwrap();
        assert_eq!(
            v.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["faults", "edge", "bitsPerPixel", "starvationFactor", "repairable", "repairEdge"]
        );
        assert_eq!(v["faults"], serde_json::json!(["oversized"]));
    }
}
