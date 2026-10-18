//! "Download as": which conversions a file is offered, and how their hints and
//! refusals read (`ModlightDownloadMenu.cs`, `ModlightConvertText.cs`).

use crate::l10n::{Key, s, t};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadHint {
    None,
    KeepsAnimation,
    SingleFrame,
    FrameIfAnimated,
    ContainerCopy,
    AnimationClip,
    KeepsMotion,
    AudioTrack,
    AudioFormat,
    AudioAsVideo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadOption {
    /// The target format, as the `/convert?to=` route spells it.
    pub to: String,
    pub hint: DownloadHint,
    /// Listed under the "frame" heading: the conversion keeps one frame of something
    /// that moves.
    pub loses_motion: bool,
}

const IMAGE_TARGETS: [&str; 5] = ["pdf", "png", "jpg", "webp", "avif"];
const ANIMATION_CAPABLE: [&str; 4] = ["image/gif", "image/apng", "image/webp", "image/avif"];
const ANIMATION_READABLE: [&str; 3] = ["image/gif", "image/apng", "image/webp"];
const SELF_TARGET: [(&str, &str); 5] = [
    ("image/png", "png"),
    ("image/apng", "png"),
    ("image/jpeg", "jpg"),
    ("image/webp", "webp"),
    ("image/avif", "avif"),
];
const SOURCE_CONTAINER: [(&str, &str); 8] = [
    ("video/mp4", "mp4"),
    ("video/x-m4v", "mp4"),
    ("video/m4v", "mp4"),
    ("video/3gpp", "mp4"),
    ("video/3gpp2", "mp4"),
    ("video/quicktime", "mov"),
    ("video/x-matroska", "mkv"),
    ("video/webm", "webm"),
];
const AUDIO_TARGETS: [&str; 3] = ["mp3", "ogg", "wav"];
const AUDIO_SELF: [(&str, &str); 7] = [
    ("audio/mpeg", "mp3"),
    ("audio/mp3", "mp3"),
    ("audio/ogg", "ogg"),
    ("audio/vorbis", "ogg"),
    ("audio/wav", "wav"),
    ("audio/x-wav", "wav"),
    ("audio/wave", "wav"),
];
const FFMPEG_ONLY_ANIMATION: [&str; 3] = ["avif", "heic", "heif"];
const USELESS_TYPES: [&str; 4] = [
    "application/octet-stream",
    "binary/octet-stream",
    "application/unknown",
    "unknown/unknown",
];
const TYPE_FROM_EXTENSION: [(&str, &str); 18] = [
    ("png", "image/png"),
    ("apng", "image/apng"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("avif", "image/avif"),
    ("mp4", "video/mp4"),
    ("m4v", "video/x-m4v"),
    ("mov", "video/quicktime"),
    ("mkv", "video/x-matroska"),
    ("webm", "video/webm"),
    ("mp3", "audio/mpeg"),
    ("ogg", "audio/ogg"),
    ("oga", "audio/ogg"),
    ("wav", "audio/wav"),
    ("m4a", "audio/mp4"),
    ("flac", "audio/flac"),
];

fn lookup<'a>(table: &[(&str, &'a str)], key: &str) -> Option<&'a str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

fn container_offers(source: &str) -> &'static [&'static str] {
    match source {
        "mp4" => &["mov", "mkv"],
        "mov" => &["mp4", "mkv"],
        "mkv" => &["mp4", "mov"],
        "webm" => &["mkv"],
        _ => &["mp4", "mov", "mkv"],
    }
}

fn normalise(extension: Option<&str>) -> String {
    extension
        .unwrap_or_default()
        .trim_start_matches('.')
        .to_ascii_lowercase()
}

/// The MIME type, unless it says nothing; then the extension's.
fn effective_type(mime: Option<&str>, extension: Option<&str>) -> String {
    let mime = mime
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if !mime.is_empty() && !USELESS_TYPES.contains(&mime.as_str()) {
        return mime;
    }
    let ext = normalise(extension);
    TYPE_FROM_EXTENSION
        .iter()
        .find(|(e, _)| e.eq_ignore_ascii_case(&ext))
        .map(|(_, t)| t.to_string())
        .unwrap_or(mime)
}

