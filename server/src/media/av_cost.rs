//! Port of `lib/media/av-cost.ts`: what one ffmpeg job is allowed to cost.
//!
//! The image side (`media::transform_cost`) budgets PIXELS, because a still
//! encoder's cost is a function of area. That is the wrong meter for video: a
//! 37 MB clip and a 37 MB clip of the same resolution can differ by a hundredfold
//! in work depending on whether an encoder runs at all. So the meters here are
//! the ones the operation is actually bound by - bytes for a copy, seconds for an
//! encode - and each ceiling is solved from a measurement.
//!
//! Measured in the production image (ffmpeg 5.1.9), in a container pinned to
//! `--cpus 2 --memory 2g` to match production, against real production uploads:
//!
//! ```text
//!   operation                              wall      output      throughput
//!   .mov -> .mp4, 184 kB (copy)            128 ms     188 kB     ~1.4 GB/s
//!   .mov -> .mkv, 184 kB (copy)            127 ms     187 kB
//!   .mkv -> .mov, 37 MB  (copy)            269 ms      37 MB     ~140 MB/s
//!   .mkv -> .mkv, 37 MB  (copy)            234 ms      37 MB
//!   5.0 MB / 209 s MP3 -> MP4 still       2196 ms     5.1 MB     ~95x realtime
//!   ...same, audio re-encoded to AAC     10920 ms     5.2 MB     ~19x realtime
//!   2.4 MB / 5.1 s GIF -> MP4              231 ms      32 kB     ~22x realtime
//!   209 s MP3 -> OGG (encode)             4001 ms     3.1 MB     ~52x realtime
//!   5 s clip @480px -> gif (encode)        968 ms                 ~5x realtime
//!   43-frame AVIF -> anim WebP             925 ms     457 kB
//!   whole 32 s file -> 480p h264          5556 ms                 ~6x realtime
//! ```
//!
//! Two conclusions drive everything below.
//!
//! First, MEMORY IS NOT THE CONSTRAINT for video the way it is for stills. The
//! worst case measured is 166 MB, against the 838 MB a 36 Mpx WebP encode costs
//! and the 1380 MB an AVIF encode costs. ffmpeg streams; libvips materialises.
//! So a video job takes the same two-unit conversion slot as an image conversion
//! and lands comfortably inside its 512 MiB share, and the ceilings below are
//! about TIME instead.
//!
//! Second, at six times realtime a full re-encode of a video's PICTURE cannot be
//! an HTTP request. Ten minutes of video is a hundred seconds of both cores. That
//! is why every container entry in av-targets.ts is a stream copy, and why the
//! only encodes on offer are bounded by construction: a clip of at most 15
//! seconds, an audio track, or an animation that is short because animations are.
//!
//! The throughput numbers are for a job with both cores. Two may run at once, so
//! every ceiling is solved against HALF the measured rate.

use crate::media::av_targets::{AvCost, AvSourceKind, AvTarget, VideoContainer};

// ---------------------------------------------------------------------------
// Slot and deadlines
// ---------------------------------------------------------------------------

/// Units of the shared limiter an ffmpeg job holds - the same two an image
/// conversion takes, so at most two run at once and neither kind can crowd the
/// other out. Sized by memory (peak 166 MB against a 512 MiB share) with room for
/// inputs larger than anything on production today.
pub const AV_WEIGHT: u32 = 2;

/// Deadline for a stream copy.
///
/// A 512 MiB remux is 3.7 s of measured throughput, 7.5 s if a second job is
/// sharing the cores. 45 s is that with room for a cold page cache, and it is a
/// bound on the RESPONSE - the process itself is killed by its own group timeout
/// (`media::ffmpeg`), which for these is the same number.
pub const AV_COPY_TIMEOUT_MS: u64 = 45_000;

/// Deadline for anything that runs an encoder.
pub const AV_ENCODE_TIMEOUT_MS: u64 = 60_000;

