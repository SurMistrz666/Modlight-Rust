//! Port of `lib/media/mime.ts`: MIME helpers, governing how files are
//! previewed, embedded, and protected against XSS.

use super::sniff::{is_useless_mime, mime_from_ext};

/// Image types the browser can render inline (preview as <img>).
pub const DISPLAYABLE_IMAGES: &[&str] = &[
    "image/apng",
    "image/bmp",
    "image/gif",
    "image/x-icon",
    "image/jpeg",
    "image/png",
    "image/tiff",
    "image/webp",
    "image/avif",
];

/// Types worth rich-embedding (Open Graph) for bots like Discord/Twitter/Telegram.
fn is_embeddable_type(m: &str) -> bool {
    DISPLAYABLE_IMAGES.contains(&m) || matches!(m, "video/mp4" | "video/ogg" | "video/webm")
}

/// Strip parameters and casing: `Text/HTML; charset=UTF-8` -> `text/html`.
fn normalize_mime(mime: &str) -> String {
    crate::config::js_trim(mime.split(';').next().unwrap_or("")).to_lowercase()
}

/// The type to REASON with, given what we stored and what the file is called.
///
/// A stored type is only evidence when it says something. ShareX and curl send
/// `application/octet-stream` for anything Windows has no registry entry for,
/// and an imported row keeps whatever its old host guessed years ago - so for
/// those rows the name on disk is the better clue.
///
/// It changes what we THINK a file is, never what we SERVE it as:
/// [`serve_decision`] is an allowlist, so resolving an upload to text/html or
/// image/svg+xml still puts `text/plain` on the wire.
pub fn effective_type(mime: Option<&str>, ext: Option<&str>) -> String {
    let m = normalize_mime(mime.unwrap_or(""));
    if !m.is_empty() && !is_useless_mime(Some(&m)) {
        return m;
    }
    mime_from_ext(ext).map(str::to_string).unwrap_or(m)
}

pub fn is_displayable_image(mime: Option<&str>, ext: Option<&str>) -> bool {
    DISPLAYABLE_IMAGES.contains(&effective_type(mime, ext).as_str())
}

pub fn is_embeddable(mime: Option<&str>, ext: Option<&str>) -> bool {
    is_embeddable_type(&effective_type(mime, ext))
}

/// Types that must never be served with their own Content-Type, because the
/// browser would execute them in our origin (stored XSS).
pub fn must_be_escaped(mime: Option<&str>) -> bool {
    let Some(mime) = mime.filter(|m| !m.is_empty()) else {
        return false;
    };
    let m = normalize_mime(mime);
    m.starts_with("text/htm") || m.contains("svg") || m.ends_with("+xml") || m == "text/xml"
}

/// Types the browser may render in our origin as-is. Everything outside this
/// set is either downgraded to plain text or pushed out as a download - an
/// allowlist, because a blocklist only ever covers the markup types someone
/// thought of.
fn is_inline_safe(m: &str) -> bool {
    DISPLAYABLE_IMAGES.contains(&m)
        || matches!(
            m,
            "video/mp4"
                | "video/webm"
                | "video/ogg"
                | "video/quicktime"
                | "audio/mpeg"
                | "audio/mp4"
                | "audio/ogg"
                | "audio/wav"
                | "audio/x-wav"
                | "audio/webm"
                | "audio/flac"
                | "application/pdf"
                | "text/plain"
        )
}

/// Text-ish payloads worth showing in the browser instead of downloading. They
/// are served as text/plain, which is inert regardless of what they contain.
fn is_text_like(mime: &str) -> bool {
    mime.starts_with("text/")
        || mime.ends_with("+xml")
        || mime.ends_with("+json")
        || matches!(
            mime,
            "application/json"
                | "application/xml"
                | "application/javascript"
                | "application/x-javascript"
                | "application/x-sh"
                | "application/sql"
        )
}

/// Container type -> the type to ADVERTISE, or `None` when no browser can
/// decode it. A container is not a codec: QuickTime and MP4 are both ISO base
/// media format, and Chrome and Firefox reject `video/quicktime` on the LABEL
/// alone. Matroska, AVI, WMV, FLV, MPEG-PS and MPEG-TS have no browser demuxer,
/// so no label helps.
fn video_plays_as(m: &str) -> Option<&'static str> {
    Some(match m {
        "video/mp4" => "video/mp4",
        "video/webm" => "video/webm",
        "video/ogg" => "video/ogg",
        "application/ogg" => "video/ogg",
        "video/quicktime" => "video/mp4",
        "video/x-m4v" => "video/mp4",
        "video/m4v" => "video/mp4",
        "video/3gpp" => "video/mp4",
        "video/3gpp2" => "video/mp4",
        _ => return None,
    })
}

