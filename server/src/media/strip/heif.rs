//! HEIC and AVIF (HEIF): the metadata items are overwritten in place.
//!
//! A HEIF file keeps EXIF and XMP as items of its `meta` box, whose bytes `iloc`
//! locates in `mdat` or `idat`. Those bytes are replaced where they are, at the
//! same length: EXIF by a minimal block that holds only the Orientation tag (or
//! none), XMP by an empty packet, and the rest of each item by zeros. No box
//! moves and no offset changes, so the coded picture, its rotation (`irot`,
//! `imir`), its colour profile and its depth and gain maps are the same bytes as
//! before, and no codec is needed: the server's libvips cannot decode HEVC.
//!
//! An image sequence (`avis`, `msf1`) also has a movie box: its creation and
//! modification times are zeroed, and its `udta` and `meta` boxes become `free`
//! boxes full of zeros.

use std::io::{Read, Seek, SeekFrom, Write};

use super::jpeg::{exif_orientation, minimal_exif};
use super::{RewriteError, pass, read_all};

/// The largest `meta` or `moov` box that is read into memory. A HEIC's is a few
/// kilobytes.
const MAX_BOX_IN_MEMORY: u64 = 16 * 1024 * 1024;

/// Items `iinf` and `iloc` may list. A grid photo with its tiles, thumbnails,
/// depth and gain maps has a few hundred; the checks below compare items with
/// items, so a meta box of millions of empty ones never finished.
const MAX_ITEMS: usize = 4096;

/// Extents `iloc` may list, all items together. A grid photo has a few dozen;
/// with offset and length fields of size 0 an extent costs no bytes of the
/// file, so without a ceiling eight bytes of `iloc` asked for a megabyte.
const MAX_EXTENTS: usize = 16_384;

/// Top-level boxes a picture may have. A HEIC has four or five; every one is
/// remembered, and a box costs only eight bytes of the file.
const MAX_TOP_LEVEL_BOXES: usize = 4096;

/// An empty XMP packet.
const EMPTY_XMP: &[u8] = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"/>"#;

/// One run of bytes to replace: its offset in the file and the new bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    pub at: u64,
    pub bytes: Vec<u8>,
}

/// A box header: type, where its contents start and where it ends.
#[derive(Debug, Clone, Copy)]
struct BoxHeader {
    kind: [u8; 4],
    start: u64,
    body: u64,
    end: u64,
}

/// Read the boxes of a buffer that starts at file offset `base`.
fn boxes_in(buf: &[u8], base: u64) -> Result<Vec<BoxHeader>, RewriteError> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < buf.len() {
        let header = buf.get(at..at + 8).ok_or(RewriteError::Malformed("HEIF: a box header is cut off"))?;
        let size = u64::from(u32::from_be_bytes([header[0], header[1], header[2], header[3]]));
        let kind = [header[4], header[5], header[6], header[7]];
        let (length, header_len) = match size {
            0 => ((buf.len() - at) as u64, 8),
            1 => {
                let large = buf.get(at + 8..at + 16).ok_or(RewriteError::Malformed("HEIF: a box header is cut off"))?;
                (u64::from_be_bytes(large.try_into().unwrap_or_default()), 16)
            }
            n => (n, 8),
        };
        let header_len = if &kind == b"uuid" { header_len + 16 } else { header_len };
        // A 64-bit size near the top wraps the sum: checked, or the walk would
        // jump back to an earlier box and go round for ever.
        let fits = (at as u64).checked_add(length).is_some_and(|end| end <= buf.len() as u64);
        if length < header_len as u64 || !fits {
            return Err(RewriteError::Malformed("HEIF: a box does not fit its parent"));
        }
        out.push(BoxHeader {
            kind,
            start: base + at as u64,
            body: base + (at + header_len) as u64,
            end: base + at as u64 + length,
        });
        at += length as usize;
    }
    Ok(out)
}

/// The contents of a box out of a buffer that starts at file offset `base`.
fn body<'a>(buf: &'a [u8], base: u64, b: &BoxHeader) -> &'a [u8] {
    &buf[(b.body - base) as usize..(b.end - base) as usize]
}

/// A cursor over the bytes of one box.
struct Fields<'a> {
    buf: &'a [u8],
    at: usize,
}