/// ...and the longer one for the single target that re-encodes a video's whole
/// PICTURE, which is a different order of work from everything else here.
///
/// Measured on the production container (two cores, 2 GiB, ffmpeg 5.1.9) against
/// real uploads, converting each to a 1920-edge H.264 copy:
///
/// ```text
///   source                              total pixels   wall     rate
///   5056x1924, 403 frames, 2.1 MB          3.92 Gpx    9.5 s   412 Mpx/s
///   2560x1440, 1150 frames, 76 MB          4.24 Gpx   31.6 s   134 Mpx/s
///   3840x2160,  903 frames, 72 MB          7.49 Gpx   61.7 s   121 Mpx/s
/// ```
///
/// The three rates differ by more than three to one, and the reason is worth
/// writing down: the first file is the starved capture this feature exists for -
/// 1.3 Mbit/s of residual for 9.7 Mpx frames - so its DECODE is nearly free
/// (3.4 s measured on its own, against 19.4 s for the 4K file). Source pixels is
/// therefore the conservative meter, and the ceiling below is solved against the
/// WORST of the three rather than the average.
///
/// 8 Gpx at 121 Mpx/s is 66 s alone and about 132 s with a second job sharing the
/// cores, which is what AV_WEIGHT allows. 150 s is that with headroom.
pub const AV_WEB_TIMEOUT_MS: u64 = 150_000;

/// ffprobe is a header read. If it has not answered by now the file is broken.
pub const AV_PROBE_TIMEOUT_MS: u64 = 10_000;

/// The deadline for one option, in one place.
///
/// Both the argument builder and the limiter need this number, and they used to
/// derive it separately from `cost`. That was fine while every encode shared one
/// ceiling; the moment one of them did not, two derivations became two chances to
/// give a job less time than its own process is allowed - which shows up as a
/// limiter that gives up while ffmpeg is still working, and a slot released to a
/// caller whose predecessor is still spending both cores.
pub fn av_timeout_for(to: AvTarget, cost: AvCost) -> u64 {
    if to == AvTarget::Webmp4 {
        return AV_WEB_TIMEOUT_MS;
    }
    if cost == AvCost::Encode { AV_ENCODE_TIMEOUT_MS } else { AV_COPY_TIMEOUT_MS }
}

// ---------------------------------------------------------------------------
// Input ceilings
// ---------------------------------------------------------------------------

/// Largest file ffmpeg will be pointed at.
///
/// Comfortably above everything on production (largest video: 400 MB) while
/// keeping the worst-case copy inside its deadline. Note this is far above
/// MAX_TRANSFORM_BYTES (32 MB) on the image path, and it can be: nothing here
/// reads the file into memory, ffmpeg streams from disk and writes to disk.
pub const AV_MAX_INPUT_BYTES: u64 = 512 * 1024 * 1024;

/// Longest source an audio extraction will re-encode.
///
/// 52x realtime measured, 26x assuming a second job shares the cores: 15 minutes
/// of audio is 35 s of work, which fits the encode deadline. Beyond this the
/// honest answer is a refusal that names the limit, not a truncated file the user
/// has no way of knowing is truncated.
pub const AV_MAX_AUDIO_SECONDS: u64 = 15 * 60;

/// ...and the much shorter one for WAV, which is bounded by its OUTPUT.
///
/// PCM does not compress: 16-bit stereo at 44.1 kHz is 10.6 MB per minute
/// whatever went in. At the general 15 minute ceiling a single click would
/// produce a 158 MB download and a 158 MB cache entry, from a 5 MB source. Five
/// minutes caps that at roughly 53 MB, which is a file somebody might actually
/// want. The other two encoders shrink their input, so only this one needs it.
pub const AV_MAX_WAV_SECONDS: u64 = 5 * 60;

