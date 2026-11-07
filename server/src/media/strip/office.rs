//! Office and OpenDocument files: a zip whose property parts are replaced with
//! neutral ones. Every other entry is copied byte for byte from the source,
//! local header and all (the compressed data is never touched), which is also
//! what keeps ODF's first, stored `mimetype` where it must be.
//!
//! - **OOXML** (docx, xlsx, pptx and their relatives): `docProps/core.xml` and
//!   `docProps/app.xml` become empty property sets; `docProps/custom.xml` goes,
//!   with its entry in `[Content_Types].xml` and its relationship in `_rels/.rels`
//!   (a dangling one makes Word offer to repair the file).
//! - **ODF** (odt, ods, odp...): `meta.xml` becomes an empty `office:meta`.
//!
//! A signed package stays as it came (the signature covers these parts), and so
//! does a password-protected ODF. A zip that is not a document is none of this
//! module's business.
//!
//! Memory: the central directory and the few small parts it replaces; the rest
//! streams from file to file.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use flate2::read::DeflateDecoder;

use super::{KeptReason, RewriteError, pass};

const LOCAL: &[u8; 4] = b"PK\x03\x04";
const CENTRAL: &[u8; 4] = b"PK\x01\x02";
const EOCD: &[u8; 4] = b"PK\x05\x06";
const EOCD_SIZE: usize = 22;
const CENTRAL_SIZE: usize = 46;
const LOCAL_SIZE: usize = 30;

/// How much of the end of a stored file is enough to find the directory of a
/// document: the end record with the longest comment, and a directory of a few
/// thousand entries.
pub const TAIL_BYTES: u64 = 512 * 1024;

/// The longest central directory read. A document has hundreds of entries.
const MAX_DIRECTORY: u64 = 16 * 1024 * 1024;

/// The longest part that is read and edited.
const MAX_EDITED_PART: u64 = 8 * 1024 * 1024;

const CONTENT_TYPES: &str = "[Content_Types].xml";
const ROOT_RELS: &str = "_rels/.rels";
const CORE: &str = "docProps/core.xml";
const APP: &str = "docProps/app.xml";
const CUSTOM: &str = "docProps/custom.xml";
const CUSTOM_RELS: &str = "docProps/_rels/custom.xml.rels";
const MIMETYPE: &str = "mimetype";
const ODF_META: &str = "meta.xml";
const ODF_MANIFEST: &str = "META-INF/manifest.xml";

/// Parts only a word processor, a spreadsheet or a presentation has: the other
/// packages built the same way (NuGet, VSIX, XPS) are left alone.
const OOXML_MAIN_PARTS: [&str; 3] = ["word/document.xml", "xl/workbook.xml", "ppt/presentation.xml"];

const NEUTRAL_CORE: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:dcmitype="http://purl.org/dc/dcmitype/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"/>"#;

const NEUTRAL_APP: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"/>"#;

/// `meta.xml` with nothing in it, for a document of ODF `version`.
fn neutral_meta(version: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-meta xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" office:version=\"{version}\"><office:meta/></office:document-meta>"
    )
    .into_bytes()
}

/// One entry of the central directory.
#[derive(Debug, Clone)]
struct Entry {
    name: String,
    /// The whole directory record, with its name, extra and comment.
    record: Vec<u8>,
    method: u16,
    flags: u16,
    crc: u32,
    compressed: u64,
    size: u64,
    /// Where its local header is, and where the next thing in the file starts.
    start: u64,
    end: u64,
}

fn u16_at(buf: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([buf[at], buf[at + 1]])
}

fn u32_at(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]])
}

/// The end-of-central-directory record in `tail`: its offset in `tail`, the
/// directory's size and its offset in the file.
fn find_end(tail: &[u8]) -> Option<(usize, u64, u64, u16)> {
    let last = tail.len().checked_sub(EOCD_SIZE)?;
    (0..=last).rev().find_map(|at| {
        if &tail[at..at + 4] != EOCD {
            return None;
        }
        let comment = u16_at(tail, at + 20) as usize;
        if at + EOCD_SIZE + comment > tail.len() {
            return None;
        }
        let (entries, disk) = (u16_at(tail, at + 10), u16_at(tail, at + 4));
        let (size, offset) = (u32_at(tail, at + 12), u32_at(tail, at + 16));
        // A ZIP64 archive and a split one say so here; neither is a document.
        if size == u32::MAX || offset == u32::MAX || entries == u16::MAX || disk != 0 {
            return None;
        }
        Some((at, u64::from(size), u64::from(offset), entries))
    })
}