impl<'a> Fields<'a> {
    fn new(buf: &'a [u8]) -> Fields<'a> {
        Fields { buf, at: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], RewriteError> {
        let end = self.at.checked_add(n).ok_or(RewriteError::Malformed("HEIF: a field is cut off"))?;
        let out = self.buf.get(self.at..end).ok_or(RewriteError::Malformed("HEIF: a field is cut off"))?;
        self.at = end;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, RewriteError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, RewriteError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, RewriteError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// An unsigned number of 0, 4 or 8 bytes (the sizes `iloc` allows).
    fn sized(&mut self, size: u8) -> Result<u64, RewriteError> {
        match size {
            0 => Ok(0),
            4 => Ok(u64::from(self.u32()?)),
            8 => {
                let b = self.take(8)?;
                Ok(u64::from_be_bytes(b.try_into().unwrap_or_default()))
            }
            _ => Err(RewriteError::Malformed("HEIF: a field size iloc does not allow")),
        }
    }

    /// A NUL-terminated string.
    fn string(&mut self) -> Result<&'a [u8], RewriteError> {
        let rest = self.buf.get(self.at..).unwrap_or_default();
        let len = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        self.at = (self.at + len + 1).min(self.buf.len());
        Ok(&rest[..len])
    }
}

/// What an item holds, as far as this module cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemKind {
    Exif,
    Xmp,
    Other,
}

/// `infe` boxes: item id and kind.
fn item_kinds(iinf: &[u8]) -> Result<Vec<(u32, ItemKind)>, RewriteError> {
    let mut f = Fields::new(iinf);
    let version = f.u8()?;
    f.take(3)?;
    let _count = if version == 0 { u32::from(f.u16()?) } else { f.u32()? };
    let offset = f.at;
    let mut out = Vec::new();
    for infe in boxes_in(&iinf[offset..], offset as u64)? {
        if &infe.kind != b"infe" {
            continue;
        }
        let mut f = Fields::new(body(iinf, 0, &infe));
        let version = f.u8()?;
        f.take(3)?;
        let (id, kind) = if version >= 2 {
            let id = if version == 2 { u32::from(f.u16()?) } else { f.u32()? };
            f.u16()?; // protection index
            let item_type = f.take(4)?;
            let kind = match item_type {
                b"Exif" => ItemKind::Exif,
                b"mime" => {
                    f.string()?; // item name
                    if is_xmp_type(f.string()?) { ItemKind::Xmp } else { ItemKind::Other }
                }
                _ => ItemKind::Other,
            };
            (id, kind)
        } else {
            let id = u32::from(f.u16()?);
            f.u16()?;
            f.string()?;
            (id, if is_xmp_type(f.string()?) { ItemKind::Xmp } else { ItemKind::Other })
        };
        out.push((id, kind));
        if out.len() > MAX_ITEMS {
            return Err(RewriteError::Malformed("HEIF: more items than a picture has"));
        }
    }
    Ok(out)
}

fn is_xmp_type(content_type: &[u8]) -> bool {
    let t = String::from_utf8_lossy(content_type).to_ascii_lowercase();
    t.contains("rdf+xml") || t.contains("xmp")
}

/// Where one item's bytes are.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Location {
    id: u32,
    /// 0: the file, 1: `idat`, 2: another item.
    method: u8,
    extents: Vec<(u64, u64)>,
}

/// `iloc`, versions 0 to 2.
fn locations(iloc: &[u8]) -> Result<Vec<Location>, RewriteError> {
    let mut f = Fields::new(iloc);
    let version = f.u8()?;
    if version > 2 {
        return Err(RewriteError::Malformed("HEIF: an iloc version this does not know"));
    }
    f.take(3)?;
    let sizes = f.u8()?;
    let (offset_size, length_size) = (sizes >> 4, sizes & 0x0F);
    let sizes = f.u8()?;
    let base_size = sizes >> 4;
    let index_size = if version >= 1 { sizes & 0x0F } else { 0 };
    let count = if version < 2 { u32::from(f.u16()?) } else { f.u32()? };
    if u64::from(count) > MAX_ITEMS as u64 {
        return Err(RewriteError::Malformed("HEIF: more items than a picture has"));
    }
    let mut out = Vec::new();
    let mut listed = 0usize;
    for _ in 0..count {
        let id = if version < 2 { u32::from(f.u16()?) } else { f.u32()? };
        let method = if version >= 1 { (f.u16()? & 0x0F) as u8 } else { 0 };
        f.u16()?; // data reference index
        let base = f.sized(base_size)?;
        let extent_count = f.u16()?;
        listed += usize::from(extent_count);
        if listed > MAX_EXTENTS {
            return Err(RewriteError::Malformed("HEIF: more extents than a picture has"));
        }
        let mut extents = Vec::with_capacity(usize::from(extent_count).min(64));
        for _ in 0..extent_count {
            f.sized(index_size)?;
            let offset = f.sized(offset_size)?;
            let length = f.sized(length_size)?;
            let start = base.checked_add(offset).ok_or(RewriteError::Malformed("HEIF: an extent past the end"))?;
            extents.push((start, length));
        }
        out.push(Location { id, method, extents });
    }
    Ok(out)
}

