//! The part of JSZip 3.10.1 that `JSZip.loadAsync(buffer)` and
//! `zip.file(name).async("uint8array")` run for mammoth: finding the end of
//! the central directory, walking the central directory and the local
//! headers, naming the entries, and expanding one on request.
//!
//! What JSZip does and the zip guard does not (all reproduced here):
//!
//! - the end of central directory is the LAST `PK\x05\x06` from 4 bytes
//!   before the end (the guard starts 22 bytes before it);
//! - every central record is read while the signature matches, whatever count
//!   the trailer declares; a declared count with no record at all fails;
//! - when the directory ends before the trailer, the difference is taken as
//!   data prepended to the archive and every offset is shifted by it;
//! - 4-byte fields are read as signed 32-bit numbers (`(x << 8) + byte`), so
//!   `0xFFFFFFFF` is -1, the ZIP64 marker;
//! - the entry name comes from the LOCAL header (decoded as UTF-8, or from a
//!   valid Info-ZIP Unicode Path field 0x7075 when the UTF-8 flag is off),
//!   then `..`, `.` and empty segments are resolved; a later entry with the
//!   same name replaces an earlier one;
//! - an entry is a folder (and never found by `file(name)`) when its DOS or
//!   Unix attributes say so or its name ends with `/`;
//! - an entry whose declared size is 0 reads as empty whatever its data;
//! - expanding checks the real size against the declared one ("Bug :
//!   uncompressed data size mismatch") but not the CRC (`checkCRC32` is off);
//! - an encrypted entry or an unknown method fails the whole load.
//!
//! The one deliberate difference: an entry is expanded with a ceiling
//! ([`ZipFile::open`]'s `max_entry`). Every entry the zip guard inflated is
//! within it already; only an entry the guard never saw (a central record
//! beyond the declared count, or the directory JSZip finds after the
//! prepended-data shift) can reach it, and JSZip would expand that one without
//! any bound.

use std::collections::HashMap;

use crate::media::zip_guard::{InflateError, inflate_raw_to_vec};

const LOCAL_FILE_HEADER: [u8; 4] = *b"PK\x03\x04";
const CENTRAL_FILE_HEADER: [u8; 4] = *b"PK\x01\x02";
const CENTRAL_DIRECTORY_END: [u8; 4] = *b"PK\x05\x06";
const ZIP64_CENTRAL_DIRECTORY_LOCATOR: [u8; 4] = *b"PK\x06\x07";
const ZIP64_CENTRAL_DIRECTORY_END: [u8; 4] = *b"PK\x06\x06";

const MAX_VALUE_16BITS: f64 = 65535.0;
/// `utils.MAX_VALUE_32BITS`: "\xFF\xFF\xFF\xFF" is parsed as -1.
const MAX_VALUE_32BITS: f64 = -1.0;

/// Why loading or reading failed (JSZip's message, for debugging only: every
/// failure means the same thing to mammoth's caller).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipError(pub String);

fn err<T>(message: impl Into<String>) -> Result<T, ZipError> {
    Err(ZipError(message.into()))
}

