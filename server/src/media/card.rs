//! Port of `lib/media/card.ts`: which type card a file gets in a link preview,
//! and where that card lives.
//!
//! Pure on purpose (no database, no storage, no libvips): the mapping is the
//! part worth pinning in a test, and keeping it here is also what makes the card
//! route unable to touch a file row at all. A card is addressed by (category,
//! token), never by a file code, so it cannot leak whether a code exists, cannot
//! spend a burn-after-view budget, and one cache entry is shared by every ZIP on
//! the instance rather than minted per upload.
//!
//! (Upstream reads `EXT_CATEGORY` as a plain object, so an extension spelled
//! like an `Object.prototype` member, `constructor`, found a function there and
//! advertised a nonsense card URL. Only the listed extensions match here.)

use crate::media::mime::{MediaKind, is_displayable_image, media_kind};

/// Version segment, carried in the PATH rather than a query string.
///
/// Discord copies an og:image onto media.discordapp.net and never revalidates,
/// so a URL's bytes must be frozen forever. Redrawing the art means minting new
/// URLs; old messages keep the old picture, which is the honest outcome.
pub const EMBED_CARD_VERSION: &str = "v1";

/// Open Graph's canonical card size. Nothing crops at exactly 1.91:1.
pub const CARD_WIDTH: u32 = 1200;
pub const CARD_HEIGHT: u32 = 630;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CardCategory {
    Code,
    Archive,
    Document,
    Sheet,
    Audio,
    Text,
    Image,
    Binary,
}

pub const CARD_CATEGORIES: [CardCategory; 8] = [
    CardCategory::Code,
    CardCategory::Archive,
    CardCategory::Document,
    CardCategory::Sheet,
    CardCategory::Audio,
    CardCategory::Text,
    CardCategory::Image,
    CardCategory::Binary,
];

impl CardCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            CardCategory::Code => "code",
            CardCategory::Archive => "archive",
            CardCategory::Document => "document",
            CardCategory::Sheet => "sheet",
            CardCategory::Audio => "audio",
            CardCategory::Text => "text",
            CardCategory::Image => "image",
            CardCategory::Binary => "binary",
        }
    }

    /// `isCardCategory(value)`
    pub fn parse(value: &str) -> Option<CardCategory> {
        CARD_CATEGORIES.into_iter().find(|c| c.as_str() == value)
    }

    /// Word shown when the extension is missing or unprintable.
    ///
    /// Every one of these is inside [A-Z] by construction, which is the whole
    /// mechanism: the card's alphabet is closed, so the font cannot be asked for a
    /// glyph it lacks and tofu is impossible however the file was named.
    pub fn fallback_token(self) -> &'static str {
        match self {
            CardCategory::Code => "CODE",
            CardCategory::Archive => "ARCHIVE",
            CardCategory::Document => "DOCUMENT",
            CardCategory::Sheet => "SHEET",
            CardCategory::Audio => "AUDIO",
            CardCategory::Text => "TEXT",
            CardCategory::Image => "IMAGE",
            CardCategory::Binary => "FILE",
        }
    }
}

/// Longest token the card lays out. "DOCUMENT" is the longest word we emit.
pub const MAX_TOKEN_LENGTH: usize = 8;

