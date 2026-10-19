//! Port of `lib/media/paste-type.ts`: what type is an in-app paste, really?
//!
//! A paste's `language` is not a filename: it is written only by `createPaste`,
//! only from the closed `LANG_EXT` list, and only because a signed-in author
//! picked it from a menu - the same act as uploading a .html file. NONE of this
//! touches how bytes are served: `serveDecision()` is an allowlist, and
//! [`paste_mime_type`] refuses to return any type that would not leave the
//! server as text.

use super::mime::is_text_payload;
use super::sniff::{mime_from_ext, normalize_mime};
use crate::paste_languages::lang_ext;

/// The MIME a paste declares by its language, or `None` when the language says
/// nothing usable.
///
/// Two guards, both load-bearing: a language NAME is not a file extension
/// (`LANG_EXT.ts` is "ts", which the extension table reads as `video/mp2t`), and
/// the result must be something `/raw` would hand over as text.
pub fn paste_mime_type(language: Option<&str>) -> Option<String> {
    let key = crate::config::js_trim(language.unwrap_or("")).to_lowercase();
    let ext = lang_ext(&key)?;
    let mime = mime_from_ext(Some(ext))?;
    if is_text_payload(Some(mime), None) { Some(normalize_mime(Some(mime))) } else { None }
}

/// The type to reason with for a row: the stored type, or for a paste what its
/// language declares (which also repairs pastes stored before this existed).
pub fn row_mime_type(kind: Option<&str>, language: Option<&str>, mime_type: &str) -> String {
    if kind != Some("paste") {
        return mime_type.to_string();
    }
    paste_mime_type(language).unwrap_or_else(|| mime_type.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_map_to_text_types_only() {
        assert_eq!(paste_mime_type(Some("html")).as_deref(), Some("text/html"));
        assert_eq!(paste_mime_type(Some(" JSON ")).as_deref(), Some("application/json"));
        // "ts" is video/mp2t in the extension table: refused.
        assert_eq!(paste_mime_type(Some("ts")), None);
        assert_eq!(paste_mime_type(Some("text")).as_deref(), Some("text/plain"));
        assert_eq!(paste_mime_type(Some("kt")), None);
        assert_eq!(paste_mime_type(None), None);
        assert_eq!(row_mime_type(Some("paste"), Some("css"), "text/plain; charset=utf-8"), "text/css");
        assert_eq!(row_mime_type(Some("file"), Some("css"), "image/png"), "image/png");
    }
}
