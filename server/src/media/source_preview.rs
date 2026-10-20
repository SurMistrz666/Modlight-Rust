//! Port of `lib/media/source-preview.ts`: "does this file read as source, and
//! what readings does it have?"
//!
//! One module, because the answer used to be written down twice - once in the
//! preview page (to decide whether to read the bytes) and once in the viewer
//! (to decide what to draw) - in two conditions that were nearly, but not
//! exactly, the same. Neither matched the raw route, which is how a .json
//! opened as a download card whose "Open raw" button then showed the very same
//! bytes as text.
//!
//! Nothing here is a new definition of "is this text". `is_text_payload`
//! derives that from `serve_decision`, the anti-XSS gate itself, and
//! `row_mime_type` is the paste rule from `paste_type`. This module only
//! composes them.

use serde::Serialize;

use crate::media::mime::is_text_payload;
use crate::media::paste_type::row_mime_type;
use crate::media::sniff::normalize_ext;
use crate::paste_languages::lang_ext;

/// How much of a file may be read into the page as text. The preview is
/// server-rendered and the string travels in the payload, so this bounds a
/// public request rather than a private one.
pub const MAX_INLINE_TEXT: i64 = 512 * 1024;

/// `SourceSubject`
#[derive(Debug, Clone, Default)]
pub struct SourceSubject<'a> {
    pub mime_type: &'a str,
    pub ext: Option<&'a str>,
    /// 'file' | 'paste' - a paste's type is declared by its language.
    pub kind: Option<&'a str>,
    pub language: Option<&'a str>,
}

/// `showsSource(subject)`: should this file's preview show its contents rather
/// than a download card? True for exactly the files `/raw` serves as text.
pub fn shows_source(subject: &SourceSubject<'_>) -> bool {
    let mime = row_mime_type(subject.kind, subject.language, subject.mime_type);
    is_text_payload(Some(&mime), subject.ext)
}

/// `inlineTextPlan(size)`: read the bytes, or refuse and say why.
pub fn inline_text_plan(size: i64) -> &'static str {
    if size > MAX_INLINE_TEXT { "too-large" } else { "read" }
}

/// `SourceMode`: `source` - the contents, formatted, on this page; `raw` - the
/// plain bytes at the raw URL; `page` - the file executed as a document, in
/// the sandboxed frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceMode {
    Source,
    Raw,
    Page,
}

/// `SourceReading`: where a reading lives, or `None` when it is rendered in
/// place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceReading {
    pub mode: SourceMode,
    pub href: Option<String>,
}

/// `sourceReadings({ rawUrl, renderPath })`: every reading this file has, in
/// the order they are offered. `raw` is last because it is the only one that
/// leaves the page.
pub fn source_readings(raw_url: &str, render_path: Option<&str>) -> Vec<SourceReading> {
    let mut readings = vec![SourceReading { mode: SourceMode::Source, href: None }];
    if let Some(p) = render_path.filter(|p| !p.is_empty()) {
        readings.push(SourceReading { mode: SourceMode::Page, href: Some(p.to_string()) });
    }
    readings.push(SourceReading { mode: SourceMode::Raw, href: Some(raw_url.to_string()) });
    readings
}

