//! Port of `lib/media/av-targets.ts`: what a video, an audio file or an
//! animation can be turned into, and what that costs. Like convert-targets,
//! this is the ONLY answer to "what may be asked for": the menu renders from
//! [`av_targets_for`] and the route admits through [`av_option_for`].
//!
//! The static question ("does a .mkv offer MP4 at all?") is answered here
//! from the name; the dynamic one ("do THIS file's codecs fit?") by
//! [`container_verdict`] over a probe.

use crate::media::mime::{MediaKind, effective_type, media_kind};

/// Containers a video can be poured into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoContainer {
    Mp4,
    Mov,
    Mkv,
    Webm,
}

/// `AvTarget`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AvTarget {
    Container(VideoContainer),
    Webmp4,
    Mp3,
    Ogg,
    Wav,
    Gif,
    Webp,
}

impl VideoContainer {
    pub fn as_str(self) -> &'static str {
        match self {
            VideoContainer::Mp4 => "mp4",
            VideoContainer::Mov => "mov",
            VideoContainer::Mkv => "mkv",
            VideoContainer::Webm => "webm",
        }
    }

    /// `isVideoContainer(v)`
    pub fn parse(v: &str) -> Option<VideoContainer> {
        match v {
            "mp4" => Some(VideoContainer::Mp4),
            "mov" => Some(VideoContainer::Mov),
            "mkv" => Some(VideoContainer::Mkv),
            "webm" => Some(VideoContainer::Webm),
            _ => None,
        }
    }
}

/// `isVideoContainer(v)`
pub fn is_video_container(v: &str) -> bool {
    VideoContainer::parse(v).is_some()
}

impl AvTarget {
    /// `isAvTarget(v)`
    pub fn parse(v: Option<&str>) -> Option<AvTarget> {
        ALL_TARGETS.into_iter().find(|t| Some(t.as_str()) == v)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AvTarget::Container(VideoContainer::Mp4) => "mp4",
            AvTarget::Container(VideoContainer::Mov) => "mov",
            AvTarget::Container(VideoContainer::Mkv) => "mkv",
            AvTarget::Container(VideoContainer::Webm) => "webm",
            AvTarget::Webmp4 => "webmp4",
            AvTarget::Mp3 => "mp3",
            AvTarget::Ogg => "ogg",
            AvTarget::Wav => "wav",
            AvTarget::Gif => "gif",
            AvTarget::Webp => "webp",
        }
    }
}

/// How expensive the operation is, and therefore who may ask for it.
///
/// - `copy`: no encoder runs. The streams are moved into another container
///   untouched. The cost is bounded by input SIZE and is independent of what
///   is in the file, which is what makes it safe to expose publicly.
/// - `encode`: an encoder runs, and the caller chooses the workload (which
///   seconds, how many). Owner only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AvCost {
    Copy,
    Encode,
}

impl AvCost {
    pub fn as_str(self) -> &'static str {
        match self {
            AvCost::Copy => "copy",
            AvCost::Encode => "encode",
        }
    }
}

/// The picture size the ROW carries (uploads.width / uploads.height).
///
/// Both None is the normal state for anything uploaded before the columns
/// existed, and it means "nobody has measured", never "small". Every decision
/// that reads it therefore fails CLOSED: an unmeasured video is offered no
/// rescue, which is the only safe answer when the whole point of the entry is
/// that it applies to a minority of files.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AvSourceSize {
    pub width: Option<f64>,
    pub height: Option<f64>,
}

