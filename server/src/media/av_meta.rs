//! Port of `lib/media/av-meta.ts`: what a stored audio or video file IS, in
//! the five columns the row keeps.
//!
//! Until this existed, `select count(*) filter (where width is not null) from
//! uploads where mime_type like 'video/%'` answered 0 out of 467 on production.
//! Round 8 backfilled 8052 images and video was never done, so every consumer
//! downstream was blind: a tile could not be sized from the row, an embed fell
//! back to a hardcoded 1280x720 whatever the file really was, and the question
//! that started this work - "why does this recording stutter?" - could not be
//! asked at all without opening the bytes again.
//!
//! The answers live here rather than in the two callers because the upload path
//! and the backfill script must write the SAME numbers. Two readings of one
//! ffprobe output is how a repaired row stops matching a freshly uploaded one.
//!
//! Cost, measured in the production image (ffmpeg 5.1.9, `--cpus 2`) over 59 real
//! videos from 0.2 MB to 419 MB: 136 ms median warm, 158 ms median cold, and of
//! that 120 ms is the bare cost of spawning ffprobe at all (measured separately
//! against /dev/null). So the file's SIZE barely enters into it - the 419 MB file
//! answered in 144 ms warm - because a probe reads a header and seeks; it never
//! decodes. That is the whole reason this is affordable on the upload path.

use crate::config::Config;
use crate::media::abort::AbortSignal;
use crate::media::av_cost::{AV_MAX_INPUT_BYTES, AV_PROBE_TIMEOUT_MS};
use crate::media::av_probe::{AvProbe, probe_av};
use crate::media::av_workspace::{OpenedWorkspace, open_av_workspace};
use crate::media::ffmpeg::ffmpeg_available;
use crate::media::playback_health::{PlaybackSource, frame_total};
use crate::storage::StorageDriver;

/// The columns an audio/video row can learn from one probe.
///
/// Integers where the columns are integers: ffprobe reports dimensions and
/// `nb_frames` as whole numbers, and a value that is not one (which ffprobe
/// never writes) is treated as unknown rather than rounded into the row.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AvMeta {
    /// Picture size. None for audio, and for a video stream ffprobe cannot size.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Frames ffprobe COUNTED (`nb_frames`), never a number derived from anything
    /// else. Absent on about one production video in twenty - see frame_count() for
    /// the fallback, which deliberately lives outside the stored column so that
    /// "counted" and "estimated" stay distinguishable in the database.
    pub frames: Option<u32>,
    /// Seconds, or None when the container does not say.
    pub duration_seconds: Option<f64>,
    /// Frames per second; see AvStream.fps.
    pub fps: Option<f64>,
}

/// Nothing learned. Used as the "probe said no" answer so callers never branch
/// on a missing struct.
pub const NO_AV_META: AvMeta = AvMeta { width: None, height: None, frames: None, duration_seconds: None, fps: None };

fn whole(v: Option<f64>) -> Option<u32> {
    v.filter(|n| n.fract() == 0.0 && *n >= 0.0 && *n <= u32::MAX as f64).map(|n| n as u32)
}

/// Project a probe onto the row's columns.
///
/// Pure, so every shape that has ever surprised us is a unit test rather than a
/// production surprise. `probe.video` is already the non-cover-art stream, so an
/// MP3 with an embedded JPEG contributes a duration and nothing else - recording
/// 600x600 for a file that is three minutes of audio would make it look like a
/// picture to every consumer that asks the row whether it has dimensions.
pub fn av_meta_from(probe: &AvProbe) -> AvMeta {
    let video = probe.video.as_ref();
    AvMeta {
        width: whole(video.and_then(|v| v.width)),
        height: whole(video.and_then(|v| v.height)),
        frames: whole(video.and_then(|v| v.frames)),
        duration_seconds: probe.duration,
        fps: video.and_then(|v| v.fps),
    }
}