/// Longest audio that will be wrapped in a video with a still picture, when the
/// sound has to be RE-ENCODED to get there.
///
/// Two very different rates hide behind one menu entry, so there are two
/// ceilings. Measured on a 209 s production MP3: with `-c:a copy` (which MP4
/// allows for AAC, MP3 and ALAC) the whole job is 2196 ms, about 95x realtime -
/// the still picture is nearly all of it, and 15 minutes costs under 20 s even
/// with a second job on the cores. Re-encoding to AAC instead takes 10920 ms,
/// 19x realtime, so at half rate 15 minutes would be 95 s and blow the 60 s
/// deadline. Six minutes leaves it at 38 s, which fits with room to spare.
///
/// A WAV or a FLAC therefore gets the shorter ceiling and an MP3 does not, which
/// is exactly the distinction the user would draw if they knew how the file was
/// built.
pub const AV_MAX_AUDIO_VIDEO_SECONDS: u64 = 6 * 60;

/// Longest animation that will be re-encoded whole.
///
/// Unlike a video clip, an animation is converted in full - taking the first six
/// seconds of a 43-frame loop would be vandalism. So the bound is the source's
/// own length, and 2 minutes is far past anything a GIF or an AVIF sequence
/// plausibly is (the largest on production is 5.1 s). At the 22x realtime
/// measured for GIF -> MP4, and half that under load, 2 minutes is 11 s of work.
pub const AV_MAX_ANIMATION_SECONDS: u64 = 120;

/// Most source pixels a web-playable copy will re-encode: width x height x
/// frames, counted over the whole file.
///
/// SECONDS is the wrong meter here, and it is the one every other encode on this
/// menu uses. A minute of 3840x2160 at 60 fps is four times the work of a minute
/// of 2560x1440 at 30, and this target is only ever offered to files above
/// 1920 - that is, to exactly the recordings where the difference is largest. So
/// the budget counts frames, the way the image side counts them
/// (`media::transform_cost`), rather than trusting a duration.
///
/// What 8 Gpx admits, in the shapes that actually turn up here:
///
/// ```text
///   3840x2160 @60     15 s        2560x1440 @60     36 s
///   3840x2160 @30     31 s        2560x1440 @30     72 s
///   5056x1924 @30     27 s        2560x1080 @60     48 s
/// ```
///
/// Every ShareX capture on this server fits. A ten minute 4K recording does not,
/// and is refused with `too-many-pixels` and the limit, because at roughly two
/// minutes of both cores it is not an HTTP request - it is a background job this
/// application does not have.
pub const AV_MAX_WEB_PIXELS: u64 = 8_000_000_000;

/// Longest clip an animation target will produce from a VIDEO, and the default
/// when the caller does not ask for one.
///
/// A GIF runs at about 5x realtime and a 15 s clip at 480px is roughly 3 s of
/// work. The cap is also a kindness: a 30-second GIF is tens of megabytes and
/// nobody wants the file they just asked for.
pub const AV_MAX_CLIP_SECONDS: f64 = 15.0;
pub const AV_DEFAULT_CLIP_SECONDS: f64 = 6.0;

/// Frame rate and width an animation is rendered at. Bounded, never chosen.
pub const AV_ANIMATION_FPS: u32 = 12;
pub const AV_ANIMATION_WIDTH: u32 = 480;

/// Frame rate of the still-picture video built around an audio track.
///
/// One frame per second, not the 25 a video normally carries, because every frame
/// is the same frame. Measured on the 209 s MP3: at 1 fps the encode is 2196 ms
/// and the picture adds 86 kB to a 5.0 MB source; at 2 fps it is 4210 ms and
/// 125 kB. Nobody watching a static logo can tell the difference, and halving the
/// work of the single most expensive entry in the menu is worth having.
pub const AV_STILL_FPS: u32 = 1;

// ---------------------------------------------------------------------------
// Verdicts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvBudgetReason {
    TooLarge,
    TooLong,
    TooManyPixels,
}

