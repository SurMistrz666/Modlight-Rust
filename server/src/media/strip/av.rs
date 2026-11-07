//! Audio and video: ffmpeg with a stream copy, so not one sample is re-encoded.
//!
//! `-map 0 -map_metadata -1 -map_chapters -1 -c copy` with these additions:
//!
//! - `-map -0:d?`: data streams go. That is where the telemetry of a phone or an
//!   action camera lives (Apple's `mebx`, GoPro's `gpmd` with its GPS track), and
//!   the mp4 muxer could not write most of them anyway.
//! - `-fflags +bitexact`: no `encoder` tag, and the same input gives the same
//!   bytes (no random Matroska ids), so duplicates of a cleaned file are found.
//! - `-strict unofficial`: the mov muxer keeps Dolby Vision and 360-degree
//!   side data only with it.
//! - `+faststart` on the mov family, as every conversion here does.
//! - `-brand`: an `M4V ` file stays one.
//!
//! The rotation of a phone video is side data of the stream, which survives
//! `-map_metadata -1` (ffmpeg 5.1 in the image and 9 alike, tests/media_strip.rs).
//! That is checked here as well, on every file, with ffprobe: the same streams,
//! the same rotation, and no tag naming a place or a moment. A remux that fails
//! any of it is thrown away.

use std::path::Path;

use serde_json::Value;

use super::{KeptReason, ftyp_brands};
use crate::media::abort::AbortSignal;
use crate::media::av_cost::{AV_COPY_TIMEOUT_MS, AV_PROBE_TIMEOUT_MS};
use crate::media::av_probe::{AvProbe, parse_av_probe};
use crate::media::ffmpeg::{
    AvAbortedError, AvTimeoutError, AvTool, AvToolError, RunAvOptions, ffmpeg_available, input_args, probe_input_args,
    run_av_tool,
};

/// What is asked of ffprobe: enough for [`parse_av_probe`], the rotation (side
/// data since ffmpeg 5, a `rotate` tag before) and, on the result, every tag.
const SHOW_ENTRIES: &str = "format=format_name,duration:format_tags:stream=codec_type,codec_name,width,height,nb_frames,avg_frame_rate,duration:stream_tags:stream_disposition=attached_pic:stream_side_data=rotation";

/// The same without tags, for the upload itself: a file's own tags can be long
/// enough to run past what is kept of ffprobe's answer.
const SHOW_ENTRIES_NO_TAGS: &str = "format=format_name,duration:stream=codec_type,codec_name,width,height,nb_frames,avg_frame_rate,duration:stream_tags=rotate:stream_disposition=attached_pic:stream_side_data=rotation";

/// Words in a tag's name that say where or when. No muxer writes one of these
/// after `-map_metadata -1`, which is the point of looking.
const TELLING_TAGS: [&str; 6] = ["location", "gps", "creation_time", "date", "artist", "author"];

/// What ffprobe says about a file.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub probe: AvProbe,
    /// Rotation of the first picture stream, in degrees.
    pub rotation: i64,
    /// Names of the tags left, lower-cased.
    pub tags: Vec<String>,
}

/// One stream's rotation: the display matrix, or the tag older ffmpegs use.
fn rotation_of(stream: &Value) -> Option<i64> {
    let side = stream.get("side_data_list").and_then(Value::as_array).into_iter().flatten();
    let from_side = side.filter_map(|d| d.get("rotation")).find_map(|r| match r {
        Value::Number(n) => n.as_f64().map(|v| v.round() as i64),
        Value::String(s) => s.parse().ok(),
        _ => None,
    });
    from_side.or_else(|| stream.get("tags")?.get("rotate")?.as_str()?.parse().ok())
}

/// Read an ffprobe answer. `None` is "not media this server accepts".
pub fn parse_report(raw: &Value) -> Option<Report> {
    let probe = parse_av_probe(raw)?;
    let streams = raw.get("streams").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
    let picture = streams.iter().find(|s| {
        s.get("codec_type").and_then(Value::as_str) == Some("video")
            && s.get("disposition").and_then(|d| d.get("attached_pic")).and_then(Value::as_i64) != Some(1)
    });
    let rotation = picture.and_then(rotation_of).unwrap_or(0);
    let mut tags = Vec::new();
    for holder in std::iter::once(raw.get("format")).chain(streams.iter().map(Some)).flatten() {
        if let Some(map) = holder.get("tags").and_then(Value::as_object) {
            tags.extend(map.keys().map(|k| k.to_lowercase()));
        }
    }
    Some(Report { probe, rotation, tags })
}

