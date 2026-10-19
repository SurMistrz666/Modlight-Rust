//! The ZIP writer behind `app/a/[userCode]/[slug]/zip/route.ts`: what
//! `archiver("zip", { zlib: { level: 6 } })` 8.0.0 (zip-stream 7.0.5,
//! compress-commons 7.0.1) writes for entries appended as STREAMS, which is
//! the only way the route appends them.
//!
//! Per entry, in order, exactly as compress-commons lays it out:
//! - local file header: version needed 20 (data descriptor), general purpose
//!   bit 3 (sizes follow the data) plus bit 11 when the name is not ASCII,
//!   method 8 (deflate), the DOS time of the moment the entry was appended
//!   (UTC), CRC and sizes zero, no extra field;
//! - the raw deflate stream (zlib level 6);
//! - data descriptor: signature, CRC-32, compressed and uncompressed size
//!   (8 bytes each when either exceeds 0xFFFFFFFF, the ZIP64 rule);
//!
//! then the central directory (version made by 0x032D: Unix, 4.5; external
//! attributes `0644 | S_IFREG` << 16 | archive bit; a ZIP64 extra field for an
//! entry over 4 GiB or at an offset past 4 GiB) and the end record (with the
//! ZIP64 end record and locator when the archive needs them).
//!
//! Names go through archiver's `sanitizePath` three times, as the three
//! layers each apply it (archiver core, zip-stream, the entry itself).
//! The deflate bytes are the Rust `flate2` encoder's, not Node's zlib, so the
//! archive is equivalent rather than byte-identical (it carries a timestamp
//! anyway); every field that describes the entries is the same.

use std::io::Write;

use bytes::Bytes;
use chrono::{DateTime, Datelike, Timelike, Utc};
use flate2::Compression;
use flate2::write::DeflateEncoder;

const SIG_LFH: u32 = 0x0403_4b50;
const SIG_DD: u32 = 0x0807_4b50;
const SIG_CFH: u32 = 0x0201_4b50;
const SIG_EOCD: u32 = 0x0605_4b50;
const SIG_ZIP64_EOCD: u32 = 0x0606_4b50;
const SIG_ZIP64_EOCD_LOC: u32 = 0x0706_4b50;
const MIN_VERSION_DATA_DESCRIPTOR: u16 = 20;
const MIN_VERSION_ZIP64: u16 = 45;
const VERSION_MADEBY: u16 = 45;
const PLATFORM_UNIX: u16 = 3;
const METHOD_DEFLATED: u16 = 8;
const ZIP64_MAGIC: u64 = 0xffff_ffff;
const ZIP64_MAGIC_SHORT: u64 = 0xffff;
const ZIP64_EXTRA_ID: u16 = 0x0001;
const DATA_DESCRIPTOR_FLAG: u16 = 1 << 3;
const UTF8_NAMES_FLAG: u16 = 1 << 11;
/// `setUnixMode(0644)`: `(0644 | S_IFREG) << 16 | S_DOS_A`.
const FILE_EXTERNAL_ATTRIBUTES: u32 = ((0o644 | 0o100000) << 16) | 0x20;

/// `dateToDos(d, false)`: UTC fields; years before 1980 and from 2044 on are
/// clamped to the format's ends.
pub fn date_to_dos(d: DateTime<Utc>) -> u32 {
    let year = d.year();
    if year < 1980 {
        return 2_162_688;
    }
    if year >= 2044 {
        return 2_141_175_677;
    }
    (((year - 1980) as u32) << 25)
        | (d.month() << 21)
        | (d.day() << 16)
        | (d.hour() << 11)
        | (d.minute() << 5)
        | (d.second() / 2)
}

/// `normalize-path` 3 with `stripTrailing = false`.
fn normalize_path(path: &str) -> String {
    if path == "\\" || path == "/" {
        return "/".to_string();
    }
    let units: Vec<char> = path.chars().collect();
    if units.len() <= 1 {
        return path.to_string();
    }
    let mut rest = path;
    let mut prefix = "";
    if units.len() > 4 && units[3] == '\\' && (units[2] == '?' || units[2] == '.') && path.starts_with("\\\\") {
        rest = &path[2..];
        prefix = "//";
    }
    let mut out = String::from(prefix);
    let mut in_sep = false;
    for c in rest.chars() {
        if c == '/' || c == '\\' {
            if !in_sep {
                out.push('/');
                in_sep = true;
            }
        } else {
            out.push(c);
            in_sep = false;
        }
    }
    out
}