/// Display names for the languages this deployment actually sees, keyed by
/// extension. Presentation only.
fn language_name(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "bash" | "sh" | "zsh" => "Shell",
        "bat" | "cmd" => "Batch",
        "c" | "h" => "C",
        "cc" | "cpp" | "hpp" => "C++",
        "cfg" | "conf" | "ini" => "INI",
        "cjs" | "js" | "mjs" => "JavaScript",
        "cs" => "C#",
        "css" => "CSS",
        "csv" => "CSV",
        "diff" | "patch" => "Diff",
        "dockerfile" => "Dockerfile",
        "env" => "Env",
        "go" => "Go",
        "htm" | "html" => "HTML",
        "java" => "Java",
        "json" | "jsonc" | "sxcu" => "JSON",
        "jsx" => "JSX",
        "kt" => "Kotlin",
        "log" => "Log",
        "lua" => "Lua",
        "markdown" | "md" => "Markdown",
        "php" => "PHP",
        "pl" => "Perl",
        "ps1" => "PowerShell",
        "py" => "Python",
        "r" => "R",
        "rb" => "Ruby",
        "rs" => "Rust",
        "scss" => "SCSS",
        "sql" => "SQL",
        "srt" | "vtt" => "Subtitles",
        "svelte" => "Svelte",
        "svg" => "SVG",
        "swift" => "Swift",
        "tf" => "Terraform",
        "toml" => "TOML",
        "ts" => "TypeScript",
        "tsv" => "TSV",
        "tsx" => "TSX",
        "txt" => "Text",
        "vue" => "Vue",
        "xhtml" => "XHTML",
        "xml" => "XML",
        "yaml" | "yml" => "YAML",
        "zig" => "Zig",
        _ => return None,
    })
}

/// `codeLanguageLabel(subject)`: what to call the language on screen, or
/// `None` when there is nothing to say. The fallback echoes the extension,
/// bounded the way card.ts bounds its token: letters and digits only, at most
/// eight of them.
///
/// A paste's language goes through `LANG_EXT` with an exact key, so only the
/// listed languages map (upstream's object lookup also found what every
/// JavaScript object inherits, e.g. `constructor`; SKELETON section 9, item 15).
pub fn code_language_label(subject: &SourceSubject<'_>) -> Option<String> {
    let from_language =
        if subject.kind == Some("paste") { lang_ext(&subject.language.unwrap_or("").to_lowercase()) } else { None };
    let ext = normalize_ext(from_language.or(subject.ext));
    if ext.is_empty() {
        return None;
    }
    if let Some(named) = language_name(&ext) {
        return Some(named.to_string());
    }
    let tame = (1..=8).contains(&ext.len()) && ext.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    tame.then(|| ext.to_uppercase())
}

#[cfg(test)]
mod tests {
    //! tests/unit/source-preview.test.ts (the render toggle's `sandboxShape`
    //! is the media serving port's render-ticket module and is tested there).
    use super::*;
    use crate::media::mime::{MediaKind, media_kind, serve_decision};
    use crate::media::paste_type::paste_mime_type;
    use crate::media::sniff::{UploadTypeInput, resolve_upload_type};
    use crate::paste_languages::paste_languages;

    fn stored_type_of(ext: &str) -> String {
        resolve_upload_type(UploadTypeInput {
            head: Some(b"const x = 1;\nprint('hello')\n"),
            ext: Some(ext),
            filename: None,
            client_mime: Some("application/octet-stream"),
        })
    }

    fn subject<'a>(mime: &'a str, ext: &'a str) -> SourceSubject<'a> {
        SourceSubject { mime_type: mime, ext: Some(ext), ..Default::default() }
    }

    #[test]
    fn code_opens_as_code() {
        for ext in ["py", "js", "ts", "tsx", "json", "sh", "css", "md", "yml", "toml", "sql", "xml", "c"] {
            let mime = stored_type_of(ext);
            assert!(shows_source(&subject(&mime, ext)), "{ext} -> {mime}");
        }
        for ext in ["json", "sh", "sql", "xml", "sxcu", "svg"] {
            let mime = stored_type_of(ext);
            assert_eq!(media_kind(Some(&mime), Some(ext)), MediaKind::Other, "{ext} kind");
            assert!(shows_source(&subject(&mime, ext)), "{ext} source");
        }
        assert!(shows_source(&subject("application/octet-stream", "py")));
        assert!(!shows_source(&subject("application/octet-stream", "")));
        for (mime, ext) in [
            ("image/png", "png"),
            ("video/mp4", "mp4"),
            ("audio/mpeg", "mp3"),
            ("application/pdf", "pdf"),
            ("application/zip", "zip"),
            ("application/octet-stream", "gr2"),
            ("application/vnd.openxmlformats-officedocument.wordprocessingml.document", "docx"),
        ] {
            assert!(!shows_source(&subject(mime, ext)), "{ext}");
        }
    }