/// Longest edge a web-playable copy is scaled down to.
///
/// NOT 4096, although 4096 is where the hardware decoders stop (Intel QuickSync,
/// NVDEC and AMD all refuse an H.264 picture wider than that and the browser
/// falls back to software, which is the whole reason the recording that started
/// this stutters). Two measurements argue against parking the output right under
/// that wall, both taken on the production container - two cores, 2 GiB,
/// ffmpeg 5.1.9 - against the reported file (5056x1924, 403 frames, 13.43 s):
///
/// ```text
///     scaled to 4096 wide     21867 ms    1353 kB    H.264 level 5.1
///     scaled to 1920 wide      9459 ms     409 kB    H.264 level 4.0
///     scaled to 1280 wide      8105 ms     212 kB    H.264 level 3.1
/// ```
///
/// The 4096 output costs 2.3x the encode for a picture that is still 6.4 Mpx and
/// still level 5.1 - a level plenty of phones and older laptops decline even
/// though the width is legal. 1920 lands on High@4.0, which is the single most
/// widely decodable H.264 configuration there is, and 1.4 Mpx at 30 fps is
/// 42 Mpx/s against the ~292 Mpx/s a software decoder manages, so there is a
/// seven-fold margin even where hardware decoding is unavailable.
///
/// It is also the OFFER threshold, and deliberately the same number: the entry
/// appears exactly when it would change the picture size, so a menu row can
/// never produce a file identical to the one the reader already has.
pub const WEB_MAX_EDGE: f64 = 1920.0;

/// Would a web-playable copy be smaller than this file?
///
/// The longest edge rather than the width, so a portrait recording and a rotated
/// one are judged the same way. Unknown dimensions answer false - see
/// AvSourceSize.
pub fn needs_web_rescue(size: Option<&AvSourceSize>) -> bool {
    let w = size.and_then(|s| s.width).unwrap_or(0.0);
    let h = size.and_then(|s| s.height).unwrap_or(0.0);
    w.max(h) > WEB_MAX_EDGE
}

/// What the SOURCE is, as the upload row already classifies it.
///
/// Not inferred from the probe, and that matters: an animated AVIF and a short
/// silent MP4 are indistinguishable to ffprobe (both demux as `mov`, both have
/// one video stream and no audio), but they want opposite treatment. It lives
/// here rather than beside the planner because both the budget and the planner
/// need it, and neither may import the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AvSourceKind {
    Video,
    Audio,
    ImageSequence,
}

// ---------------------------------------------------------------------------
// The static offer
// ---------------------------------------------------------------------------

/// `AvYield`: what comes out, so the menu can label each entry truthfully.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AvYield {
    Video,
    WebVideo,
    Clip,
    Animation,
    Audio,
    AudioFormat,
    AudioVideo,
}

impl AvYield {
    pub fn as_str(self) -> &'static str {
        match self {
            AvYield::Video => "video",
            AvYield::WebVideo => "webVideo",
            AvYield::Clip => "clip",
            AvYield::Animation => "animation",
            AvYield::Audio => "audio",
            AvYield::AudioFormat => "audioFormat",
            AvYield::AudioVideo => "audioVideo",
        }
    }
}

/// `AvOption`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvOption {
    pub to: AvTarget,
    pub cost: AvCost,
    pub yields: AvYield,
}

/// Every target name, in upstream's order.
pub const ALL_TARGETS: [AvTarget; 10] = [
    AvTarget::Container(VideoContainer::Mp4),
    AvTarget::Container(VideoContainer::Mov),
    AvTarget::Container(VideoContainer::Mkv),
    AvTarget::Container(VideoContainer::Webm),
    AvTarget::Webmp4,
    AvTarget::Mp3,
    AvTarget::Ogg,
    AvTarget::Wav,
    AvTarget::Gif,
    AvTarget::Webp,
];

/// `isAvTarget(v)`
pub fn is_av_target(v: Option<&str>) -> bool {
    AvTarget::parse(v).is_some()
}

/// Video codecs an MP4/MOV may carry and every browser plays (no VP8/VP9:
/// Safari will not play them from MP4).
const MP4_VIDEO_CODECS: &[&str] = &["h264", "hevc", "av1", "mpeg4"];
const MP4_AUDIO_CODECS: &[&str] = &["aac", "mp3", "alac"];
/// WebM is a profile of Matroska with a short, fixed codec list.
const WEBM_VIDEO_CODECS: &[&str] = &["vp8", "vp9", "av1"];
const WEBM_AUDIO_CODECS: &[&str] = &["opus", "vorbis"];