/// zip-stream's `sanitizePath`: `normalizePath(p, false).replace(/^\w+:/, "")
/// .replace(/^(\.\.\/|\/)+/, "")`.
pub fn sanitize_path(path: &str) -> String {
    let normalized = normalize_path(path);
    let word = normalized.bytes().take_while(|b| b.is_ascii_alphanumeric() || *b == b'_').count();
    let mut s = if word > 0 && normalized.as_bytes().get(word) == Some(&b':') {
        &normalized[word + 1..]
    } else {
        normalized.as_str()
    };
    loop {
        if let Some(r) = s.strip_prefix("../") {
            s = r;
        } else if let Some(r) = s.strip_prefix('/') {
            s = r;
        } else {
            break;
        }
    }
    s.to_string()
}

/// The name an entry ends up with: archiver core, zip-stream and the entry
/// each sanitize it once.
pub fn entry_name(name: &str) -> String {
    sanitize_path(&sanitize_path(&sanitize_path(name)))
}

struct Entry {
    name: Vec<u8>,
    utf8: bool,
    dos_time: u32,
    crc: u32,
    size: u64,
    csize: u64,
    offset: u64,
}

impl Entry {
    fn is_zip64(&self) -> bool {
        self.csize > ZIP64_MAGIC || self.size > ZIP64_MAGIC
    }
}

/// A ZIP written as it streams: [`ZipWriter::begin_entry`], any number of
/// [`ZipWriter::write`], [`ZipWriter::end_entry`], then [`ZipWriter::finish`].
/// Each call returns the bytes to send next.
pub struct ZipWriter {
    entries: Vec<Entry>,
    offset: u64,
    current: Option<(Entry, DeflateEncoder<Vec<u8>>, crc32fast::Hasher)>,
    level: u32,
}

impl ZipWriter {
    /// `archiver("zip", { zlib: { level } })`
    pub fn new(level: u32) -> ZipWriter {
        ZipWriter { entries: Vec::new(), offset: 0, current: None, level }
    }

    fn emit(&mut self, buf: Vec<u8>) -> Bytes {
        self.offset += buf.len() as u64;
        Bytes::from(buf)
    }

