//! One file of the account as a card shows it (`CloudFileViewModel.cs`, `CloudFileType`).

use chrono::{DateTime, Utc};
use modlight_api::Upload;

use crate::format::{age_of, cloud_size, n0};
use crate::icons;
use crate::l10n::{s, t};
use crate::theme::Rgba;

/// The panel's families of file, each with its own plate colour and emblem.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TypeFamily {
    Image,
    Video,
    Audio,
    Pdf,
    Document,
    Data,
    Code,
    Archive,
    Font,
    App,
    Binary,
}

const ARCHIVE_MIMES: &[&str] = &[
    "application/zip",
    "application/x-7z-compressed",
    "application/vnd.rar",
    "application/x-rar-compressed",
    "application/gzip",
    "application/x-tar",
    "application/x-bzip2",
    "application/x-xz",
    "application/zstd",
];

const DOCUMENT_MIMES: &[&str] = &[
    "application/pdf",
    "application/msword",
    "application/vnd.ms-excel",
    "application/vnd.ms-powerpoint",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "application/vnd.oasis.opendocument.text",
    "application/vnd.oasis.opendocument.presentation",
    "application/rtf",
    "application/epub+zip",
];

const DATA_MIMES: &[&str] = &[
    "text/csv",
    "text/tab-separated-values",
    "application/vnd.ms-excel",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.oasis.opendocument.spreadsheet",
    "application/vnd.sqlite3",
];

const CODE_MIMES: &[&str] = &[
    "text/html",
    "text/css",
    "text/javascript",
    "text/xml",
    "application/xhtml+xml",
    "application/javascript",
    "application/x-javascript",
    "application/json",
    "application/xml",
    "application/x-sh",
    "application/sql",
    "application/jwt",
    "application/wasm",
];

const APP_MIMES: &[&str] = &[
    "application/vnd.microsoft.portable-executable",
    "application/x-msdownload",
    "application/x-msi",
    "application/vnd.android.package-archive",
    "application/java-archive",
    "application/vnd.debian.binary-package",
    "application/x-rpm",
    "application/x-apple-diskimage",
];

const EXT_FAMILIES: &[(&str, TypeFamily)] = &[
    ("pdf", TypeFamily::Pdf),
    ("zip", TypeFamily::Archive),
    ("7z", TypeFamily::Archive),
    ("rar", TypeFamily::Archive),
    ("gz", TypeFamily::Archive),
    ("tar", TypeFamily::Archive),
    ("bz2", TypeFamily::Archive),
    ("xz", TypeFamily::Archive),
    ("zst", TypeFamily::Archive),
    ("exe", TypeFamily::App),
    ("msi", TypeFamily::App),
    ("apk", TypeFamily::App),
    ("jar", TypeFamily::App),
    ("deb", TypeFamily::App),
    ("rpm", TypeFamily::App),
    ("dmg", TypeFamily::App),
    ("appimage", TypeFamily::App),
    ("csv", TypeFamily::Data),
    ("tsv", TypeFamily::Data),
    ("xls", TypeFamily::Data),
    ("xlsx", TypeFamily::Data),
    ("ods", TypeFamily::Data),
    ("db", TypeFamily::Data),
    ("sqlite", TypeFamily::Data),
    ("html", TypeFamily::Code),
    ("htm", TypeFamily::Code),
    ("css", TypeFamily::Code),
    ("js", TypeFamily::Code),
    ("mjs", TypeFamily::Code),
    ("ts", TypeFamily::Code),
    ("tsx", TypeFamily::Code),
    ("jsx", TypeFamily::Code),
    ("json", TypeFamily::Code),
    ("xml", TypeFamily::Code),
    ("yml", TypeFamily::Code),
    ("yaml", TypeFamily::Code),
    ("sh", TypeFamily::Code),
    ("bat", TypeFamily::Code),
    ("cmd", TypeFamily::Code),
    ("ps1", TypeFamily::Code),
    ("py", TypeFamily::Code),
    ("rb", TypeFamily::Code),
    ("php", TypeFamily::Code),
    ("lua", TypeFamily::Code),
    ("sql", TypeFamily::Code),
    ("c", TypeFamily::Code),
    ("cpp", TypeFamily::Code),
    ("h", TypeFamily::Code),
    ("cs", TypeFamily::Code),
    ("go", TypeFamily::Code),
    ("rs", TypeFamily::Code),
    ("java", TypeFamily::Code),
    ("ttf", TypeFamily::Font),
    ("otf", TypeFamily::Font),
    ("woff", TypeFamily::Font),
    ("woff2", TypeFamily::Font),
    ("txt", TypeFamily::Document),
    ("log", TypeFamily::Document),
    ("md", TypeFamily::Document),
    ("rtf", TypeFamily::Document),
    ("doc", TypeFamily::Document),
    ("docx", TypeFamily::Document),
    ("odt", TypeFamily::Document),
    ("ppt", TypeFamily::Document),
    ("pptx", TypeFamily::Document),
    ("odp", TypeFamily::Document),
    ("epub", TypeFamily::Document),
    ("srt", TypeFamily::Document),
];