/// `CONTAINER_CODECS[to]`: `None` means "anything" (Matroska).
fn container_codecs(to: VideoContainer) -> (Option<&'static [&'static str]>, Option<&'static [&'static str]>) {
    match to {
        VideoContainer::Mp4 | VideoContainer::Mov => (Some(MP4_VIDEO_CODECS), Some(MP4_AUDIO_CODECS)),
        VideoContainer::Mkv => (None, None),
        VideoContainer::Webm => (Some(WEBM_VIDEO_CODECS), Some(WEBM_AUDIO_CODECS)),
    }
}

/// `CONTAINER_MUXER[to]`: the muxer to force with `-f`, never from a caller.
pub fn container_muxer(to: VideoContainer) -> &'static str {
    match to {
        VideoContainer::Mp4 => "mp4",
        VideoContainer::Mov => "mov",
        VideoContainer::Mkv => "matroska",
        VideoContainer::Webm => "webm",
    }
}

/// MP4 and MOV are both ISO base media format, and only they take faststart.
pub fn is_iso_bmff(to: VideoContainer) -> bool {
    matches!(to, VideoContainer::Mp4 | VideoContainer::Mov)
}

/// `SOURCE_CONTAINER[type] ?? "other"` (`None` = other).
fn source_container(t: &str) -> Option<VideoContainer> {
    match t {
        "video/mp4" | "video/x-m4v" | "video/m4v" | "video/3gpp" | "video/3gpp2" => Some(VideoContainer::Mp4),
        "video/quicktime" => Some(VideoContainer::Mov),
        "video/x-matroska" => Some(VideoContainer::Mkv),
        "video/webm" => Some(VideoContainer::Webm),
        _ => None,
    }
}

/// `CONTAINER_OFFERS[source]`, in menu order (every entry is a stream copy).
fn container_offers(source: Option<VideoContainer>) -> &'static [VideoContainer] {
    use VideoContainer::*;
    match source {
        Some(Mp4) => &[Mov, Mkv],
        Some(Mov) => &[Mp4, Mkv],
        Some(Mkv) => &[Mp4, Mov],
        Some(Webm) => &[Mkv],
        None => &[Mp4, Mov, Mkv],
    }
}

/// Image containers libvips cannot animate for us, but ffmpeg can.
const FFMPEG_ONLY_ANIMATION_EXT: &[&str] = &["avif", "heic", "heif"];

/// Sound formats we encode into.
const AUDIO_TARGETS: [AvTarget; 3] = [AvTarget::Mp3, AvTarget::Ogg, AvTarget::Wav];

/// `AUDIO_SELF[type]`: the target a sound file already is.
fn audio_self(t: &str) -> Option<AvTarget> {
    match t {
        "audio/mpeg" | "audio/mp3" => Some(AvTarget::Mp3),
        "audio/ogg" | "audio/vorbis" => Some(AvTarget::Ogg),
        "audio/wav" | "audio/x-wav" | "audio/wave" => Some(AvTarget::Wav),
        _ => None,
    }
}

/// `ext(name)`: without a leading dot, lowercased.
fn ext_of(name: Option<&str>) -> String {
    let n = name.unwrap_or("");
    n.strip_prefix('.').unwrap_or(n).to_lowercase()
}