    /// The local file header of a stream entry named `name` (already through
    /// [`entry_name`]; must not be empty), dated `date`.
    pub fn begin_entry(&mut self, name: &str, date: DateTime<Utc>) -> Bytes {
        let utf8 = !name.is_ascii();
        let entry = Entry {
            name: name.as_bytes().to_vec(),
            utf8,
            dos_time: date_to_dos(date),
            crc: 0,
            size: 0,
            csize: 0,
            offset: self.offset,
        };
        let flags = DATA_DESCRIPTOR_FLAG | if utf8 { UTF8_NAMES_FLAG } else { 0 };
        let mut h = Vec::with_capacity(30 + entry.name.len());
        h.extend_from_slice(&SIG_LFH.to_le_bytes());
        h.extend_from_slice(&MIN_VERSION_DATA_DESCRIPTOR.to_le_bytes());
        h.extend_from_slice(&flags.to_le_bytes());
        h.extend_from_slice(&METHOD_DEFLATED.to_le_bytes());
        h.extend_from_slice(&entry.dos_time.to_le_bytes());
        h.extend_from_slice(&[0u8; 12]);
        h.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes());
        h.extend_from_slice(&entry.name);
        let encoder = DeflateEncoder::new(Vec::new(), Compression::new(self.level));
        self.current = Some((entry, encoder, crc32fast::Hasher::new()));
        self.emit(h)
    }

    /// Feed bytes of the current entry; returns whatever compressed output is
    /// ready (possibly nothing).
    pub fn write(&mut self, data: &[u8]) -> std::io::Result<Bytes> {
        let Some((entry, encoder, hasher)) = self.current.as_mut() else {
            return Ok(Bytes::new());
        };
        hasher.update(data);
        entry.size += data.len() as u64;
        encoder.write_all(data)?;
        let out = std::mem::take(encoder.get_mut());
        entry.csize += out.len() as u64;
        Ok(self.emit(out))
    }

    /// Finish the current entry: the rest of the deflate stream and the data
    /// descriptor.
    pub fn end_entry(&mut self) -> std::io::Result<Bytes> {
        let Some((mut entry, encoder, hasher)) = self.current.take() else {
            return Ok(Bytes::new());
        };
        let mut out = encoder.finish()?;
        entry.csize += out.len() as u64;
        entry.crc = hasher.finalize();
        out.extend_from_slice(&SIG_DD.to_le_bytes());
        out.extend_from_slice(&entry.crc.to_le_bytes());
        if entry.is_zip64() {
            out.extend_from_slice(&entry.csize.to_le_bytes());
            out.extend_from_slice(&entry.size.to_le_bytes());
        } else {
            out.extend_from_slice(&(entry.csize as u32).to_le_bytes());
            out.extend_from_slice(&(entry.size as u32).to_le_bytes());
        }
        self.entries.push(entry);
        Ok(self.emit(out))
    }

    /// The central directory and the end records.
    pub fn finish(&mut self) -> Bytes {
        let central_offset = self.offset;
        let mut out = Vec::new();
        for e in &self.entries {
            let flags = DATA_DESCRIPTOR_FLAG | if e.utf8 { UTF8_NAMES_FLAG } else { 0 };
            let (mut version, mut csize, mut size, mut offset) =
                (MIN_VERSION_DATA_DESCRIPTOR, e.csize, e.size, e.offset);
            let mut extra = Vec::new();
            if e.is_zip64() || e.offset > ZIP64_MAGIC {
                version = MIN_VERSION_ZIP64;
                (csize, size, offset) = (ZIP64_MAGIC, ZIP64_MAGIC, ZIP64_MAGIC);
                extra.extend_from_slice(&ZIP64_EXTRA_ID.to_le_bytes());
                extra.extend_from_slice(&24u16.to_le_bytes());
                extra.extend_from_slice(&e.size.to_le_bytes());
                extra.extend_from_slice(&e.csize.to_le_bytes());
                extra.extend_from_slice(&e.offset.to_le_bytes());
            }
            out.extend_from_slice(&SIG_CFH.to_le_bytes());
            out.extend_from_slice(&((PLATFORM_UNIX << 8) | VERSION_MADEBY).to_le_bytes());
            out.extend_from_slice(&version.to_le_bytes());
            out.extend_from_slice(&flags.to_le_bytes());
            out.extend_from_slice(&METHOD_DEFLATED.to_le_bytes());
            out.extend_from_slice(&e.dos_time.to_le_bytes());
            out.extend_from_slice(&e.crc.to_le_bytes());
            out.extend_from_slice(&(csize as u32).to_le_bytes());
            out.extend_from_slice(&(size as u32).to_le_bytes());
            out.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
            out.extend_from_slice(&(extra.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // comment
            out.extend_from_slice(&0u16.to_le_bytes()); // disk number start
            out.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
            out.extend_from_slice(&FILE_EXTERNAL_ATTRIBUTES.to_le_bytes());
            out.extend_from_slice(&(offset as u32).to_le_bytes());
            out.extend_from_slice(&e.name);
            out.extend_from_slice(&extra);
        }
        let central_length = out.len() as u64;
        let count = self.entries.len() as u64;
        let zip64 = count > ZIP64_MAGIC_SHORT || central_length > ZIP64_MAGIC || central_offset > ZIP64_MAGIC;
        if zip64 {
            out.extend_from_slice(&SIG_ZIP64_EOCD.to_le_bytes());
            out.extend_from_slice(&44u64.to_le_bytes());
            out.extend_from_slice(&MIN_VERSION_ZIP64.to_le_bytes());
            out.extend_from_slice(&MIN_VERSION_ZIP64.to_le_bytes());
            out.extend_from_slice(&[0u8; 8]);
            out.extend_from_slice(&count.to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
            out.extend_from_slice(&central_length.to_le_bytes());
            out.extend_from_slice(&central_offset.to_le_bytes());
            out.extend_from_slice(&SIG_ZIP64_EOCD_LOC.to_le_bytes());
            out.extend_from_slice(&[0u8; 4]);
            out.extend_from_slice(&(central_offset + central_length).to_le_bytes());
            out.extend_from_slice(&1u32.to_le_bytes());
        }
        let (records, size, offset) =
            if zip64 { (ZIP64_MAGIC_SHORT, ZIP64_MAGIC, ZIP64_MAGIC) } else { (count, central_length, central_offset) };
        out.extend_from_slice(&SIG_EOCD.to_le_bytes());
        out.extend_from_slice(&[0u8; 4]);
        out.extend_from_slice(&(records as u16).to_le_bytes());
        out.extend_from_slice(&(records as u16).to_le_bytes());
        out.extend_from_slice(&(size as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // comment
        self.emit(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn dos_time_like_compress_commons() {
        let t = Utc.with_ymd_and_hms(2026, 10, 20, 13, 45, 59).unwrap();
        // ((2026-1980)<<25)|(10<<21)|(20<<16)|(13<<11)|(45<<5)|(59/2|0)
        assert_eq!(date_to_dos(t), (46 << 25) | (10 << 21) | (20 << 16) | (13 << 11) | (45 << 5) | 29);
        assert_eq!(date_to_dos(Utc.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap()), 2_162_688);
        assert_eq!(date_to_dos(Utc.with_ymd_and_hms(2050, 1, 1, 0, 0, 0).unwrap()), 2_141_175_677);
    }

    #[test]
    fn names_are_sanitized_like_archiver() {
        assert_eq!(entry_name("photo.png"), "photo.png");
        assert_eq!(entry_name("C:report.pdf"), "report.pdf");
        // Three layers, three strips.
        assert_eq!(entry_name("a:b:c:d.txt"), "d.txt");
        assert_eq!(entry_name("a:b:c:d:e.txt"), "d:e.txt");
        assert_eq!(entry_name("../../x"), "x");
        assert_eq!(entry_name("//a\\\\b"), "a/b");
        assert_eq!(entry_name("x:"), "");
    }

    #[test]
    fn a_round_trip_through_a_reader() {
        let date = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 6).unwrap();
        let mut w = ZipWriter::new(6);
        let mut zip = Vec::new();
        for (name, body) in [("a.txt", b"hello hello hello".to_vec()), ("zażółć.bin", vec![7u8; 100_000])] {
            zip.extend_from_slice(&w.begin_entry(name, date));
            for chunk in body.chunks(4096) {
                zip.extend_from_slice(&w.write(chunk).unwrap());
            }
            zip.extend_from_slice(&w.end_entry().unwrap());
        }
        zip.extend_from_slice(&w.finish());

        // End record: 2 entries, central directory where it says.
        let eocd = zip.len() - 22;
        assert_eq!(&zip[eocd..eocd + 4], &SIG_EOCD.to_le_bytes());
        assert_eq!(u16::from_le_bytes([zip[eocd + 8], zip[eocd + 9]]), 2);
        let cd_size = u32::from_le_bytes(zip[eocd + 12..eocd + 16].try_into().unwrap()) as usize;
        let cd_offset = u32::from_le_bytes(zip[eocd + 16..eocd + 20].try_into().unwrap()) as usize;
        assert_eq!(cd_offset + cd_size, eocd);

        // Walk the central directory and inflate every entry from its local header.
        let mut p = cd_offset;
        let mut seen = Vec::new();
        while p < eocd {
            assert_eq!(&zip[p..p + 4], &SIG_CFH.to_le_bytes());
            assert_eq!(u16::from_le_bytes([zip[p + 4], zip[p + 5]]), 0x032d);
            let flags = u16::from_le_bytes([zip[p + 8], zip[p + 9]]);
            let crc = u32::from_le_bytes(zip[p + 16..p + 20].try_into().unwrap());
            let csize = u32::from_le_bytes(zip[p + 20..p + 24].try_into().unwrap()) as usize;
            let size = u32::from_le_bytes(zip[p + 24..p + 28].try_into().unwrap()) as usize;
            let name_len = u16::from_le_bytes([zip[p + 28], zip[p + 29]]) as usize;
            let attrs = u32::from_le_bytes(zip[p + 38..p + 42].try_into().unwrap());
            let local = u32::from_le_bytes(zip[p + 42..p + 46].try_into().unwrap()) as usize;
            let name = String::from_utf8(zip[p + 46..p + 46 + name_len].to_vec()).unwrap();
            assert_eq!(attrs, 0x81a4_0020);
            assert_eq!(flags & DATA_DESCRIPTOR_FLAG, DATA_DESCRIPTOR_FLAG);
            assert_eq!(flags & UTF8_NAMES_FLAG != 0, !name.is_ascii());
            let data = local + 30 + name_len;
            let mut inflated = Vec::new();
            std::io::Read::read_to_end(&mut flate2::read::DeflateDecoder::new(&zip[data..data + csize]), &mut inflated)
                .unwrap();
            assert_eq!(inflated.len(), size);
            assert_eq!(crc32fast::hash(&inflated), crc);
            // The data descriptor repeats the three numbers.
            let dd = data + csize;
            assert_eq!(&zip[dd..dd + 4], &SIG_DD.to_le_bytes());
            assert_eq!(u32::from_le_bytes(zip[dd + 4..dd + 8].try_into().unwrap()), crc);
            seen.push((name, size));
            p += 46 + name_len;
        }
        assert_eq!(seen, vec![("a.txt".to_string(), 17), ("zażółć.bin".to_string(), 100_000)]);
    }

    #[test]
    fn an_empty_archive_is_valid() {
        let mut w = ZipWriter::new(6);
        let out = w.finish();
        assert_eq!(out.len(), 22);
        assert_eq!(&out[..4], &SIG_EOCD.to_le_bytes());
    }
}