fn in_list(list: &[&str], value: &str) -> bool {
    list.iter().any(|item| item.eq_ignore_ascii_case(value))
}

impl TypeFamily {
    /// The MIME type first, the extension when the MIME type says nothing, the server's
    /// coarse kind last.
    pub fn resolve(mime: Option<&str>, ext: Option<&str>, kind: Option<&str>) -> TypeFamily {
        let family = Self::from_mime(mime);
        if family != TypeFamily::Binary {
            return family;
        }
        if let Some(ext) = ext.filter(|e| !e.is_empty())
            && let Some((_, family)) = EXT_FAMILIES
                .iter()
                .find(|(e, _)| e.eq_ignore_ascii_case(ext))
        {
            return *family;
        }
        Self::from_kind(kind)
    }

    fn from_mime(mime: Option<&str>) -> TypeFamily {
        let Some(mime) = mime.map(str::trim).filter(|m| !m.is_empty()) else {
            return TypeFamily::Binary;
        };
        let top = mime.split('/').next().unwrap_or(mime);
        let lower = mime.to_ascii_lowercase();
        if top.eq_ignore_ascii_case("image") {
            return TypeFamily::Image;
        }
        if top.eq_ignore_ascii_case("video") {
            return TypeFamily::Video;
        }
        if top.eq_ignore_ascii_case("audio") {
            return TypeFamily::Audio;
        }
        if top.eq_ignore_ascii_case("font") {
            return TypeFamily::Font;
        }
        if lower == "application/pdf" {
            return TypeFamily::Pdf;
        }
        if in_list(DATA_MIMES, mime) {
            return TypeFamily::Data;
        }
        if in_list(CODE_MIMES, mime)
            || lower.starts_with("text/x-")
            || lower.ends_with("+xml")
            || lower.ends_with("+json")
        {
            return TypeFamily::Code;
        }
        if in_list(ARCHIVE_MIMES, mime) {
            return TypeFamily::Archive;
        }
        if in_list(APP_MIMES, mime) {
            return TypeFamily::App;
        }
        if top.eq_ignore_ascii_case("text")
            || in_list(DOCUMENT_MIMES, mime)
            || lower == "application/x-subrip"
        {
            return TypeFamily::Document;
        }
        TypeFamily::Binary
    }

    fn from_kind(kind: Option<&str>) -> TypeFamily {
        match kind {
            Some("image") => TypeFamily::Image,
            Some("video") => TypeFamily::Video,
            Some("audio") => TypeFamily::Audio,
            Some("archive") => TypeFamily::Archive,
            Some("paste") | Some("document") => TypeFamily::Document,
            _ => TypeFamily::Binary,
        }
    }