/// What an item's bytes become: `content`, then zeros.
fn replacement(kind: ItemKind, original: Option<&[u8]>, length: u64) -> Vec<u8> {
    let content: Vec<u8> = match kind {
        ItemKind::Exif => {
            // `exif_tiff_header_offset`, then the bytes it skips, then TIFF.
            let orientation = original.and_then(|data| {
                let skip = u32::from_be_bytes(data.get(..4)?.try_into().ok()?) as usize;
                exif_orientation(data.get(4usize.checked_add(skip)?..)?)
            });
            let mut v = 6u32.to_be_bytes().to_vec();
            let minimal = minimal_exif(orientation.filter(|&o| o != 1).unwrap_or(1));
            if orientation.is_some_and(|o| o != 1) {
                v.extend_from_slice(&minimal);
            } else {
                // No tag at all: "Exif\0\0", the TIFF header and an empty IFD.
                v.extend_from_slice(&minimal[..6 + 8]);
                v.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
            }
            v
        }
        ItemKind::Xmp => EMPTY_XMP.to_vec(),
        ItemKind::Other => Vec::new(),
    };
    let length = length as usize;
    let mut out = if content.len() <= length { content } else { Vec::new() };
    out.resize(length, 0);
    out
}

/// Patches for the `meta` box of a HEIF file, read into `meta` (which starts
/// at file offset `base`, with its header). `read_at` reads bytes of the file
/// outside the box (an EXIF item in `mdat`), bounded by the item's length.
///
/// A metadata item may only be overwritten where nothing else is: inside the
/// payload of a top-level `mdat` (`payloads`) or of `idat`, and away from the
/// bytes of every other item. A file whose items say otherwise is refused,
/// because writing there would break its structure or its picture.
fn meta_patches(
    meta: &[u8],
    header: &BoxHeader,
    payloads: &[(u64, u64)],
    read_at: &mut dyn FnMut(u64, u64) -> Result<Vec<u8>, RewriteError>,
) -> Result<Vec<Patch>, RewriteError> {
    let base = header.start;
    let inner = body(meta, base, header);
    if inner.len() < 4 {
        return Err(RewriteError::Malformed("HEIF: meta is cut off"));
    }
    let children = boxes_in(&inner[4..], header.body + 4)?;
    let find = |kind: &[u8; 4]| children.iter().find(|b| &b.kind == kind);
    let handler = find(b"hdlr").ok_or(RewriteError::Malformed("HEIF: meta has no handler"))?;
    if body(meta, base, handler).get(8..12) != Some(b"pict") {
        return Err(RewriteError::Malformed("HEIF: not a picture"));
    }
    let Some(iinf) = find(b"iinf") else {
        return Ok(Vec::new());
    };
    let kinds = item_kinds(body(meta, base, iinf))?;
    if kinds.iter().all(|(_, k)| *k == ItemKind::Other) {
        return Ok(Vec::new());
    }
    let iloc = find(b"iloc").ok_or(RewriteError::Malformed("HEIF: items without locations"))?;
    let locations = locations(body(meta, base, iloc))?;
    let idat = find(b"idat");
    let mut allowed = payloads.to_vec();
    if let Some(idat) = idat {
        allowed.push((idat.body, idat.end));
    }
    // Where an extent is in the file: `(start, end)`.
    let span = |method: u8, offset: u64, length: u64| -> Result<(u64, u64), RewriteError> {
        if length == 0 {
            return Err(RewriteError::Malformed("HEIF: an extent without a length"));
        }
        let start = match method {
            0 => offset,
            1 => idat
                .ok_or(RewriteError::Malformed("HEIF: an item in a missing idat"))?
                .body
                .checked_add(offset)
                .ok_or(RewriteError::Malformed("HEIF: past idat"))?,
            _ => return Err(RewriteError::Malformed("HEIF: an item built from another item")),
        };
        let end = start.checked_add(length).ok_or(RewriteError::Malformed("HEIF: an extent past the end"))?;
        Ok((start, end))
    };
    // The bytes of every other item: never to be touched.
    let mut protected = Vec::new();
    for location in &locations {
        let other = kinds.iter().any(|(i, k)| *i == location.id && *k == ItemKind::Other)
            || !kinds.iter().any(|(i, _)| *i == location.id);
        if other && location.method <= 1 {
            for &(offset, length) in &location.extents {
                if length > 0
                    && let Ok(s) = span(location.method, offset, length)
                {
                    protected.push(s);
                }
            }
        }
    }

    let mut patches = Vec::new();
    // What all the replacements hold together. Each item is bounded, but items
    // may name the same bytes (overlaps are refused only once every patch is
    // made), so thousands of them over one region asked for thousands of copies.
    let mut held: u64 = 0;
    for (id, kind) in kinds.iter().copied().filter(|(_, k)| *k != ItemKind::Other) {
        let Some(location) = locations.iter().find(|l| l.id == id) else {
            continue;
        };
        // The bytes of the item, extent by extent, in the file.
        let mut spans = Vec::new();
        for &(offset, length) in &location.extents {
            let (start, end) = span(location.method, offset, length)?;
            if !allowed.iter().any(|&(from, to)| from <= start && end <= to) {
                return Err(RewriteError::Malformed("HEIF: a metadata item outside the media data"));
            }
            if protected.iter().any(|&(from, to)| start < to && from < end) {
                return Err(RewriteError::Malformed("HEIF: a metadata item shares bytes with another item"));
            }
            spans.push((start, length));
        }
        let total: u64 = spans.iter().map(|s| s.1).sum();
        held = held.saturating_add(total);
        if total > MAX_BOX_IN_MEMORY || held > MAX_BOX_IN_MEMORY {
            return Err(RewriteError::Malformed("HEIF: a metadata item too large"));
        }
        // The item as one run of bytes, to read the orientation out of it.
        let original = if kind == ItemKind::Exif {
            let mut data = Vec::with_capacity(total as usize);
            for &(at, length) in &spans {
                if at >= base && at + length <= base + meta.len() as u64 {
                    data.extend_from_slice(&meta[(at - base) as usize..(at - base + length) as usize]);
                } else {
                    data.extend_from_slice(&read_at(at, length)?);
                }
            }
            Some(data)
        } else {
            None
        };
        let mut new = replacement(kind, original.as_deref(), total);
        for (at, length) in spans {
            let rest = new.split_off(length as usize);
            patches.push(Patch { at, bytes: new });
            new = rest;
        }
    }
    Ok(patches)
}