/// `DataReader` over a Node `Buffer` (`NodeBufferReader`).
struct Reader<'a> {
    data: &'a [u8],
    index: f64,
    zero: f64,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Reader { data, index: 0.0, zero: 0.0 }
    }

    fn len(&self) -> f64 {
        self.data.len() as f64
    }

    fn check_index(&self, new_index: f64) -> Result<(), ZipError> {
        if self.len() < self.zero + new_index || new_index < 0.0 {
            return err(format!(
                "End of data reached (data length = {}, asked index = {new_index}). Corrupted zip ?",
                self.data.len()
            ));
        }
        Ok(())
    }

    fn set_index(&mut self, new_index: f64) -> Result<(), ZipError> {
        self.check_index(new_index)?;
        self.index = new_index;
        Ok(())
    }

    fn skip(&mut self, n: f64) -> Result<(), ZipError> {
        self.set_index(self.index + n)
    }

    fn byte_at(&self, i: f64) -> f64 {
        self.data.get((self.zero + i) as usize).copied().unwrap_or(0) as f64
    }

    /// `readInt`: `result = (result << 8) + byte`, high byte first.
    fn read_int(&mut self, size: u32) -> Result<f64, ZipError> {
        self.check_index(self.index + size as f64)?;
        let mut result: f64 = 0.0;
        let mut i = self.index + size as f64 - 1.0;
        while i >= self.index {
            let shifted = (super::js::to_int32(result) as i64) << 8;
            result = super::js::to_int32(shifted as f64) as f64 + self.byte_at(i);
            i -= 1.0;
        }
        self.index += size as f64;
        Ok(result)
    }

    /// `readData`: `Buffer.slice`, which clamps and is empty when reversed.
    fn read_data(&mut self, size: f64) -> Result<&'a [u8], ZipError> {
        self.check_index(self.index + size)?;
        let start = (self.zero + self.index).max(0.0) as usize;
        let end = (self.zero + self.index + size).max(0.0) as usize;
        let start = start.min(self.data.len());
        let end = end.min(self.data.len());
        let out = if end > start { &self.data[start..end] } else { &self.data[0..0] };
        self.index += size;
        Ok(out)
    }

    fn read_and_check_signature(&mut self, sig: [u8; 4]) -> Result<bool, ZipError> {
        let data = self.read_data(4.0)?;
        Ok(data.len() == 4 && data == sig)
    }

    fn last_index_of_signature(&self, sig: [u8; 4]) -> f64 {
        if self.data.len() >= 4 {
            let mut i = self.data.len() - 4;
            loop {
                if self.data[i..i + 4] == sig {
                    return i as f64 - self.zero;
                }
                if i == 0 {
                    break;
                }
                i -= 1;
            }
        }
        -1.0
    }

    fn is_signature(&mut self, asked_index: f64, sig: [u8; 4]) -> Result<bool, ZipError> {
        let current = self.index;
        self.set_index(asked_index)?;
        let signature = self.read_data(4.0)?;
        let result = signature == sig;
        self.set_index(current)?;
        Ok(result)
    }

    fn check_signature(&mut self, sig: [u8; 4]) -> Result<(), ZipError> {
        if !self.read_and_check_signature(sig)? {
            return err("Corrupted zip or bug: unexpected signature");
        }
        Ok(())
    }
}

/// The compressed bytes of one entry and what to do with them.
#[derive(Debug, Clone)]
enum Content {
    /// `isCompressedEmpty`, a folder, or no data: `""`.
    Empty,
    Stored {
        start: usize,
        end: usize,
        uncompressed: f64,
    },
    Deflated {
        start: usize,
        end: usize,
        uncompressed: f64,
    },
}

#[derive(Debug, Clone)]
struct ZipObject {
    dir: bool,
    content: Content,
}

/// `JSZip.loadAsync(buffer)`: the loaded archive.
#[derive(Debug)]
pub struct ZipFile {
    data: bytes::Bytes,
    /// `this.files` (a null-prototype object: any name is an ordinary key).
    files: HashMap<String, ZipObject>,
    max_entry: u64,
}

struct CentralEntry {
    version_made_by: f64,
    bit_flag: f64,
    compression_method: [u8; 2],
    compressed_size: f64,
    uncompressed_size: f64,
    external_file_attributes: f64,
    local_header_offset: f64,
    extra_fields: HashMap<u32, Vec<u8>>,
    // filled by the local part
    file_name: Vec<u8>,
    data_start: usize,
    data_end: usize,
}