/// Same idea for audio: the container lies, the codec is what plays.
fn audio_plays_as(m: &str) -> Option<&'static str> {
    Some(match m {
        "audio/mpeg" | "audio/mp3" => "audio/mpeg",
        "audio/mp4" | "audio/x-m4a" | "audio/m4a" => "audio/mp4",
        "audio/aac" => "audio/aac",
        "audio/ogg" | "audio/opus" => "audio/ogg",
        "audio/webm" => "audio/webm",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "audio/wav",
        "audio/flac" | "audio/x-flac" => "audio/flac",
        _ => return None,
    })
}

/// The type to put on a <source> and on the Content-Type of an inline video,
/// or `None` when the browser cannot play this file at all.
pub fn playable_video_type(mime: Option<&str>, ext: Option<&str>) -> Option<&'static str> {
    // A stored `video/*` type is trusted even when it is bad news, so a .mkv
    // renamed to .mp4 does not get to claim it plays.
    video_plays_as(&effective_type(mime, ext))
}

pub fn playable_audio_type(mime: Option<&str>, ext: Option<&str>) -> Option<&'static str> {
    audio_plays_as(&effective_type(mime, ext))
}

/// `ServeDecision`: how a raw file may be handed to the browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeDecision {
    /// "inline" | "attachment"
    pub disposition: &'static str,
    pub content_type: String,
}

fn attachment() -> ServeDecision {
    ServeDecision { disposition: "attachment", content_type: "application/octet-stream".to_string() }
}

/// How a raw file may be handed to the browser (see the allowlists above).
pub fn serve_decision(mime: Option<&str>, ext: Option<&str>) -> ServeDecision {
    // The extension rescues the rows whose stored type is useless; the
    // allowlist is unchanged, so this can only ever move a file from
    // "unopenable download" towards "shown safely".
    let m = effective_type(mime, ext);
    if m.is_empty() || is_useless_mime(Some(&m)) {
        return attachment();
    }
    if m.starts_with("video/")
        && let Some(playable) = playable_video_type(Some(&m), ext)
    {
        return ServeDecision { disposition: "inline", content_type: playable.to_string() };
    }
    if m.starts_with("audio/")
        && let Some(playable) = playable_audio_type(Some(&m), ext)
    {
        return ServeDecision { disposition: "inline", content_type: playable.to_string() };
    }
    if is_inline_safe(&m) {
        let content_type = if m == "text/plain" { "text/plain; charset=utf-8".to_string() } else { m };
        return ServeDecision { disposition: "inline", content_type };
    }
    if is_text_like(&m) {
        return ServeDecision { disposition: "inline", content_type: "text/plain; charset=utf-8".to_string() };
    }
    attachment()
}

/// Would `/raw` hand these bytes to a browser as readable text? Derived from
/// [`serve_decision`], never from a list of its own.
pub fn is_text_payload(mime: Option<&str>, ext: Option<&str>) -> bool {
    let d = serve_decision(mime, ext);
    d.disposition == "inline" && d.content_type.starts_with("text/plain")
}

/// `MediaKind`
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Text,
    Pdf,
    Archive,
    Other,
}

impl MediaKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MediaKind::Image => "image",
            MediaKind::Video => "video",
            MediaKind::Audio => "audio",
            MediaKind::Text => "text",
            MediaKind::Pdf => "pdf",
            MediaKind::Archive => "archive",
            MediaKind::Other => "other",
        }
    }
}

pub fn media_kind(mime: Option<&str>, ext: Option<&str>) -> MediaKind {
    let m = effective_type(mime, ext);
    if m.is_empty() {
        return MediaKind::Other;
    }
    let top = m.split('/').next().unwrap_or("");
    match top {
        "image" => {
            if DISPLAYABLE_IMAGES.contains(&m.as_str()) {
                MediaKind::Image
            } else {
                MediaKind::Other
            }
        }
        "video" => MediaKind::Video,
        "audio" => MediaKind::Audio,
        "text" => MediaKind::Text,
        _ if m == "application/pdf" => MediaKind::Pdf,
        // /zip|gzip|tar|rar|7z-compressed/
        _ if ["zip", "gzip", "tar", "rar", "7z-compressed"].iter().any(|n| m.contains(n)) => MediaKind::Archive,
        _ => MediaKind::Other,
    }
}