    #[test]
    fn a_paste_behaves_as_a_paste() {
        for language in paste_languages() {
            let stored = paste_mime_type(Some(language)).unwrap_or_else(|| "text/plain; charset=utf-8".into());
            let row = SourceSubject {
                kind: Some("paste"),
                language: Some(language),
                mime_type: &stored,
                ext: lang_ext(language),
            };
            assert!(shows_source(&row), "{language}");
        }
        for language in paste_languages() {
            let Some(stored) = paste_mime_type(Some(language)) else { continue };
            let decision = serve_decision(Some(&stored), lang_ext(language));
            assert_eq!(decision.disposition, "inline", "{language}");
            assert_eq!(decision.content_type, "text/plain; charset=utf-8", "{language}");
        }
        assert_eq!(paste_mime_type(Some("ts")), None);
        let row = SourceSubject {
            kind: Some("paste"),
            language: Some("ts"),
            mime_type: "text/plain; charset=utf-8",
            ext: Some("ts"),
        };
        assert!(shows_source(&row));
    }

    #[test]
    fn the_readings_a_preview_offers() {
        let raw = "https://h.example.com/ToqOse/abc123/raw.html";
        let render = "/ToqOse/abc123/render?t=ticket";
        let modes = |r: &[SourceReading]| r.iter().map(|x| x.mode).collect::<Vec<_>>();
        let plain = source_readings(raw, None);
        assert_eq!(modes(&plain), [SourceMode::Source, SourceMode::Raw]);
        assert_eq!(plain[0].href, None);
        assert_eq!(plain[1].href.as_deref(), Some(raw));
        let rendered = source_readings(raw, Some(render));
        assert_eq!(modes(&rendered), [SourceMode::Source, SourceMode::Page, SourceMode::Raw]);
        assert_eq!(rendered[1].href.as_deref(), Some(render));
        for r in &rendered {
            if let Some(h) = &r.href {
                assert!(h.contains("abc123"));
            }
        }
        assert_eq!(serde_json::to_string(&rendered[0]).unwrap(), r#"{"mode":"source","href":null}"#);
    }

    #[test]
    fn the_size_ceiling() {
        assert_eq!(inline_text_plan(0), "read");
        assert_eq!(inline_text_plan(MAX_INLINE_TEXT), "read");
        assert_eq!(inline_text_plan(MAX_INLINE_TEXT + 1), "too-large");
        assert_eq!(MAX_INLINE_TEXT, 512 * 1024);
    }

    #[test]
    fn naming_the_language_on_screen() {
        assert_eq!(code_language_label(&subject("text/x-python", "py")).as_deref(), Some("Python"));
        assert_eq!(code_language_label(&subject("text/plain", "ts")).as_deref(), Some("TypeScript"));
        assert_eq!(code_language_label(&subject("application/json", "sxcu")).as_deref(), Some("JSON"));
        let paste = |language: &'static str, mime: &'static str, ext: &'static str| SourceSubject {
            kind: Some("paste"),
            language: Some(language),
            mime_type: mime,
            ext: Some(ext),
        };
        assert_eq!(code_language_label(&paste("html", "text/html", "html")).as_deref(), Some("HTML"));
        assert_eq!(code_language_label(&paste("cpp", "text/x-c", "cpp")).as_deref(), Some("C++"));
        assert_eq!(code_language_label(&subject("text/plain", "zzz")).as_deref(), Some("ZZZ"));
        assert_eq!(code_language_label(&subject("text/plain", "a-very-long-extension")), None);
        assert_eq!(code_language_label(&subject("text/plain", "<svg")), None);
        assert_eq!(code_language_label(&subject("text/plain", "")), None);
    }
}