/// Patches for a movie box (or a box inside one, `header`, out of `moov`, the
/// movie box read whole from file offset `base`): times zeroed, `udta` and
/// `meta` emptied.
fn moov_patches(moov: &[u8], base: u64, header: &BoxHeader, depth: usize) -> Result<Vec<Patch>, RewriteError> {
    if depth > 8 {
        return Err(RewriteError::Malformed("HEIF: boxes nested too deep"));
    }
    let mut patches = Vec::new();
    for child in boxes_in(body(moov, base, header), header.body)? {
        match &child.kind {
            b"mvhd" | b"tkhd" | b"mdhd" => {
                let data = body(moov, base, &child);
                let width = match data.first() {
                    Some(0) => 4,
                    Some(1) => 8,
                    _ => return Err(RewriteError::Malformed("HEIF: a header version this does not know")),
                };
                if data.len() < 4 + 2 * width {
                    return Err(RewriteError::Malformed("HEIF: a header is cut off"));
                }
                patches.push(Patch { at: child.body + 4, bytes: vec![0; 2 * width] });
            }
            b"udta" | b"meta" => {
                patches.push(Patch { at: child.start + 4, bytes: b"free".to_vec() });
                patches.push(Patch { at: child.body, bytes: vec![0; (child.end - child.body) as usize] });
            }
            b"trak" | b"mdia" | b"edts" | b"minf" => {
                patches.extend(moov_patches(moov, base, &child, depth + 1)?);
            }
            _ => {}
        }
    }
    Ok(patches)
}

/// Read one top-level box whole.
fn read_box<R: Read + Seek>(input: &mut R, header: &BoxHeader) -> Result<Vec<u8>, RewriteError> {
    let length = header.end - header.start;
    if length > MAX_BOX_IN_MEMORY {
        return Err(RewriteError::Malformed("HEIF: a box too large to read"));
    }
    input.seek(SeekFrom::Start(header.start))?;
    let mut buf = vec![0u8; length as usize];
    read_all(input, &mut buf, "HEIF: a box is cut off")?;
    Ok(buf)
}

/// The top-level boxes of a file of `length` bytes, headers only.
fn top_level<R: Read + Seek>(input: &mut R, length: u64) -> Result<Vec<BoxHeader>, RewriteError> {
    let mut out = Vec::new();
    let mut at = 0u64;
    while at < length {
        if out.len() >= MAX_TOP_LEVEL_BOXES {
            return Err(RewriteError::Malformed("HEIF: more boxes than a picture has"));
        }
        input.seek(SeekFrom::Start(at))?;
        let mut header = [0u8; 16];
        let available = (length - at).min(16) as usize;
        read_all(input, &mut header[..available], "HEIF: a box header is cut off")?;
        let size = u64::from(u32::from_be_bytes(header[..4].try_into().unwrap_or_default()));
        let box_len = match size {
            0 => length - at,
            1 if available >= 16 => u64::from_be_bytes(header[8..16].try_into().unwrap_or_default()),
            1 => return Err(RewriteError::Malformed("HEIF: a box header is cut off")),
            n => n,
        };
        if box_len < 8 || at.checked_add(box_len).is_none_or(|end| end > length) {
            return Err(RewriteError::Malformed("HEIF: a box runs past the end of the file"));
        }
        let header_len = if size == 1 { 16 } else { 8 } + if &header[4..8] == b"uuid" { 16 } else { 0 };
        if box_len < header_len {
            return Err(RewriteError::Malformed("HEIF: a box shorter than its header"));
        }
        out.push(BoxHeader {
            kind: [header[4], header[5], header[6], header[7]],
            start: at,
            body: at + header_len,
            end: at + box_len,
        });
        at += box_len;
    }
    Ok(out)
}