/// Parse the records of a central directory.
fn parse_directory(directory: &[u8], expected: u16) -> Result<Vec<Entry>, RewriteError> {
    let mut entries = Vec::with_capacity(usize::from(expected).min(4096));
    let mut at = 0usize;
    while at < directory.len() {
        let fixed = directory.get(at..at + CENTRAL_SIZE).ok_or(RewriteError::Malformed("zip: a record is cut off"))?;
        if &fixed[..4] != CENTRAL {
            return Err(RewriteError::Malformed("zip: not a directory record"));
        }
        let (name_len, extra_len, comment_len) =
            (u16_at(fixed, 28) as usize, u16_at(fixed, 30) as usize, u16_at(fixed, 32) as usize);
        let total = CENTRAL_SIZE + name_len + extra_len + comment_len;
        let record = directory.get(at..at + total).ok_or(RewriteError::Malformed("zip: a record is cut off"))?;
        let (compressed, size, start) = (u32_at(fixed, 20), u32_at(fixed, 24), u32_at(fixed, 42));
        if compressed == u32::MAX || size == u32::MAX || start == u32::MAX {
            return Err(RewriteError::Malformed("zip: 64-bit entries"));
        }
        entries.push(Entry {
            name: String::from_utf8_lossy(&record[CENTRAL_SIZE..CENTRAL_SIZE + name_len]).into_owned(),
            record: record.to_vec(),
            method: u16_at(fixed, 10),
            flags: u16_at(fixed, 8),
            crc: u32_at(fixed, 16),
            compressed: u64::from(compressed),
            size: u64::from(size),
            start: u64::from(start),
            end: 0,
        });
        at += total;
    }
    if entries.len() != usize::from(expected) {
        return Err(RewriteError::Malformed("zip: the directory does not hold what the end record says"));
    }
    Ok(entries)
}

/// Whether the end of a file can belong to a document; `tail_start` is where
/// `tail` begins in the file. A directory further back than `tail` cannot be
/// read from here, so the answer is yes.
pub fn may_be_document(tail: &[u8], tail_start: u64) -> bool {
    let Some((at, size, offset, entries)) = find_end(tail) else {
        return false;
    };
    let Some(from) = offset.checked_sub(tail_start) else {
        return true;
    };
    let from = from as usize;
    let Some(directory) = from.checked_add(size as usize).and_then(|end| tail.get(from..end)) else {
        return false;
    };
    if from + size as usize > at {
        return false;
    }
    match parse_directory(directory, entries) {
        Ok(entries) => entries.iter().any(|e| e.name == CONTENT_TYPES || e.name == MIMETYPE),
        Err(_) => false,
    }
}

/// The first entry named by the local header at the start of a file: all that
/// is left to go on when the directory cannot be read.
fn first_name(head: &[u8]) -> Option<&[u8]> {
    let length = usize::from(u16::from_le_bytes([*head.get(26)?, *head.get(27)?]));
    head.get(LOCAL_SIZE..LOCAL_SIZE + length)
}

/// Whether a zip that cannot be read looks like a document that was meant to be
/// read: a broken .docx is worth saying something about, a broken .zip is not.
fn starts_like_document(head: &[u8]) -> bool {
    first_name(head).is_some_and(|name| {
        name == MIMETYPE.as_bytes()
            || name == CONTENT_TYPES.as_bytes()
            || [&b"_rels/"[..], b"docProps/", b"word/", b"xl/", b"ppt/"].iter().any(|prefix| name.starts_with(prefix))
    })
}

/// The directory of a zip file, every entry's span checked.
fn read_directory<R: Read + Seek>(file: &mut R, length: u64) -> Result<(Vec<Entry>, u64, bool), RewriteError> {
    const MALFORMED: RewriteError = RewriteError::Malformed("zip: no readable directory");
    let tail_len = length.min((EOCD_SIZE + 0xFFFF) as u64);
    file.seek(SeekFrom::Start(length - tail_len))?;
    let mut tail = vec![0u8; tail_len as usize];
    file.read_exact(&mut tail)?;
    let (at, size, offset, count) = find_end(&tail).ok_or(MALFORMED)?;
    let end_record = length - tail_len + at as u64;
    if offset.checked_add(size) != Some(end_record) || size > MAX_DIRECTORY {
        return Err(MALFORMED);
    }
    // Anything after the end record, or a comment in it.
    let leftovers = (at + EOCD_SIZE) as u64 != tail_len || u16_at(&tail, at + 20) != 0;

    file.seek(SeekFrom::Start(offset))?;
    let mut directory = vec![0u8; size as usize];
    file.read_exact(&mut directory)?;
    let mut entries = parse_directory(&directory, count)?;
    entries.sort_by_key(|e| e.start);
    if entries.first().is_none_or(|e| e.start != 0) {
        return Err(RewriteError::Malformed("zip: data before the first entry"));
    }
    let ends: Vec<u64> = entries.iter().skip(1).map(|e| e.start).chain(std::iter::once(offset)).collect();
    for (entry, end) in entries.iter_mut().zip(ends) {
        entry.end = end;
    }
    Ok((entries, offset, leftovers))
}