    /// The plate colour: fixed, so one badge reads on all fourteen skins.
    pub fn plate(self) -> Rgba {
        let hex = match self {
            TypeFamily::Image => "#6D28D9",
            TypeFamily::Video => "#BE123C",
            TypeFamily::Audio => "#B45309",
            TypeFamily::Pdf => "#B91C1C",
            TypeFamily::Document => "#0369A1",
            TypeFamily::Data => "#047857",
            TypeFamily::Code => "#4338CA",
            TypeFamily::Archive => "#C2410C",
            TypeFamily::Font => "#A21CAF",
            TypeFamily::App => "#475569",
            TypeFamily::Binary => "#52525B",
        };
        Rgba::parse(hex).unwrap()
    }

    pub fn emblem(self) -> char {
        match self {
            TypeFamily::Image => icons::IMAGE,
            TypeFamily::Video => icons::FILM,
            TypeFamily::Audio => icons::MUSIC,
            TypeFamily::Pdf => icons::BOOK_TEXT,
            TypeFamily::Document => icons::ALIGN_LEFT,
            TypeFamily::Data => icons::TABLE_2,
            TypeFamily::Code => icons::CODE,
            TypeFamily::Archive => icons::ARCHIVE,
            TypeFamily::Font => icons::TYPE,
            TypeFamily::App => icons::APP_WINDOW,
            TypeFamily::Binary => icons::BINARY,
        }
    }
}

/// The word printed on the plate: the extension, else the MIME subtype, at most 8
/// characters, upper case; "FILE" when there is nothing to go on.
pub fn type_label(ext: Option<&str>, mime: Option<&str>) -> String {
    fn clip(label: String) -> String {
        label.chars().take(8).collect()
    }
    if let Some(ext) = ext.filter(|e| !e.is_empty()) {
        return clip(ext.to_uppercase());
    }
    let Some(mime) = mime.filter(|m| !m.is_empty()) else {
        return "FILE".into();
    };
    let sub = match mime.find('/') {
        Some(slash) if slash + 1 < mime.len() => &mime[slash + 1..],
        _ => "",
    };
    let sub = match sub.find(['.', '+', ';']) {
        Some(cut) => &sub[..cut],
        None => sub,
    };
    if sub.is_empty() {
        "FILE".into()
    } else {
        clip(sub.to_uppercase())
    }
}

/// How large the plate's word is printed, by its length.
pub fn type_label_size(label: &str) -> f32 {
    match label.chars().count() {
        0..=3 => 13.0,
        4 => 11.0,
        5 => 10.0,
        _ => 8.0,
    }
}

/// The glyph for a server kind (drag ghost, details placeholder).
pub fn icon_for_kind(kind: Option<&str>) -> char {
    match kind {
        Some("image") => icons::FILE_IMAGE,
        Some("video") => icons::FILE_VIDEO,
        Some("audio") => icons::FILE_AUDIO,
        Some("document") => icons::FILE_TEXT,
        Some("archive") => icons::FILE_ARCHIVE,
        Some("paste") => icons::CLIPBOARD,
        _ => icons::FILE,
    }
}

/// The link formats of "Copy as" and of the details drawer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinkFormat {
    Direct,
    Raw,
    Markdown,
    BBCode,
    Html,
}

impl LinkFormat {
    pub const ALL: [LinkFormat; 5] = [
        LinkFormat::Direct,
        LinkFormat::Raw,
        LinkFormat::Markdown,
        LinkFormat::BBCode,
        LinkFormat::Html,
    ];

    pub fn label(self) -> &'static str {
        match self {
            LinkFormat::Direct => t(s::MySpace_LinkFormatDirect),
            LinkFormat::Raw => t(s::MySpace_LinkFormatRaw),
            LinkFormat::Markdown => t(s::MySpace_LinkFormatMarkdown),
            LinkFormat::BBCode => t(s::MySpace_LinkFormatBBCode),
            LinkFormat::Html => t(s::MySpace_LinkFormatHtml),
        }
    }
}

fn first_non_empty<'a>(candidates: &[Option<&'a str>]) -> &'a str {
    candidates
        .iter()
        .flatten()
        .find(|c| !c.trim().is_empty())
        .copied()
        .unwrap_or("")
}

