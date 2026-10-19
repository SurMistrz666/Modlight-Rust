//! Port of `lib/media/sniff.ts`: what kind of file is this, really?
//!
//! The stored `mimeType` used to be whatever the client claimed, which is the
//! weakest evidence available: on Windows both ShareX and the file picker read
//! the type out of `HKCR\.<ext>\Content Type`, and Windows does not register the
//! modern formats at all, so a perfectly ordinary .avif/.heic/.opus arrives as
//! `application/octet-stream`. A row with that type has no preview, no inline
//! display and no conversion, which is exactly the bug this module exists for.
//!
//! Three sources of truth, deliberately ranked (see [`resolve_upload_type`]):
//! signature, then extension, then the client's claim. No dependencies and no
//! I/O, so the upload paths, the migration script and the tests all share one
//! implementation.

// ---------------------------------------------------------------------------
// Extension -> MIME
// ---------------------------------------------------------------------------

/// Extension -> canonical MIME, including the formats Windows is most likely to
/// shrug at.
///
/// `application/octet-stream` entries are not omissions: they record a format
/// we recognise and for which "unknown binary" is the honest answer.
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

/// Normalise ".PNG" / "png" / "" to a bare lowercase key.
pub fn normalize_ext(ext: Option<&str>) -> String {
    let t = crate::config::js_trim(ext.unwrap_or("")).to_lowercase();
    t.strip_prefix('.').map(str::to_string).unwrap_or(t)
}

/// `/\.([a-z0-9]+)$/i` on a name: the extension without the dot, lower-cased,
/// "" when there is none.
pub fn ext_of_name(filename: Option<&str>) -> String {
    let name = filename.unwrap_or("");
    let tail: String = name.chars().rev().take_while(|c| c.is_ascii_alphanumeric()).collect();
    if tail.is_empty() {
        return String::new();
    }
    let tail_len: usize = tail.chars().map(char::len_utf8).sum();
    let before = &name[..name.len() - tail_len];
    if before.ends_with('.') { tail.chars().rev().collect::<String>().to_lowercase() } else { String::new() }
}

/// MIME for an extension, or `None` when we have nothing to say about it.
///
/// `None` rather than `application/octet-stream` on purpose: every caller wants
/// to distinguish "no opinion" from "definitely an unknown binary", and only the
/// caller knows which default belongs in its place.
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

/// Strip parameters and casing: `Image/PNG; charset=x` -> `image/png`.
pub fn normalize_mime(mime: Option<&str>) -> String {
    let m = mime.unwrap_or("");
    let first = m.split(';').next().unwrap_or("");
    crate::config::js_trim(first).to_lowercase()
}

/// Does this MIME carry any information at all?
pub fn is_useless_mime(mime: Option<&str>) -> bool {
    USELESS_MIMES.contains(&normalize_mime(mime).as_str())
}

// ---------------------------------------------------------------------------
// Byte signatures
// ---------------------------------------------------------------------------

struct Signature {
    mime: &'static str,
    /// Bytes to match, as a hex string.
    hex: &'static str,
    /// Offset the pattern starts at.
    at: usize,
}

const fn sig(mime: &'static str, hex: &'static str) -> Signature {
    Signature { mime, hex, at: 0 }
}