/// How many picture frames the file holds, counted if possible and estimated if
/// not.
///
/// The estimate is NOT written to `uploads.frames`: that column means "somebody
/// counted", which is exactly the distinction convert-targets draws when it
/// decides whether a WebP is animated. A derived number stored there would make
/// "43 frames" and "roughly 460 frames" indistinguishable forever. Callers that
/// only want a total - total pixels, bits per pixel - ask here.
pub fn frame_count(meta: &AvMeta) -> Option<f64> {
    // One implementation, in the module the browser is allowed to import
    // (playback_health). Two copies of this arithmetic is how the panel's warning
    // and the backfill script would come to disagree about how many frames a file
    // holds.
    frame_total(&PlaybackSource {
        frames: meta.frames.map(f64::from),
        duration_seconds: meta.duration_seconds,
        fps: meta.fps,
        ..Default::default()
    })
}

/// Picture pixels in the whole file: width x height x frames.
///
/// The meter a full re-encode is budgeted against (av_cost), and the same
/// arithmetic that makes a stuttering recording diagnosable - bits per pixel is
/// the file's size in bits divided by this. None when the file holds no picture,
/// or when neither a counted frame total nor a duration-and-rate pair is
/// available to build one.
pub fn total_source_pixels(probe: &AvProbe) -> Option<f64> {
    let meta = av_meta_from(probe);
    let (width, height) = (meta.width.filter(|&w| w > 0)?, meta.height.filter(|&h| h > 0)?);
    let frames = frame_count(&meta).filter(|&f| f > 0.0)?;
    Some(width as f64 * height as f64 * frames)
}

#[derive(Clone, Default)]
pub struct ProbeStoredOptions<'a> {
    /// May the bytes be COPIED out of storage first? `Some(config)` says yes,
    /// and names where the copy goes (`config.uploads.tmp_path`, as
    /// openAvWorkspace reads it).
    ///
    /// None on the upload path, and that is the load-bearing setting. With the
    /// local driver the stored file is already on a filesystem ffmpeg can open, so
    /// a probe costs one process; with S3 there is no such path and honouring this
    /// would mean downloading the whole object back out of the bucket seconds
    /// after uploading it to it. An offline backfill can pay that and says so; a
    /// request finishing an upload cannot.
    pub stage_remote: Option<&'a Config>,
    pub signal: Option<AbortSignal>,
}

/// Probe a file that is already in storage, without ever failing the caller
/// (`probeStoredAvMeta(storageKey)` with the default options).
pub async fn probe_stored_av_meta(storage: &dyn StorageDriver, key: &str) -> Option<AvMeta> {
    probe_stored_av_meta_with(storage, key, ProbeStoredOptions::default()).await
}

/// `probeStoredAvMeta(storageKey, opts)`.
///
/// Returns None - not an error - for every reason there might be no answer: no
/// ffmpeg in the image, no local path and no permission to stage one, a file
/// that is not media we accept, a probe that timed out, a file that vanished.
///
/// Swallowing the timeout is correct HERE and wrong in av_action, and the
/// difference is worth stating because the opposite mistake was a real bug:
/// av_action REMEMBERS a verdict for ten minutes and then serves it, so a
/// killed probe cached as "unreadable" turned a healthy video into a broken one
/// for everybody. Nothing is remembered here. A probe that does not answer
/// leaves the columns NULL, which is what they already were, and the backfill
/// script picks the row up on its next run.
pub async fn probe_stored_av_meta_with(
    storage: &dyn StorageDriver,
    key: &str,
    opts: ProbeStoredOptions<'_>,
) -> Option<AvMeta> {
    if !ffmpeg_available().await {
        return None;
    }

    // A key the driver refuses (traversal) has no local path; upstream threw
    // here, past its own "never fails" promise.
    if let Ok(Some(direct)) = storage.local_path(key) {
        return probe_or_none(&direct.to_string_lossy(), opts.signal).await;
    }

    let config = opts.stage_remote?;

    let Ok(OpenedWorkspace::Ready(ws)) = open_av_workspace(storage, config, key, AV_MAX_INPUT_BYTES).await else {
        return None;
    };
    let meta = probe_or_none(&ws.input.to_string_lossy(), opts.signal).await;
    ws.dispose().await;
    meta
}