/// The file's public address with no extension on the end, which every other route
/// (`/download`, `/convert`) hangs off.
pub fn base_of(url: Option<&str>, ext: Option<&str>) -> String {
    let Some(url) = url.filter(|u| !u.is_empty()) else {
        return String::new();
    };
    let Some(ext) = ext.filter(|e| !e.is_empty()) else {
        return url.to_string();
    };
    let suffix = format!(".{ext}");
    // Compared as bytes: in a link that does not end in its extension the cut can fall
    // inside a character, and slicing the text there panics. A match begins with the
    // ASCII dot, so wherever the bytes match the cut is a character boundary.
    let Some(cut) = url.len().checked_sub(suffix.len()) else {
        return url.to_string();
    };
    if url.as_bytes()[cut..].eq_ignore_ascii_case(suffix.as_bytes()) {
        url[..cut].to_string()
    } else {
        url.to_string()
    }
}

/// One file, everything a card and its menus need, derived from the listing row.
#[derive(Clone, Debug, PartialEq)]
pub struct CloudFile {
    pub id: String,
    pub row: Upload,
    pub code: String,
    pub folder_id: Option<String>,
    pub size: i64,
    pub name: String,
    pub thumbnail_url: String,
    pub motion_url: String,
    pub copy_link: String,
    pub open_link: String,
    pub page_link: String,
    pub raw_link: String,
    pub media_base: String,
    pub is_trashed: bool,
    pub has_password: bool,
    pub published: bool,
    /// An expiry or a view budget is set.
    pub expires: bool,
    pub icon: char,
    pub family: TypeFamily,
    pub type_label: String,
}

impl CloudFile {
    pub fn new(upload: Upload) -> CloudFile {
        let mut file = CloudFile {
            id: upload.id.clone().unwrap_or_default(),
            row: Upload::default(),
            code: String::new(),
            folder_id: None,
            size: 0,
            name: String::new(),
            thumbnail_url: String::new(),
            motion_url: String::new(),
            copy_link: String::new(),
            open_link: String::new(),
            page_link: String::new(),
            raw_link: String::new(),
            media_base: String::new(),
            is_trashed: false,
            has_password: false,
            published: false,
            expires: false,
            icon: icons::FILE,
            family: TypeFamily::Binary,
            type_label: String::new(),
        };
        file.apply(upload);
        file
    }

    /// Take a fresher copy of the same row (after a PATCH, or a re-listing).
    pub fn apply(&mut self, upload: Upload) {
        self.code = upload.code.clone().unwrap_or_default();
        self.name = first_non_empty(&[
            upload.filename.as_deref(),
            upload.code.as_deref(),
            Some(t(s::MySpace_UntitledFile)),
        ])
        .to_string();
        self.thumbnail_url = upload.thumb_url.clone().unwrap_or_default();
        self.motion_url = upload.motion_thumb_url.clone().unwrap_or_default();
        self.copy_link = first_non_empty(&[
            upload.copy_url.as_deref(),
            upload.url.as_deref(),
            upload.raw_url.as_deref(),
        ])
        .to_string();
        self.open_link = first_non_empty(&[
            upload.url.as_deref(),
            upload.raw_url.as_deref(),
            upload.copy_url.as_deref(),
        ])
        .to_string();
        self.page_link = upload.url.clone().unwrap_or_default();
        self.raw_link = upload.raw_url.clone().unwrap_or_default();
        self.media_base = base_of(upload.url.as_deref(), upload.ext.as_deref());
        self.is_trashed = upload.url.as_deref().is_none_or(str::is_empty)
            && upload.raw_url.as_deref().is_none_or(str::is_empty);
        self.folder_id = upload.folder_id.clone().filter(|f| !f.is_empty());
        self.size = upload.size.max(0);
        self.has_password = upload.has_password;
        self.published = upload.published;
        self.expires = upload.expires_at.is_some() || upload.max_views.is_some();
        self.icon = icon_for_kind(upload.kind.as_deref());
        self.family = TypeFamily::resolve(
            upload.mime_type.as_deref(),
            upload.ext.as_deref(),
            upload.kind.as_deref(),
        );
        self.type_label = type_label(upload.ext.as_deref(), upload.mime_type.as_deref());
        self.row = upload;
    }