/// Ordered longest-evidence-first. Two-byte patterns (BM, MZ) sit at the bottom
/// because they collide with far more than they identify.
const SIGNATURES: &[Signature] = &[
    // images
    sig("image/png", "89504e470d0a1a0a"),
    sig("image/gif", "474946383761"),
    sig("image/gif", "474946383961"),
    sig("image/jpeg", "ffd8ff"),
    sig("image/tiff", "49492a00"),
    sig("image/tiff", "4d4d002a"),
    sig("image/vnd.adobe.photoshop", "38425053"),
    sig("image/jxl", "0000000c4a584c200d0a870a"),
    sig("image/jxl", "ff0a"),
    sig("image/x-icon", "00000100"),
    sig("image/svg+xml", "3c737667"), // "<svg"
    // documents / archives
    sig("application/pdf", "255044462d"),
    sig("application/zip", "504b0304"),
    sig("application/zip", "504b0506"),
    sig("application/zip", "504b0708"),
    sig("application/vnd.rar", "526172211a07"),
    sig("application/x-7z-compressed", "377abcaf271c"),
    sig("application/x-xz", "fd377a585a00"),
    sig("application/zstd", "28b52ffd"),
    sig("application/x-bzip2", "425a68"),
    sig("application/gzip", "1f8b"),
    Signature { mime: "application/x-tar", hex: "7573746172", at: 257 }, // "ustar"
    sig("application/vnd.sqlite3", "53514c69746520666f726d61742033"),
    sig("application/rtf", "7b5c727466"),
    sig("application/xml", "3c3f786d6c"), // "<?xml"
    // media containers
    sig("application/ogg", "4f676753"),
    sig("audio/flac", "664c6143"),
    sig("video/x-matroska", "1a45dfa3"),
    sig("audio/mpeg", "494433"), // ID3
    sig("audio/mpeg", "fffb"),
    sig("audio/mpeg", "fff3"),
    sig("audio/mpeg", "fff2"),
    sig("audio/aiff", "464f524d"),
    sig("audio/midi", "4d546864"),
    // fonts / code objects
    sig("font/woff2", "774f4632"),
    sig("font/woff", "774f4646"),
    sig("font/ttf", "0001000000"),
    sig("font/otf", "4f54544f"),
    sig("application/wasm", "0061736d"),
    sig("application/java-vm", "cafebabe"),
    // weak, last resort
    sig("application/vnd.microsoft.portable-executable", "4d5a"),
    sig("image/bmp", "424d"),
];

/// RIFF is one container for three unrelated formats; the tag at 8 decides.
fn riff_form(tag: &str) -> Option<&'static str> {
    match tag {
        "WEBP" => Some("image/webp"),
        "WAVE" => Some("audio/wav"),
        "AVI" => Some("video/x-msvideo"), // the tag is "AVI " on disk; `ascii()` trims it
        _ => None,
    }
}

/// ISO base media brands.
///
/// `specific` marks a brand that names the actual format ("this is AVIF"), as
/// opposed to one that only names the container family ("this is some MP4").
/// The distinction is what lets the extension refine `isom` into `audio/mp4`
/// for an .m4a while never letting it argue with `avis`.
///
/// The bug that started all this lives here: the feral upload's MAJOR brand is
/// `avis` (an AVIF image SEQUENCE), and `avif` appears only further down the
/// compatible-brands list. Comparing a fixed 12-byte prefix would have missed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Brand {
    mime: &'static str,
    specific: bool,
}

fn iso_brand(name: &str) -> Option<Brand> {
    let (mime, specific) = match name {
        "avif" | "avis" => ("image/avif", true),
        "heic" | "heix" | "hevc" | "heim" => ("image/heic", true),
        "mif1" | "msf1" => ("image/heic", false),
        "qt" => ("video/quicktime", true),
        "M4A" | "M4B" => ("audio/mp4", true),
        "M4V" => ("video/x-m4v", true),
        "M4P" => ("video/mp4", true),
        "isom" | "iso2" | "iso4" | "iso5" | "iso6" | "mp41" | "mp42" | "dash" => ("video/mp4", false),
        "3gp4" | "3gp5" => ("video/3gpp", true),
        "3g2a" => ("video/3gpp2", true),
        "crx" => ("image/x-canon-cr3", true),
        _ => return None,
    };
    Some(Brand { mime, specific })
}

/// Everything the ISO base media format can be, for the refinement rule.
const ISO_FAMILY: &[&str] = &[
    "video/mp4",
    "video/x-m4v",
    "video/quicktime",
    "video/3gpp",
    "video/3gpp2",
    "audio/mp4",
    "image/avif",
    "image/heic",
    "image/heif",
];

/// Formats that ride inside a zip and would otherwise all read as a zip.
const ZIP_FAMILY: &[&str] = &[
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "application/vnd.oasis.opendocument.text",
    "application/vnd.oasis.opendocument.spreadsheet",
    "application/vnd.oasis.opendocument.presentation",
    "application/epub+zip",
    "application/java-archive",
    "application/vnd.android.package-archive",
];

/// Text-ish types an extension may pin down once the bytes read as text.
fn is_textual(mime: &str) -> bool {
    mime.starts_with("text/")
        || mime.ends_with("+xml")
        || mime.ends_with("+json")
        || matches!(
            mime,
            "application/json"
                | "application/xml"
                | "application/sql"
                | "application/x-sh"
                | "application/jwt"
                | "application/x-subrip"
                | "image/svg+xml"
        )
}