/// Every patch a HEIF file needs, sorted, none overlapping.
pub fn plan<R: Read + Seek>(input: &mut R, length: u64) -> Result<Vec<Patch>, RewriteError> {
    let boxes = top_level(input, length)?;
    if boxes.first().is_none_or(|b| &b.kind != b"ftyp") {
        return Err(RewriteError::Malformed("HEIF: ftyp must come first"));
    }
    let meta = boxes.iter().find(|b| &b.kind == b"meta").ok_or(RewriteError::Malformed("HEIF: no meta box"))?;
    let meta_bytes = read_box(input, meta)?;
    let mut read_at = |at: u64, len: u64| -> Result<Vec<u8>, RewriteError> {
        if at.checked_add(len).is_none_or(|end| end > length) {
            return Err(RewriteError::Malformed("HEIF: an item past the end of the file"));
        }
        input.seek(SeekFrom::Start(at))?;
        let mut buf = vec![0u8; len as usize];
        read_all(input, &mut buf, "HEIF: an item is cut off")?;
        Ok(buf)
    };
    let payloads: Vec<(u64, u64)> = boxes.iter().filter(|b| &b.kind == b"mdat").map(|b| (b.body, b.end)).collect();
    let mut patches = meta_patches(&meta_bytes, meta, &payloads, &mut read_at)?;
    for moov in boxes.iter().filter(|b| &b.kind == b"moov") {
        let bytes = read_box(input, moov)?;
        patches.extend(moov_patches(&bytes, moov.start, moov, 0)?);
    }
    patches.retain(|p| !p.bytes.is_empty());
    patches.sort_by_key(|p| p.at);
    for pair in patches.windows(2) {
        if pair[0].at + pair[0].bytes.len() as u64 > pair[1].at {
            return Err(RewriteError::Malformed("HEIF: two items share bytes"));
        }
    }
    if patches.last().is_some_and(|p| p.at + p.bytes.len() as u64 > length) {
        return Err(RewriteError::Malformed("HEIF: an item past the end of the file"));
    }
    Ok(patches)
}