/// `isCardToken(value)`: `/^[A-Z0-9]{1,8}$/`
pub fn is_card_token(value: &str) -> bool {
    (1..=MAX_TOKEN_LENGTH).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// Extension to category. Ordered by how often a file of that kind is shared,
/// not alphabetically, because the tail of each list is where a wrong guess is
/// cheapest.
const CLAIMS: [(CardCategory, &str); 8] = [
    (
        CardCategory::Code,
        "html htm css scss less js mjs cjs jsx ts tsx php py rb go rs java kt swift
         c h cpp hpp cs sh bash zsh ps1 bat sql json xml yml yaml toml ini env
         vue svelte lua pl r dart ex exs astro graphql proto tf",
    ),
    (
        CardCategory::Archive,
        "zip rar 7z tar gz tgz bz2 xz zst lz lzma arj cab iso dmg jar war apk xapk
         pkg deb rpm",
    ),
    (CardCategory::Document, "pdf doc docx odt rtf tex epub mobi pages ppt pptx odp key"),
    (CardCategory::Sheet, "xls xlsx xlsm csv tsv ods numbers"),
    (CardCategory::Audio, "mp3 wav flac ogg oga opus m4a aac aiff wma mid midi amr"),
    (CardCategory::Text, "txt md markdown log nfo srt vtt rst cfg conf diff patch"),
    // Images the browser will NOT render inline, so they never get a real
    // thumbnail and today get no picture at all.
    (CardCategory::Image, "svg psd ai eps heic heif xcf raw cr2 cr3 nef arw dng orf rw2"),
    (CardCategory::Binary, "exe msi dll so dylib bin dat o a wasm class pyc ttf otf woff woff2"),
];

/// `EXT_CATEGORY[ext]`: a later claim of the same extension wins, as the
/// object assignment upstream does (no extension is claimed twice today).
fn ext_category(ext: &str) -> Option<CardCategory> {
    CLAIMS.iter().rev().find(|(_, exts)| exts.split_whitespace().any(|e| e == ext)).map(|(category, _)| *category)
}

/// `CardSubject`
#[derive(Debug, Clone, Copy)]
pub struct CardSubject<'a> {
    pub ext: &'a str,
    pub mime_type: &'a str,
    /// uploads.kind: "file" | "paste".
    pub kind: &'a str,
}

/// `CardChoice`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardChoice {
    pub category: CardCategory,
    /// Short label drawn under the glyph, always [A-Z0-9]{1,8}.
    pub token: String,
}

/// `cardChoiceFor(media)`: the card a file should advertise, or None when it
/// must keep advertising itself (displayable images and video unfurl as the
/// real picture or a real player).
pub fn card_choice_for(media: CardSubject<'_>) -> Option<CardChoice> {
    if is_displayable_image(Some(media.mime_type), Some(media.ext)) {
        return None;
    }
    let kind = media_kind(Some(media.mime_type), Some(media.ext));
    if kind == MediaKind::Video {
        return None;
    }
    let category = category_for(media, kind);
    Some(CardChoice { category, token: token_for(media.ext, category) })
}

fn category_for(media: CardSubject<'_>, kind: MediaKind) -> CardCategory {
    // An in-app paste is text whatever the extension says, and its extension is
    // whatever language the author picked.
    if media.kind == "paste" {
        return CardCategory::Text;
    }
    if let Some(by_ext) = ext_category(&media.ext.to_lowercase()) {
        return by_ext;
    }
    match kind {
        MediaKind::Audio => return CardCategory::Audio,
        MediaKind::Archive => return CardCategory::Archive,
        MediaKind::Pdf => return CardCategory::Document,
        MediaKind::Text => return CardCategory::Text,
        _ => {}
    }
    let mime = media.mime_type.to_lowercase();
    if mime.starts_with("text/") {
        return CardCategory::Text;
    }
    // Not displayable (that returned None already), so this is a raster/vector
    // format the browser declines to show: still an image to the person who has it.
    if mime.starts_with("image/") {
        return CardCategory::Image;
    }
    if mime.ends_with("+json") || mime.ends_with("+xml") || mime == "application/json" {
        return CardCategory::Code;
    }
    if mime.contains("javascript") {
        return CardCategory::Code;
    }
    if mime.contains("spreadsheet") || mime.contains("ms-excel") {
        return CardCategory::Sheet;
    }
    if mime.contains("word") || mime.contains("presentation") || mime.contains("powerpoint") {
        return CardCategory::Document;
    }
    CardCategory::Binary
}