/// May `by_ext` overrule `sniffed`?
///
/// Only ever downward, within one family: a signature identifies the container,
/// and for container formats the extension is the only thing that names the
/// payload. Without this every .docx would be stored as application/zip and
/// every .m4a as video/mp4.
fn refines(sniffed: &str, by_ext: &str) -> bool {
    if sniffed == by_ext {
        return false;
    }
    match sniffed {
        // A generic ISO brand (isom/mp42/mif1) names the container and nothing
        // else, so .m4a is the only thing that distinguishes an audio file from
        // a video one. The specific brands never reach this point.
        "video/mp4" | "image/heic" => ISO_FAMILY.contains(&by_ext),
        "application/zip" => ZIP_FAMILY.contains(&by_ext),
        "application/ogg" => matches!(by_ext, "video/ogg" | "audio/ogg" | "audio/opus"),
        "video/x-matroska" => matches!(by_ext, "video/webm" | "audio/webm" | "audio/x-matroska"),
        "image/png" => by_ext == "image/apng",
        "application/xml" | "text/plain" => is_textual(by_ext),
        // The TrueType signature is also what a bare OpenType/TTC file starts with.
        "font/ttf" => matches!(by_ext, "font/otf" | "font/collection"),
        _ => false,
    }
}

fn hex_byte(hex: &str, i: usize) -> u8 {
    u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(0)
}

fn matches_sig(head: &[u8], sig: &Signature) -> bool {
    let bytes = sig.hex.len() / 2;
    if head.len() < sig.at + bytes {
        return false;
    }
    (0..bytes).all(|i| head[sig.at + i] == hex_byte(sig.hex, i))
}

/// A 32-bit big-endian value, or `None` when the buffer is too short.
fn u32_be(head: &[u8], at: usize) -> Option<u32> {
    head.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// `head.toString("latin1", from, to).replace(/\0/g, "").trim()`, or "" when
/// the buffer is too short.
fn ascii(head: &[u8], from: usize, to: usize) -> String {
    if head.len() < to {
        return String::new();
    }
    let s: String = head[from..to].iter().filter(|b| **b != 0).map(|b| *b as char).collect();
    crate::config::js_trim(&s).to_string()
}

/// Type of an ISO base media file, from its `ftyp` box.
///
/// The major brand is tried first and the compatible-brands list is the
/// fallback, because that is the order the specification assigns authority in -
/// and because a specific brand anywhere in the list beats a generic one at the
/// front.
fn sniff_iso_bmff(head: &[u8]) -> Option<Brand> {
    if ascii(head, 4, 8) != "ftyp" {
        return None;
    }
    let major = iso_brand(&ascii(head, 8, 12));
    if let Some(m) = major
        && m.specific
    {
        return Some(m);
    }
    // Compatible brands run from offset 16 to the end of the ftyp box.
    let box_size = u32_be(head, 0).unwrap_or(0) as usize;
    let end = (if box_size > 16 { box_size } else { head.len() }).min(head.len());
    let mut at = 16;
    while at + 4 <= end {
        if let Some(b) = iso_brand(&ascii(head, at, at + 4))
            && b.specific
        {
            return Some(b);
        }
        at += 4;
    }
    Some(major.unwrap_or(Brand { mime: "video/mp4", specific: false }))
}

/// Do the first bytes read as text?
///
/// Used only as a last resort, and deliberately strict: valid UTF-8 with no
/// control characters beyond tab/newline/carriage return. That is what rescues
/// the log file sitting in production as an unopenable binary download.
fn looks_like_text(head: &[u8]) -> bool {
    if head.is_empty() {
        return false;
    }
    // A truncated read can cut a multi-byte character in half, which is not
    // evidence of anything - drop up to 3 trailing continuation bytes.
    let mut end = head.len();
    for _ in 0..3 {
        if end == 0 {
            break;
        }
        let b = head[end - 1];
        if b < 0x80 {
            break;
        }
        end -= 1;
        if b & 0xc0 == 0xc0 {
            break; // dropped the lead byte itself
        }
    }
    let slice = &head[..end];
    if slice.is_empty() {
        return false;
    }
    if std::str::from_utf8(slice).is_err() {
        return false;
    }
    slice.iter().all(|&b| matches!(b, 0x09 | 0x0a | 0x0d) || (b >= 0x20 && b != 0x7f))
}

/// The type the bytes themselves claim, or `None` when they say nothing.
///
/// `head` only needs the first few hundred bytes; 4 KiB covers every pattern
/// here (the tar magic sits at offset 257) and keeps the read cheap enough to do
/// on a multi-gigabyte upload.
pub fn sniff_mime(head: Option<&[u8]>) -> Option<&'static str> {
    let head = head.filter(|h| !h.is_empty())?;
    if let Some(iso) = sniff_iso_bmff(head) {
        return Some(iso.mime);
    }
    if matches_sig(head, &sig("", "52494646"))
        && let Some(form) = riff_form(&ascii(head, 8, 12))
    {
        return Some(form);
    }
    if let Some(s) = SIGNATURES.iter().find(|s| matches_sig(head, s)) {
        return Some(s.mime);
    }
    if looks_like_text(head) { Some("text/plain") } else { None }
}

