//! Port of `lib/media/sniff.ts`, NARROW: only the extension table and the
//! type normalisation `effectiveType` (lib/media/mime.ts) is built on
//! (`MIME_BY_EXT`, `normalizeExt`, `mimeFromExt`, `normalizeMime`,
//! `isUselessMime`). The byte signatures and `resolveUploadType` belong to
//! the library port, which will reconcile this file with its own.
//!
//! What kind of file is this, really? The stored `mimeType` used to be
//! whatever the client claimed, which is the weakest evidence available: on
//! Windows both ShareX and the file picker read the type out of
//! `HKCR\.<ext>\Content Type`, and Windows does not register the modern
//! formats at all, so a perfectly ordinary .avif/.heic/.opus arrives as
//! `application/octet-stream`.

use crate::config::js_trim;

/// Extension -> canonical MIME, including the formats Windows is most likely
/// to shrug at.
///
/// `application/octet-stream` entries are not omissions: they record a format
/// we recognise and for which "unknown binary" is the honest answer.
///
/// Upstream this is a plain object, so a lookup of an inherited property name
/// (`MIME_BY_EXT["constructor"]`) yields a function instead of `undefined`;
/// after `normalizeExt` lower-cases the key, `constructor` is the only such
/// name an extension can spell. Here it is simply absent (no entry).
pub const MIME_BY_EXT: &[(&str, &str)] = &[
    // images
    ("png", "image/png"),
    ("apng", "image/apng"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("jfif", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("avif", "image/avif"),
    ("avifs", "image/avif"),
    ("jxl", "image/jxl"),
    ("bmp", "image/bmp"),
    ("ico", "image/x-icon"),
    ("cur", "image/x-icon"),
    ("tif", "image/tiff"),
    ("tiff", "image/tiff"),
    ("svg", "image/svg+xml"),
    ("heic", "image/heic"),
    ("heif", "image/heif"),
    ("psd", "image/vnd.adobe.photoshop"),
    // video
    ("mp4", "video/mp4"),
    ("m4v", "video/x-m4v"),
    ("mov", "video/quicktime"),
    ("qt", "video/quicktime"),
    ("webm", "video/webm"),
    ("ogv", "video/ogg"),
    ("mkv", "video/x-matroska"),
    ("avi", "video/x-msvideo"),
    ("wmv", "video/x-ms-wmv"),
    ("flv", "video/x-flv"),
    ("mpg", "video/mpeg"),
    ("mpeg", "video/mpeg"),
    ("ts", "video/mp2t"),
    ("3gp", "video/3gpp"),
    ("3g2", "video/3gpp2"),
    // audio
    ("mp3", "audio/mpeg"),
    ("ogg", "audio/ogg"),
    ("oga", "audio/ogg"),
    ("opus", "audio/opus"),
    ("wav", "audio/wav"),
    ("flac", "audio/flac"),
    ("m4a", "audio/mp4"),
    ("mka", "audio/x-matroska"),
    ("aac", "audio/aac"),
    ("aif", "audio/aiff"),
    ("aiff", "audio/aiff"),
    ("wma", "audio/x-ms-wma"),
    ("mid", "audio/midi"),
    ("midi", "audio/midi"),
    ("aup3", "audio/x-audacity-project"),
    // text / code
    ("txt", "text/plain"),
    ("md", "text/markdown"),
    ("markdown", "text/markdown"),
    ("csv", "text/csv"),
    ("tsv", "text/tab-separated-values"),
    ("log", "text/plain"),
    ("ini", "text/plain"),
    ("cfg", "text/plain"),
    ("conf", "text/plain"),
    ("html", "text/html"),
    ("htm", "text/html"),
    ("xhtml", "application/xhtml+xml"),
    ("css", "text/css"),
    ("js", "text/javascript"),
    ("mjs", "text/javascript"),
    ("cjs", "text/javascript"),
    ("jsx", "text/plain"),
    ("tsx", "text/plain"),
    ("lua", "text/x-lua"),
    ("py", "text/x-python"),
    ("rb", "text/x-ruby"),
    ("sh", "application/x-sh"),
    ("bat", "text/plain"),
    ("ps1", "text/plain"),
    ("c", "text/x-c"),
    ("h", "text/x-c"),
    ("cpp", "text/x-c"),
    ("cs", "text/plain"),
    ("java", "text/x-java-source"),
    ("go", "text/plain"),
    ("rs", "text/plain"),
    ("php", "text/x-php"),
    ("sql", "application/sql"),
    ("json", "application/json"),
    ("jsonc", "application/json"),
    ("jwt", "application/jwt"),
    ("xml", "application/xml"),
    ("yml", "text/plain"),
    ("yaml", "text/plain"),
    ("toml", "text/plain"),
    ("srt", "application/x-subrip"),
    ("vtt", "text/vtt"),
    ("ics", "text/calendar"),
    ("sxcu", "application/json"), // ShareX custom-uploader config
    // documents
    ("pdf", "application/pdf"),
    ("rtf", "application/rtf"),
    ("doc", "application/msword"),
    ("docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
    ("xls", "application/vnd.ms-excel"),
    ("xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
    ("ppt", "application/vnd.ms-powerpoint"),
    ("pptx", "application/vnd.openxmlformats-officedocument.presentationml.presentation"),
    ("odt", "application/vnd.oasis.opendocument.text"),
    ("ods", "application/vnd.oasis.opendocument.spreadsheet"),
    ("odp", "application/vnd.oasis.opendocument.presentation"),
    ("epub", "application/epub+zip"),
    // archives
    ("zip", "application/zip"),
    ("rar", "application/vnd.rar"),
    ("7z", "application/x-7z-compressed"),
    ("gz", "application/gzip"),
    ("bz2", "application/x-bzip2"),
    ("xz", "application/x-xz"),
    ("zst", "application/zstd"),
    ("tar", "application/x-tar"),
    ("tgz", "application/gzip"),
    // executables / installers / fonts
    ("exe", "application/vnd.microsoft.portable-executable"),
    ("msi", "application/x-msi"),
    ("apk", "application/vnd.android.package-archive"),
    ("jar", "application/java-archive"),
    ("wasm", "application/wasm"),
    ("ttf", "font/ttf"),
    ("otf", "font/otf"),
    ("woff", "font/woff"),
    ("woff2", "font/woff2"),
    // 3D / game assets
    ("3mf", "model/3mf"),
    ("stl", "model/stl"),
    ("obj", "model/obj"),
    ("gr2", "application/octet-stream"), // Granny2 mesh - no registered type
];

/// `normalizeExt`: ".PNG" / "png" / "" to a bare lowercase key.
pub fn normalize_ext(ext: Option<&str>) -> String {
    let lowered = js_trim(ext.unwrap_or("")).to_lowercase();
    match lowered.strip_prefix('.') {
        Some(rest) => rest.to_string(),
        None => lowered,
    }
}

/// `mimeFromExt`: MIME for an extension, or `None` when we have nothing to
/// say about it.
///
/// `None` rather than `application/octet-stream` on purpose: every caller
/// wants to distinguish "no opinion" from "definitely an unknown binary", and
/// only the caller knows which default belongs in its place.
pub fn mime_from_ext(ext: Option<&str>) -> Option<&'static str> {
    let key = normalize_ext(ext);
    MIME_BY_EXT.iter().find(|(e, _)| *e == key).map(|(_, m)| *m)
}

/// Types that mean "I did not look": treat them as absent evidence.
const USELESS_MIMES: &[&str] = &[
    "",
    "*/*",
    "application/octet-stream",
    "binary/octet-stream",
    "application/octetstream",
    "application/x-download",
    "application/download",
    "application/force-download",
    "application/unknown",
    "unknown/unknown",
];

/// `normalizeMime`: strip parameters and casing, `Image/PNG; charset=x` ->
/// `image/png`.
pub fn normalize_mime(mime: Option<&str>) -> String {
    js_trim(mime.unwrap_or("").split(';').next().unwrap_or("")).to_lowercase()
}

/// `isUselessMime`: does this MIME carry any information at all?
pub fn is_useless_mime(mime: Option<&str>) -> bool {
    USELESS_MIMES.contains(&normalize_mime(mime).as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    // tests/unit/sniff.test.ts: "mimeFromExt"
    #[test]
    fn mime_from_ext_says_nothing_rather_than_guessing() {
        assert_eq!(mime_from_ext(Some("qqq")), None);
        assert_eq!(mime_from_ext(Some("")), None);
        assert_eq!(mime_from_ext(None), None);
    }

    #[test]
    fn mime_from_ext_normalises_the_key() {
        assert_eq!(mime_from_ext(Some("PNG")), Some("image/png"));
        assert_eq!(mime_from_ext(Some(".Mp4")), Some("video/mp4"));
        assert_eq!(mime_from_ext(Some("  .avif ")), Some("image/avif"));
        // An inherited property name of the upstream object is not an entry.
        assert_eq!(mime_from_ext(Some("constructor")), None);
    }

    // tests/unit/sniff.test.ts: "isUselessMime / isSharperType" (the first half)
    #[test]
    fn knows_which_types_carry_no_information() {
        assert!(is_useless_mime(Some("application/octet-stream")));
        assert!(is_useless_mime(Some("binary/octet-stream")));
        assert!(is_useless_mime(Some("")));
        assert!(is_useless_mime(None));
        assert!(is_useless_mime(Some(" Application/Octet-Stream; x=1")));
        assert!(!is_useless_mime(Some("image/png")));
    }

    #[test]
    fn normalisers() {
        assert_eq!(normalize_mime(Some("Image/PNG; charset=x")), "image/png");
        assert_eq!(normalize_mime(None), "");
        assert_eq!(normalize_ext(Some(".PNG")), "png");
        assert_eq!(normalize_ext(Some("..png")), ".png");
        assert_eq!(normalize_ext(None), "");
    }

    #[test]
    fn the_table_is_upstream_size_and_has_unique_keys() {
        assert_eq!(MIME_BY_EXT.len(), 127);
        let mut keys: Vec<&str> = MIME_BY_EXT.iter().map(|(k, _)| *k).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), MIME_BY_EXT.len());
    }
}