/// `crc32` of JSZip (for the Unicode Path extra field).
fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (n, slot) in table.iter_mut().enumerate() {
        let mut c = n as u32;
        for _ in 0..8 {
            c = if c & 1 == 1 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *slot = c;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

/// `Buffer.toString("utf-8")`.
fn utf8_decode(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// `utils.resolve(path)`.
fn resolve(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    let last = parts.len() - 1;
    let mut result: Vec<&str> = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        if *part == "." || (part.is_empty() && index != 0 && index != last) {
            continue;
        } else if *part == ".." {
            result.pop();
        } else {
            result.push(part);
        }
    }
    result.join("/")
}

impl CentralEntry {
    fn read_central_part(reader: &mut Reader<'_>) -> Result<CentralEntry, ZipError> {
        let version_made_by = reader.read_int(2)?;
        reader.skip(2.0)?;
        let bit_flag = reader.read_int(2)?;
        let method = reader.read_data(2.0)?;
        let compression_method = [method.first().copied().unwrap_or(0), method.get(1).copied().unwrap_or(0)];
        let _date = reader.read_int(4)?;
        let _crc32 = reader.read_int(4)?;
        let mut compressed_size = reader.read_int(4)?;
        let mut uncompressed_size = reader.read_int(4)?;
        let file_name_length = reader.read_int(2)?;
        let extra_fields_length = reader.read_int(2)?;
        let file_comment_length = reader.read_int(2)?;
        let disk_number_start = reader.read_int(2)?;
        let _internal = reader.read_int(2)?;
        let external_file_attributes = reader.read_int(4)?;
        let mut local_header_offset = reader.read_int(4)?;

        if (super::js::to_int32(bit_flag) & 0x0001) == 0x0001 {
            return err("Encrypted zip are not supported");
        }

        reader.skip(file_name_length)?;

        // readExtraFields
        let end = reader.index + extra_fields_length;
        let mut extra_fields = HashMap::new();
        while reader.index + 4.0 < end {
            let id = reader.read_int(2)?;
            let length = reader.read_int(2)?;
            let value = reader.read_data(length)?.to_vec();
            extra_fields.insert(id as u32, value);
        }
        reader.set_index(end)?;

        // parseZIP64ExtraField
        if let Some(value) = extra_fields.get(&1) {
            let mut extra = Reader::new(value);
            if uncompressed_size == MAX_VALUE_32BITS {
                uncompressed_size = extra.read_int(8)?;
            }
            if compressed_size == MAX_VALUE_32BITS {
                compressed_size = extra.read_int(8)?;
            }
            if local_header_offset == MAX_VALUE_32BITS {
                local_header_offset = extra.read_int(8)?;
            }
            if disk_number_start == MAX_VALUE_32BITS {
                extra.read_int(4)?;
            }
        }

        reader.read_data(file_comment_length)?;

        Ok(CentralEntry {
            version_made_by,
            bit_flag,
            compression_method,
            compressed_size,
            uncompressed_size,
            external_file_attributes,
            local_header_offset,
            extra_fields,
            file_name: Vec::new(),
            data_start: 0,
            data_end: 0,
        })
    }

    fn read_local_part(&mut self, reader: &mut Reader<'_>) -> Result<(), ZipError> {
        reader.skip(22.0)?;
        let file_name_length = reader.read_int(2)?;
        let local_extra_fields_length = reader.read_int(2)?;
        self.file_name = reader.read_data(file_name_length)?.to_vec();
        reader.skip(local_extra_fields_length)?;

        if self.compressed_size == -1.0 || self.uncompressed_size == -1.0 {
            return err("Bug or corrupted zip : didn't get enough information from the central directory");
        }
        if self.compression_method != [0, 0] && self.compression_method != [8, 0] {
            return err("Corrupted zip : compression unknown");
        }
        let start = (reader.zero + reader.index).max(0.0) as usize;
        let data = reader.read_data(self.compressed_size)?;
        self.data_start = start.min(reader.data.len());
        self.data_end = self.data_start + data.len();
        Ok(())
    }

    fn use_utf8(&self) -> bool {
        (super::js::to_int32(self.bit_flag) & 0x0800) == 0x0800
    }

    /// `findExtraFieldUnicodePath`.
    fn unicode_path(&self) -> Result<Option<String>, ZipError> {
        let Some(value) = self.extra_fields.get(&0x7075) else {
            return Ok(None);
        };
        let mut reader = Reader::new(value);
        if reader.read_int(1)? != 1.0 {
            return Ok(None);
        }
        let stored = reader.read_int(4)?;
        // JSZip's crc32 ends in `crc ^ (-1)`: a signed 32-bit number, as
        // `readInt(4)` reads the stored one.
        if crc32(&self.file_name) as i32 as f64 != stored {
            return Ok(None);
        }
        let rest = reader.read_data(value.len() as f64 - 5.0)?;
        Ok(Some(utf8_decode(rest)))
    }

    /// `handleUTF8`: the name, and the failures reading the comment's
    /// Unicode field can raise (its value is never used).
    fn file_name_str(&self) -> Result<String, ZipError> {
        if self.use_utf8() {
            return Ok(utf8_decode(&self.file_name));
        }
        let name = match self.unicode_path()? {
            Some(upath) => upath,
            None => utf8_decode(&self.file_name),
        };
        // findExtraFieldUnicodeComment: readInt(1), then readInt(4) when the
        // version is 1; only those reads can throw.
        if let Some(value) = self.extra_fields.get(&0x6375)
            && (value.is_empty() || (value[0] == 1 && value.len() < 5))
        {
            return err("End of data reached (unicode comment field). Corrupted zip ?");
        }
        Ok(name)
    }
}

impl ZipFile {
    /// `JSZip.loadAsync(buffer)`; `max_entry` bounds what one entry may
    /// expand to (see the module docs).
    pub fn open(data: bytes::Bytes, max_entry: u64) -> Result<ZipFile, ZipError> {
        let mut files: HashMap<String, ZipObject> = HashMap::new();
        {
            let buf: &[u8] = &data;
            let mut reader = Reader::new(buf);
            let (central_dir_records, central_dir_offset, zip64) = read_end_of_central(&mut reader)?;

            // readCentralDir
            reader.set_index(central_dir_offset)?;
            let mut entries = Vec::new();
            while reader.read_and_check_signature(CENTRAL_FILE_HEADER)? {
                let _ = zip64;
                entries.push(CentralEntry::read_central_part(&mut reader)?);
            }
            if central_dir_records != entries.len() as f64 && central_dir_records != 0.0 && entries.is_empty() {
                return err("Corrupted zip or bug: expected records in central dir, got 0");
            }

            // readLocalFiles
            let mut named = Vec::with_capacity(entries.len());
            for mut entry in entries {
                reader.set_index(entry.local_header_offset)?;
                reader.check_signature(LOCAL_FILE_HEADER)?;
                entry.read_local_part(&mut reader)?;
                let name = entry.file_name_str()?;
                // processAttributes
                let made_by = super::js::to_int32(entry.version_made_by) >> 8;
                let external = super::js::to_int32(entry.external_file_attributes);
                let mut dir = external & 0x0010 != 0;
                let dos_permissions = (made_by == 0x00).then_some(external & 0x3F);
                let unix_permissions = (made_by == 0x03).then_some((external >> 16) & 0xFFFF);
                if !dir && name.ends_with('/') {
                    dir = true;
                }
                named.push((entry, name, dir, dos_permissions, unix_permissions));
            }

            // addFiles: zip.file(safeName, input.decompressed, {...})
            for (entry, name, input_dir, dos_permissions, unix_permissions) in named {
                let unresolved = resolve(&name);
                let mut safe_name = unresolved.clone();
                let mut dir = input_dir;
                if unix_permissions.is_some_and(|p| p != 0 && p & 0x4000 != 0) {
                    dir = true;
                }
                if dos_permissions.is_some_and(|p| p != 0 && p & 0x0010 != 0) {
                    dir = true;
                }
                if dir && !safe_name.ends_with('/') {
                    safe_name.push('/');
                }
                let compressed_empty = entry.uncompressed_size == 0.0;
                let content = if compressed_empty || dir {
                    Content::Empty
                } else if entry.compression_method == [0, 0] {
                    Content::Stored {
                        start: entry.data_start,
                        end: entry.data_end,
                        uncompressed: entry.uncompressed_size,
                    }
                } else {
                    Content::Deflated {
                        start: entry.data_start,
                        end: entry.data_end,
                        uncompressed: entry.uncompressed_size,
                    }
                };
                files.insert(safe_name, ZipObject { dir, content });
                // `if (!input.dir) zip.file(safeName).unsafeOriginalName = ...`:
                // an entry that only became a folder through its Unix mode was
                // stored under "name/", so `file(safeName)` is null and the
                // assignment throws.
                if !input_dir && files.get(&unresolved).is_none_or(|o| o.dir) {
                    return err("Cannot set properties of null (setting 'unsafeOriginalName')");
                }
            }
        }
        Ok(ZipFile { data, files, max_entry })
    }

    /// `zipFile.file(name) !== null`.
    pub fn exists(&self, name: &str) -> bool {
        self.files.get(name).is_some_and(|o| !o.dir)
    }

    /// `zipFile.file(name).async("uint8array")`. A missing name is the
    /// `TypeError` of calling `.async` on `null`.
    pub fn read(&self, name: &str) -> Result<Vec<u8>, ZipError> {
        let Some(object) = self.files.get(name).filter(|o| !o.dir) else {
            return err(format!("Cannot read properties of null (reading 'async'): {name}"));
        };
        match &object.content {
            Content::Empty => Ok(Vec::new()),
            Content::Stored { start, end, uncompressed } => {
                let out = self.data[*start..*end].to_vec();
                check_length(out.len(), *uncompressed)?;
                Ok(out)
            }
            Content::Deflated { start, end, uncompressed } => {
                // pako stops quietly on a broken stream and JSZip only notices
                // through the length check below; a stream the guard never saw
                // that is broken fails here instead (the length could only
                // match by accident).
                let out = inflate_raw_to_vec(&self.data[*start..*end], self.max_entry).map_err(|e| match e {
                    InflateError::TooLarge => ZipError("entry expands beyond the ceiling".into()),
                    InflateError::Truncated | InflateError::Invalid => ZipError("invalid deflate data".into()),
                })?;
                check_length(out.len(), *uncompressed)?;
                Ok(out)
            }
        }
    }
}

fn check_length(len: usize, uncompressed: f64) -> Result<(), ZipError> {
    if len as f64 != uncompressed {
        return err("Bug : uncompressed data size mismatch");
    }
    Ok(())
}

/// `readEndOfCentral`: (centralDirRecords, centralDirOffset, zip64).
fn read_end_of_central(reader: &mut Reader<'_>) -> Result<(f64, f64, bool), ZipError> {
    let offset = reader.last_index_of_signature(CENTRAL_DIRECTORY_END);
    if offset < 0.0 {
        if !reader.is_signature(0.0, LOCAL_FILE_HEADER)? {
            return err("Can't find end of central directory : is this a zip file ?");
        }
        return err("Corrupted zip: can't find end of central directory");
    }
    reader.set_index(offset)?;
    let end_of_central_dir_offset = offset;
    reader.check_signature(CENTRAL_DIRECTORY_END)?;

    // readBlockEndOfCentral
    let disk_number = reader.read_int(2)?;
    let disk_with_central_dir_start = reader.read_int(2)?;
    let central_dir_records_on_this_disk = reader.read_int(2)?;
    let mut central_dir_records = reader.read_int(2)?;
    let mut central_dir_size = reader.read_int(4)?;
    let mut central_dir_offset = reader.read_int(4)?;
    let zip_comment_length = reader.read_int(2)?;
    reader.read_data(zip_comment_length)?;

    let mut zip64 = false;
    let mut zip64_end_of_central_size = 0.0;
    if disk_number == MAX_VALUE_16BITS
        || disk_with_central_dir_start == MAX_VALUE_16BITS
        || central_dir_records_on_this_disk == MAX_VALUE_16BITS
        || central_dir_records == MAX_VALUE_16BITS
        || central_dir_size == MAX_VALUE_32BITS
        || central_dir_offset == MAX_VALUE_32BITS
    {
        zip64 = true;
        let offset = reader.last_index_of_signature(ZIP64_CENTRAL_DIRECTORY_LOCATOR);
        if offset < 0.0 {
            return err("Corrupted zip: can't find the ZIP64 end of central directory locator");
        }
        reader.set_index(offset)?;
        reader.check_signature(ZIP64_CENTRAL_DIRECTORY_LOCATOR)?;
        // readBlockZip64EndOfCentralLocator
        let _disk = reader.read_int(4)?;
        let mut relative_offset = reader.read_int(8)?;
        let disks_count = reader.read_int(4)?;
        if disks_count > 1.0 {
            return err("Multi-volumes zip are not supported");
        }
        if !reader.is_signature(relative_offset, ZIP64_CENTRAL_DIRECTORY_END)? {
            relative_offset = reader.last_index_of_signature(ZIP64_CENTRAL_DIRECTORY_END);
            if relative_offset < 0.0 {
                return err("Corrupted zip: can't find the ZIP64 end of central directory");
            }
        }
        reader.set_index(relative_offset)?;
        reader.check_signature(ZIP64_CENTRAL_DIRECTORY_END)?;
        // readBlockZip64EndOfCentral
        zip64_end_of_central_size = reader.read_int(8)?;
        reader.skip(4.0)?;
        let _disk_number = reader.read_int(4)?;
        let _disk_start = reader.read_int(4)?;
        let _records_on_disk = reader.read_int(8)?;
        central_dir_records = reader.read_int(8)?;
        central_dir_size = reader.read_int(8)?;
        central_dir_offset = reader.read_int(8)?;
        let extra_data_size = zip64_end_of_central_size - 44.0;
        // `while (index < extraDataSize)` with an `index` JSZip never
        // advances: any extensible data loops until a read runs off the end
        // of the buffer. A negative length can walk the reader backwards so
        // that it never does, and the upstream process spins forever; here
        // that is a failure.
        if extra_data_size > 0.0 {
            let mut rounds = 0usize;
            loop {
                rounds += 1;
                if rounds > reader.data.len() + 1 {
                    return err("zip64 extensible data never ends");
                }
                let _id = reader.read_int(2)?;
                let length = reader.read_int(4)?;
                reader.read_data(length)?;
            }
        }
    }

    let mut expected = central_dir_offset + central_dir_size;
    if zip64 {
        expected += 20.0;
        expected += 12.0 + zip64_end_of_central_size;
    }
    let extra_bytes = end_of_central_dir_offset - expected;
    if extra_bytes > 0.0 {
        if !reader.is_signature(end_of_central_dir_offset, CENTRAL_FILE_HEADER)? {
            reader.zero = extra_bytes;
        }
    } else if extra_bytes < 0.0 {
        return err(format!("Corrupted zip: missing {} bytes.", extra_bytes.abs()));
    }
    Ok((central_dir_records, central_dir_offset, zip64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_names_like_jszip() {
        assert_eq!(resolve("word//document.xml"), "word/document.xml");
        assert_eq!(resolve("./word/./a.xml"), "word/a.xml");
        assert_eq!(resolve("/word/a.xml"), "/word/a.xml");
        assert_eq!(resolve("a/../b.xml"), "b.xml");
        assert_eq!(resolve("../../b.xml"), "b.xml");
        assert_eq!(resolve("dir/"), "dir/");
    }

    #[test]
    fn reads_signed_32_bit_fields() {
        let data = [0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x80];
        let mut r = Reader::new(&data);
        assert_eq!(r.read_int(4).unwrap(), -1.0);
        assert_eq!(r.read_int(4).unwrap(), -2_147_483_648.0);
    }
}