impl AvBudgetReason {
    /// The wire spelling (`"too-large"`).
    pub fn as_str(self) -> &'static str {
        match self {
            AvBudgetReason::TooLarge => "too-large",
            AvBudgetReason::TooLong => "too-long",
            AvBudgetReason::TooManyPixels => "too-many-pixels",
        }
    }
}

/// `AvBudget`: `{ ok: true, clipSeconds }` or `{ ok: false, reason, limit }`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AvBudget {
    Ok { clip_seconds: Option<f64> },
    Refused { reason: AvBudgetReason, limit: u64 },
}

impl AvBudget {
    pub fn ok(&self) -> bool {
        matches!(self, AvBudget::Ok { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AvBudgetInput {
    pub bytes: u64,
    /// Seconds, or None when the container does not say.
    pub duration: Option<f64>,
    /// What the ROW says the source is - see AvSourceKind.
    pub source_kind: Option<AvSourceKind>,
    /// Can the audio track be copied into MP4 rather than re-encoded? Known only
    /// after the probe, and it moves the audio-to-video ceiling by a factor of five.
    pub audio_copyable: bool,
    /// Picture pixels in the whole file (width x height x frames), from the probe.
    ///
    /// None means the file did not say how many frames it holds AND its duration
    /// or frame rate is missing too - see `media::av_meta`, which counts first
    /// and estimates second. Only the web target reads it, and it treats None as
    /// "cannot promise to finish" rather than as "small".
    pub pixels: Option<f64>,
}

/// May this job run, and over how much of the file?
///
/// Pure, so the interesting cases are unit tests. `duration` may be None - some
/// containers do not carry one - and that is treated as "unknown, not unlimited"
/// for whole-file encodes: a file we cannot measure is one we cannot promise to
/// finish, and the clip targets bound themselves anyway.
pub fn av_budget(to: AvTarget, cost: AvCost, input: &AvBudgetInput, requested_clip: Option<f64>) -> AvBudget {
    if input.bytes > AV_MAX_INPUT_BYTES {
        return AvBudget::Refused { reason: AvBudgetReason::TooLarge, limit: AV_MAX_INPUT_BYTES };
    }

    // A copy is bounded by size alone; length does not enter into it.
    if cost == AvCost::Copy {
        return AvBudget::Ok { clip_seconds: None };
    }

    let whole_animation = input.source_kind == Some(AvSourceKind::ImageSequence);

    // The one target whose cost is neither bytes nor seconds. See
    // AV_MAX_WEB_PIXELS for why, and for what the number admits.
    if to == AvTarget::Webmp4 {
        return match input.pixels {
            // Unmeasurable, which is not the same as small. Refused as "too long" -
            // the honest reading of "we cannot tell how much of this there is" - and
            // it is the same refusal an audio encode gives for a missing duration.
            None => AvBudget::Refused { reason: AvBudgetReason::TooLong, limit: AV_MAX_WEB_PIXELS },
            Some(p) if p <= 0.0 => AvBudget::Refused { reason: AvBudgetReason::TooLong, limit: AV_MAX_WEB_PIXELS },
            Some(p) if p > AV_MAX_WEB_PIXELS as f64 => {
                AvBudget::Refused { reason: AvBudgetReason::TooManyPixels, limit: AV_MAX_WEB_PIXELS }
            }
            Some(_) => AvBudget::Ok { clip_seconds: None },
        };
    }

    if to == AvTarget::Gif || to == AvTarget::Webp {
        if whole_animation {
            return animation_budget(input.duration);
        }
        let asked = requested_clip.unwrap_or(AV_DEFAULT_CLIP_SECONDS);
        // Clamped, never refused: asking for a longer GIF than we make is a
        // preference, not an error, and the response says how long it really is.
        // `Math.min(Math.max(1, asked), 15)`; NaN stays NaN in both.
        let clip = asked.clamp(1.0, AV_MAX_CLIP_SECONDS);
        let clip_seconds = match input.duration {
            Some(d) if d != 0.0 && !d.is_nan() => clip.min(d),
            _ => clip,
        };
        return AvBudget::Ok { clip_seconds: Some(clip_seconds) };
    }

    // MP4 is the one target reachable from two very different sources: an
    // animation being re-encoded whole, and an audio track being given a picture.
    if to == AvTarget::Container(VideoContainer::Mp4) {
        if whole_animation {
            return animation_budget(input.duration);
        }
        let ceiling = if input.audio_copyable { AV_MAX_AUDIO_SECONDS } else { AV_MAX_AUDIO_VIDEO_SECONDS };
        return whole_file(input.duration, ceiling);
    }

    // Whole-file audio encode: the only target whose cost really is the length.
    // WAV answers to its output size instead - see AV_MAX_WAV_SECONDS.
    whole_file(input.duration, if to == AvTarget::Wav { AV_MAX_WAV_SECONDS } else { AV_MAX_AUDIO_SECONDS })
}

fn whole_file(duration: Option<f64>, ceiling: u64) -> AvBudget {
    match duration {
        Some(d) if d > ceiling as f64 => AvBudget::Refused { reason: AvBudgetReason::TooLong, limit: ceiling },
        Some(_) => AvBudget::Ok { clip_seconds: None },
        None => AvBudget::Refused { reason: AvBudgetReason::TooLong, limit: ceiling },
    }
}

/// An animation is converted whole, so there is no clip to return - only a
/// refusal if it is implausibly long. A missing duration is allowed here, unlike
/// for audio: an animation's cost is bounded by its own frame count, and the
/// input size ceiling already bounds that.
fn animation_budget(duration: Option<f64>) -> AvBudget {
    match duration {
        Some(d) if d > AV_MAX_ANIMATION_SECONDS as f64 => {
            AvBudget::Refused { reason: AvBudgetReason::TooLong, limit: AV_MAX_ANIMATION_SECONDS }
        }
        _ => AvBudget::Ok { clip_seconds: None },
    }
}

/// Width to render an animation at: never up, only down.
///
/// Upscaling a 160px source to 480 makes a bigger file out of the same picture,
/// which is the opposite of what a share-ready clip wants.
pub fn animation_width(source_width: Option<f64>) -> f64 {
    match source_width {
        Some(w) if w > 0.0 => w.min(AV_ANIMATION_WIDTH as f64),
        _ => AV_ANIMATION_WIDTH as f64,
    }
}

#[cfg(test)]
mod tests {
    //! The av-cost parts of tests/unit/av-targets.test.ts ("the video budget")
    //! and tests/unit/web-playable.test.ts ("the pixel budget", deadlines).
    use super::*;

    const MP4: AvTarget = AvTarget::Container(VideoContainer::Mp4);

    fn input(bytes: u64, duration: Option<f64>) -> AvBudgetInput {
        AvBudgetInput { bytes, duration, ..Default::default() }
    }

    fn refused(reason: AvBudgetReason, limit: u64) -> AvBudget {
        AvBudget::Refused { reason, limit }
    }

    #[test]
    fn refuses_an_input_larger_than_ffmpeg_is_ever_pointed_at() {
        let verdict = av_budget(MP4, AvCost::Copy, &input(AV_MAX_INPUT_BYTES + 1, Some(10.0)), None);
        assert_eq!(verdict, refused(AvBudgetReason::TooLarge, AV_MAX_INPUT_BYTES));
    }

    #[test]
    fn does_not_care_how_long_a_stream_copy_is_only_how_big() {
        // A copy's cost is bytes moved. A three-hour file that fits the size ceiling
        // is 4 s of work, and refusing it for its length would be superstition.
        for c in [VideoContainer::Mp4, VideoContainer::Mov, VideoContainer::Mkv, VideoContainer::Webm] {
            assert_eq!(
                av_budget(AvTarget::Container(c), AvCost::Copy, &input(1000, Some(3.0 * 3600.0)), None),
                AvBudget::Ok { clip_seconds: None }
            );
        }
    }

    #[test]
    fn refuses_an_audio_encode_longer_than_it_can_finish_inside_the_request() {
        assert_eq!(
            av_budget(AvTarget::Mp3, AvCost::Encode, &input(1000, Some(AV_MAX_AUDIO_SECONDS as f64 + 1.0)), None),
            refused(AvBudgetReason::TooLong, AV_MAX_AUDIO_SECONDS)
        );
        assert!(av_budget(AvTarget::Mp3, AvCost::Encode, &input(1000, Some(60.0)), None).ok());
    }

    #[test]
    fn holds_wav_to_a_much_shorter_ceiling_because_pcm_does_not_compress() {
        const { assert!(AV_MAX_WAV_SECONDS < AV_MAX_AUDIO_SECONDS) };
        let long = input(1000, Some(AV_MAX_WAV_SECONDS as f64 + 1.0));
        assert_eq!(
            av_budget(AvTarget::Wav, AvCost::Encode, &long, None),
            refused(AvBudgetReason::TooLong, AV_MAX_WAV_SECONDS)
        );
        // ...while the same length is fine as an MP3.
        assert!(av_budget(AvTarget::Mp3, AvCost::Encode, &long, None).ok());
    }

    #[test]
    fn charges_audio_as_video_by_whether_the_sound_has_to_be_reencoded() {
        // Measured on a 209 s MP3: 2196 ms when MP4 takes the track as it is,
        // 10920 ms when it has to become AAC.
        const { assert!(AV_MAX_AUDIO_VIDEO_SECONDS < AV_MAX_AUDIO_SECONDS) };
        let long = AvBudgetInput {
            bytes: 1000,
            duration: Some(AV_MAX_AUDIO_SECONDS as f64 - 1.0),
            source_kind: Some(AvSourceKind::Audio),
            ..Default::default()
        };
        assert!(av_budget(MP4, AvCost::Encode, &AvBudgetInput { audio_copyable: true, ..long }, None).ok());
        assert_eq!(
            av_budget(MP4, AvCost::Encode, &AvBudgetInput { audio_copyable: false, ..long }, None),
            refused(AvBudgetReason::TooLong, AV_MAX_AUDIO_VIDEO_SECONDS)
        );
    }

    #[test]
    fn treats_an_unknown_duration_as_unknown_not_as_unlimited() {
        assert!(!av_budget(AvTarget::Mp3, AvCost::Encode, &input(1000, None), None).ok());
        let audio =
            AvBudgetInput { bytes: 1000, duration: None, source_kind: Some(AvSourceKind::Audio), ..Default::default() };
        assert!(!av_budget(MP4, AvCost::Encode, &audio, None).ok());
    }

    #[test]
    fn converts_an_animation_whole_and_refuses_one_that_is_implausibly_long() {
        let sequence = AvBudgetInput {
            bytes: 1000,
            duration: Some(2.15),
            source_kind: Some(AvSourceKind::ImageSequence),
            ..Default::default()
        };
        assert_eq!(
            av_budget(AvTarget::Webp, AvCost::Encode, &sequence, Some(6.0)),
            AvBudget::Ok { clip_seconds: None }
        );
        assert_eq!(av_budget(MP4, AvCost::Encode, &sequence, None), AvBudget::Ok { clip_seconds: None });
        // A duration we could not read is fine: the frame count bounds the work.
        assert!(av_budget(MP4, AvCost::Encode, &AvBudgetInput { duration: None, ..sequence }, None).ok());
        assert_eq!(
            av_budget(
                AvTarget::Gif,
                AvCost::Encode,
                &AvBudgetInput { duration: Some(AV_MAX_ANIMATION_SECONDS as f64 + 1.0), ..sequence },
                None
            ),
            refused(AvBudgetReason::TooLong, AV_MAX_ANIMATION_SECONDS)
        );
    }

    #[test]
    fn clamps_a_clip_instead_of_refusing_it_and_never_past_the_end_of_the_file() {
        let gif = |duration: f64, clip: Option<f64>| {
            av_budget(AvTarget::Gif, AvCost::Encode, &input(1000, Some(duration)), clip)
        };
        assert_eq!(gif(600.0, Some(9999.0)), AvBudget::Ok { clip_seconds: Some(AV_MAX_CLIP_SECONDS) });
        assert_eq!(gif(600.0, None), AvBudget::Ok { clip_seconds: Some(AV_DEFAULT_CLIP_SECONDS) });
        // A 2 s source cannot yield a 6 s clip.
        assert_eq!(gif(2.15, Some(6.0)), AvBudget::Ok { clip_seconds: Some(2.15) });
        // Zero and negatives land on the floor rather than producing an empty file.
        assert_eq!(gif(600.0, Some(-5.0)), AvBudget::Ok { clip_seconds: Some(1.0) });
    }

    #[test]
    fn scales_an_animation_down_but_never_up() {
        assert_eq!(animation_width(Some(1920.0)), 480.0);
        assert_eq!(animation_width(Some(320.0)), 320.0);
        assert_eq!(animation_width(None), 480.0);
    }

    // describe("the pixel budget")

    fn web(pixels: Option<f64>) -> AvBudget {
        av_budget(
            AvTarget::Webmp4,
            AvCost::Encode,
            &AvBudgetInput { bytes: 2_179_538, duration: Some(13.434), pixels, ..Default::default() },
            None,
        )
    }

    #[test]
    fn admits_the_recording_it_was_built_for() {
        // 5056 x 1924 x 403 = 3.92 Gpx, measured at 9.5 s on the production container.
        assert_eq!(web(Some(5056.0 * 1924.0 * 403.0)), AvBudget::Ok { clip_seconds: None });
    }

    #[test]
    fn counts_frames_not_seconds() {
        let at = |pixels: f64| {
            av_budget(
                AvTarget::Webmp4,
                AvCost::Encode,
                &AvBudgetInput { bytes: 1, duration: Some(60.0), pixels: Some(pixels), ..Default::default() },
                None,
            )
        };
        assert!(at(2_000_000_000.0).ok());
        assert!(!at(20_000_000_000.0).ok());
    }

    #[test]
    fn refuses_over_the_ceiling_with_the_reason_and_the_limit() {
        assert_eq!(
            web(Some(AV_MAX_WEB_PIXELS as f64 + 1.0)),
            refused(AvBudgetReason::TooManyPixels, AV_MAX_WEB_PIXELS)
        );
    }

    #[test]
    fn treats_an_unmeasurable_file_as_too_long_rather_than_as_small() {
        assert!(!web(None).ok());
        assert!(!web(Some(0.0)).ok());
    }

    #[test]
    fn still_refuses_a_file_past_the_byte_ceiling_before_it_looks_at_pixels() {
        let verdict = av_budget(
            AvTarget::Webmp4,
            AvCost::Encode,
            &AvBudgetInput { bytes: 600 * 1024 * 1024, duration: Some(10.0), pixels: Some(1.0), ..Default::default() },
            None,
        );
        assert!(matches!(verdict, AvBudget::Refused { reason: AvBudgetReason::TooLarge, .. }));
    }

    #[test]
    fn gives_the_web_job_the_long_deadline_and_says_so_in_one_place() {
        assert_eq!(av_timeout_for(AvTarget::Webmp4, AvCost::Encode), AV_WEB_TIMEOUT_MS);
        assert!(av_timeout_for(AvTarget::Mp3, AvCost::Encode) < AV_WEB_TIMEOUT_MS);
        assert!(av_timeout_for(AvTarget::Container(VideoContainer::Mkv), AvCost::Copy) < AV_WEB_TIMEOUT_MS);
    }
}
