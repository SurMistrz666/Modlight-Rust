//! Port of `lib/media/av-probe.ts`: what ffprobe says a file is, and whether
//! we believe it.
//!
//! This runs BEFORE any encoder, and that ordering is the point: a duration and a
//! codec cost one cheap process (measured: 50-100 ms), and knowing them is what
//! lets an over-long or unsupported input be refused for free instead of being
//! discovered halfway through a job that has already taken a slot.
//!
//! The parsing is separated from the spawning so every interesting shape - a
//! stream with no duration, a container claiming to be six formats at once, an
//! audio-only file with a cover-art "video" stream - is a unit test.

use serde_json::Value;

use crate::config::js_number;
use crate::media::abort::AbortSignal;
use crate::media::ffmpeg::{self, AvAbortedError, AvTimeoutError, AvTool, RunAvOptions, probe_input_args};

/// Demuxers we are willing to force with `-f`.
///
/// An allowlist, and a short one. Everything ffmpeg can read that is NOT here
/// (concat, hls, dash, lavfi, image2 with a glob pattern…) is a way to make one
/// "file" mean many files, which is the shape of every local-file-disclosure bug
/// in this area. The key is the family ffprobe reports, the value is the single
/// demuxer name we hand back to ffmpeg.
///
/// `mov,mp4,m4a,3gp,3g2,mj2` covers more than its name suggests: it is also how
/// ffmpeg 5.1 reads an ANIMATED AVIF, which is the format that started this work
/// (measured on the production file: 43 frames of AV1, 274x498, 2.15 s - where
/// libvips hands back only the cover frame).
pub const CONTAINER_ALLOWLIST: [(&str, &str); 21] = [
    ("mov,mp4,m4a,3gp,3g2,mj2", "mov"),
    ("matroska", "matroska"),
    ("matroska,webm", "matroska"),
    ("webm", "matroska"),
    ("avi", "avi"),
    ("asf", "asf"),
    ("flv", "flv"),
    ("mpegts", "mpegts"),
    ("mpeg", "mpeg"),
    ("ogg", "ogg"),
    ("gif", "gif"),
    ("mp3", "mp3"),
    ("mp3,mp2,mp1", "mp3"),
    ("wav", "wav"),
    ("flac", "flac"),
    ("aac", "aac"),
    ("aac,adts", "aac"),
    ("ape", "ape"),
    ("wv,wvpack", "wv"),
    ("opus", "ogg"),
    ("m4v", "m4v"),
];

fn allowed_container(format_name: &str) -> Option<&'static str> {
    CONTAINER_ALLOWLIST.iter().find(|(family, _)| *family == format_name).map(|(_, demuxer)| *demuxer)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvStreamKind {
    Video,
    Audio,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AvStream {
    pub kind: AvStreamKind,
    pub codec: String,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub frames: Option<f64>,
    /// Frames per second, from ffprobe's `avg_frame_rate`.
    ///
    /// It sits next to `frames` because the two are not equally available, which
    /// is a measurement rather than a guess: over 59 production videos probed in
    /// the runtime image, `nb_frames` was missing on 3 of them (an MPEG-TS stream
    /// wearing an .mp4 name, a WebM, and one damaged MP4) while `avg_frame_rate`
    /// was present on all 59. So this is the field that still answers when the
    /// counted one does not, and "how many pixels does this file contain" - the
    /// question behind a stuttering recording - stays answerable either way.
    ///
    /// None when ffprobe writes `0/0`, which is its word for "no idea".
    pub fps: Option<f64>,
    /// True for a still picture carried in an audio file (album art). ffprobe
    /// reports it as a video stream, and treating it as one turns "extract the
    /// audio from this MP3" into "this file already has video".
    pub attached_picture: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AvProbe {
    /// The demuxer to FORCE on every later invocation. Never inferred again.
    pub container: String,
    /// Raw `format_name`, kept for diagnostics and error messages.
    pub format_name: String,
    /// Seconds, or None when the container does not say (some streamed files).
    pub duration: Option<f64>,
    pub video: Option<AvStream>,
    pub audio: Option<AvStream>,
    /// Every stream, so callers can tell "two audio tracks" from "one".
    pub streams: Vec<AvStream>,
}

/// `num`: a finite number, from a JSON number or a numeric string.
fn num(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64().filter(|f| f.is_finite()),
        Value::String(s) => Some(js_number(s)).filter(|f| f.is_finite()),
        _ => None,
    }
}

/// `positive`.
fn positive(v: Option<&Value>) -> Option<f64> {
    num(v).filter(|&n| n > 0.0)
}

/// ffprobe writes a frame rate as a rational STRING, not a number: "30/1" for a
/// fixed-rate recording, "122972160/4355237" for a variable one (a real
/// production file), and "0/0" when it does not know. Reading it with `Number()`
/// yields NaN for every one of those, so it needs its own parser.
///
/// A bare number is accepted too, since some builds emit one, and anything that
/// is not a positive finite rate comes back None rather than 0 - zero would pass
/// every "is this fast enough" test ever written.
pub fn parse_rational(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64().filter(|f| f.is_finite() && *f > 0.0),
        Value::String(s) => {
            let (numerator, denominator) = match s.find('/') {
                None => (js_number(s), 1.0),
                Some(slash) => (js_number(&s[..slash]), js_number(&s[slash + 1..])),
            };
            if !numerator.is_finite() || !denominator.is_finite() {
                return None;
            }
            if numerator <= 0.0 || denominator <= 0.0 {
                return None;
            }
            Some(numerator / denominator)
        }
        _ => None,
    }
}