    pub fn can_trash(&self) -> bool {
        !self.id.is_empty()
    }

    pub fn has_link(&self) -> bool {
        !self.copy_link.is_empty()
    }

    pub fn can_open(&self) -> bool {
        !self.open_link.is_empty()
    }

    pub fn can_motion(&self) -> bool {
        !self.motion_url.is_empty()
    }

    pub fn has_media_base(&self) -> bool {
        !self.media_base.is_empty()
    }

    pub fn download_link(&self) -> String {
        if self.media_base.is_empty() {
            String::new()
        } else {
            format!("{}/download", self.media_base)
        }
    }

    /// The `/convert?to=` link for a target format.
    pub fn convert_link(&self, to: &str) -> String {
        if self.media_base.is_empty() {
            String::new()
        } else {
            format!(
                "{}/convert?to={}",
                self.media_base,
                modlight_api::escape_data_string(to)
            )
        }
    }

    pub fn size_text(&self) -> String {
        cloud_size(self.size)
    }

    pub fn age_text(&self, now: DateTime<Utc>) -> String {
        age_of(self.row.created_at, now)
    }

    /// "58,4 KB  ·  22 min ago": the second line of a card.
    pub fn meta(&self, now: DateTime<Utc>) -> String {
        let age = self.age_text(now);
        if age.is_empty() {
            self.size_text()
        } else {
            format!("{}  ·  {}", self.size_text(), age)
        }
    }

    pub fn views_text(&self) -> String {
        n0(self.row.views)
    }

    /// The link in one of the "Copy as" shapes. A picture is embedded; anything else
    /// is linked to its page.
    pub fn link_format(&self, format: LinkFormat) -> String {
        let is_image = self.row.kind.as_deref() == Some("image");
        match format {
            LinkFormat::Direct => self.page_link.clone(),
            LinkFormat::Raw => self.raw_link.clone(),
            LinkFormat::Markdown => {
                if is_image {
                    format!("![{}]({})", self.name, self.raw_link)
                } else {
                    format!("[{}]({})", self.name, self.page_link)
                }
            }
            LinkFormat::BBCode => {
                if is_image {
                    format!("[img]{}[/img]", self.raw_link)
                } else {
                    format!("[url]{}[/url]", self.page_link)
                }
            }
            LinkFormat::Html => {
                if is_image {
                    format!("<img src=\"{}\" alt=\"{}\" />", self.raw_link, self.name)
                } else {
                    format!("<a href=\"{}\">{}</a>", self.page_link, self.name)
                }
            }
        }
    }