/// Every entry's local header is where the directory says, and its data ends
/// before the next entry starts. Asked only of a document, after its names.
fn verify_spans<R: Read + Seek>(file: &mut R, entries: &[Entry]) -> Result<(), RewriteError> {
    for entry in entries {
        file.seek(SeekFrom::Start(entry.start))?;
        let mut local = [0u8; LOCAL_SIZE];
        file.read_exact(&mut local)?;
        if &local[..4] != LOCAL {
            return Err(RewriteError::Malformed("zip: an entry is not where the directory says"));
        }
        let data = LOCAL_SIZE as u64 + u64::from(u16_at(&local, 26)) + u64::from(u16_at(&local, 28));
        if entry.start + data + entry.compressed > entry.end {
            return Err(RewriteError::Malformed("zip: entries overlap"));
        }
    }
    Ok(())
}

/// The contents of a small entry.
fn read_entry<R: Read + Seek>(file: &mut R, entry: &Entry) -> Result<Vec<u8>, RewriteError> {
    if entry.flags & 1 != 0 || entry.size > MAX_EDITED_PART {
        return Err(RewriteError::Malformed("zip: a part that cannot be read"));
    }
    file.seek(SeekFrom::Start(entry.start))?;
    let mut local = [0u8; LOCAL_SIZE];
    file.read_exact(&mut local)?;
    let skip = i64::from(u16_at(&local, 26)) + i64::from(u16_at(&local, 28));
    file.seek(SeekFrom::Current(skip))?;
    let raw = (&mut *file).take(entry.compressed);
    let mut out = Vec::new();
    match entry.method {
        0 => raw.take(MAX_EDITED_PART + 1).read_to_end(&mut out)?,
        8 => DeflateDecoder::new(raw).take(MAX_EDITED_PART + 1).read_to_end(&mut out)?,
        _ => return Err(RewriteError::Malformed("zip: a compression method not used in documents")),
    };
    if out.len() as u64 != entry.size {
        return Err(RewriteError::Malformed("zip: a part is not the size it says"));
    }
    Ok(out)
}

/// What happens to one entry.
#[derive(Debug, Clone)]
enum Action {
    Copy,
    Drop,
    Write(Vec<u8>),
}

/// Remove from `xml` every element named `tag` whose attribute `attribute` is
/// `target`, with or without a leading slash.
fn without_element(xml: &str, tag: &str, attribute: &str, target: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    let open = format!("<{tag}");
    while let Some(at) = rest.find(&open) {
        let Some(close) = rest[at..].find('>') else {
            break;
        };
        let element = &rest[at..at + close + 1];
        let names_target = [format!("{attribute}=\"{target}\""), format!("{attribute}=\"/{target}\"")]
            .iter()
            .any(|needle| element.contains(needle.as_str()));
        out.push_str(&rest[..at]);
        if !names_target {
            out.push_str(element);
        }
        rest = &rest[at + close + 1..];
    }
    out.push_str(rest);
    out
}

/// `office:version` of a `meta.xml`, when it is a plain number.
fn odf_version(meta: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(meta).ok()?;
    let value = text.split("office:version=\"").nth(1)?.split('"').next()?;
    (!value.is_empty() && value.chars().all(|c| c.is_ascii_digit() || c == '.')).then(|| value.to_string())
}

/// The neutral part, or a copy when the part already is that: a cleaned file
/// cleaned again stays the same file.
fn neutral(entry: &Entry, content: Vec<u8>) -> Action {
    let same = entry.size == content.len() as u64 && entry.crc == crc32fast::hash(&content);
    if same { Action::Copy } else { Action::Write(content) }
}

