//! `lib/media/docx-support.ts`: which documents get a preview, and how big one
//! may be.
//!
//! Split from docx.ts because that module pulls in mammoth and sharp behind a
//! `server-only` guard, and these two answers are needed by places that must stay
//! light: the render ticket (which is framework-agnostic and unit-tested), the
//! viewer deciding whether to show a toggle, and the file menu in the browser.

use crate::media::mammoth::js::js_trim;

/// Largest .docx accepted for preview.
///
/// The 90th percentile of the documents on this deployment is 2.3 MB and the
/// largest is 16 MB. 10 MB previews almost all of them while keeping the archive
/// small enough that the zip guard's verifying decompression stays cheap. The one
/// file above the line gets a clear message rather than an empty page.
pub const MAX_DOCX_BYTES: usize = 10 * 1024 * 1024;

/// `DocxKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocxKind {
    Docx,
    LegacyDoc,
}

impl DocxKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DocxKind::Docx => "docx",
            DocxKind::LegacyDoc => "legacy-doc",
        }
    }
}

/// Does this file get a document preview at all?
///
/// `.doc` is the case worth naming rather than lumping in with "no". It is not a
/// ZIP and not XML - it is the OLE2 compound binary format Word used until 2007,
/// and mammoth cannot read a byte of it. There is exactly one such file on this
/// deployment, and the honest thing is for the UI to say the old format is not
/// supported rather than render an empty page and let the reader conclude the
/// document itself is empty.
///
/// The extension is consulted alongside the type because the stored type is so
/// often useless: ShareX and curl send application/octet-stream for anything
/// Windows has no registry entry for, and an imported row keeps whatever its
/// old host guessed years ago.
pub fn docx_support(mime: Option<&str>, ext: Option<&str>) -> Option<DocxKind> {
    let ext = ext.unwrap_or("");
    let e = ext.strip_prefix('.').unwrap_or(ext).to_lowercase();
    let mime = mime.unwrap_or("");
    let m = js_trim(mime.split(';').next().unwrap_or("")).to_lowercase();

    if e == "docx" || m == "application/vnd.openxmlformats-officedocument.wordprocessingml.document" {
        return Some(DocxKind::Docx);
    }
    if e == "doc" || m == "application/msword" {
        return Some(DocxKind::LegacyDoc);
    }
    None
}

#[cfg(test)]
mod tests {
    //! The `docxSupport` cases of tests/integration/docx.test.ts.
    use super::*;

    #[test]
    fn recognises_a_docx_by_type_or_by_name() {
        let docx_mime = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
        assert_eq!(docx_support(Some(docx_mime), Some("docx")), Some(DocxKind::Docx));
        // The common case on this deployment: ShareX and old imports store
        // octet-stream, so the name is the only evidence there is.
        assert_eq!(docx_support(Some("application/octet-stream"), Some("docx")), Some(DocxKind::Docx));
        assert_eq!(docx_support(Some(docx_mime), None), Some(DocxKind::Docx));
    }

    #[test]
    fn names_the_legacy_doc_case_instead_of_lumping_it_in_with_no() {
        // mammoth cannot read a byte of OLE2, so the UI has to say the format is
        // unsupported rather than render an empty page.
        assert_eq!(docx_support(Some("application/msword"), Some("doc")), Some(DocxKind::LegacyDoc));
        assert_eq!(docx_support(Some("application/octet-stream"), Some("doc")), Some(DocxKind::LegacyDoc));
    }

    #[test]
    fn claims_nothing_else() {
        assert_eq!(docx_support(Some("application/pdf"), Some("pdf")), None);
        assert_eq!(docx_support(Some("image/png"), Some("png")), None);
        assert_eq!(docx_support(Some("application/vnd.ms-excel"), Some("xls")), None);
    }

    #[test]
    fn normalises_like_upstream() {
        assert_eq!(docx_support(None, Some(".DOCX")), Some(DocxKind::Docx));
        assert_eq!(docx_support(Some(" Application/MSWord ; charset=x"), None), Some(DocxKind::LegacyDoc));
        assert_eq!(docx_support(None, Some("..docx")), None);
        assert_eq!(docx_support(None, None), None);
    }
}