    /// The name without its extension (for "Download as").
    pub fn stem(&self) -> String {
        let name = &self.name;
        match name.rfind('.') {
            Some(dot) if dot > 0 => name[..dot].to_string(),
            _ => name.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upload(kind: &str, mime: &str, ext: &str) -> Upload {
        Upload {
            id: Some("id-1".into()),
            code: Some("Bocugavi42".into()),
            filename: Some(format!("brand-palette.{ext}")),
            ext: Some(ext.into()),
            mime_type: Some(mime.into()),
            kind: Some(kind.into()),
            size: 10_444,
            url: Some(format!("https://h.example/T/Bocugavi42.{ext}")),
            raw_url: Some(format!("https://h.example/T/Bocugavi42/raw.{ext}")),
            copy_url: Some(format!("https://h.example/T/Bocugavi42.{ext}")),
            ..Default::default()
        }
    }

    #[test]
    fn families_follow_the_panels_mark() {
        assert_eq!(
            TypeFamily::resolve(Some("image/png"), Some("png"), Some("image")),
            TypeFamily::Image
        );
        assert_eq!(
            TypeFamily::resolve(Some("application/pdf"), None, None),
            TypeFamily::Pdf
        );
        assert_eq!(
            TypeFamily::resolve(Some("text/x-python"), Some("py"), Some("document")),
            TypeFamily::Code
        );
        assert_eq!(
            TypeFamily::resolve(Some("application/octet-stream"), Some("py"), None),
            TypeFamily::Code
        );
        assert_eq!(
            TypeFamily::resolve(
                Some("application/octet-stream"),
                Some("avif"),
                Some("image")
            ),
            TypeFamily::Image
        );
        assert_eq!(
            TypeFamily::resolve(Some("text/csv"), None, None),
            TypeFamily::Data
        );
        assert_eq!(
            TypeFamily::resolve(Some("application/zip"), None, None),
            TypeFamily::Archive
        );
        assert_eq!(
            TypeFamily::resolve(Some("text/plain"), None, None),
            TypeFamily::Document
        );
        assert_eq!(TypeFamily::resolve(None, None, None), TypeFamily::Binary);
        assert_eq!(TypeFamily::Code.plate(), Rgba::parse("#4338CA").unwrap());
        assert_eq!(TypeFamily::Pdf.emblem(), icons::BOOK_TEXT);
    }

    #[test]
    fn labels_are_short_and_loud() {
        assert_eq!(type_label(Some("py"), None), "PY");
        assert_eq!(type_label(Some("appimagex"), None), "APPIMAGE");
        assert_eq!(type_label(None, Some("application/vnd.ms-excel")), "VND");
        assert_eq!(type_label(None, Some("image/svg+xml")), "SVG");
        assert_eq!(type_label(None, Some("text/")), "FILE");
        assert_eq!(type_label(None, None), "FILE");
        assert_eq!(type_label_size("PDF"), 13.0);
        assert_eq!(type_label_size("JSON"), 11.0);
        assert_eq!(type_label_size("XLSXX"), 10.0);
        assert_eq!(type_label_size("APPIMAGE"), 8.0);
    }

    #[test]
    fn links_and_formats() {
        let image = CloudFile::new(upload("image", "image/png", "png"));
        assert_eq!(image.media_base, "https://h.example/T/Bocugavi42");
        assert_eq!(
            image.download_link(),
            "https://h.example/T/Bocugavi42/download"
        );
        assert_eq!(
            image.convert_link("jpg"),
            "https://h.example/T/Bocugavi42/convert?to=jpg"
        );
        assert_eq!(
            image.link_format(LinkFormat::Markdown),
            "![brand-palette.png](https://h.example/T/Bocugavi42/raw.png)"
        );
        assert_eq!(
            image.link_format(LinkFormat::BBCode),
            "[img]https://h.example/T/Bocugavi42/raw.png[/img]"
        );
        assert!(!image.is_trashed);
        assert_eq!(image.stem(), "brand-palette");

        let pdf = CloudFile::new(upload("document", "application/pdf", "pdf"));
        assert_eq!(
            pdf.link_format(LinkFormat::Html),
            "<a href=\"https://h.example/T/Bocugavi42.pdf\">brand-palette.pdf</a>"
        );

        let mut trashed = upload("image", "image/png", "png");
        trashed.url = None;
        trashed.raw_url = None;
        trashed.copy_url = None;
        let trashed = CloudFile::new(trashed);
        assert!(trashed.is_trashed);
        assert!(!trashed.has_link());
        assert!(!trashed.has_media_base());
    }

    /// The listing row's link is the server's string: one that does not end in its
    /// extension may end in characters the extension's length cuts in half.
    #[test]
    fn a_link_cut_inside_a_character_is_not_a_panic() {
        assert_eq!(
            base_of(Some("https://h.example/T/ąąę"), Some("jpeg")),
            "https://h.example/T/ąąę"
        );
        assert_eq!(
            base_of(Some("https://h.example/T/zdjęcie.JPEG"), Some("jpeg")),
            "https://h.example/T/zdjęcie"
        );
        let mut row = upload("image", "image/jpeg", "jpeg");
        row.url = Some("https://h.example/T/ąąę".into());
        assert_eq!(CloudFile::new(row).media_base, "https://h.example/T/ąąę");
    }
}