/// Turn ffprobe's JSON into something with a type, or None when the file is not
/// one we are prepared to touch.
///
/// None has one meaning here and it is deliberate: "no supported media in a
/// container we allow". The caller answers 415 with that, which is a fact the
/// person can act on, unlike the 500 a thrown encoder produces.
pub fn parse_av_probe(raw: &Value) -> Option<AvProbe> {
    let root = raw.as_object()?;

    let empty = serde_json::Map::new();
    let format = root.get("format").and_then(Value::as_object).unwrap_or(&empty);
    let format_name = format.get("format_name").and_then(Value::as_str).unwrap_or("").to_string();
    let container = allowed_container(&format_name)?;

    let raw_streams: &[Value] = root.get("streams").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
    let streams: Vec<AvStream> = raw_streams
        .iter()
        .map(|s| {
            let entry = s.as_object().unwrap_or(&empty);
            let kind = match entry.get("codec_type").and_then(Value::as_str) {
                Some("video") => AvStreamKind::Video,
                Some("audio") => AvStreamKind::Audio,
                _ => AvStreamKind::Other,
            };
            let disposition = entry.get("disposition").and_then(Value::as_object).unwrap_or(&empty);
            AvStream {
                kind,
                codec: entry.get("codec_name").and_then(Value::as_str).unwrap_or("").to_string(),
                width: positive(entry.get("width")),
                height: positive(entry.get("height")),
                frames: positive(entry.get("nb_frames")),
                fps: parse_rational(entry.get("avg_frame_rate")),
                attached_picture: disposition.get("attached_pic").and_then(Value::as_f64) == Some(1.0),
            }
        })
        .collect();

    // Cover art is not video. An MP3 with an embedded JPEG reports a `video`
    // stream of one frame, and picking it as THE video stream would offer "extract
    // a frame at 00:30" for a file that is thirty minutes of audio.
    let video = streams.iter().find(|s| s.kind == AvStreamKind::Video && !s.attached_picture).cloned();
    let audio = streams.iter().find(|s| s.kind == AvStreamKind::Audio).cloned();
    if video.is_none() && audio.is_none() {
        return None;
    }

    // Format duration first: a stream may omit it, and for a remux the container's
    // answer is the one that matters. Falling back to the stream keeps files whose
    // header is thin (some Matroska writers) from reading as "unknown length" and
    // being refused by the duration ceiling.
    let duration = positive(format.get("duration"))
        .or_else(|| positive(raw_streams.first().and_then(Value::as_object).and_then(|s| s.get("duration"))));

    Some(AvProbe { container: container.to_string(), format_name, duration, video, audio, streams })
}