async fn report(path: &str, with_tags: bool, signal: &AbortSignal) -> anyhow::Result<Option<Report>> {
    let mut args = vec!["-v".to_string(), "error".to_string()];
    args.extend(probe_input_args(path));
    let entries = if with_tags { SHOW_ENTRIES } else { SHOW_ENTRIES_NO_TAGS };
    args.extend(["-show_entries".to_string(), entries.to_string(), "-of".to_string(), "json".to_string()]);
    let result = run_av_tool(
        AvTool::Ffprobe,
        &args,
        RunAvOptions { timeout_ms: AV_PROBE_TIMEOUT_MS, signal: Some(signal.clone()), capture_stdout: true },
    )
    .await;
    let out = match result {
        Ok(out) => out.stdout,
        Err(err) if err.is::<AvTimeoutError>() || err.is::<AvAbortedError>() => return Err(err),
        // ffprobe could not read it at all.
        Err(_) => return Ok(None),
    };
    Ok(serde_json::from_str::<Value>(&out).ok().as_ref().and_then(parse_report))
}

/// The muxer that writes what came in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Muxer {
    pub name: &'static str,
    /// The major brand to keep, on the mp4 family.
    pub brand: Option<String>,
}

impl Muxer {
    fn of(name: &'static str) -> Option<Muxer> {
        Some(Muxer { name, brand: None })
    }

    fn is_mov_family(&self) -> bool {
        matches!(self.name, "mov" | "mp4" | "ipod" | "3gp" | "3g2")
    }
}

/// `DocType` of a Matroska file's EBML header.
fn is_webm(head: &[u8]) -> bool {
    let Some(at) = head.windows(2).position(|w| w == [0x42, 0x82]) else {
        return false;
    };
    let Some(&size) = head.get(at + 2) else {
        return false;
    };
    let length = usize::from(size & 0x7F);
    head.get(at + 3..at + 3 + length) == Some(b"webm")
}

/// Which muxer to use, from the demuxer ffprobe named and the file's own header.
/// `None` for the containers this feature does not cover.
pub fn muxer_for(container: &str, head: &[u8]) -> Option<Muxer> {
    match container {
        "mov" => {
            let Some((major, _)) = ftyp_brands(head) else {
                return Muxer::of("mov"); // QuickTime from before `ftyp`
            };
            match major {
                b"qt  " => Muxer::of("mov"),
                b"M4A " | b"M4B " | b"M4P " => Muxer::of("ipod"),
                brand if brand.starts_with(b"3gp") => Muxer::of("3gp"),
                brand if brand.starts_with(b"3g2") => Muxer::of("3g2"),
                brand => Some(Muxer {
                    name: "mp4",
                    brand: std::str::from_utf8(brand)
                        .ok()
                        .filter(|b| b.chars().all(|c| c.is_ascii_graphic() || c == ' '))
                        .map(str::to_string),
                }),
            }
        }
        "matroska" => Muxer::of(if is_webm(head) { "webm" } else { "matroska" }),
        "mp3" | "flac" | "ogg" | "wav" | "avi" => Muxer::of(match container {
            "mp3" => "mp3",
            "flac" => "flac",
            "ogg" => "ogg",
            "wav" => "wav",
            _ => "avi",
        }),
        _ => None,
    }
}

/// The whole argument vector.
pub fn strip_args(input: &str, output: &str, container: &str, muxer: &Muxer) -> Vec<String> {
    let mut args = input_args(input, Some(container), &[]);
    args.extend(
        [
            "-map",
            "0",
            "-map",
            "-0:d?",
            "-map_metadata",
            "-1",
            "-map_chapters",
            "-1",
            "-c",
            "copy",
            "-ignore_unknown",
            "-fflags",
            "+bitexact",
            "-strict",
            "unofficial",
        ]
        .map(str::to_string),
    );
    if muxer.is_mov_family() {
        args.extend(["-movflags".to_string(), "+faststart".to_string()]);
    }
    if let Some(brand) = &muxer.brand {
        args.extend(["-brand".to_string(), brand.clone()]);
    }
    args.extend(["-f".to_string(), muxer.name.to_string(), "-y".to_string(), output.to_string()]);
    args
}