/// Every conversion the server can be asked for this file, main ones first, the ones
/// that keep only one frame of something that moves last.
pub fn options_for(
    kind: Option<&str>,
    mime: Option<&str>,
    extension: Option<&str>,
) -> Vec<DownloadOption> {
    let ty = effective_type(mime, extension);
    let ext = normalise(extension);

    let av = audio_video_targets(kind, &ty, &ext);
    let mut main: Vec<DownloadOption> = av.clone();
    let claimed: Vec<String> = av.iter().map(|o| o.to.clone()).collect();

    let images: Vec<DownloadOption> = image_targets(kind, &ty)
        .into_iter()
        .filter(|o| !claimed.contains(&o.to))
        .collect();
    let anything_keeps_motion = images
        .iter()
        .any(|o| o.hint == DownloadHint::KeepsAnimation)
        || av.iter().any(|o| o.hint == DownloadHint::KeepsMotion);

    let mut frame = Vec::new();
    for option in images {
        let demote = option.hint == DownloadHint::SingleFrame
            || (option.hint == DownloadHint::FrameIfAnimated && anything_keeps_motion);
        let moved = DownloadOption {
            to: option.to,
            hint: option.hint,
            loses_motion: demote,
        };
        if demote {
            frame.push(moved);
        } else {
            main.push(moved);
        }
    }
    main.extend(frame);
    main
}

fn option(to: &str, hint: DownloadHint) -> DownloadOption {
    DownloadOption {
        to: to.to_string(),
        hint,
        loses_motion: false,
    }
}

fn audio_video_targets(kind: Option<&str>, ty: &str, ext: &str) -> Vec<DownloadOption> {
    let mut options = Vec::new();
    match kind {
        Some("video") => {
            let source = lookup(&SOURCE_CONTAINER, ty).unwrap_or("other");
            for to in container_offers(source) {
                options.push(option(to, DownloadHint::ContainerCopy));
            }
            options.push(option("gif", DownloadHint::AnimationClip));
            options.push(option("mp3", DownloadHint::AudioTrack));
        }
        Some("audio") => {
            for to in AUDIO_TARGETS {
                let is_self = lookup(&AUDIO_SELF, ty) == Some(to) || ext.eq_ignore_ascii_case(to);
                if !is_self {
                    options.push(option(to, DownloadHint::AudioFormat));
                }
            }
            options.push(option("mp4", DownloadHint::AudioAsVideo));
        }
        Some("image") => {
            if FFMPEG_ONLY_ANIMATION
                .iter()
                .any(|e| e.eq_ignore_ascii_case(ext))
            {
                options.push(option("gif", DownloadHint::KeepsMotion));
                options.push(option("webp", DownloadHint::KeepsMotion));
                return options;
            }
            if ty == "image/gif" {
                options.push(option("mp4", DownloadHint::KeepsMotion));
            }
        }
        _ => {}
    }
    options
}

fn image_targets(kind: Option<&str>, ty: &str) -> Vec<DownloadOption> {
    let mut options = Vec::new();
    if kind != Some("image") {
        return options;
    }
    let can_animate = ANIMATION_CAPABLE.contains(&ty);
    let keeps_motion = ANIMATION_READABLE.contains(&ty);
    for to in IMAGE_TARGETS {
        if lookup(&SELF_TARGET, ty) == Some(to) {
            continue;
        }
        if !can_animate {
            options.push(option(to, DownloadHint::None));
            continue;
        }
        let hint = if to == "webp" && keeps_motion {
            DownloadHint::KeepsAnimation
        } else {
            DownloadHint::FrameIfAnimated
        };
        options.push(option(to, hint));
    }
    // Stable: the one that keeps the animation first, the rest in their order.
    options.sort_by_key(|o| {
        if o.hint == DownloadHint::KeepsAnimation {
            0
        } else {
            1
        }
    });
    options
}