// ---------------------------------------------------------------------------
// The decision
// ---------------------------------------------------------------------------

/// `UploadTypeInput`
#[derive(Debug, Clone, Copy, Default)]
pub struct UploadTypeInput<'a> {
    /// Content-Type the client sent (multipart part header, or tus metadata).
    pub client_mime: Option<&'a str>,
    /// Original filename, used for its extension. Ignored when `ext` is given.
    pub filename: Option<&'a str>,
    /// Extension without the dot, when the caller already has it.
    pub ext: Option<&'a str>,
    /// First bytes of the payload (4 KiB is plenty).
    pub head: Option<&'a [u8]>,
}

/// The type to STORE for an upload.
///
/// Ranked by how hard each source is to be wrong about:
///
///  1. the signature, because bytes are the one thing the sender does not get
///     to narrate - except where the signature only identifies a container, in
///     which case
///  2. the extension refines it downward within the same family (zip -> docx,
///     ftyp -> m4a), and also stands alone when the bytes said nothing;
///  3. the client's Content-Type, last and only when it is specific, because on
///     Windows it is a registry lookup that simply has no entry for anything
///     released this decade - which is how a .avif became an unopenable binary;
///  4. `application/octet-stream` as an honest "no idea".
///
/// This never decides how a file is SERVED. `serveDecision()` stays an
/// allowlist, so recognising an upload as text/html or image/svg+xml still
/// results in `text/plain` on the wire.
pub fn resolve_upload_type(input: UploadTypeInput<'_>) -> String {
    // `input.ext ?? extOfName(input.filename)`: an explicit "" wins.
    let ext = match input.ext {
        Some(e) => normalize_ext(Some(e)),
        None => normalize_ext(Some(&ext_of_name(input.filename))),
    };
    let by_ext = mime_from_ext(Some(&ext));
    let useful_ext = by_ext.filter(|m| !is_useless_mime(Some(m)));
    let sniffed = sniff_mime(input.head);

    if let Some(sniffed) = sniffed {
        if let Some(u) = useful_ext
            && refines(sniffed, u)
        {
            return u.to_string();
        }
        return sniffed.to_string();
    }
    if let Some(u) = useful_ext {
        return u.to_string();
    }
    let client = normalize_mime(input.client_mime);
    if !client.is_empty() && !is_useless_mime(Some(&client)) {
        return client;
    }
    "application/octet-stream".to_string()
}

/// Is `candidate` a better answer than `current` for an already-stored row?
///
/// The migration script's guard rail: a backfill may sharpen a type, never
/// blunt one.
pub fn is_sharper_type(current: Option<&str>, candidate: &str) -> bool {
    let now = normalize_mime(current);
    let next = normalize_mime(Some(candidate));
    if next.is_empty() || next == now {
        return false;
    }
    if is_useless_mime(Some(&next)) {
        return false;
    }
    is_useless_mime(Some(&now))
}

#[cfg(test)]
mod tests {
    //! tests/unit/sniff.test.ts
    use super::*;