/// Not upstream: probe a local file (an upload's temp file, read before the
/// file vault encrypts it; docs/ENCRYPTION.md). Same rules as
/// [`probe_stored_av_meta`]: None for every reason there is no answer.
pub async fn probe_local_av_meta(path: &std::path::Path) -> Option<AvMeta> {
    if !ffmpeg_available().await {
        return None;
    }
    probe_or_none(&path.to_string_lossy(), None).await
}

async fn probe_or_none(path: &str, signal: Option<AbortSignal>) -> Option<AvMeta> {
    // AvTimeoutError / AvAbortedError / a spawn failure. All of them mean "no
    // answer this time", and none of them is worth failing an upload whose
    // bytes are already on disk.
    match probe_av(path, AV_PROBE_TIMEOUT_MS, signal).await {
        Ok(Some(probe)) => Some(av_meta_from(&probe)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    //! tests/unit/av-meta.test.ts (+ the totalSourcePixels cases of
    //! web-playable.test.ts).
    //!
    //! Every fixture below is output ffprobe 5.1.9 really produced, captured from
    //! production files, not invented. That matters most for the three videos that
    //! carry no `nb_frames`: they are the reason `fps` is stored at all, and a
    //! hand-written fixture would never have thought to omit the field.
    use serde_json::{Value, json};

    use super::*;
    use crate::media::av_probe::parse_av_probe;

    /// The ShareX recording the complaint was about: 5056 px wide, 0.0044 bpp.
    fn stuttering_capture() -> Value {
        json!({
            "programs": [],
            "streams": [{"codec_name": "h264", "codec_type": "video", "width": 5056, "height": 1924,
                         "avg_frame_rate": "30/1", "duration": "13.433333", "nb_frames": "403",
                         "disposition": {"attached_pic": 0}}],
            "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "13.434000"}
        })
    }
    const STUTTERING_CAPTURE_BYTES: f64 = 2_179_538.0;

    /// A WebM: the container gives a duration and a rate, but never a frame count.
    fn webm_without_frame_count() -> Value {
        json!({
            "programs": [],
            "streams": [{"codec_name": "vp8", "codec_type": "video", "width": 768, "height": 610,
                         "avg_frame_rate": "32/1", "disposition": {"attached_pic": 0}}],
            "format": {"format_name": "matroska,webm", "duration": "15.409000"}
        })
    }

    /// An MPEG-TS stream wearing an .mp4 name, with a variable frame rate.
    fn variable_rate() -> Value {
        json!({
            "programs": [],
            "streams": [{"codec_name": "h264", "codec_type": "video", "width": 48, "height": 96,
                         "avg_frame_rate": "122972160/4355237", "duration": "41.666667",
                         "disposition": {"attached_pic": 0}}],
            "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "41.666667"}
        })
    }

    /// A plain MP3, and the same MP3 with cover art bolted on.
    fn mp3() -> Value {
        json!({
            "programs": [],
            "streams": [{"codec_name": "mp3", "codec_type": "audio", "disposition": {"attached_pic": 0}}],
            "format": {"format_name": "mp3", "duration": "208.718367"}
        })
    }
    fn mp3_with_cover() -> Value {
        json!({
            "programs": [],
            "streams": [
                {"codec_name": "mp3", "codec_type": "audio", "disposition": {"attached_pic": 0}},
                {"codec_name": "png", "codec_type": "video", "width": 600, "height": 600, "nb_frames": "1",
                 "avg_frame_rate": "90000/1", "disposition": {"attached_pic": 1}}
            ],
            "format": {"format_name": "mp3", "duration": "208.718367"}
        })
    }

    fn meta_of(raw: Value) -> AvMeta {
        av_meta_from(&parse_av_probe(&raw).expect("probe"))
    }

    // describe("projecting a probe onto the row's columns")

    #[test]
    fn records_what_the_stuttering_sharex_capture_actually_is() {
        assert_eq!(
            meta_of(stuttering_capture()),
            AvMeta {
                width: Some(5056),
                height: Some(1924),
                frames: Some(403),
                duration_seconds: Some(13.434),
                fps: Some(30.0)
            }
        );
    }

    #[test]
    fn makes_the_recordings_two_defects_computable_from_the_row_alone() {
        let meta = meta_of(stuttering_capture());
        let total = frame_count(&meta);
        assert_eq!(total, Some(403.0));

        // 1. Past 4096 px no hardware H.264 decoder will touch it.
        assert!(meta.width.unwrap() > 4096);

        // 2. And the encoder was given 0.0044 bits per pixel.
        let bpp = (STUTTERING_CAPTURE_BYTES * 8.0)
            / (meta.width.unwrap() as f64 * meta.height.unwrap() as f64 * total.unwrap());
        assert!((bpp - 0.00445).abs() < 0.000005);
    }

    #[test]
    fn still_yields_a_frame_total_when_the_container_never_counted() {
        let meta = meta_of(webm_without_frame_count());
        // The counted column stays empty, because nothing counted.
        assert_eq!(meta.frames, None);
        assert_eq!(meta.fps, Some(32.0));
        assert_eq!(meta.duration_seconds, Some(15.409));
        // 15.409 s at 32 fps. Estimated, and deliberately never written back into
        // uploads.frames - see the comment on frame_count().
        assert_eq!(frame_count(&meta), Some(493.0));
    }

    #[test]
    fn handles_a_variable_frame_rate_without_turning_it_into_nan() {
        let meta = meta_of(variable_rate());
        assert!((meta.fps.unwrap() - 28.235).abs() < 0.0005);
        assert_eq!(frame_count(&meta), Some(1176.0));
    }

    #[test]
    fn gives_audio_a_duration_and_nothing_else() {
        let meta = meta_of(mp3());
        assert_eq!(meta, AvMeta { duration_seconds: Some(208.718367), ..NO_AV_META });
        assert_eq!(frame_count(&meta), None);
    }

    #[test]
    fn does_not_let_cover_art_give_an_mp3_a_picture_size() {
        let meta = meta_of(mp3_with_cover());
        assert_eq!((meta.width, meta.height, meta.frames), (None, None, None));
        assert!((meta.duration_seconds.unwrap() - 208.72).abs() < 0.005);
    }

    #[test]
    fn reports_nothing_rather_than_zero_when_a_probe_measured_nothing() {
        assert_eq!(frame_count(&NO_AV_META), None);
        // A duration with no rate, and a rate with no duration, are both unanswerable.
        assert_eq!(frame_count(&AvMeta { duration_seconds: Some(10.0), ..NO_AV_META }), None);
        assert_eq!(frame_count(&AvMeta { fps: Some(30.0), ..NO_AV_META }), None);
    }

    // web-playable.test.ts: describe("totalSourcePixels")

    #[test]
    fn total_pixels_falls_back_to_duration_times_frame_rate_when_nothing_counted_the_frames() {
        let raw = json!({
            "programs": [],
            "streams": [{"codec_name": "h264", "codec_type": "video", "width": 2560, "height": 1440,
                         "avg_frame_rate": "60/1", "disposition": {"attached_pic": 0}}],
            "format": {"format_name": "matroska,webm", "duration": "20.000000"}
        });
        assert_eq!(total_source_pixels(&parse_av_probe(&raw).unwrap()), Some(2560.0 * 1440.0 * 1200.0));
    }

    #[test]
    fn total_pixels_has_no_answer_for_sound_and_says_so() {
        let raw = json!({
            "programs": [],
            "streams": [{"codec_name": "mp3", "codec_type": "audio", "disposition": {"attached_pic": 0}}],
            "format": {"format_name": "mp3", "duration": "209.000000"}
        });
        assert_eq!(total_source_pixels(&parse_av_probe(&raw).unwrap()), None);
    }

    #[test]
    fn total_pixels_of_the_capture() {
        assert_eq!(total_source_pixels(&parse_av_probe(&stuttering_capture()).unwrap()), Some(5056.0 * 1924.0 * 403.0));
    }
}
