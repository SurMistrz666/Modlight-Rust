//! Port of `lib/media/mime.ts`, NARROW: `effectiveType` and `mediaKind` (the
//! file-kind rule the admin disk breakdown folds by). Serving decisions,
//! playable types and icons belong to the library port, which will
//! reconcile this file with its own.

use serde::Serialize;

use super::sniff::{is_useless_mime, mime_from_ext};
use crate::config::js_trim;

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

/// Strip parameters and casing: `Text/HTML; charset=UTF-8` -> `text/html`.
fn normalize_mime(mime: &str) -> String {
    js_trim(mime.split(';').next().unwrap_or("")).to_lowercase()
}

/// The type to REASON with, given what we stored and what the file is called.
///
/// A stored type is only evidence when it says something. ShareX and curl
/// send `application/octet-stream` for anything Windows has no registry entry
/// for, and an imported row keeps whatever its old host guessed years ago -
/// so for those rows the name on disk is the better clue, and without
/// consulting it a perfectly ordinary .avif has no preview, no inline display
/// and no conversion.
///
/// It changes what we THINK a file is, never what we SERVE it as.
pub fn effective_type(mime: Option<&str>, ext: Option<&str>) -> String {
    let m = normalize_mime(mime.unwrap_or(""));
    if !m.is_empty() && !is_useless_mime(Some(&m)) {
        return m;
    }
    match mime_from_ext(ext) {
        Some(from_ext) => from_ext.to_string(),
        None => m,
    }
}

/// `MediaKind`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
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

/// `mediaKind(mime, ext)`
pub fn media_kind(mime: Option<&str>, ext: Option<&str>) -> MediaKind {
    let m = effective_type(mime, ext);
    if m.is_empty() {
        return MediaKind::Other;
    }
    let top = m.split('/').next().unwrap_or("");
    match top {
        "image" if DISPLAYABLE_IMAGES.contains(&m.as_str()) => MediaKind::Image,
        "image" => MediaKind::Other,
        "video" => MediaKind::Video,
        "audio" => MediaKind::Audio,
        "text" => MediaKind::Text,
        _ if m == "application/pdf" => MediaKind::Pdf,
        // /zip|gzip|tar|rar|7z-compressed/
        _ if ["zip", "gzip", "tar", "rar", "7z-compressed"].iter().any(|needle| m.contains(needle)) => {
            MediaKind::Archive
        }
        _ => MediaKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // tests/unit/mime.test.ts: "mime helpers" > "mediaKind"
    #[test]
    fn media_kinds() {
        assert_eq!(media_kind(Some("image/png"), None), MediaKind::Image);
        assert_eq!(media_kind(Some("video/mp4"), None), MediaKind::Video);
        assert_eq!(media_kind(Some("text/plain"), None), MediaKind::Text);
        assert_eq!(media_kind(Some("application/pdf"), None), MediaKind::Pdf);
        assert_eq!(media_kind(Some("application/zip"), None), MediaKind::Archive);
        assert_eq!(media_kind(Some("image/svg+xml"), None), MediaKind::Other);
        assert_eq!(media_kind(Some("application/octet-stream"), None), MediaKind::Other);
        // The archive rule is a substring test, as upstream's regex.
        assert_eq!(media_kind(Some("application/x-gtar"), None), MediaKind::Archive);
        assert_eq!(media_kind(Some("application/epub+zip"), None), MediaKind::Archive);
        assert_eq!(media_kind(Some("audio/mpeg"), None), MediaKind::Audio);
        assert_eq!(media_kind(None, None), MediaKind::Other);
    }

    // tests/unit/mime.test.ts: "a stored type that says nothing falls back to the name"
    #[test]
    fn consults_the_extension_only_when_the_stored_type_is_useless() {
        assert_eq!(effective_type(Some("image/png"), Some("gif")), "image/png"); // stored type wins
        assert_eq!(effective_type(Some("application/octet-stream"), Some("avif")), "image/avif");
        assert_eq!(effective_type(Some("Image/PNG; charset=x"), Some("")), "image/png");
        assert_eq!(effective_type(Some("application/octet-stream"), Some("gr2")), "application/octet-stream");
        assert_eq!(effective_type(None, None), "");
        // A useless type with no extension stays as normalised.
        assert_eq!(effective_type(Some(" Binary/Octet-Stream "), None), "binary/octet-stream");
    }

    #[test]
    fn gives_the_reported_avif_its_card_back() {
        assert_eq!(media_kind(Some("application/octet-stream"), Some("avif")), MediaKind::Image);
    }

    #[test]
    fn leaves_a_genuinely_unknown_binary_as_other() {
        assert_eq!(media_kind(Some("application/octet-stream"), Some("gr2")), MediaKind::Other);
    }

    #[test]
    fn kinds_serialize_lowercase() {
        assert_eq!(serde_json::to_value(MediaKind::Pdf).unwrap(), "pdf");
        assert_eq!(MediaKind::Archive.as_str(), "archive");
    }
}