/// lucide-react icon name for a given MIME. The top-level type is asked as a
/// PREFIX, not as a substring anywhere in the string (a .dmg is
/// `application/x-apple-diskimage`, not a photo).
pub fn mime_to_icon(mime: Option<&str>, ext: Option<&str>) -> &'static str {
    let m = effective_type(mime, ext);
    if m.is_empty() {
        return "File";
    }
    match m.split('/').next().unwrap_or("") {
        "image" => return "FileImage",
        "audio" => return "FileAudio",
        "video" => return "FileVideo",
        _ => {}
    }
    const MAP: &[(&str, &str)] = &[
        ("application/pdf", "FileText"),
        ("msword", "FileText"),
        ("wordprocessing", "FileText"),
        ("spreadsheet", "FileSpreadsheet"),
        ("ms-excel", "FileSpreadsheet"),
        ("presentation", "FileText"),
        ("powerpoint", "FileText"),
        ("text/html", "FileCode"),
        ("text/x-", "FileCode"),
        ("application/json", "FileCode"),
        ("javascript", "FileCode"),
        ("text/plain", "FileText"),
        ("epub", "FileText"),
        // After the document types on purpose: .docx, .xlsx, .pptx and .epub
        // are all zip containers.
        ("zip", "FileArchive"),
        ("gzip", "FileArchive"),
        ("tar", "FileArchive"),
        ("rar", "FileArchive"),
    ];
    MAP.iter().find(|(needle, _)| m.contains(needle)).map(|(_, icon)| *icon).unwrap_or("File")
}

#[cfg(test)]
mod tests {
    //! tests/unit/mime.test.ts
    use super::*;

    #[test]
    fn mime_helpers() {
        assert!(is_displayable_image(Some("image/png"), None));
        assert!(!is_displayable_image(Some("image/svg+xml"), None));
        assert!(!is_displayable_image(Some("application/pdf"), None));
        assert!(!is_displayable_image(None, None));
        assert!(is_embeddable(Some("video/mp4"), None));
        assert!(is_embeddable(Some("image/webp"), None));
        assert!(!is_embeddable(Some("application/zip"), None));
        assert!(must_be_escaped(Some("image/svg+xml")));
        assert!(must_be_escaped(Some("text/html")));
        assert!(!must_be_escaped(Some("image/png")));
        assert_eq!(media_kind(Some("image/png"), None), MediaKind::Image);
        assert_eq!(media_kind(Some("video/mp4"), None), MediaKind::Video);
        assert_eq!(media_kind(Some("text/plain"), None), MediaKind::Text);
        assert_eq!(media_kind(Some("application/pdf"), None), MediaKind::Pdf);
        assert_eq!(media_kind(Some("application/zip"), None), MediaKind::Archive);
        assert_eq!(media_kind(Some("image/svg+xml"), None), MediaKind::Other);
        assert_eq!(media_kind(Some("application/octet-stream"), None), MediaKind::Other);
    }

    #[test]
    fn consults_the_extension_only_when_the_stored_type_is_useless() {
        assert_eq!(effective_type(Some("image/png"), Some("gif")), "image/png");
        assert_eq!(effective_type(Some("application/octet-stream"), Some("avif")), "image/avif");
        assert_eq!(effective_type(Some("Image/PNG; charset=x"), Some("")), "image/png");
        assert_eq!(effective_type(Some("application/octet-stream"), Some("gr2")), "application/octet-stream");
        assert_eq!(effective_type(None, None), "");
    }

    #[test]
    fn gives_the_reported_avif_its_preview_its_display_and_its_card_back() {
        let (m, e) = (Some("application/octet-stream"), Some("avif"));
        assert_eq!(media_kind(m, e), MediaKind::Image);
        assert!(is_displayable_image(m, e));
        assert!(is_embeddable(m, e));
        assert_eq!(serve_decision(m, e), ServeDecision { disposition: "inline", content_type: "image/avif".into() });
    }

    #[test]
    fn does_not_hand_markup_its_own_content_type_through_the_same_door() {
        for ext in ["svg", "html", "xhtml"] {
            assert_eq!(
                serve_decision(Some("application/octet-stream"), Some(ext)),
                ServeDecision { disposition: "inline", content_type: "text/plain; charset=utf-8".into() }
            );
        }
        assert!(!is_displayable_image(Some("application/octet-stream"), Some("svg")));
    }

    #[test]
    fn leaves_a_genuinely_unknown_binary_as_a_download() {
        assert_eq!(
            serve_decision(Some("application/octet-stream"), Some("gr2")),
            ServeDecision { disposition: "attachment", content_type: "application/octet-stream".into() }
        );
        assert_eq!(media_kind(Some("application/octet-stream"), Some("gr2")), MediaKind::Other);
    }

    #[test]
    fn players_and_icons() {
        assert_eq!(playable_video_type(Some("video/quicktime"), None), Some("video/mp4"));
        assert_eq!(playable_video_type(Some("video/x-matroska"), None), None);
        assert_eq!(playable_video_type(Some("application/octet-stream"), Some("mov")), Some("video/mp4"));
        assert_eq!(playable_audio_type(Some("audio/opus"), None), Some("audio/ogg"));
        assert_eq!(mime_to_icon(Some("application/x-apple-diskimage"), None), "File");
        assert_eq!(
            mime_to_icon(Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"), None),
            "FileText"
        );
        assert_eq!(mime_to_icon(Some("application/zip"), None), "FileArchive");
        assert!(is_text_payload(Some("application/json"), None));
        assert!(!is_text_payload(Some("video/mp2t"), None));
    }
}