    /// An ISO base media header: [size]["ftyp"][major][minor][compatible...],
    /// zero-padded to at least 32 bytes like the upstream helper.
    fn ftyp(major: &str, compatible: &[&str]) -> Vec<u8> {
        let size = 16 + compatible.len() * 4;
        let mut b = vec![0u8; size.max(32)];
        b[0..4].copy_from_slice(&(size as u32).to_be_bytes());
        b[4..8].copy_from_slice(b"ftyp");
        b[8..12].copy_from_slice(format!("{major:<4}").as_bytes());
        for (i, c) in compatible.iter().enumerate() {
            b[16 + i * 4..20 + i * 4].copy_from_slice(format!("{c:<4}").as_bytes());
        }
        b
    }

    fn sniff(b: &[u8]) -> Option<&'static str> {
        sniff_mime(Some(b))
    }

    fn resolve(client: Option<&str>, filename: Option<&str>, head: Option<&[u8]>) -> String {
        resolve_upload_type(UploadTypeInput { client_mime: client, filename, ext: None, head })
    }

    #[test]
    fn reads_an_avif_image_sequence_whose_major_brand_is_not_avif() {
        let head = ftyp("avis", &["avis", "avif", "msf1", "iso8", "mif1", "miaf", "MA1B"]);
        assert_eq!(sniff(&head), Some("image/avif"));
    }

    #[test]
    fn reads_a_still_avif_a_heic_and_a_quicktime_out_of_the_same_box() {
        assert_eq!(sniff(&ftyp("avif", &["avif", "mif1"])), Some("image/avif"));
        assert_eq!(sniff(&ftyp("heic", &["mif1", "heic"])), Some("image/heic"));
        assert_eq!(sniff(&ftyp("qt", &["qt"])), Some("video/quicktime"));
        assert_eq!(sniff(&ftyp("M4A", &["M4A", "isom"])), Some("audio/mp4"));
    }

    #[test]
    fn falls_back_to_the_container_when_no_brand_is_specific() {
        assert_eq!(sniff(&ftyp("isom", &["isom", "avc1", "mp42"])), Some("video/mp4"));
    }

    #[test]
    fn prefers_a_specific_compatible_brand_over_a_generic_major_one() {
        assert_eq!(sniff(&ftyp("mif1", &["mif1", "avif"])), Some("image/avif"));
    }

    #[test]
    fn recognises_the_common_signatures() {
        assert_eq!(sniff(&[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), Some("image/png"));
        assert_eq!(sniff(b"GIF89a"), Some("image/gif"));
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), Some("image/jpeg"));
        assert_eq!(sniff(b"%PDF-1.7"), Some("application/pdf"));
        assert_eq!(sniff(&[0x50, 0x4b, 0x03, 0x04]), Some("application/zip"));
        assert_eq!(sniff(&[0x1a, 0x45, 0xdf, 0xa3]), Some("video/x-matroska"));
        assert_eq!(sniff(b"OggS"), Some("application/ogg"));
    }

    #[test]
    fn splits_riff_by_its_form_tag() {
        let riff = |form: &[u8; 4]| {
            let mut b = vec![0u8; 16];
            b[0..4].copy_from_slice(b"RIFF");
            b[8..12].copy_from_slice(form);
            b
        };
        assert_eq!(sniff(&riff(b"WEBP")), Some("image/webp"));
        assert_eq!(sniff(&riff(b"WAVE")), Some("audio/wav"));
        assert_eq!(sniff(&riff(b"AVI ")), Some("video/x-msvideo"));
    }

    #[test]
    fn recognises_plain_text_as_a_last_resort() {
        let log = b"[2025-09-28 15:52:42.808] [client] [error] invalid idx 0\n";
        assert_eq!(sniff(log), Some("text/plain"));
        assert_eq!(sniff("zażółć gęślą jaźń".as_bytes()), Some("text/plain"));
    }

    #[test]
    fn does_not_mistake_binary_for_text() {
        assert_eq!(sniff(&[0x29, 0xde, 0x6c, 0xc0, 0xba, 0xa4, 0x53, 0x2b]), None);
        assert_eq!(sniff(&[]), None);
    }

    #[test]
    fn survives_a_multi_byte_character_cut_in_half_by_the_read_window() {
        let full = "ąęśćżźół".as_bytes();
        assert_eq!(sniff(&full[..full.len() - 1]), Some("text/plain"));
    }

    #[test]
    fn rescues_the_reported_file() {
        let head = ftyp("avis", &["avis", "avif", "msf1"]);
        assert_eq!(resolve(Some("application/octet-stream"), Some("convert.avif"), Some(&head)), "image/avif");
    }

    #[test]
    fn works_from_the_name_alone_when_the_bytes_are_unavailable() {
        assert_eq!(resolve(Some("application/octet-stream"), Some("a.avif"), None), "image/avif");
    }

    #[test]
    fn believes_the_bytes_over_both_the_name_and_the_client() {
        let png = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
        assert_eq!(resolve(Some("image/jpeg"), Some("screenshot.jpg"), Some(&png)), "image/png");
    }

    #[test]
    fn lets_the_extension_name_the_payload_inside_a_container() {
        let zip = [0x50, 0x4b, 0x03, 0x04];
        assert_eq!(
            resolve(None, Some("report.docx"), Some(&zip)),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        );
        assert_eq!(resolve(None, Some("app.jar"), Some(&zip)), "application/java-archive");
        assert_eq!(resolve(None, Some("backup.zip"), Some(&zip)), "application/zip");
        assert_eq!(resolve(None, Some("nameless"), Some(&zip)), "application/zip");
    }

    #[test]
    fn refines_text_and_iso_containers_the_same_way() {
        let text = br#"{"Version":"16.1.0"}"#;
        assert_eq!(resolve(None, Some("modlight.sxcu"), Some(text)), "application/json");
        assert_eq!(resolve(None, Some("notes.md"), Some(text)), "text/markdown");
        assert_eq!(resolve(None, Some("song.m4a"), Some(&ftyp("isom", &["isom", "mp42"]))), "audio/mp4");
    }

    #[test]
    fn never_lets_the_extension_argue_with_a_specific_signature() {
        assert_eq!(resolve(None, Some("clip.mp4"), Some(&ftyp("avis", &["avis", "avif"]))), "image/avif");
    }

    #[test]
    fn uses_the_clients_type_only_when_it_is_specific_and_nothing_else_knows() {
        let garbage = [0x29, 0xde, 0x6c, 0xc0, 0xba, 0xa4, 0x53, 0x2b];
        assert_eq!(resolve(Some("application/x-thing"), Some("a.qqq"), Some(&garbage)), "application/x-thing");
        assert_eq!(
            resolve(Some("application/octet-stream"), Some("a.qqq"), Some(&garbage)),
            "application/octet-stream"
        );
    }

    #[test]
    fn keeps_octet_stream_when_that_is_the_honest_answer() {
        let granny = [0x29, 0xde, 0x6c, 0xc0, 0xba, 0xa4, 0x53, 0x2b];
        assert_eq!(resolve(None, Some("mesh.gr2"), Some(&granny)), "application/octet-stream");
    }

    #[test]
    fn mime_from_ext_says_nothing_rather_than_guessing_and_normalises() {
        assert_eq!(mime_from_ext(Some("qqq")), None);
        assert_eq!(mime_from_ext(Some("")), None);
        assert_eq!(mime_from_ext(None), None);
        assert_eq!(mime_from_ext(Some("PNG")), Some("image/png"));
        assert_eq!(mime_from_ext(Some(".Mp4")), Some("video/mp4"));
    }

    #[test]
    fn knows_which_types_carry_no_information_and_only_sharpens() {
        assert!(is_useless_mime(Some("application/octet-stream")));
        assert!(is_useless_mime(Some("binary/octet-stream")));
        assert!(is_useless_mime(Some("")));
        assert!(!is_useless_mime(Some("image/png")));
        assert!(is_sharper_type(Some("application/octet-stream"), "image/avif"));
        assert!(!is_sharper_type(Some("image/avif"), "application/octet-stream"));
        assert!(!is_sharper_type(Some("image/png"), "image/jpeg"));
        assert!(!is_sharper_type(Some("image/png"), "image/png"));
    }

    #[test]
    fn extension_helpers() {
        assert_eq!(normalize_ext(Some(" .TAR ")), "tar");
        assert_eq!(ext_of_name(Some("Archive.TAR.GZ")), "gz");
        assert_eq!(ext_of_name(Some("noext")), "");
        assert_eq!(ext_of_name(Some("a.b-c")), "");
        assert_eq!(ext_of_name(Some(".gitignore")), "gitignore");
    }
}