/// `avTargetsFor(mime, ext, frames, size)`: everything this file offers, in
/// menu order. `frames` is the row's count (1 = still, `None` = uncounted).
pub fn av_targets_for(
    mime: Option<&str>,
    extension: Option<&str>,
    frames: Option<f64>,
    size: Option<&AvSourceSize>,
) -> Vec<AvOption> {
    let e = ext_of(extension);
    let t = effective_type(mime, extension);
    let kind = media_kind(mime, extension);
    let opt = |to, cost, yields| AvOption { to, cost, yields };

    if kind == MediaKind::Video {
        let mut out = Vec::new();
        if needs_web_rescue(size) {
            out.push(opt(AvTarget::Webmp4, AvCost::Encode, AvYield::WebVideo));
        }
        for c in container_offers(source_container(&t)) {
            out.push(opt(AvTarget::Container(*c), AvCost::Copy, AvYield::Video));
        }
        out.push(opt(AvTarget::Gif, AvCost::Encode, AvYield::Clip));
        out.push(opt(AvTarget::Mp3, AvCost::Encode, AvYield::Audio));
        return out;
    }

    if kind == MediaKind::Audio {
        let own = audio_self(&t);
        let mut out: Vec<AvOption> = AUDIO_TARGETS
            .into_iter()
            .filter(|to| Some(*to) != own && to.as_str() != e)
            .map(|to| opt(to, AvCost::Encode, AvYield::AudioFormat))
            .collect();
        out.push(opt(AvTarget::Container(VideoContainer::Mp4), AvCost::Encode, AvYield::AudioVideo));
        return out;
    }

    if kind == MediaKind::Image && frames != Some(1.0) {
        if FFMPEG_ONLY_ANIMATION_EXT.contains(&e.as_str()) {
            return vec![
                opt(AvTarget::Gif, AvCost::Encode, AvYield::Animation),
                opt(AvTarget::Webp, AvCost::Encode, AvYield::Animation),
            ];
        }
        if t == "image/gif" {
            return vec![opt(AvTarget::Container(VideoContainer::Mp4), AvCost::Encode, AvYield::Animation)];
        }
    }
    Vec::new()
}

/// `avOptionFor(...)`: the route's admission test.
pub fn av_option_for(
    mime: Option<&str>,
    extension: Option<&str>,
    frames: Option<f64>,
    to: Option<&str>,
    size: Option<&AvSourceSize>,
) -> Option<AvOption> {
    let target = AvTarget::parse(to)?;
    av_targets_for(mime, extension, frames, size).into_iter().find(|o| o.to == target)
}

// ---------------------------------------------------------------------------
// The dynamic half
// ---------------------------------------------------------------------------

/// `ContainerVerdict`: `Ok(())`, or why the bytes cannot be copied in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerRefusal {
    VideoCodec,
    NoVideo,
}

impl ContainerRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            ContainerRefusal::VideoCodec => "video-codec",
            ContainerRefusal::NoVideo => "no-video",
        }
    }
}

/// `containerVerdict(to, probe)`: can these streams be poured into `to`
/// without touching a sample? (An incompatible AUDIO track is dropped by the
/// caller, not fatal.)
pub fn container_verdict(to: VideoContainer, video_codec: Option<&str>) -> Result<(), ContainerRefusal> {
    let Some(codec) = video_codec else {
        return Err(ContainerRefusal::NoVideo);
    };
    match container_codecs(to).0 {
        Some(allowed) if !allowed.contains(&codec) => Err(ContainerRefusal::VideoCodec),
        _ => Ok(()),
    }
}

/// `audioCopyableTo(to, codec)`
pub fn audio_copyable_to(to: VideoContainer, codec: Option<&str>) -> bool {
    let Some(codec) = codec.filter(|c| !c.is_empty()) else {
        return false;
    };
    container_codecs(to).1.is_none_or(|allowed| allowed.contains(&codec))
}

