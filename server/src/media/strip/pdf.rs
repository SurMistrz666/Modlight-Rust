//! PDF: the Info dictionary and every XMP stream go, through lopdf.
//!
//! The document is loaded and written anew, which also drops what the old
//! revisions of an incrementally saved file still held (the previous Info is
//! not a live object, so it is not written). Pages, fonts and pictures are the
//! same objects as before; a later revision of the same document is the same
//! document.
//!
//! Not cleaned, and said so: a password-protected file (nothing in it can be read
//! without the password) and a signed one (a rewrite would break the signature).
//! A file whose cross-reference table had to be rebuilt is damaged: writing out
//! a guess is how a repaired file loses a page.
//!
//! Memory is the file and its decoded objects, so the size is bounded.

use std::cell::Cell;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use lopdf::{Document, LoadOptions, Object};

use super::KeptReason;

/// Largest PDF that is rewritten: a whole file and its objects live in memory,
/// inside a conversion's 512 MiB share of the media limiter.
pub const PDF_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// Bound on one object stream while loading (decompression bombs).
const MAX_INFLATED: usize = 64 * 1024 * 1024;

/// `SigFlags` bit 1, SignaturesExist.
fn is_signed(doc: &Document) -> bool {
    let Ok(catalog) = doc.catalog() else {
        return false;
    };
    let Ok((_, form)) = catalog.get_deref(b"AcroForm", doc).map(|o| ((), o)) else {
        return false;
    };
    let Ok(form) = form.as_dict() else {
        return false;
    };
    form.get_deref(b"SigFlags", doc).and_then(Object::as_i64).is_ok_and(|flags| flags & 1 != 0)
}

/// Rewrite a loaded document; `Ok(false)` when there is nothing to remove.
pub fn clean(doc: &mut Document) -> Result<bool, KeptReason> {
    if doc.was_encrypted() || doc.is_encrypted() {
        return Err(KeptReason::Protected);
    }
    if doc.xref_recovery.is_some() {
        return Err(KeptReason::Damaged);
    }
    if is_signed(doc) {
        return Err(KeptReason::Signed);
    }
    let had_info = doc.trailer.remove(b"Info").is_some();
    let had_xmp = Cell::new(false);
    doc.traverse_objects(|object| {
        let dictionary = match object {
            Object::Dictionary(dictionary) => dictionary,
            Object::Stream(stream) => &mut stream.dict,
            _ => return,
        };
        if dictionary.remove(b"Metadata").is_some() {
            had_xmp.set(true);
        }
    });
    if !had_info && !had_xmp.get() && doc.revision_count() <= 1 {
        return Ok(false);
    }
    doc.prune_objects();
    Ok(true)
}