fn plan_ooxml<R: Read + Seek>(file: &mut R, entries: &[Entry]) -> Result<Vec<Action>, RewriteError> {
    let has_custom = entries.iter().any(|e| e.name == CUSTOM);
    let mut actions = Vec::with_capacity(entries.len());
    for entry in entries {
        actions.push(match entry.name.as_str() {
            CORE => neutral(entry, NEUTRAL_CORE.to_vec()),
            APP => neutral(entry, NEUTRAL_APP.to_vec()),
            CUSTOM | CUSTOM_RELS => Action::Drop,
            CONTENT_TYPES | ROOT_RELS if has_custom => {
                let xml = read_entry(file, entry)?;
                let xml = String::from_utf8(xml).map_err(|_| RewriteError::Malformed("zip: a part is not text"))?;
                let edited = if entry.name == CONTENT_TYPES {
                    without_element(&xml, "Override", "PartName", CUSTOM)
                } else {
                    without_element(&xml, "Relationship", "Target", CUSTOM)
                };
                if edited.contains("custom.xml") {
                    return Err(RewriteError::Malformed("zip: a reference to the custom properties stays"));
                }
                Action::Write(edited.into_bytes())
            }
            _ => Action::Copy,
        });
    }
    Ok(actions)
}

fn plan_odf<R: Read + Seek>(file: &mut R, entries: &[Entry]) -> Result<Vec<Action>, RewriteError> {
    let mut actions = Vec::with_capacity(entries.len());
    for entry in entries {
        actions.push(if entry.name == ODF_META {
            let current = read_entry(file, entry)?;
            let version = odf_version(&current).unwrap_or_else(|| "1.2".to_string());
            neutral(entry, neutral_meta(&version))
        } else {
            Action::Copy
        });
    }
    Ok(actions)
}

/// A new entry: stored, with the zip epoch for a time.
fn write_new_entry<W: Write>(
    output: &mut W,
    name: &str,
    content: &[u8],
    offset: u32,
) -> Result<(u32, Vec<u8>), RewriteError> {
    let (size, crc) = (content.len() as u32, crc32fast::hash(content));
    let mut common = Vec::with_capacity(26);
    common.extend_from_slice(&20u16.to_le_bytes()); // version needed
    common.extend_from_slice(&0u16.to_le_bytes()); // flags
    common.extend_from_slice(&0u16.to_le_bytes()); // stored
    common.extend_from_slice(&0u16.to_le_bytes()); // time
    common.extend_from_slice(&0x0021u16.to_le_bytes()); // 1980-01-01
    common.extend_from_slice(&crc.to_le_bytes());
    common.extend_from_slice(&size.to_le_bytes());
    common.extend_from_slice(&size.to_le_bytes());
    common.extend_from_slice(&(name.len() as u16).to_le_bytes());
    common.extend_from_slice(&0u16.to_le_bytes()); // extra
    output.write_all(LOCAL)?;
    output.write_all(&common)?;
    output.write_all(name.as_bytes())?;
    output.write_all(content)?;

    let mut record = CENTRAL.to_vec();
    record.extend_from_slice(&20u16.to_le_bytes()); // version made by
    record.extend_from_slice(&common);
    record.extend_from_slice(&[0; 2]); // comment
    record.extend_from_slice(&[0; 2]); // disk
    record.extend_from_slice(&[0; 2]); // internal attributes
    record.extend_from_slice(&[0; 4]); // external attributes
    record.extend_from_slice(&offset.to_le_bytes());
    record.extend_from_slice(name.as_bytes());
    let written = (LOCAL_SIZE + name.len() + content.len()) as u32;
    Ok((written, record))
}

/// Write the zip again, entry by entry, in the order of the file.
fn rewrite<R: Read + Seek, W: Write>(
    file: &mut R,
    output: &mut W,
    entries: &[Entry],
    actions: &[Action],
) -> Result<(), RewriteError> {
    let too_large = RewriteError::Malformed("zip: too large");
    let mut offset: u32 = 0;
    let mut directory = Vec::new();
    let mut count: u16 = 0;
    for (entry, action) in entries.iter().zip(actions) {
        match action {
            Action::Drop => continue,
            Action::Copy => {
                file.seek(SeekFrom::Start(entry.start))?;
                let span = entry.end - entry.start;
                pass(file, Some(&mut *output), span, "zip: an entry is cut off")?;
                let mut record = entry.record.clone();
                record[42..46].copy_from_slice(&offset.to_le_bytes());
                directory.extend_from_slice(&record);
                offset = u32::try_from(u64::from(offset) + span).map_err(|_| too_large_error())?;
            }
            Action::Write(content) => {
                let (written, record) = write_new_entry(output, &entry.name, content, offset)?;
                directory.extend_from_slice(&record);
                offset = offset.checked_add(written).ok_or_else(too_large_error)?;
            }
        }
        count = count.checked_add(1).ok_or_else(too_large_error)?;
    }
    output.write_all(&directory)?;
    output.write_all(EOCD)?;
    output.write_all(&[0; 4])?; // disks
    output.write_all(&count.to_le_bytes())?;
    output.write_all(&count.to_le_bytes())?;
    let Ok(size) = u32::try_from(directory.len()) else {
        return Err(too_large);
    };
    output.write_all(&size.to_le_bytes())?;
    output.write_all(&offset.to_le_bytes())?;
    output.write_all(&[0; 2])?; // no comment
    Ok(())
}