/// The `-show_entries` selector `probeAv` asks for.
///
/// `disposition` is a nested object, so it needs its own `stream_disposition`
/// selector - asking for it inside `stream=` silently returns nothing, and
/// then every MP3 with cover art looks like a file that has video in it.
/// `avg_frame_rate` rides along for free: it is one more field in a request
/// that is already being made, and it is the only frame-rate answer some
/// containers give (see AvStream.fps).
pub const SHOW_ENTRIES: &str = "format=format_name,duration:stream=codec_type,codec_name,width,height,nb_frames,avg_frame_rate,duration:stream_disposition=attached_pic";

/// The exact argument vector of `probeAv`.
pub fn probe_av_args(path: &str) -> Vec<String> {
    let mut args = vec!["-v".to_string(), "error".to_string()];
    args.extend(probe_input_args(path));
    args.extend(["-show_entries".to_string(), SHOW_ENTRIES.to_string(), "-of".to_string(), "json".to_string()]);
    args
}

/// What `probeAv` does with a finished (or failed) ffprobe run. Split from the
/// spawn so av-probe-abort.test.ts can drive it without a process.
fn interpret(result: anyhow::Result<ffmpeg::AvResult>) -> anyhow::Result<Option<AvProbe>> {
    let out = match result {
        Ok(res) => res.stdout,
        Err(err) => {
            // A KILLED probe is not an answer about the file.
            //
            // `None` from here means one thing only: ffprobe looked and this is not
            // media we accept. It used to also mean "the caller hung up" and "we ran
            // out of time", because a bare catch cannot tell them apart - and the
            // conversion path turns `None` into a REMEMBERED "unreadable" verdict
            // that is then served, for ten minutes, to every later request for that
            // file, including real downloads. One reload at the wrong moment, or one
            // busy minute on the server, and a perfectly good video starts reporting
            // that it cannot be opened. Both of those are facts about the server or
            // the socket, never about the bytes, so they propagate and let the
            // caller decide.
            if err.is::<AvTimeoutError>() || err.is::<AvAbortedError>() {
                return Err(err);
            }
            return Ok(None);
        }
    };
    Ok(serde_json::from_str::<Value>(&out).ok().as_ref().and_then(parse_av_probe))
}