/// Clean one file.
pub fn strip_file(input: &Path, output: &Path) -> Result<bool, KeptReason> {
    let size = std::fs::metadata(input).map_err(|_| KeptReason::Tool)?.len();
    if size > PDF_MAX_BYTES {
        return Err(KeptReason::TooLarge);
    }
    let options = LoadOptions { max_decompressed_size: Some(MAX_INFLATED), ..LoadOptions::default() };
    let mut doc = Document::load_with_options(input, options).map_err(|err| {
        tracing::debug!("[strip] PDF: {err}");
        if matches!(err, lopdf::Error::Decryption(_) | lopdf::Error::AlreadyEncrypted) {
            KeptReason::Protected
        } else {
            KeptReason::Damaged
        }
    })?;
    let pages = doc.get_pages().len();
    if pages == 0 {
        return Err(KeptReason::Damaged);
    }
    if !clean(&mut doc)? {
        return Ok(false);
    }

    let mut writer = BufWriter::new(File::create(output).map_err(|_| KeptReason::Tool)?);
    doc.save_to(&mut writer).map_err(|err| {
        tracing::warn!("[strip] PDF could not be written: {err}");
        KeptReason::Tool
    })?;
    writer.flush().map_err(|_| KeptReason::Tool)?;
    drop(writer);

    // The result must read back as the same document.
    let again = Document::load(output).map_err(|_| KeptReason::Tool)?;
    if again.get_pages().len() != pages || again.trailer.has(b"Info") {
        return Err(KeptReason::Tool);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use lopdf::{Dictionary, Stream, dictionary};

    use super::*;

    /// A one-page document with an Info dictionary and an XMP stream.
    fn document() -> Document {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let contents = doc.add_object(Stream::new(dictionary! {}, b"BT (kept page) Tj ET".to_vec()));
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => contents,
            "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }),
        );
        let xmp = doc.add_object(Stream::new(
            dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
            b"<x:xmpmeta>PLANTED-XMP</x:xmpmeta>".to_vec(),
        ));
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id, "Metadata" => xmp });
        let info = doc.add_object(dictionary! {
            "Author" => Object::string_literal("PLANTED-AUTHOR"),
            "Producer" => Object::string_literal("PLANTED-PRODUCER"),
        });
        doc.trailer.set("Root", catalog);
        doc.trailer.set("Info", info);
        doc
    }

    fn saved(doc: &mut Document) -> Vec<u8> {
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    fn strip_bytes(pdf: &[u8]) -> (Result<bool, KeptReason>, Option<Vec<u8>>) {
        let dir = tempfile::tempdir().unwrap();
        let (input, output) = (dir.path().join("in.pdf"), dir.path().join("out.pdf"));
        std::fs::write(&input, pdf).unwrap();
        let result = strip_file(&input, &output);
        (result, std::fs::read(output).ok())
    }

    #[test]
    fn info_and_xmp_are_gone_and_the_page_is_not() {
        let original = saved(&mut document());
        assert!(contains(&original, b"PLANTED-AUTHOR") && contains(&original, b"PLANTED-XMP"));
        let (result, out) = strip_bytes(&original);
        assert_eq!(result, Ok(true));
        let out = out.unwrap();
        for planted in [&b"PLANTED-AUTHOR"[..], b"PLANTED-PRODUCER", b"PLANTED-XMP", b"/Metadata", b"/Info"] {
            assert!(!contains(&out, planted), "{}", String::from_utf8_lossy(planted));
        }
        let doc = Document::load_mem(&out).unwrap();
        assert_eq!(doc.get_pages().len(), 1);
        let page = *doc.get_pages().get(&1).unwrap();
        assert!(doc.get_page_content(page).starts_with(b"BT (kept page) Tj ET"));
    }

    #[test]
    fn xmp_on_a_page_or_a_picture_goes_too() {
        let mut doc = document();
        let xmp = doc.add_object(Stream::new(dictionary! {}, b"PLANTED-PAGE-XMP".to_vec()));
        let page = *doc.get_pages().get(&1).unwrap();
        doc.get_dictionary_mut(page).unwrap().set("Metadata", xmp);
        let (result, out) = strip_bytes(&saved(&mut doc));
        assert_eq!(result, Ok(true));
        assert!(!contains(&out.unwrap(), b"PLANTED-PAGE-XMP"));
    }

    #[test]
    fn a_document_with_nothing_to_remove_is_left_alone() {
        let mut doc = document();
        doc.trailer.remove(b"Info");
        doc.catalog_mut().unwrap().remove(b"Metadata");
        doc.prune_objects();
        let (result, out) = strip_bytes(&saved(&mut doc));
        assert_eq!(result, Ok(false));
        assert_eq!(out, None);
    }

    #[test]
    fn an_older_revision_does_not_survive() {
        // An incremental update on top of the original: a new Info object, and
        // the old one still in the bytes.
        let mut original = saved(&mut document());
        let start = original.len();
        original.extend_from_slice(b"99 0 obj\n<< /Title (NEW-TITLE) >>\nendobj\n");
        let doc = Document::load_mem(&original[..start]).unwrap();
        let root = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let xref_at = original.len();
        let prev = {
            let tail = String::from_utf8_lossy(&original[..start]);
            tail.rsplit("startxref").next().unwrap().split_whitespace().next().unwrap().parse::<usize>().unwrap()
        };
        original.extend_from_slice(
            format!(
                "xref\n99 1\n{start:010} 00000 n \ntrailer\n<< /Size 100 /Root {} {} R /Info 99 0 R /Prev {prev} >>\nstartxref\n{xref_at}\n%%EOF\n",
                root.0, root.1
            )
            .as_bytes(),
        );
        assert!(contains(&original, b"PLANTED-AUTHOR"));
        let (result, out) = strip_bytes(&original);
        assert_eq!(result, Ok(true));
        let out = out.unwrap();
        assert!(!contains(&out, b"PLANTED-AUTHOR"));
        assert!(!contains(&out, b"NEW-TITLE"));
    }

    #[test]
    fn a_signed_document_is_kept() {
        let mut doc = document();
        let form = doc.add_object(dictionary! { "SigFlags" => 3, "Fields" => Vec::<Object>::new() });
        doc.catalog_mut().unwrap().set("AcroForm", form);
        assert_eq!(strip_bytes(&saved(&mut doc)).0, Err(KeptReason::Signed));

        let mut doc = document();
        let mut form = Dictionary::new();
        form.set("SigFlags", 1);
        doc.catalog_mut().unwrap().set("AcroForm", form);
        assert_eq!(strip_bytes(&saved(&mut doc)).0, Err(KeptReason::Signed));

        // A form without a signature is a form.
        let mut doc = document();
        let mut form = Dictionary::new();
        form.set("Fields", Vec::<Object>::new());
        doc.catalog_mut().unwrap().set("AcroForm", form);
        assert_eq!(strip_bytes(&saved(&mut doc)).0, Ok(true));
    }

    #[test]
    fn what_is_not_a_document_is_kept_as_damaged() {
        for broken in [&b"%PDF-1.7\n"[..], b"%PDF-1.7\nnothing here\n%%EOF\n", b"%PDF-"] {
            assert_eq!(strip_bytes(broken).0, Err(KeptReason::Damaged), "{broken:?}");
        }
    }

    #[test]
    fn a_file_that_needed_repair_is_not_rewritten() {
        // The startxref of the file points nowhere: lopdf rebuilds the table.
        let mut pdf = saved(&mut document());
        let at = pdf.windows(9).rposition(|w| w == b"startxref").unwrap();
        pdf.truncate(at);
        pdf.extend_from_slice(b"startxref\n999999\n%%EOF\n");
        assert_eq!(strip_bytes(&pdf).0, Err(KeptReason::Damaged));
    }

    #[test]
    fn deep_nesting_does_not_end_the_process() {
        let mut pdf = b"%PDF-1.7\n1 0 obj\n".to_vec();
        pdf.extend(std::iter::repeat_n(b'[', 300_000));
        pdf.extend(std::iter::repeat_n(b']', 300_000));
        pdf.extend_from_slice(b"\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n");
        assert!(strip_bytes(&pdf).0.is_err());
    }
}