/// The menu header for one option: "JPG   -   one frame".
pub fn option_header(option: &DownloadOption) -> String {
    let hint = hint_text(option.hint);
    if hint.is_empty() {
        option.to.to_uppercase()
    } else {
        format!("{}   -   {}", option.to.to_uppercase(), hint)
    }
}

pub fn hint_text(hint: DownloadHint) -> &'static str {
    let key: Option<Key> = match hint {
        DownloadHint::KeepsAnimation => Some(s::MySpace_ConvertKeepsAnimation),
        DownloadHint::SingleFrame => Some(s::MySpace_ConvertSingleFrame),
        DownloadHint::FrameIfAnimated => Some(s::MySpace_ConvertFrameIfAnimated),
        DownloadHint::ContainerCopy => Some(s::MySpace_ConvertContainerCopy),
        DownloadHint::AnimationClip => Some(s::MySpace_ConvertAnimationClip),
        DownloadHint::KeepsMotion => Some(s::MySpace_ConvertKeepsMotion),
        DownloadHint::AudioTrack => Some(s::MySpace_ConvertAudioTrack),
        DownloadHint::AudioFormat => Some(s::MySpace_ConvertAudioFormat),
        DownloadHint::AudioAsVideo => Some(s::MySpace_ConvertAudioAsVideo),
        DownloadHint::None => None,
    };
    key.map(t).unwrap_or("")
}

/// The sentence for a conversion refusal token (`X-Convert-Refused`).
pub fn refusal_text(reason: &str) -> &'static str {
    t(match reason {
        "encode-requires-owner" => s::MySpace_RefusedEncodeRequiresOwner,
        "ffmpeg-unavailable" => s::MySpace_RefusedFfmpegUnavailable,
        "video-codec" => s::MySpace_RefusedVideoCodec,
        "audio-codec" => s::MySpace_RefusedAudioCodec,
        "no-video" => s::MySpace_RefusedNoVideo,
        "no-audio" => s::MySpace_RefusedNoAudio,
        "unreadable" => s::MySpace_RefusedUnreadable,
        "too-large" => s::MySpace_RefusedTooLarge,
        "too-long" => s::MySpace_RefusedTooLong,
        "too-many-pixels" => s::MySpace_RefusedTooManyPixels,
        "wrong-target" => s::MySpace_RefusedWrongTarget,
        "empty-output" => s::MySpace_RefusedEmptyOutput,
        "busy" => s::MySpace_RefusedBusy,
        "timeout" => s::MySpace_RefusedTimeout,
        "missing" => s::MySpace_RefusedMissing,
        "offline" => s::MySpace_RefusedOffline,
        _ => s::MySpace_RefusedUnknown,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets(options: &[DownloadOption]) -> String {
        options
            .iter()
            .map(|o| {
                if o.loses_motion {
                    format!("({})", o.to)
                } else {
                    o.to.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn pictures_offer_the_other_formats() {
        assert_eq!(
            targets(&options_for(Some("image"), Some("image/png"), Some("png"))),
            "pdf jpg webp avif"
        );
        assert_eq!(
            targets(&options_for(Some("image"), Some("image/gif"), Some("gif"))),
            "mp4 webp (pdf) (png) (jpg) (avif)"
        );
        assert_eq!(
            targets(&options_for(
                Some("image"),
                Some("application/octet-stream"),
                Some("avif")
            )),
            "gif webp (pdf) (png) (jpg)"
        );
    }

    #[test]
    fn video_and_audio_offer_containers_and_tracks() {
        assert_eq!(
            targets(&options_for(Some("video"), Some("video/mp4"), Some("mp4"))),
            "mov mkv gif mp3"
        );
        assert_eq!(
            targets(&options_for(
                Some("video"),
                Some("video/x-flv"),
                Some("flv")
            )),
            "mp4 mov mkv gif mp3"
        );
        assert_eq!(
            targets(&options_for(Some("audio"), Some("audio/mpeg"), Some("mp3"))),
            "ogg wav mp4"
        );
        assert!(options_for(Some("document"), Some("application/pdf"), Some("pdf")).is_empty());
    }
}
