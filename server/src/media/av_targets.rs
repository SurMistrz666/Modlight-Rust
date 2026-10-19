//! Port of `lib/media/av-targets.ts` - PARTIAL: the vocabulary `av_cost` and
//! `playback_health` are written in (target and cost names, the source size,
//! the web-rescue rule). The menu itself (`avTargetsFor`, `avOptionFor`,
//! `containerVerdict`, `AV_OUTPUT`, ...) needs `lib/media/mime.ts` and arrives
//! with the media agent's port.

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

impl AvTarget {
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