fn too_large_error() -> RewriteError {
    RewriteError::Malformed("zip: too large")
}

/// The whole job, over anything seekable. `Ok(None)` when the zip is not a
/// document; `Ok(Some(changed))` otherwise.
fn strip<R: Read + Seek, W: Write>(file: &mut R, length: u64, output: &mut W) -> Result<Option<bool>, Kept> {
    let (entries, _directory_at, leftovers) = read_directory(file, length).map_err(Kept::Unreadable)?;
    let names = |wanted: &str| entries.iter().any(|e| e.name == wanted);

    let first_is_mimetype = entries.first().is_some_and(|e| e.name == MIMETYPE && e.method == 0 && e.size < 256);
    let odf = first_is_mimetype
        && read_entry(file, &entries[0])
            .map(|m| m.starts_with(b"application/vnd.oasis.opendocument."))
            .map_err(Kept::Unreadable)?;
    let ooxml = names(CONTENT_TYPES) && OOXML_MAIN_PARTS.iter().any(|p| names(p));
    if !odf && !ooxml {
        return Ok(None);
    }
    verify_spans(file, &entries).map_err(Kept::Unreadable)?;

    if entries.iter().any(|e| e.name.starts_with("_xmlsignatures/") || e.name == "META-INF/documentsignatures.xml") {
        return Err(Kept::Reason(KeptReason::Signed));
    }
    if odf
        && let Some(manifest) = entries.iter().find(|e| e.name == ODF_MANIFEST)
        && read_entry(file, manifest).map_err(Kept::Unreadable)?.windows(15).any(|w| w == b"encryption-data")
    {
        return Err(Kept::Reason(KeptReason::Protected));
    }

    let actions = if odf { plan_odf(file, &entries) } else { plan_ooxml(file, &entries) }.map_err(Kept::Unreadable)?;
    let changed = leftovers || actions.iter().any(|a| !matches!(a, Action::Copy));
    if changed {
        rewrite(file, output, &entries, &actions).map_err(Kept::Unreadable)?;
    }
    Ok(Some(changed))
}

/// Why [`strip`] stopped.
#[derive(Debug)]
enum Kept {
    /// The zip could not be followed: a problem only if it is a document.
    Unreadable(RewriteError),
    Reason(KeptReason),
}