/// Ask ffprobe about a file on disk.
///
/// The one invocation allowed to let ffmpeg guess the container, because
/// discovering it is the entire job - and it is safe here precisely because
/// ffprobe only ever reports. Nothing is demuxed into an output, and the result
/// is checked against the allowlist before any second process exists.
pub async fn probe_av(path: &str, timeout_ms: u64, signal: Option<AbortSignal>) -> anyhow::Result<Option<AvProbe>> {
    let args = probe_av_args(path);
    let result =
        ffmpeg::run_av_tool(AvTool::Ffprobe, &args, RunAvOptions { timeout_ms, signal, capture_stdout: true }).await;
    interpret(result)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::media::ffmpeg::{AvResult, AvToolError};

    // tests/unit/av-probe.test.ts

    /// The exact shape ffprobe 5.1.9 emitted for the production .mkv.
    fn real_video() -> Value {
        json!({
            "programs": [],
            "streams": [
                {"codec_name": "h264", "codec_type": "video", "width": 1280, "height": 720, "nb_frames": "968",
                 "duration": "32.231000", "disposition": {"attached_pic": 0}},
                {"codec_name": "aac", "codec_type": "audio", "duration": "32.234646", "nb_frames": "1511",
                 "disposition": {"attached_pic": 0}}
            ],
            "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "32.234646"}
        })
    }

    /// ...and for the animated AVIF from the bug report.
    fn real_avif() -> Value {
        json!({
            "programs": [],
            "streams": [
                {"codec_name": "av1", "codec_type": "video", "width": 274, "height": 498, "duration": "2.150000",
                 "nb_frames": "43", "disposition": {"attached_pic": 0}}
            ],
            "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "2.150000"}
        })
    }

    #[test]
    fn maps_a_container_family_to_the_single_demuxer_we_will_force() {
        let probe = parse_av_probe(&real_video()).unwrap();
        // `-f mov` is what stops ffmpeg from ever choosing the concat demuxer for
        // this file. Measured: without it, a text file naming /tmp/secret.txt is
        // read as an ffconcat playlist.
        assert_eq!(probe.container, "mov");
        assert_eq!(probe.format_name, "mov,mp4,m4a,3gp,3g2,mj2");
    }

    #[test]
    fn reads_the_streams_duration_and_codecs_of_a_real_video() {
        let probe = parse_av_probe(&real_video()).unwrap();
        let video = probe.video.unwrap();
        assert_eq!(video.codec, "h264");
        assert_eq!(video.width, Some(1280.0));
        assert_eq!(video.height, Some(720.0));
        assert_eq!(probe.audio.unwrap().codec, "aac");
        assert!((probe.duration.unwrap() - 32.23).abs() < 0.05);
    }

    #[test]
    fn sees_all_43_frames_of_the_avif_sequence_libvips_reports_as_one() {
        let probe = parse_av_probe(&real_avif()).unwrap();
        assert_eq!(probe.video.as_ref().unwrap().frames, Some(43.0));
        assert_eq!(probe.video.as_ref().unwrap().codec, "av1");
        assert_eq!(probe.audio, None);
    }

    #[test]
    fn refuses_a_container_that_is_not_on_the_allowlist() {
        // Everything absent from the list is a way to make one file mean many -
        // concat, hls, dash - which is the shape of every local-file-disclosure bug
        // in this area.
        for format_name in ["concat", "hls", "dash", "lavfi", "image2", "tty"] {
            assert_eq!(
                parse_av_probe(&json!({"format": {"format_name": format_name}, "streams": []})),
                None,
                "{format_name}"
            );
        }
    }

    #[test]
    fn does_not_mistake_cover_art_for_a_video_track() {
        // An MP3 with an embedded JPEG reports a one-frame `video` stream. Treating
        // it as video would offer "grab a frame at 00:30" for a file that is thirty
        // minutes of audio, and would make remuxVerdict say yes to an MP3.
        let with_art = parse_av_probe(&json!({
            "format": {"format_name": "mp3", "duration": "212.5"},
            "streams": [
                {"codec_name": "mp3", "codec_type": "audio", "disposition": {"attached_pic": 0}},
                {"codec_name": "png", "codec_type": "video", "width": 600, "height": 600, "disposition": {"attached_pic": 1}}
            ]
        }))
        .unwrap();
        assert_eq!(with_art.video, None);
        assert_eq!(with_art.audio.as_ref().unwrap().codec, "mp3");
        // ...but the stream is still visible to anything that wants the full list.
        assert_eq!(with_art.streams.len(), 2);
    }

    #[test]
    fn falls_back_to_a_stream_duration_when_the_container_omits_one() {
        let probe = parse_av_probe(&json!({
            "format": {"format_name": "matroska,webm"},
            "streams": [{"codec_name": "h264", "codec_type": "video", "width": 640, "height": 480, "duration": "12.5",
                         "disposition": {"attached_pic": 0}}]
        }))
        .unwrap();
        // Without this, a thin Matroska header reads as "unknown length" and every
        // whole-file encode refuses it.
        assert_eq!(probe.duration, Some(12.5));
    }

    #[test]
    fn reports_an_unknown_duration_as_none_rather_than_as_zero() {
        let probe = parse_av_probe(&json!({
            "format": {"format_name": "mpegts"},
            "streams": [{"codec_name": "h264", "codec_type": "video", "disposition": {"attached_pic": 0}}]
        }))
        .unwrap();
        // Zero would pass every "shorter than the ceiling" test ever written.
        assert_eq!(probe.duration, None);
    }

    #[test]
    fn survives_the_shapes_a_broken_or_hostile_file_produces() {
        assert_eq!(parse_av_probe(&Value::Null), None);
        assert_eq!(parse_av_probe(&json!("not json")), None);
        assert_eq!(parse_av_probe(&json!({})), None);
        assert_eq!(parse_av_probe(&json!({"format": {}, "streams": []})), None);
        // A known container carrying nothing we can use is still nothing we can use.
        assert_eq!(parse_av_probe(&json!({"format": {"format_name": "mp3"}, "streams": []})), None);
        let probe = parse_av_probe(&json!({
            "format": {"format_name": "mp3", "duration": "abc"},
            "streams": [{"codec_type": "audio"}]
        }))
        .unwrap();
        assert_eq!(probe.duration, None);
    }

    #[test]
    fn treats_a_negative_or_zero_dimension_as_absent() {
        let probe = parse_av_probe(&json!({
            "format": {"format_name": "avi", "duration": "5"},
            "streams": [{"codec_name": "mpeg4", "codec_type": "video", "width": 0, "height": -1, "disposition": {}}]
        }))
        .unwrap();
        assert_eq!(probe.video.as_ref().unwrap().width, None);
        assert_eq!(probe.video.as_ref().unwrap().height, None);
    }

    #[test]
    fn asks_ffprobe_exactly_what_upstream_asks() {
        assert_eq!(
            probe_av_args("/data/a.mp4"),
            [
                "-v",
                "error",
                "-protocol_whitelist",
                "file,pipe",
                "-i",
                "/data/a.mp4",
                "-show_entries",
                "format=format_name,duration:stream=codec_type,codec_name,width,height,nb_frames,avg_frame_rate,duration:stream_disposition=attached_pic",
                "-of",
                "json"
            ]
        );
    }

    // tests/unit/av-probe-abort.test.ts
    //
    // A killed probe must not be mistaken for a broken file. `probeAv` returning
    // `None` feeds av_action, which REMEMBERS it as "unreadable" for ten minutes
    // and then serves that verdict to every later request for the same file. So
    // the three things a bare `catch` used to flatten together had very
    // different consequences: only "ffprobe looked and this is not media we
    // accept" is an answer about the file. The other two propagate.

    #[test]
    fn passes_on_the_abort_when_the_caller_went_away() {
        let err = interpret(Err(AvAbortedError.into())).unwrap_err();
        assert!(err.is::<AvAbortedError>());
    }

    #[test]
    fn passes_on_the_timeout_when_the_probe_ran_out_of_time() {
        let err = interpret(Err(AvTimeoutError { ms: 1000 }.into())).unwrap_err();
        assert!(err.is::<AvTimeoutError>());
    }

    #[test]
    fn still_answers_none_when_ffprobe_looked_and_refused_the_content() {
        // The tool ran and failed on the bytes. That IS an answer about the file,
        // and the conversion path is right to remember it.
        let tool = AvToolError { tool: AvTool::Ffprobe, code: Some(1), stderr: "Invalid data found".into() };
        assert_eq!(interpret(Err(tool.into())).unwrap(), None);
    }

    fn stdout(s: &str) -> anyhow::Result<AvResult> {
        Ok(AvResult { stdout: s.to_string(), stderr: String::new() })
    }

    #[test]
    fn still_answers_none_for_a_container_we_do_not_accept() {
        let out = json!({"format": {"format_name": "concat"}, "streams": []}).to_string();
        assert_eq!(interpret(stdout(&out)).unwrap(), None);
    }

    #[test]
    fn still_answers_none_for_output_that_is_not_json_at_all() {
        assert_eq!(interpret(stdout("not json")).unwrap(), None);
    }

    #[test]
    fn parses_a_real_answer() {
        let out = json!({
            "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "12.5"},
            "streams": [{"codec_type": "video", "codec_name": "h264", "width": 1920, "height": 1080}]
        })
        .to_string();
        let probe = interpret(stdout(&out)).unwrap().unwrap();
        assert_eq!(probe.container, "mov");
        assert_eq!(probe.duration, Some(12.5));
        assert_eq!(probe.video.unwrap().codec, "h264");
    }

    #[test]
    fn rational_frame_rates() {
        // (from tests/unit/av-meta.test.ts) "parses the rational form, which Number() cannot"
        assert_eq!(parse_rational(Some(&json!("30/1"))), Some(30.0));
        assert_eq!(parse_rational(Some(&json!("60/1"))), Some(60.0));
        // A real variable-rate production file. Number("122972160/4355237") is NaN.
        assert!((parse_rational(Some(&json!("122972160/4355237"))).unwrap() - 28.235).abs() < 0.0005);
        assert_eq!(parse_rational(Some(&json!("25"))), Some(25.0));
        assert!((parse_rational(Some(&json!(29.97))).unwrap() - 29.97).abs() < 0.005);

        // "treats every way of saying 'no idea' as null, never as zero"
        for bad in [
            json!("0/0"),
            json!("0/1"),
            json!("1/0"),
            json!(""),
            json!("N/A"),
            json!("abc/2"),
            Value::Null,
            json!({}),
            json!(-30),
        ] {
            assert_eq!(parse_rational(Some(&bad)), None, "{bad}");
        }
        assert_eq!(parse_rational(None), None);
    }
}