/// Audio and picture streams, codec by codec.
fn media_streams(probe: &AvProbe) -> Vec<(bool, String)> {
    use crate::media::av_probe::AvStreamKind::{Audio, Video};
    probe
        .streams
        .iter()
        .filter(|s| matches!(s.kind, Audio | Video))
        .map(|s| (s.kind == Video, s.codec.clone()))
        .collect()
}

/// A word of [`TELLING_TAGS`] among the tags left.
fn telling_tag(tags: &[String]) -> Option<&str> {
    tags.iter().map(String::as_str).find(|tag| TELLING_TAGS.iter().any(|word| tag.contains(word)))
}

fn tool_failure(err: &anyhow::Error) -> KeptReason {
    if err.is::<AvTimeoutError>() || err.is::<AvAbortedError>() {
        KeptReason::TooLarge
    } else if err.is::<AvToolError>() {
        KeptReason::Damaged
    } else {
        KeptReason::Tool
    }
}

/// Clean one file; `Ok(false)` when it is not media this feature covers.
pub async fn strip(
    input: &Path,
    output: &Path,
    head: &[u8],
    weak: bool,
    signal: AbortSignal,
) -> Result<bool, KeptReason> {
    if !ffmpeg_available().await {
        return Err(KeptReason::Tool);
    }
    let (input, output) = (input.to_string_lossy(), output.to_string_lossy());
    let before = match report(&input, false, &signal).await {
        Ok(Some(report)) => report,
        Ok(None) if weak => return Ok(false),
        Ok(None) => return Err(KeptReason::Damaged),
        Err(err) => return Err(tool_failure(&err)),
    };
    let Some(muxer) = muxer_for(&before.probe.container, head) else {
        return Ok(false);
    };
    let args = strip_args(&input, &output, &before.probe.container, &muxer);
    let run = run_av_tool(
        AvTool::Ffmpeg,
        &args,
        RunAvOptions { timeout_ms: AV_COPY_TIMEOUT_MS, signal: Some(signal.clone()), capture_stdout: false },
    )
    .await;
    if let Err(err) = run {
        tracing::warn!("[strip] ffmpeg: {err}");
        return Err(tool_failure(&err));
    }

    let after = match report(&output, true, &signal).await {
        Ok(Some(report)) => report,
        Ok(None) => return Err(KeptReason::Tool),
        Err(err) => return Err(tool_failure(&err)),
    };
    if media_streams(&before.probe) != media_streams(&after.probe) {
        tracing::warn!("[strip] the streams changed in the remux of a {}", muxer.name);
        return Err(KeptReason::Tool);
    }
    if before.rotation != after.rotation {
        tracing::warn!("[strip] the rotation changed in the remux of a {}", muxer.name);
        return Err(KeptReason::Tool);
    }
    if let Some(tag) = telling_tag(&after.tags) {
        tracing::warn!("[strip] the tag {tag} survived the remux of a {}", muxer.name);
        return Err(KeptReason::Tool);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn ftyp(major: &[u8; 4]) -> Vec<u8> {
        let mut v = 16u32.to_be_bytes().to_vec();
        v.extend_from_slice(b"ftyp");
        v.extend_from_slice(major);
        v.extend_from_slice(&[0; 4]);
        v
    }

    fn name(container: &str, head: &[u8]) -> Option<&'static str> {
        muxer_for(container, head).map(|m| m.name)
    }

    #[test]
    fn the_container_stays_what_it_was() {
        assert_eq!(name("mov", &ftyp(b"qt  ")), Some("mov"));
        assert_eq!(name("mov", b"\0\0\0\x08wide\0\0\0\0mdat"), Some("mov"));
        assert_eq!(name("mov", &ftyp(b"M4A ")), Some("ipod"));
        assert_eq!(name("mov", &ftyp(b"3gp4")), Some("3gp"));
        assert_eq!(name("mov", &ftyp(b"3g2a")), Some("3g2"));
        let mp4 = muxer_for("mov", &ftyp(b"isom")).unwrap();
        assert_eq!((mp4.name, mp4.brand.as_deref()), ("mp4", Some("isom")));
        assert_eq!(muxer_for("mov", &ftyp(b"M4V ")).unwrap().brand.as_deref(), Some("M4V "));
        assert_eq!(muxer_for("mov", &ftyp(b"\xFF\x01ab")).unwrap().brand, None);
        for c in ["mp3", "flac", "ogg", "wav", "avi"] {
            assert_eq!(name(c, b""), Some(c));
        }
        assert_eq!(name("matroska", b"\x1a\x45\xdf\xa3\x42\x82\x88matroska"), Some("matroska"));
        assert_eq!(name("matroska", b"\x1a\x45\xdf\xa3\x42\x82\x84webm"), Some("webm"));
        assert_eq!(name("matroska", b"\x1a\x45\xdf\xa3"), Some("matroska"));
        for other in ["asf", "flv", "mpegts", "mpeg", "aac", "ape", "wv", "gif", "m4v"] {
            assert_eq!(name(other, b""), None, "{other}");
        }
    }

    #[test]
    fn the_arguments_guard_the_input_and_copy_every_stream() {
        let muxer = muxer_for("mov", &ftyp(b"isom")).unwrap();
        let args = strip_args("/in", "/out", "mov", &muxer);
        let line = args.join(" ");
        assert!(
            line.starts_with("-nostdin -protocol_whitelist file,pipe -f mov -use_absolute_path 0 -i /in "),
            "{line}"
        );
        assert!(line.contains(" -map 0 -map -0:d? -map_metadata -1 -map_chapters -1 -c copy "), "{line}");
        assert!(line.ends_with(" -movflags +faststart -brand isom -f mp4 -y /out"), "{line}");
        let flac = strip_args("/in", "/out", "flac", &Muxer { name: "flac", brand: None }).join(" ");
        assert!(!flac.contains("faststart"), "{flac}");
    }

    fn answer(tags: Value, side: Value) -> Value {
        json!({
            "streams": [
                {"codec_name": "mjpeg", "codec_type": "video", "disposition": {"attached_pic": 1},
                 "side_data_list": [{"rotation": 180}]},
                {"codec_name": "h264", "codec_type": "video", "disposition": {"attached_pic": 0}, "tags": tags,
                 "side_data_list": side},
                {"codec_name": "aac", "codec_type": "audio", "disposition": {"attached_pic": 0}}
            ],
            "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2", "tags": {"Location": "x", "major_brand": "isom"}}
        })
    }

    #[test]
    fn rotation_comes_from_the_picture_not_from_cover_art() {
        let r = parse_report(&answer(json!({}), json!([{"rotation": -90}]))).unwrap();
        assert_eq!(r.rotation, -90);
        let r = parse_report(&answer(json!({"rotate": "270"}), json!([]))).unwrap();
        assert_eq!(r.rotation, 270);
        let r = parse_report(&answer(json!({}), json!([{"side_data_type": "x"}]))).unwrap();
        assert_eq!(r.rotation, 0);
        assert_eq!(parse_report(&json!({"format": {"format_name": "tty"}})), None);
    }

    #[test]
    fn a_tag_that_tells_something_is_noticed() {
        let r = parse_report(&answer(json!({"com.apple.quicktime.creationdate": "2021"}), json!([]))).unwrap();
        assert!(r.tags.contains(&"location".to_string()));
        assert_eq!(telling_tag(&r.tags), Some("location"));
        assert_eq!(telling_tag(&["handler_name".to_string(), "vendor_id".to_string()]), None);
        assert_eq!(
            telling_tag(&["com.apple.quicktime.creationdate".to_string()]),
            Some("com.apple.quicktime.creationdate")
        );
    }

    #[test]
    fn failures_keep_their_meaning() {
        let timeout: anyhow::Error = AvTimeoutError { ms: 1 }.into();
        assert_eq!(tool_failure(&timeout), KeptReason::TooLarge);
        let refused: anyhow::Error = AvToolError { tool: AvTool::Ffmpeg, code: Some(1), stderr: String::new() }.into();
        assert_eq!(tool_failure(&refused), KeptReason::Damaged);
        assert_eq!(tool_failure(&anyhow::anyhow!("spawn failed")), KeptReason::Tool);
    }
}