/// Clean one file. A zip that is not a document is `Ok(false)`, and so is a
/// broken zip that does not start like one.
pub fn strip_file(input: &Path, output: &Path) -> Result<bool, KeptReason> {
    let file = File::open(input).map_err(|_| KeptReason::Tool)?;
    let length = file.metadata().map_err(|_| KeptReason::Tool)?.len();
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut writer = BufWriter::with_capacity(256 * 1024, File::create(output).map_err(|_| KeptReason::Tool)?);
    match strip(&mut reader, length, &mut writer) {
        Ok(Some(changed)) => {
            writer.flush().map_err(|_| KeptReason::Tool)?;
            Ok(changed)
        }
        Ok(None) => Ok(false),
        Err(Kept::Reason(reason)) => Err(reason),
        Err(Kept::Unreadable(err)) => {
            tracing::debug!("[strip] zip: {err}");
            let mut head = [0u8; 512];
            reader.seek(SeekFrom::Start(0)).map_err(|_| KeptReason::Tool)?;
            let got = reader.read(&mut head).map_err(|_| KeptReason::Tool)?;
            if starts_like_document(&head[..got]) { Err(KeptReason::Damaged) } else { Ok(false) }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    struct Part<'a> {
        name: &'a str,
        data: &'a [u8],
        deflated: bool,
    }

    fn stored<'a>(name: &'a str, data: &'a [u8]) -> Part<'a> {
        Part { name, data, deflated: false }
    }

    fn deflated<'a>(name: &'a str, data: &'a [u8]) -> Part<'a> {
        Part { name, data, deflated: true }
    }

    /// A zip as an office suite writes one, with a time on every entry.
    fn zip(parts: &[Part], comment: &[u8]) -> Vec<u8> {
        use flate2::{Compression, write::DeflateEncoder};
        let mut out = Vec::new();
        let mut directory = Vec::new();
        for part in parts {
            let start = out.len() as u32;
            let body = if part.deflated {
                let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
                e.write_all(part.data).unwrap();
                e.finish().unwrap()
            } else {
                part.data.to_vec()
            };
            let mut common = Vec::new();
            common.extend_from_slice(&20u16.to_le_bytes());
            common.extend_from_slice(&0u16.to_le_bytes());
            common.extend_from_slice(&(if part.deflated { 8u16 } else { 0 }).to_le_bytes());
            common.extend_from_slice(&0x6000u16.to_le_bytes()); // a real time...
            common.extend_from_slice(&0x5AA5u16.to_le_bytes()); // ...and date
            common.extend_from_slice(&crc32fast::hash(part.data).to_le_bytes());
            common.extend_from_slice(&(body.len() as u32).to_le_bytes());
            common.extend_from_slice(&(part.data.len() as u32).to_le_bytes());
            common.extend_from_slice(&(part.name.len() as u16).to_le_bytes());
            common.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(LOCAL);
            out.extend_from_slice(&common);
            out.extend_from_slice(part.name.as_bytes());
            out.extend_from_slice(&body);
            directory.extend_from_slice(CENTRAL);
            directory.extend_from_slice(&20u16.to_le_bytes());
            directory.extend_from_slice(&common);
            directory.extend_from_slice(&[0; 10]);
            directory.extend_from_slice(&start.to_le_bytes());
            directory.extend_from_slice(part.name.as_bytes());
        }
        let at = out.len() as u32;
        out.extend_from_slice(&directory);
        out.extend_from_slice(EOCD);
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(parts.len() as u16).to_le_bytes());
        out.extend_from_slice(&(parts.len() as u16).to_le_bytes());
        out.extend_from_slice(&(directory.len() as u32).to_le_bytes());
        out.extend_from_slice(&at.to_le_bytes());
        out.extend_from_slice(&(comment.len() as u16).to_le_bytes());
        out.extend_from_slice(comment);
        out
    }

    fn contents(zip: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut file = Cursor::new(zip.to_vec());
        let (entries, _, _) = read_directory(&mut file, zip.len() as u64).unwrap();
        entries.iter().map(|e| (e.name.clone(), read_entry(&mut file, e).unwrap())).collect()
    }

    fn part<'a>(all: &'a [(String, Vec<u8>)], name: &str) -> Option<&'a [u8]> {
        all.iter().find(|(n, _)| n == name).map(|(_, d)| d.as_slice())
    }

    fn run(input: &[u8]) -> Result<Option<(bool, Vec<u8>)>, Kept> {
        let mut out = Vec::new();
        let changed = strip(&mut Cursor::new(input.to_vec()), input.len() as u64, &mut out)?;
        Ok(changed.map(|c| (c, out)))
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    const TYPES: &[u8] = br#"<?xml version="1.0"?><Types xmlns="x"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="a"/><Override PartName="/docProps/core.xml" ContentType="b"/><Override PartName="/docProps/app.xml" ContentType="c"/><Override ContentType="d" PartName="/docProps/custom.xml"/></Types>"#;
    const RELS: &[u8] = br#"<?xml version="1.0"?><Relationships xmlns="r"><Relationship Id="rId1" Type="t1" Target="word/document.xml"/><Relationship Id="rId2" Type="t2" Target="docProps/core.xml"/><Relationship Id="rId3" Type="t3" Target="docProps/custom.xml"/></Relationships>"#;
    const CORE_XML: &[u8] = b"<cp:coreProperties><dc:creator>PLANTED Jan</dc:creator><dcterms:created>2021</dcterms:created></cp:coreProperties>";
    const APP_XML: &[u8] = b"<Properties><Company>PLANTED Firma</Company><Application>Word</Application></Properties>";
    const CUSTOM_XML: &[u8] = b"<Properties><property name='PLANTED'/></Properties>";
    const BODY: &[u8] = b"<w:document>the words of the document, repeated and repeated</w:document>";

    fn docx() -> Vec<u8> {
        zip(
            &[
                deflated(CONTENT_TYPES, TYPES),
                deflated(ROOT_RELS, RELS),
                deflated("word/document.xml", BODY),
                deflated(CORE, CORE_XML),
                deflated(APP, APP_XML),
                deflated(CUSTOM, CUSTOM_XML),
                stored("word/media/image1.png", b"\x89PNGdata"),
            ],
            b"",
        )
    }

    #[test]
    fn the_properties_of_a_docx_become_neutral_and_the_rest_stays() {
        let (changed, out) = run(&docx()).unwrap().unwrap();
        assert!(changed);
        let all = contents(&out);
        assert_eq!(part(&all, CORE), Some(NEUTRAL_CORE));
        assert_eq!(part(&all, APP), Some(NEUTRAL_APP));
        assert_eq!(part(&all, CUSTOM), None);
        assert_eq!(part(&all, "word/document.xml"), Some(BODY));
        assert_eq!(part(&all, "word/media/image1.png"), Some(&b"\x89PNGdata"[..]));
        assert!(!contains(&out, b"PLANTED"));
    }

    #[test]
    fn the_custom_properties_leave_no_reference_behind() {
        let (_, out) = run(&docx()).unwrap().unwrap();
        let all = contents(&out);
        let types = String::from_utf8(part(&all, CONTENT_TYPES).unwrap().to_vec()).unwrap();
        let rels = String::from_utf8(part(&all, ROOT_RELS).unwrap().to_vec()).unwrap();
        assert!(!types.contains("custom.xml") && types.contains("/docProps/core.xml"), "{types}");
        assert!(!rels.contains("custom.xml") && rels.contains("docProps/core.xml"), "{rels}");
    }

    #[test]
    fn entries_that_stay_are_byte_identical_local_headers_included() {
        let source = docx();
        let (_, out) = run(&source).unwrap().unwrap();
        let mut original = Cursor::new(source.clone());
        let (entries, _, _) = read_directory(&mut original, source.len() as u64).unwrap();
        let body = entries.iter().find(|e| e.name == "word/document.xml").unwrap();
        let span = &source[body.start as usize..body.end as usize];
        assert!(contains(&out, span));
        // The time on the entry is untouched; a replaced part has the zip epoch.
        let mut cleaned = Cursor::new(out.clone());
        let (entries, _, _) = read_directory(&mut cleaned, out.len() as u64).unwrap();
        let core = entries.iter().find(|e| e.name == CORE).unwrap();
        assert_eq!(u16_at(&core.record, 14), 0x0021);
        assert_eq!(entries.iter().find(|e| e.name == "word/document.xml").unwrap().crc, body.crc);
    }

    #[test]
    fn a_clean_file_stays_the_same_file() {
        let (_, once) = run(&docx()).unwrap().unwrap();
        let (changed, _) = run(&once).unwrap().unwrap();
        assert!(!changed);
    }

    #[test]
    fn an_archive_comment_and_trailing_bytes_are_dropped() {
        let parts = [deflated(CONTENT_TYPES, b"<Types/>"), deflated("xl/workbook.xml", b"<workbook/>")];
        assert!(!run(&zip(&parts, b"")).unwrap().unwrap().0);
        let (changed, out) = run(&zip(&parts, b"PLANTED comment")).unwrap().unwrap();
        assert!(changed);
        assert!(!contains(&out, b"PLANTED"));
        assert_eq!(contents(&out).len(), 2);
    }

    fn odt(mimetype_first: bool, meta: &[u8]) -> Vec<u8> {
        let mimetype = stored(MIMETYPE, b"application/vnd.oasis.opendocument.text");
        let content = deflated("content.xml", b"<office:text>body</office:text>");
        let meta = deflated(ODF_META, meta);
        let manifest = deflated(ODF_MANIFEST, b"<manifest:manifest/>");
        if mimetype_first {
            zip(&[mimetype, content, meta, manifest], b"")
        } else {
            zip(&[content, mimetype, meta, manifest], b"")
        }
    }

    const ODF_META_XML: &[u8] = br#"<office:document-meta office:version="1.3"><office:meta><dc:creator>PLANTED</dc:creator></office:meta></office:document-meta>"#;

    #[test]
    fn meta_xml_of_an_odf_is_blanked_and_mimetype_stays_first_and_stored() {
        let source = odt(true, ODF_META_XML);
        let (changed, out) = run(&source).unwrap().unwrap();
        assert!(changed);
        assert!(!contains(&out, b"PLANTED"));
        assert_eq!(&out[..4], LOCAL);
        // The bytes a `file` command looks for, at the offset ODF promises.
        assert_eq!(&out[30..38], b"mimetype");
        assert_eq!(&out[38..77], b"application/vnd.oasis.opendocument.text");
        let all = contents(&out);
        assert_eq!(all[0].0, MIMETYPE);
        let meta = String::from_utf8(part(&all, ODF_META).unwrap().to_vec()).unwrap();
        assert!(meta.contains(r#"office:version="1.3""#) && meta.contains("<office:meta/>"), "{meta}");
        assert_eq!(part(&all, "content.xml"), Some(&b"<office:text>body</office:text>"[..]));
        assert!(!run(&out).unwrap().unwrap().0);
    }

    #[test]
    fn a_signed_package_or_a_protected_odf_is_kept() {
        let signed = zip(
            &[
                deflated(CONTENT_TYPES, TYPES),
                deflated("word/document.xml", BODY),
                stored("_xmlsignatures/sig1.xml", b"x"),
            ],
            b"",
        );
        assert!(matches!(run(&signed), Err(Kept::Reason(KeptReason::Signed))));
        let protected = zip(
            &[
                stored(MIMETYPE, b"application/vnd.oasis.opendocument.text"),
                deflated(ODF_META, ODF_META_XML),
                deflated(ODF_MANIFEST, b"<manifest:encryption-data manifest:checksum='x'/>"),
            ],
            b"",
        );
        assert!(matches!(run(&protected), Err(Kept::Reason(KeptReason::Protected))));
    }

    #[test]
    fn other_zips_are_none_of_this_modules_business() {
        for other in [
            zip(&[deflated("readme.txt", b"hello")], b""),
            zip(&[deflated(CONTENT_TYPES, b"<Types/>"), deflated("nuget/x.nuspec", b"<a/>")], b""),
            zip(&[stored(MIMETYPE, b"application/epub+zip"), deflated("content.opf", b"<a/>")], b""),
            odt(false, ODF_META_XML),
        ] {
            assert!(run(&other).unwrap().is_none());
        }
    }

    #[test]
    fn the_end_of_a_stored_file_says_enough() {
        let doc = docx();
        assert!(may_be_document(&doc, 0));
        let other = zip(&[deflated("readme.txt", b"hello")], b"");
        assert!(!may_be_document(&other, 0));
        assert!(!may_be_document(b"not a zip at all", 0));
        // Only the end record in view: the directory is further back.
        let tail = &doc[doc.len() - EOCD_SIZE..];
        assert!(may_be_document(tail, (doc.len() - EOCD_SIZE) as u64));
    }

    #[test]
    fn replaced_elements_are_found_in_any_attribute_order() {
        let xml = r#"<a><Override ContentType="x" PartName="/docProps/custom.xml"/><Override PartName="/keep"/></a>"#;
        assert_eq!(without_element(xml, "Override", "PartName", CUSTOM), r#"<a><Override PartName="/keep"/></a>"#);
        assert_eq!(without_element("<a><Override broken", "Override", "PartName", CUSTOM), "<a><Override broken");
    }

    #[test]
    fn broken_zips_are_refused_without_panicking() {
        let doc = docx();
        for cut in [0, 3, 21, 22, 30, 100, doc.len() / 2, doc.len() - 23, doc.len() - 1] {
            assert!(matches!(run(&doc[..cut]), Err(Kept::Unreadable(_))), "cut at {cut}");
        }
        // The directory moved, an entry overlapping the next, a lying size.
        let mut moved = doc.clone();
        let at = moved.len() - 6;
        moved[at..at + 4].copy_from_slice(&5u32.to_le_bytes());
        assert!(run(&moved).is_err());
        let mut lying = doc.clone();
        let (_, directory_at, _) = read_directory(&mut Cursor::new(doc.clone()), doc.len() as u64).unwrap();
        let record = directory_at as usize;
        lying[record + 20..record + 24].copy_from_slice(&0x00FF_FFFFu32.to_le_bytes());
        assert!(run(&lying).is_err());
    }

    #[test]
    fn a_broken_zip_says_something_only_when_it_is_a_document() {
        let dir = tempfile::tempdir().unwrap();
        let (input, output) = (dir.path().join("in"), dir.path().join("out"));
        let doc = docx();
        std::fs::write(&input, &doc[..doc.len() / 2]).unwrap();
        assert_eq!(strip_file(&input, &output), Err(KeptReason::Damaged));
        let other = zip(&[deflated("readme.txt", b"hello")], b"");
        std::fs::write(&input, &other[..other.len() / 2]).unwrap();
        assert_eq!(strip_file(&input, &output), Ok(false));
    }
}