/// Copy the file with its patches applied. `Ok(false)`: nothing to change.
pub fn strip<R: Read + Seek, W: Write>(input: &mut R, output: &mut W) -> Result<bool, RewriteError> {
    let length = input.seek(SeekFrom::End(0))?;
    let patches = plan(input, length)?;
    // An item that already is what it would become changes nothing.
    let mut changed = false;
    for patch in &patches {
        input.seek(SeekFrom::Start(patch.at))?;
        let mut current = vec![0u8; patch.bytes.len()];
        read_all(input, &mut current, "HEIF: an item is cut off")?;
        changed |= current != patch.bytes;
    }
    if !changed {
        return Ok(false);
    }
    input.seek(SeekFrom::Start(0))?;
    let mut at = 0u64;
    for patch in &patches {
        pass(input, Some(&mut *output), patch.at - at, "HEIF: the file is cut off")?;
        output.write_all(&patch.bytes)?;
        pass(input, None::<&mut W>, patch.bytes.len() as u64, "HEIF: the file is cut off")?;
        at = patch.at + patch.bytes.len() as u64;
    }
    pass(input, Some(&mut *output), length - at, "HEIF: the file is cut off")?;
    Ok(true)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Cursor;

    use super::*;

    pub fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(body);
        v
    }

    fn full(kind: &[u8; 4], version: u8, body: &[u8]) -> Vec<u8> {
        let mut v = vec![version, 0, 0, 0];
        v.extend_from_slice(body);
        bx(kind, &v)
    }

    fn infe(id: u16, item_type: &[u8; 4], extra: &[u8]) -> Vec<u8> {
        let mut v = id.to_be_bytes().to_vec();
        v.extend_from_slice(&[0, 0]);
        v.extend_from_slice(item_type);
        v.extend_from_slice(extra);
        full(b"infe", 2, &v)
    }

    /// Exif payload: offset 6, "Exif\0\0", TIFF with Orientation and an Artist.
    pub fn exif_item(orientation: u16) -> Vec<u8> {
        let mut v = 6u32.to_be_bytes().to_vec();
        v.extend_from_slice(b"Exif\0\0");
        v.extend_from_slice(b"II*\0\x08\0\0\0\x02\0");
        v.extend_from_slice(&0x0112u16.to_le_bytes());
        v.extend_from_slice(&3u16.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&u32::from(orientation).to_le_bytes());
        v.extend_from_slice(&0x013Bu16.to_le_bytes()); // Artist
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&14u32.to_le_bytes());
        v.extend_from_slice(&38u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(b"PLANTED-NAME\0\0");
        v
    }

    pub const XMP: &[u8] = b"<x:xmpmeta xmlns:x='adobe:ns:meta/'>PLANTED-GPS 52.2297</x:xmpmeta>";
    pub const PICTURE: &[u8] = b"coded picture data that must not change";

    /// A HEIF file: picture (1), EXIF (2) and XMP (3) in mdat, or EXIF in idat.
    pub fn heif(brand: &[u8; 4], exif_in_idat: bool, orientation: u16) -> Vec<u8> {
        let exif = exif_item(orientation);
        let ftyp = bx(b"ftyp", &[&brand[..], b"\0\0\0\0mif1", brand].concat());
        let hdlr = full(b"hdlr", 0, &[0, 0, 0, 0, b'p', b'i', b'c', b't', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let mut iinf_body = 3u16.to_be_bytes().to_vec();
        iinf_body.extend(infe(1, b"hvc1", b""));
        iinf_body.extend(infe(2, b"Exif", b""));
        iinf_body.extend(infe(3, b"mime", b"\0application/rdf+xml\0"));
        let iinf = full(b"iinf", 0, &iinf_body);
        let idat = if exif_in_idat { bx(b"idat", &exif) } else { Vec::new() };
        // iloc v1, offset/length 4 bytes, base 0, index 0; offsets filled below.
        let iloc_len = 8 + 4 + 2 + 2 + 3 * (2 + 2 + 2 + 2 + 8);
        let meta_len_without_iloc = 12 + hdlr.len() + iinf.len() + idat.len();
        let meta_len = meta_len_without_iloc + iloc_len;
        let mdat_start = ftyp.len() + meta_len;
        let picture_at = (mdat_start + 8) as u32;
        let exif_at = picture_at + PICTURE.len() as u32;
        let xmp_at = exif_at + if exif_in_idat { 0 } else { exif.len() as u32 };
        let mut iloc_body = vec![0x44, 0x00];
        iloc_body.extend_from_slice(&3u16.to_be_bytes());
        let mut item = |id: u16, method: u16, offset: u32, length: usize| {
            iloc_body.extend_from_slice(&id.to_be_bytes());
            iloc_body.extend_from_slice(&method.to_be_bytes());
            iloc_body.extend_from_slice(&0u16.to_be_bytes());
            iloc_body.extend_from_slice(&1u16.to_be_bytes());
            iloc_body.extend_from_slice(&offset.to_be_bytes());
            iloc_body.extend_from_slice(&(length as u32).to_be_bytes());
        };
        item(1, 0, picture_at, PICTURE.len());
        if exif_in_idat {
            item(2, 1, 0, exif.len());
        } else {
            item(2, 0, exif_at, exif.len());
        }
        item(3, 0, xmp_at, XMP.len());
        let iloc = full(b"iloc", 1, &iloc_body);
        assert_eq!(iloc.len(), iloc_len);
        let meta = full(b"meta", 0, &[hdlr, iinf, iloc, idat].concat());
        assert_eq!(meta.len(), meta_len);
        let mut mdat_body = PICTURE.to_vec();
        if !exif_in_idat {
            mdat_body.extend_from_slice(&exif);
        }
        mdat_body.extend_from_slice(XMP);
        [ftyp, meta, bx(b"mdat", &mdat_body)].concat()
    }

    pub fn run(input: &[u8]) -> Result<(bool, Vec<u8>), RewriteError> {
        let mut out = Vec::new();
        let changed = strip(&mut Cursor::new(input), &mut out)?;
        Ok((changed, out))
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn exif_and_xmp_are_blanked_where_they_are() {
        for exif_in_idat in [false, true] {
            let input = heif(b"heic", exif_in_idat, 1);
            let (changed, out) = run(&input).unwrap();
            assert!(changed);
            assert_eq!(out.len(), input.len());
            assert!(!contains(&out, b"PLANTED"), "{exif_in_idat}");
            assert!(contains(&out, PICTURE));
            assert!(contains(&out, EMPTY_XMP));
            // Every byte that changed belongs to a metadata item.
            let differing = input.iter().zip(&out).filter(|(a, b)| a != b).count();
            assert!(differing <= exif_item(1).len() + XMP.len());
            // Cleaning again changes nothing.
            assert!(!run(&out).unwrap().0);
        }
    }

    #[test]
    fn a_rotation_in_exif_is_kept() {
        let (_, out) = run(&heif(b"heic", false, 6)).unwrap();
        let mut f = Cursor::new(out.clone());
        let length = out.len() as u64;
        let patches = plan(&mut f, length).unwrap();
        assert!(patches.is_empty() || !run(&out).unwrap().0);
        let start = out.windows(9).position(|w| w == b"Exif\0\0II*").unwrap();
        assert_eq!(exif_orientation(&out[start + 6..]), Some(6));
        assert!(!out.windows(7).any(|w| w == b"PLANTED"));
    }

    #[test]
    fn avif_is_the_same_container() {
        let (changed, out) = run(&heif(b"avif", false, 1)).unwrap();
        assert!(changed && !contains(&out, b"PLANTED"));
    }

    #[test]
    fn a_sequence_loses_its_times_and_user_data() {
        let mvhd = full(b"mvhd", 0, &[0, 0, 0, 7, 0, 0, 0, 9, 0, 0, 0, 1, 0, 0, 0, 2]);
        let udta = bx(b"udta", &bx(b"\xa9xyz", b"+52.2297+021.0122/"));
        let tkhd = full(b"tkhd", 1, &[0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0, 9, 0, 0, 0, 1]);
        let trak = bx(b"trak", &tkhd);
        let moov = bx(b"moov", &[mvhd, trak, udta].concat());
        let mut input = heif(b"avis", false, 1);
        input.extend(moov);
        let (changed, out) = run(&input).unwrap();
        assert!(changed);
        assert!(!contains(&out, b"52.2297"));
        assert!(contains(&out, b"free"));
        assert_eq!(out.len(), input.len());
    }

    #[test]
    fn a_file_without_metadata_items_is_left_alone() {
        let ftyp = bx(b"ftyp", b"heic\0\0\0\0mif1heic");
        let hdlr = full(b"hdlr", 0, &[0, 0, 0, 0, b'p', b'i', b'c', b't', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let mut iinf_body = 1u16.to_be_bytes().to_vec();
        iinf_body.extend(infe(1, b"hvc1", b""));
        let meta = full(b"meta", 0, &[hdlr, full(b"iinf", 0, &iinf_body)].concat());
        let input = [ftyp, meta, bx(b"mdat", PICTURE)].concat();
        let (changed, out) = run(&input).unwrap();
        assert!(!changed);
        assert!(out.is_empty());
    }

    /// The file offset field of item `id`'s first extent in `iloc` (v1, 4-byte fields).
    fn extent_offset_at(file: &[u8], id: u16) -> usize {
        let iloc = file.windows(4).position(|w| w == b"iloc").unwrap();
        // version/flags, sizes, base/index sizes, item count.
        let mut at = iloc + 4 + 4 + 2 + 2;
        loop {
            let item = u16::from_be_bytes([file[at], file[at + 1]]);
            if item == id {
                return at + 2 + 2 + 2 + 2;
            }
            at += 2 + 2 + 2 + 2 + 8;
        }
    }

    #[test]
    fn an_item_aimed_at_the_picture_or_at_the_boxes_is_refused() {
        let file = heif(b"heic", false, 1);
        let picture = file.windows(PICTURE.len()).position(|w| w == PICTURE).unwrap() as u32;
        let at = extent_offset_at(&file, 2);
        let mut onto_picture = file.clone();
        onto_picture[at..at + 4].copy_from_slice(&picture.to_be_bytes());
        assert!(matches!(run(&onto_picture), Err(RewriteError::Malformed(m)) if m.contains("another item")));
        let mut onto_boxes = file.clone();
        let iinf = file.windows(4).position(|w| w == b"iinf").unwrap() as u32;
        onto_boxes[at..at + 4].copy_from_slice(&(iinf - 4).to_be_bytes());
        assert!(matches!(run(&onto_boxes), Err(RewriteError::Malformed(m)) if m.contains("outside the media data")));
        assert_eq!(file, heif(b"heic", false, 1), "the input is never written to");
    }

    #[test]
    fn a_box_size_that_wraps_around_is_refused() {
        // At offset 8, a box of 2^64 - 8 bytes: the sum wraps to 0, which
        // a release build took as "fits", and the walk went back to the first
        // box and round again for ever, collecting headers until memory ran out.
        let mut children = bx(b"free", &[]);
        children.extend_from_slice(&1u32.to_be_bytes());
        children.extend_from_slice(b"free");
        children.extend_from_slice(&(u64::MAX - 7).to_be_bytes());
        assert!(matches!(boxes_in(&children, 0), Err(RewriteError::Malformed(_))));

        // The same inside the meta box of a whole file.
        let ftyp = bx(b"ftyp", b"heic\0\0\0\0mif1heic");
        let hdlr = full(b"hdlr", 0, &[0, 0, 0, 0, b'p', b'i', b'c', b't', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let mut wrap = 1u32.to_be_bytes().to_vec();
        wrap.extend_from_slice(b"free");
        wrap.extend_from_slice(&(u64::MAX - hdlr.len() as u64 + 1).to_be_bytes());
        let meta = full(b"meta", 0, &[hdlr, wrap].concat());
        let input = [ftyp, meta, bx(b"mdat", PICTURE)].concat();
        assert!(matches!(run(&input), Err(RewriteError::Malformed(_))));
    }

    #[test]
    fn extents_that_cost_no_bytes_are_bounded() {
        // iloc v1 with offset, length, base and index sizes all 0: every item
        // takes eight bytes and lists 65535 extents that take none. Twenty items
        // were 1.3 million extents (21 MB) out of 172 bytes; a 16 MB meta box
        // asked for terabytes.
        let items = 20u16;
        let mut iloc = vec![1, 0, 0, 0, 0x00, 0x00];
        iloc.extend_from_slice(&items.to_be_bytes());
        for id in 1..=items {
            iloc.extend_from_slice(&id.to_be_bytes());
            iloc.extend_from_slice(&[0, 0, 0, 0]); // construction method, data reference
            iloc.extend_from_slice(&u16::MAX.to_be_bytes());
        }
        assert!(matches!(locations(&iloc), Err(RewriteError::Malformed(_))));
        // A real picture's handful of extents is read as before.
        let file = heif(b"heic", false, 1);
        assert!(plan(&mut Cursor::new(file.clone()), file.len() as u64).is_ok());
    }

    #[test]
    fn items_are_bounded_in_number() {
        // iloc v1, every field size 0: an item with no extent costs eight bytes.
        let items = (MAX_ITEMS + 1) as u16;
        let mut iloc = vec![1, 0, 0, 0, 0x00, 0x00];
        iloc.extend_from_slice(&items.to_be_bytes());
        for id in 1..=items {
            iloc.extend_from_slice(&id.to_be_bytes());
            iloc.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        }
        assert!(matches!(locations(&iloc), Err(RewriteError::Malformed(_))));
        let mut iinf = vec![0, 0, 0, 0];
        iinf.extend_from_slice(&items.to_be_bytes());
        for id in 1..=items {
            iinf.extend(infe(id, b"hvc1", b""));
        }
        assert!(matches!(item_kinds(&iinf), Err(RewriteError::Malformed(_))));
    }

    #[test]
    fn metadata_items_over_the_same_bytes_cannot_multiply_the_memory() {
        // Forty XMP items, each over the same megabyte of mdat: each became a
        // megabyte of replacement before the overlap was noticed at the end.
        let (k, region) = (40u16, 1024 * 1024usize);
        let ftyp = bx(b"ftyp", b"heic\0\0\0\0mif1heic");
        let hdlr = full(b"hdlr", 0, &[0, 0, 0, 0, b'p', b'i', b'c', b't', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let mut iinf_body = k.to_be_bytes().to_vec();
        for id in 1..=k {
            iinf_body.extend(infe(id, b"mime", b"\0application/rdf+xml\0"));
        }
        let iinf = full(b"iinf", 0, &iinf_body);
        let iloc_len = 8 + 4 + 2 + 2 + usize::from(k) * 16;
        let meta_len = 12 + hdlr.len() + iinf.len() + iloc_len;
        let region_at = (ftyp.len() + meta_len + 8) as u32;
        let mut iloc_body = vec![0x44, 0x00];
        iloc_body.extend_from_slice(&k.to_be_bytes());
        for id in 1..=k {
            iloc_body.extend_from_slice(&id.to_be_bytes());
            iloc_body.extend_from_slice(&[0, 0, 0, 0, 0, 1]);
            iloc_body.extend_from_slice(&region_at.to_be_bytes());
            iloc_body.extend_from_slice(&(region as u32).to_be_bytes());
        }
        let iloc = full(b"iloc", 1, &iloc_body);
        assert_eq!(iloc.len(), iloc_len);
        let meta = full(b"meta", 0, &[hdlr, iinf, iloc].concat());
        assert_eq!(meta.len(), meta_len);
        let file = [ftyp, meta, bx(b"mdat", &vec![0u8; region])].concat();

        let mut input = Cursor::new(file.clone());
        let boxes = top_level(&mut input, file.len() as u64).unwrap();
        let meta = *boxes.iter().find(|b| &b.kind == b"meta").unwrap();
        let bytes = read_box(&mut input, &meta).unwrap();
        let payloads: Vec<(u64, u64)> = boxes.iter().filter(|b| &b.kind == b"mdat").map(|b| (b.body, b.end)).collect();
        let mut read_at = |_: u64, _: u64| -> Result<Vec<u8>, RewriteError> { Ok(Vec::new()) };
        assert!(matches!(meta_patches(&bytes, &meta, &payloads, &mut read_at), Err(RewriteError::Malformed(_))));
    }

    #[test]
    fn a_file_of_endless_tiny_boxes_is_refused_without_remembering_them_all() {
        let mut file = bx(b"ftyp", b"heic\0\0\0\0mif1heic");
        for _ in 0..MAX_TOP_LEVEL_BOXES + 10 {
            file.extend(bx(b"free", &[]));
        }
        let length = file.len() as u64;
        assert!(matches!(top_level(&mut Cursor::new(file), length), Err(RewriteError::Malformed(_))));
    }

    #[test]
    fn broken_layouts_are_refused() {
        let full_file = heif(b"heic", false, 6);
        for cut in [0, 4, 8, 20, 40, 80, 120, 200, full_file.len() - 30, full_file.len() - 1] {
            assert!(run(&full_file[..cut.min(full_file.len())]).is_err(), "cut at {cut}");
        }
        // An item pointing past the end of the file.
        let mut lying = full_file.clone();
        let at = lying.windows(4).position(|w| w == b"iloc").unwrap();
        let first_offset = at + 4 + 4 + 2 + 2 + 2 + 2 + 2 + 2;
        lying[first_offset + 16 + 6..first_offset + 16 + 10].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(run(&lying).is_err());
        // Not a picture.
        let mut video = full_file.clone();
        let at = video.windows(4).position(|w| w == b"pict").unwrap();
        video[at..at + 4].copy_from_slice(b"vide");
        assert!(run(&video).is_err());
    }
}