/// `tokenFor(ext, category)`: the label on the card, from a CLOSED set.
///
/// An unrecognised extension falls back to the category's generic word rather
/// than being echoed onto the card, so the set of card URLs this deployment can
/// emit is finite and known, which is what lets the public, unauthenticated
/// render route reject everything else outright.
pub fn token_for(ext: &str, category: CardCategory) -> String {
    // ext.toUpperCase().replace(/[^A-Z0-9]/g, "").slice(0, 8): full Unicode
    // upper-casing first (so "ß" is "SS"), then the ASCII filter.
    let cleaned: String = ext
        .to_uppercase()
        .chars()
        .filter(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        .take(MAX_TOKEN_LENGTH)
        .collect();
    if !cleaned.is_empty() && ext_category(&cleaned.to_ascii_lowercase()).is_some() {
        return cleaned;
    }
    category.fallback_token().to_string()
}

/// `isRenderableChoice(category, token)`: is this pair one the app could
/// actually have produced? Pair, not token: `code-ZIP.png` is not a URL any file
/// would generate.
pub fn is_renderable_choice(category: CardCategory, token: &str) -> bool {
    if category.fallback_token() == token {
        return true;
    }
    ext_category(&token.to_ascii_lowercase()) == Some(category)
}

/// `cardPath(choice)`: `/embed/v1/archive-ZIP.png` - no file identifier anywhere.
pub fn card_path(choice: &CardChoice) -> String {
    format!("/embed/{EMBED_CARD_VERSION}/{}-{}.png", choice.category.as_str(), choice.token)
}

/// `cardUrl(origin, choice)`
pub fn card_url(origin: &str, choice: &CardChoice) -> String {
    format!("{}{}", origin.trim_end_matches('/'), card_path(choice))
}

/// `parseCardFile(file)`: read back a `{category}-{TOKEN}.png` filename. None
/// for anything the closed lists do not admit, so no arbitrary string ever
/// reaches the renderer.
pub fn parse_card_file(file: &str) -> Option<CardChoice> {
    // /^([a-z]+)-([A-Z0-9]{1,8})\.png$/
    let stem = file.strip_suffix(".png")?;
    let (category, token) = stem.split_once('-')?;
    if category.is_empty() || !category.bytes().all(|b| b.is_ascii_lowercase()) || !is_card_token(token) {
        return None;
    }
    let category = CardCategory::parse(category)?;
    // The shape check above is not enough on its own: it admits 36^8 tokens per
    // category, and every admitted URL costs one render. Only pairs this app can
    // emit get drawn.
    if !is_renderable_choice(category, token) {
        return None;
    }
    Some(CardChoice { category, token: token.to_string() })
}

#[cfg(test)]
mod tests {
    //! tests/unit/embed-card.test.ts (the card-art half belongs to the card
    //! renderer of the processing port).
    use super::*;

    fn subject<'a>(ext: &'a str, mime_type: &'a str) -> CardSubject<'a> {
        CardSubject { ext, mime_type, kind: "file" }
    }

    fn choice(category: CardCategory, token: &str) -> Option<CardChoice> {
        Some(CardChoice { category, token: token.to_string() })
    }

    #[test]
    fn extension_wins_then_mime_then_a_neutral_fallback() {
        use CardCategory::*;
        let cases = [
            ("zip", "application/zip", Archive),
            ("rar", "application/x-rar-compressed", Archive),
            ("7z", "application/x-7z-compressed", Archive),
            ("apk", "application/vnd.android.package-archive", Archive),
            ("html", "text/html", Code),
            ("php", "application/octet-stream", Code),
            ("js", "application/javascript", Code),
            ("json", "application/json", Code),
            ("pdf", "application/pdf", Document),
            ("docx", "application/octet-stream", Document),
            ("epub", "application/epub+zip", Document),
            ("xlsx", "application/octet-stream", Sheet),
            ("csv", "text/csv", Sheet),
            ("mp3", "audio/mpeg", Audio),
            ("flac", "audio/flac", Audio),
            ("txt", "text/plain", Text),
            ("log", "text/plain", Text),
            ("svg", "image/svg+xml", Image),
            ("psd", "application/octet-stream", Image),
            ("heic", "image/heic", Image),
            ("exe", "application/x-msdownload", Binary),
            ("ttf", "font/ttf", Binary),
        ];
        for (ext, mime, expected) in cases {
            assert_eq!(card_choice_for(subject(ext, mime)).map(|c| c.category), Some(expected), "{ext}");
        }
    }

    #[test]
    fn falls_back_to_the_neutral_card_for_an_unknown_extension() {
        assert_eq!(card_choice_for(subject("qqz", "application/octet-stream")), choice(CardCategory::Binary, "FILE"));
        // Prototype names are not extensions here.
        assert_eq!(
            card_choice_for(subject("constructor", "application/octet-stream")),
            choice(CardCategory::Binary, "FILE")
        );
    }

    #[test]
    fn reads_the_mime_when_there_is_no_extension() {
        assert_eq!(card_choice_for(subject("", "audio/x-aiff")), choice(CardCategory::Audio, "AUDIO"));
        assert_eq!(card_choice_for(subject("", "text/x-diff")), choice(CardCategory::Text, "TEXT"));
        assert_eq!(card_choice_for(subject("", "application/octet-stream")), choice(CardCategory::Binary, "FILE"));
    }

    #[test]
    fn treats_a_paste_as_text_whatever_language() {
        let s = CardSubject { ext: "rs", mime_type: "text/plain", kind: "paste" };
        assert_eq!(card_choice_for(s), choice(CardCategory::Text, "RS"));
    }

    #[test]
    fn media_that_already_shows_itself_gets_none() {
        for mime in ["image/png", "image/jpeg", "image/gif", "image/webp", "image/avif"] {
            assert_eq!(card_choice_for(subject("png", mime)), None, "{mime}");
        }
        for mime in ["video/mp4", "video/webm", "video/x-matroska", "video/quicktime"] {
            assert_eq!(card_choice_for(subject("mp4", mime)), None, "{mime}");
        }
    }

    #[test]
    fn tokens_stay_inside_a_closed_alphabet() {
        assert_eq!(token_for("zip", CardCategory::Archive), "ZIP");
        assert_eq!(token_for("7z", CardCategory::Archive), "7Z");
        assert_eq!(token_for("tar.gz", CardCategory::Archive), "ARCHIVE");
        assert_eq!(token_for("qqz", CardCategory::Binary), "FILE");
        assert_eq!(token_for("", CardCategory::Document), "DOCUMENT");
        assert_eq!(token_for("日本語", CardCategory::Text), "TEXT");
        assert_eq!(token_for("!!!", CardCategory::Binary), "FILE");
        assert!(token_for("averylongextension", CardCategory::Binary).len() <= 8);
        // Unicode upper-casing first, as String.prototype.toUpperCase does.
        assert_eq!(token_for("z\u{131}p", CardCategory::Archive), "ZIP");
        for category in CARD_CATEGORIES {
            assert!(is_card_token(&token_for("", category)));
        }
    }

    #[test]
    fn card_address_names_a_category_and_a_token_never_a_file() {
        let c = CardChoice { category: CardCategory::Archive, token: "ZIP".into() };
        assert_eq!(card_path(&c), "/embed/v1/archive-ZIP.png");
        assert_eq!(card_url("https://h.example.com/", &c), "https://h.example.com/embed/v1/archive-ZIP.png");
    }

    #[test]
    fn round_trips_every_category() {
        for category in CARD_CATEGORIES {
            let c = CardChoice { category, token: token_for("", category) };
            let path = card_path(&c);
            let file = path.rsplit('/').next().unwrap();
            assert_eq!(parse_card_file(file), Some(c));
        }
    }

    #[test]
    fn refuses_anything_the_closed_lists_do_not_admit() {
        for bad in [
            "archive.png",
            "archive-.png",
            "archive-zip.png",
            "unknown-ZIP.png",
            "archive-TOOLONGTOKEN.png",
            "archive-ZIP.svg",
            "../../etc/passwd",
            "archive-ZIP.png/../x",
            "ARCHIVE-ZIP.png",
            "archive-AAAAAAAA.png",
            "binary-Q7X2K9.png",
            "code-ZIP.png",
            "sheet-HTML.png",
            "archive-ZIP-x.png",
        ] {
            assert_eq!(parse_card_file(bad), None, "{bad}");
        }
        assert_eq!(parse_card_file("archive-ZIP.png"), choice(CardCategory::Archive, "ZIP"));
        assert_eq!(parse_card_file("binary-FILE.png"), choice(CardCategory::Binary, "FILE"));
    }

    #[test]
    fn keeps_every_token_a_real_file_can_emit_renderable() {
        use CardCategory::*;
        for (ext, category) in [
            ("zip", Archive),
            ("7z", Archive),
            ("html", Code),
            ("php", Code),
            ("pdf", Document),
            ("xlsx", Sheet),
            ("flac", Audio),
            ("md", Text),
            ("svg", Image),
            ("exe", Binary),
            ("qqz", Binary),
            ("", Document),
        ] {
            let token = token_for(ext, category);
            assert!(parse_card_file(&format!("{}-{token}.png", category.as_str())).is_some(), "{ext}");
        }
    }
}