/// `AV_OUTPUT[to]`: MIME and extension a target produces.
pub fn av_output(to: AvTarget) -> (&'static str, &'static str) {
    match to {
        AvTarget::Container(VideoContainer::Mp4) | AvTarget::Webmp4 => ("video/mp4", "mp4"),
        AvTarget::Container(VideoContainer::Mov) => ("video/quicktime", "mov"),
        AvTarget::Container(VideoContainer::Mkv) => ("video/x-matroska", "mkv"),
        AvTarget::Container(VideoContainer::Webm) => ("video/webm", "webm"),
        AvTarget::Mp3 => ("audio/mpeg", "mp3"),
        AvTarget::Ogg => ("audio/ogg", "ogg"),
        AvTarget::Wav => ("audio/wav", "wav"),
        AvTarget::Gif => ("image/gif", "gif"),
        AvTarget::Webp => ("image/webp", "webp"),
    }
}

#[cfg(test)]
mod menu_tests {
    //! tests/unit/av-targets.test.ts (the menu, admission and verdict parts;
    //! the budget parts are in av_cost.rs)
    use super::*;

    fn names(o: &[AvOption]) -> Vec<&'static str> {
        o.iter().map(|x| x.to.as_str()).collect()
    }
    fn menu(mime: &str, ext: &str) -> Vec<AvOption> {
        av_targets_for(Some(mime), Some(ext), None, None)
    }
    fn of<'a>(o: &'a [AvOption], to: &str) -> &'a AvOption {
        o.iter().find(|x| x.to.as_str() == to).unwrap()
    }

    #[test]
    fn video_menu() {
        assert_eq!(names(&menu("video/mp4", "mp4")), ["mov", "mkv", "gif", "mp3"]);
        assert_eq!(names(&menu("video/quicktime", "mov")), ["mp4", "mkv", "gif", "mp3"]);
        for (mime, ext) in
            [("video/mp4", "mp4"), ("video/quicktime", "mov"), ("video/x-matroska", "mkv"), ("video/webm", "webm")]
        {
            assert!(!names(&menu(mime, ext)).contains(&ext), "{ext}");
        }
        for (mime, ext) in [
            ("video/mp4", "mp4"),
            ("video/quicktime", "mov"),
            ("video/webm", "webm"),
            ("video/x-msvideo", "avi"),
            ("video/x-ms-wmv", "wmv"),
        ] {
            assert!(names(&menu(mime, ext)).contains(&"mkv"), "{ext}");
        }
        assert!(!names(&menu("video/x-matroska", "mkv")).contains(&"webm"));
        assert_eq!(names(&menu("video/webm", "webm")), ["mkv", "gif", "mp3"]);
        let video = menu("video/x-matroska", "mkv");
        assert_eq!(of(&video, "mp4").cost, AvCost::Copy);
        assert_eq!(of(&video, "mov").cost, AvCost::Copy);
        assert_eq!(of(&video, "gif").cost, AvCost::Encode);
        assert_eq!(of(&video, "mp3").cost, AvCost::Encode);
        let big = AvSourceSize { width: Some(5056.0), height: Some(1924.0) };
        assert_eq!(names(&av_targets_for(Some("video/mp4"), Some("mp4"), None, Some(&big)))[0], "webmp4");
    }

    #[test]
    fn audio_menu() {
        assert_eq!(names(&menu("audio/mpeg", "mp3")), ["ogg", "wav", "mp4"]);
        assert_eq!(names(&menu("audio/wav", "wav")), ["mp3", "ogg", "mp4"]);
        assert_eq!(names(&menu("audio/ogg", "ogg")), ["mp3", "wav", "mp4"]);
        let audio = menu("audio/mpeg", "mp3");
        assert_eq!(of(&audio, "mp4").yields, AvYield::AudioVideo);
        assert_eq!(of(&audio, "mp4").cost, AvCost::Encode);
        assert_eq!(of(&menu("audio/mp4", "m4a"), "mp3").yields, AvYield::AudioFormat);
        assert_eq!(of(&menu("video/mp4", "mp4"), "mp3").yields, AvYield::Audio);
    }

    #[test]
    fn animation_menu() {
        let options = av_targets_for(Some("image/avif"), Some("avif"), None, None);
        assert_eq!(names(&options), ["gif", "webp"]);
        assert!(options.iter().all(|o| o.cost == AvCost::Encode && o.yields == AvYield::Animation));
        let gif = av_targets_for(Some("image/gif"), Some("gif"), Some(128.0), None);
        assert_eq!(names(&gif), ["mp4"]);
        assert_eq!(of(&gif, "mp4").yields, AvYield::Animation);
        assert!(av_targets_for(Some("image/avif"), Some("avif"), Some(1.0), None).is_empty());
        assert!(av_targets_for(Some("image/gif"), Some("gif"), Some(1.0), None).is_empty());
        for (mime, ext) in [
            ("image/png", "png"),
            ("image/jpeg", "jpg"),
            ("image/webp", "webp"),
            ("application/zip", "zip"),
            ("application/pdf", "pdf"),
            ("text/plain", "txt"),
        ] {
            assert!(menu(mime, ext).is_empty(), "{mime}");
        }
    }

    #[test]
    fn the_route_admits_exactly_what_the_menu_shows() {
        let files = [
            ("video/x-matroska", "mkv"),
            ("video/mp4", "mp4"),
            ("video/quicktime", "mov"),
            ("video/webm", "webm"),
            ("video/x-msvideo", "avi"),
            ("audio/mpeg", "mp3"),
            ("audio/wav", "wav"),
            ("image/avif", "avif"),
            ("image/gif", "gif"),
            ("image/png", "png"),
            ("application/zip", "zip"),
        ];
        let asked = ["mp4", "mov", "mkv", "webm", "mp3", "ogg", "wav", "gif", "webp", "png", "jpg", "m4a", "exe", ""];
        for (mime, ext) in files {
            let visible = names(&menu(mime, ext));
            for to in asked {
                let accepted = av_option_for(Some(mime), Some(ext), None, Some(to), None).is_some();
                assert_eq!(accepted, visible.contains(&to), "{mime} -> {to}");
            }
        }
        assert!(is_av_target(Some("mp4")));
        assert!(is_av_target(Some("mkv")));
        assert!(!is_av_target(Some("png")));
        assert!(!is_av_target(Some("m4a")));
        assert!(!is_av_target(None));
        assert!(!is_av_target(Some("../../etc/passwd")));
        assert!(is_video_container("mkv"));
        assert!(!is_video_container("gif"));
    }

    #[test]
    fn verdicts() {
        for to in [VideoContainer::Mp4, VideoContainer::Mov] {
            assert_eq!(container_verdict(to, Some("h264")), Ok(()));
            assert_eq!(container_verdict(to, Some("av1")), Ok(()));
        }
        assert_eq!(container_verdict(VideoContainer::Mp4, Some("vp9")), Err(ContainerRefusal::VideoCodec));
        for codec in ["vp9", "h264", "wmv2", "theora", "prores"] {
            assert_eq!(container_verdict(VideoContainer::Mkv, Some(codec)), Ok(()));
        }
        assert!(audio_copyable_to(VideoContainer::Mkv, Some("opus")));
        assert!(audio_copyable_to(VideoContainer::Mkv, Some("pcm_s16le")));
        assert_eq!(container_verdict(VideoContainer::Webm, Some("vp9")), Ok(()));
        assert_eq!(container_verdict(VideoContainer::Webm, Some("h264")), Err(ContainerRefusal::VideoCodec));
        assert!(audio_copyable_to(VideoContainer::Webm, Some("opus")));
        assert!(!audio_copyable_to(VideoContainer::Webm, Some("aac")));
        assert_eq!(container_verdict(VideoContainer::Mp4, None), Err(ContainerRefusal::NoVideo));
        assert!(!audio_copyable_to(VideoContainer::Mp4, Some("opus")));
        assert!(audio_copyable_to(VideoContainer::Mp4, Some("aac")));
        assert!(audio_copyable_to(VideoContainer::Mp4, Some("mp3")));
        assert!(!audio_copyable_to(VideoContainer::Mp4, None));
    }
}
