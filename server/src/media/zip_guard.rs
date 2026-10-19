//! `lib/media/zip-guard.ts`: deciding whether a ZIP is safe to hand to a
//! library that will expand all of it.
//!
//! A .docx is a ZIP, and mammoth takes the whole archive as a buffer and unpacks
//! what it needs through JSZip - pure JavaScript, on the main thread, with no
//! output bound anywhere in the chain. A 10 MB upload of highly compressible
//! nothing expands to gigabytes, and the process is dead before any timeout can
//! fire, because synchronous JavaScript cannot be interrupted.
//!
//! So the check has to happen BEFORE mammoth sees the buffer, and it cannot be a
//! question the file answers about itself.
//!
//! The lazy version of this guard reads the sizes recorded in the central
//! directory and adds them up. That is worth doing - it is nearly free and it
//! catches every naive bomb - but it is a guard against accidents, not against an
//! attacker, because those fields are just numbers in the file. The archive can
//! say each entry expands to 4 kB and expand to 4 GB.
//!
//! So the declared sizes are treated as a claim to be checked, and the archive is
//! really inflated here, entry by entry, with an output bound (Node's
//! `inflateRawSync(.., { maxOutputLength })` upstream, [`inflate_raw_bounded`]
//! here, with the same answers: zlib stops and fails at the limit rather than
//! returning what it has produced, and a truncated or invalid stream fails).
//! The cost is decompressing twice, which for the 2.3 MB that is the 90th
//! percentile of the documents on this deployment is a few milliseconds, and it
//! turns "the file claims to be safe" into "the file is safe".

use flate2::{Decompress, FlushDecompress, Status};

/// Signatures, little-endian.
const EOCD: u32 = 0x0605_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;

/// A ZIP comment may be 64 kB, so the trailer can be that far from the end.
const MAX_COMMENT: usize = 0xffff;

/// The sentinel a ZIP64 archive writes where a 32-bit size would go.
const ZIP64_SENTINEL: u32 = 0xffff_ffff;

/// `ZipLimits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZipLimits {
    /// Everything in the archive, expanded, may not exceed this.
    pub total_bytes: u64,
    /// ...and no single entry may.
    pub entry_bytes: u64,
    /// Number of entries. A million empty files is also a denial of service.
    pub entry_count: u64,
}

/// The `ok: true` side of `ZipVerdict`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZipSummary {
    pub total_bytes: u64,
    pub entries: usize,
}

/// `ZipRefusal`, with the upstream tokens from [`ZipRefusal::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZipRefusal {
    NotAZip,
    /// Structurally broken: truncated, overlapping, or lying about its offsets.
    Malformed,
    /// Declared or real expansion beyond what we allow.
    TooBig,
    TooManyEntries,
    /// Needs 64-bit offsets, so it is at least 4 GB and not a document.
    Zip64,
    /// A compression method we do not implement (and neither should a .docx).
    UnsupportedMethod,
}

impl ZipRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            ZipRefusal::NotAZip => "not-a-zip",
            ZipRefusal::Malformed => "malformed",
            ZipRefusal::TooBig => "too-big",
            ZipRefusal::TooManyEntries => "too-many-entries",
            ZipRefusal::Zip64 => "zip64",
            ZipRefusal::UnsupportedMethod => "unsupported-method",
        }
    }
}

/// `ZipVerdict`: `{ ok: true, totalBytes, entries }` or `{ ok: false, reason }`.
pub type ZipVerdict = Result<ZipSummary, ZipRefusal>;

struct Entry {
    method: u16,
    compressed: u32,
    declared: u32,
    local_offset: u32,
}

fn u16_at(buf: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([buf[at], buf[at + 1]])
}

fn u32_at(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]])
}

/// Find the End of Central Directory record, scanning back from the end.
///
/// Returns the offset of the central directory and how many entries it holds.
/// `None` both when there is no record and when it needs ZIP64 (upstream
/// reports both as `zip64`).
fn read_eocd(buf: &[u8]) -> Option<(usize, u64)> {
    let earliest = buf.len().saturating_sub(MAX_COMMENT + 22);
    let mut i = buf.len() - 22;
    loop {
        if u32_at(buf, i) == EOCD {
            let count = u16_at(buf, i + 10);
            let offset = u32_at(buf, i + 16);
            let size = u32_at(buf, i + 12);
            if offset == ZIP64_SENTINEL || size == ZIP64_SENTINEL || count == 0xffff {
                return None;
            }
            if offset as u64 + size as u64 > buf.len() as u64 {
                return None;
            }
            return Some((offset as usize, count as u64));
        }
        if i == earliest {
            return None;
        }
        i -= 1;
    }
}

/// Walk the central directory into a list of entries, or `None` if it is broken.
fn read_central_directory(buf: &[u8], start: usize, count: u64) -> Option<Vec<Entry>> {
    let mut entries = Vec::with_capacity(count as usize);
    let mut p = start;

    for _ in 0..count {
        if p + 46 > buf.len() || u32_at(buf, p) != CENTRAL {
            return None;
        }

        let method = u16_at(buf, p + 10);
        let compressed = u32_at(buf, p + 20);
        let declared = u32_at(buf, p + 24);
        let name_len = u16_at(buf, p + 28) as usize;
        let extra_len = u16_at(buf, p + 30) as usize;
        let comment_len = u16_at(buf, p + 32) as usize;
        let local_offset = u32_at(buf, p + 42);

        // Any ZIP64 sentinel means the real value lives in an extra field. A
        // document that needs one is at least 4 GB, so rather than implement the
        // parsing we refuse the whole class - and refusing is the safe direction.
        if compressed == ZIP64_SENTINEL || declared == ZIP64_SENTINEL || local_offset == ZIP64_SENTINEL {
            return None;
        }

        entries.push(Entry { method, compressed, declared, local_offset });
        p += 46 + name_len + extra_len + comment_len;
    }

    Some(entries)
}

/// Where an entry's compressed bytes actually start.
///
/// The local header repeats the name and extra fields with different lengths from
/// the central one, so the data offset cannot be computed from the central
/// directory alone - a detail that has produced parser mismatches in real
/// software, which is the sort of disagreement archive attacks are built on.
fn data_range(buf: &[u8], entry: &Entry) -> Option<(usize, usize)> {
    let p = entry.local_offset as usize;
    if p + 30 > buf.len() || u32_at(buf, p) != LOCAL {
        return None;
    }
    let name_len = u16_at(buf, p + 26) as usize;
    let extra_len = u16_at(buf, p + 28) as usize;
    let start = p + 30 + name_len + extra_len;
    let end = start + entry.compressed as usize;
    if end > buf.len() || start > end {
        return None;
    }
    Some((start, end))
}

/// Is this archive safe to expand?
///
/// Every entry is really decompressed against a hard output bound. Nothing is
/// returned but the verdict: the point is to answer the question before the
/// library that will do the real work gets the buffer.
pub fn inspect_zip(buf: &[u8], limits: &ZipLimits) -> ZipVerdict {
    if buf.len() < 22 || u32_at(buf, 0) != LOCAL {
        return Err(ZipRefusal::NotAZip);
    }

    let Some((offset, count)) = read_eocd(buf) else {
        return Err(ZipRefusal::Zip64);
    };
    if count > limits.entry_count {
        return Err(ZipRefusal::TooManyEntries);
    }

    let Some(entries) = read_central_directory(buf, offset, count) else {
        return Err(ZipRefusal::Malformed);
    };

    // The cheap pass first: a bomb that is honest about itself never reaches the
    // expensive one, and a file whose own numbers already exceed the budget is
    // refused for the price of reading them.
    let mut declared_total: u64 = 0;
    for entry in &entries {
        if entry.declared as u64 > limits.entry_bytes {
            return Err(ZipRefusal::TooBig);
        }
        declared_total += entry.declared as u64;
        if declared_total > limits.total_bytes {
            return Err(ZipRefusal::TooBig);
        }
        if entry.method != 0 && entry.method != 8 {
            return Err(ZipRefusal::UnsupportedMethod);
        }
    }

    // ...and now the pass that does not take the file's word for any of it.
    let mut real_total: u64 = 0;
    for entry in &entries {
        let Some((start, end)) = data_range(buf, entry) else {
            return Err(ZipRefusal::Malformed);
        };

        // Whatever budget is left, so a set of entries that are individually fine
        // cannot add up to a bomb. (`real_total` never passes the total: the
        // check at the end of each round refuses first.)
        let remaining = limits.total_bytes - real_total;
        if remaining == 0 && entry.compressed > 0 {
            return Err(ZipRefusal::TooBig);
        }

        if entry.method == 0 {
            // Stored: the compressed length IS the real length, nothing to verify.
            real_total += entry.compressed as u64;
        } else {
            // The bound that makes this a guard rather than a formality: zlib
            // fails at the limit instead of handing back what it has managed.
            match inflate_raw_bounded(&buf[start..end], limits.entry_bytes.min(remaining) + 1) {
                Ok(n) => real_total += n,
                // Either it blew the budget or it is not valid deflate. Both are
                // reasons not to hand this file to a library that will try again.
                Err(_) => return Err(ZipRefusal::TooBig),
            }
        }

        if real_total > limits.total_bytes {
            return Err(ZipRefusal::TooBig);
        }
    }

    Ok(ZipSummary { total_bytes: real_total, entries: entries.len() })
}

/// Why a raw inflate failed (Node's error codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InflateError {
    /// `ERR_BUFFER_TOO_LARGE`: the output passed `maxOutputLength`.
    TooLarge,
    /// `Z_BUF_ERROR` "unexpected end of file": the input ended first.
    Truncated,
    /// `Z_DATA_ERROR` and friends: not valid deflate.
    Invalid,
}

/// Node's chunk size for synchronous zlib calls (`Z_DEFAULT_CHUNK`).
const CHUNK: usize = 16 * 1024;

/// Drive a raw inflate the way node:zlib's `processChunkSync` does with
/// `Z_FINISH`, handing every produced chunk to `sink`. `Ok(total)` once the
/// stream ends (anything after it is ignored, as for `inflateRaw`).
fn inflate_raw_with(input: &[u8], max_output: u64, mut sink: impl FnMut(&[u8])) -> Result<u64, InflateError> {
    let mut d = Decompress::new(false);
    let mut out = vec![0u8; CHUNK];
    let mut produced: u64 = 0;
    loop {
        let consumed = d.total_in() as usize;
        let before = d.total_out();
        let status = d
            .decompress(&input[consumed.min(input.len())..], &mut out, FlushDecompress::Finish)
            .map_err(|_| InflateError::Invalid)?;
        let have = (d.total_out() - before) as usize;
        produced += have as u64;
        if produced > max_output {
            return Err(InflateError::TooLarge);
        }
        sink(&out[..have]);
        match status {
            Status::StreamEnd => return Ok(produced),
            // Room left in the output buffer and still not at the end of the
            // stream: the input ran out ("unexpected end of file").
            Status::Ok | Status::BufError if have < CHUNK => return Err(InflateError::Truncated),
            Status::Ok | Status::BufError => {}
        }
    }
}

/// `inflateRawSync(input, { maxOutputLength }).length`: the expanded size, or
/// why Node would have thrown.
pub(crate) fn inflate_raw_bounded(input: &[u8], max_output: u64) -> Result<u64, InflateError> {
    inflate_raw_with(input, max_output, |_| {})
}

/// The expanded bytes, failing like [`inflate_raw_bounded`].
pub(crate) fn inflate_raw_to_vec(input: &[u8], max_output: u64) -> Result<Vec<u8>, InflateError> {
    let mut out = Vec::new();
    inflate_raw_with(input, max_output, |chunk| out.extend_from_slice(chunk))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    //! tests/unit/zip-guard.test.ts
    use super::*;
    use flate2::Compression;
    use flate2::write::DeflateEncoder;
    use std::io::Write;

    pub(crate) fn deflate_raw(data: &[u8]) -> Vec<u8> {
        let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    /// A minimal ZIP writer, so the tests can build archives that a real one would
    /// refuse to produce - above all, one whose central directory LIES about how big
    /// its entries are. That case is the whole reason the guard inflates rather than
    /// adding up declared sizes, and it cannot be exercised with a normal zip tool.
    struct Member {
        name: String,
        body: Vec<u8>,
        /// Overrides the true uncompressed size written into both headers.
        declared_size: Option<u32>,
        /// 0 = stored, 8 = deflate.
        method: u16,
    }

    fn member(name: &str, body: Vec<u8>) -> Member {
        Member { name: name.into(), body, declared_size: None, method: 8 }
    }

    fn build_zip(members: &[Member]) -> Vec<u8> {
        let mut locals = Vec::new();
        let mut centrals = Vec::new();
        let mut offset = 0u32;
        for m in members {
            let data = if m.method == 8 { deflate_raw(&m.body) } else { m.body.clone() };
            let name = m.name.as_bytes();
            let declared = m.declared_size.unwrap_or(m.body.len() as u32);

            let mut local = vec![0u8; 30];
            local[0..4].copy_from_slice(&0x0403_4b50u32.to_le_bytes());
            local[4..6].copy_from_slice(&20u16.to_le_bytes());
            local[8..10].copy_from_slice(&m.method.to_le_bytes());
            local[18..22].copy_from_slice(&(data.len() as u32).to_le_bytes());
            local[22..26].copy_from_slice(&declared.to_le_bytes());
            local[26..28].copy_from_slice(&(name.len() as u16).to_le_bytes());
            locals.extend_from_slice(&local);
            locals.extend_from_slice(name);
            locals.extend_from_slice(&data);

            let mut central = vec![0u8; 46];
            central[0..4].copy_from_slice(&0x0201_4b50u32.to_le_bytes());
            central[6..8].copy_from_slice(&20u16.to_le_bytes());
            central[10..12].copy_from_slice(&m.method.to_le_bytes());
            central[20..24].copy_from_slice(&(data.len() as u32).to_le_bytes());
            central[24..28].copy_from_slice(&declared.to_le_bytes());
            central[28..30].copy_from_slice(&(name.len() as u16).to_le_bytes());
            central[42..46].copy_from_slice(&offset.to_le_bytes());
            centrals.extend_from_slice(&central);
            centrals.extend_from_slice(name);

            offset += (30 + name.len() + data.len()) as u32;
        }
        let mut eocd = vec![0u8; 22];
        eocd[0..4].copy_from_slice(&0x0605_4b50u32.to_le_bytes());
        eocd[8..10].copy_from_slice(&(members.len() as u16).to_le_bytes());
        eocd[10..12].copy_from_slice(&(members.len() as u16).to_le_bytes());
        eocd[12..16].copy_from_slice(&(centrals.len() as u32).to_le_bytes());
        eocd[16..20].copy_from_slice(&(locals.len() as u32).to_le_bytes());
        let mut out = locals;
        out.extend_from_slice(&centrals);
        out.extend_from_slice(&eocd);
        out
    }

    const LIMITS: ZipLimits = ZipLimits { total_bytes: 1_000_000, entry_bytes: 500_000, entry_count: 32 };

    #[test]
    fn accepts_an_ordinary_archive() {
        let zip = build_zip(&[
            member("[Content_Types].xml", b"<Types/>".to_vec()),
            member("word/document.xml", b"<w:document>hello</w:document>".to_vec()),
        ]);
        assert_eq!(inspect_zip(&zip, &LIMITS), Ok(ZipSummary { total_bytes: 38, entries: 2 }));
    }

    #[test]
    fn refuses_anything_that_is_not_a_zip_at_all() {
        assert_eq!(inspect_zip(b"not a zip", &LIMITS), Err(ZipRefusal::NotAZip));
        // An OLE2 compound file - a real .doc - starts with this and is not a zip.
        let mut ole = vec![0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
        ole.resize(32, 0);
        assert_eq!(inspect_zip(&ole, &LIMITS), Err(ZipRefusal::NotAZip));
    }

    #[test]
    fn refuses_a_bomb_that_is_honest_about_its_size_without_decompressing_it() {
        // The cheap pass: the archive's own numbers already exceed the budget.
        let zip = build_zip(&[member("big.xml", vec![0x41; 600_000])]);
        assert_eq!(inspect_zip(&zip, &LIMITS), Err(ZipRefusal::TooBig));
    }

    #[test]
    fn refuses_a_bomb_that_lies_about_its_size() {
        // The case the whole design turns on. A megabyte of zeroes deflates to
        // about a kilobyte; the headers claim it expands to 100 bytes. Anything
        // that trusted the declared sizes would wave this straight through to
        // JSZip, which expands it for real on the main thread.
        let mut m = member("lie.xml", vec![0; 2_000_000]);
        m.declared_size = Some(100);
        assert_eq!(inspect_zip(&build_zip(&[m]), &LIMITS), Err(ZipRefusal::TooBig));
    }

    #[test]
    fn refuses_entries_that_are_each_fine_but_add_up_to_a_bomb() {
        // Every entry is a fifth of the per-entry ceiling; ten of them are not.
        let members: Vec<Member> = (0..10).map(|i| member(&format!("part{i}.bin"), vec![0; 150_000])).collect();
        assert_eq!(inspect_zip(&build_zip(&members), &LIMITS), Err(ZipRefusal::TooBig));
    }

    #[test]
    fn refuses_an_archive_with_too_many_entries() {
        let members: Vec<Member> = (0..40).map(|i| member(&format!("f{i}"), b"x".to_vec())).collect();
        assert_eq!(inspect_zip(&build_zip(&members), &LIMITS), Err(ZipRefusal::TooManyEntries));
    }

    #[test]
    fn refuses_a_compression_method_it_cannot_verify() {
        // Only stored and deflate. Anything else (bzip2, lzma, ppmd) would have to
        // be waved through unverified, which is exactly what this guard exists to
        // avoid - and a .docx never uses one.
        let mut m = member("x.xml", b"hello".to_vec());
        m.method = 12;
        assert_eq!(inspect_zip(&build_zip(&[m]), &LIMITS), Err(ZipRefusal::UnsupportedMethod));
    }

    #[test]
    fn counts_stored_entries_at_their_real_length() {
        let mut m = member("raw.bin", vec![0x42; 600_000]);
        m.method = 0;
        assert_eq!(inspect_zip(&build_zip(&[m]), &LIMITS), Err(ZipRefusal::TooBig));
    }

    #[test]
    fn refuses_a_truncated_or_overlapping_archive_rather_than_guessing() {
        let mut zip = build_zip(&[member("a.xml", b"hello".to_vec())]);
        // Corrupt the local header signature the central directory points at.
        zip[0..4].copy_from_slice(&0xdead_beefu32.to_le_bytes());
        assert!(inspect_zip(&zip, &LIMITS).is_err());
    }

    #[test]
    fn refuses_an_archive_whose_trailer_is_missing() {
        let zip = build_zip(&[member("a.xml", b"hello".to_vec())]);
        assert!(inspect_zip(&zip[..zip.len() - 22], &LIMITS).is_err());
    }

    #[test]
    fn accepts_a_zip_with_a_trailing_comment_which_is_legal() {
        let mut zip = build_zip(&[member("a.xml", b"hello".to_vec())]);
        let eocd_start = zip.len() - 22;
        let comment = b"packed by something";
        zip.extend_from_slice(comment);
        zip[eocd_start + 20..eocd_start + 22].copy_from_slice(&(comment.len() as u16).to_le_bytes());
        assert!(inspect_zip(&zip, &LIMITS).is_ok());
    }

    /// Node's answers, recorded in the reference container
    /// (`_scratch/srv-media-a/docx/probe/zlib_edges.js`, node 24.22).
    #[test]
    fn inflates_like_node_zlib() {
        let d = deflate_raw(&[0x41; 100]);
        assert_eq!(inflate_raw_bounded(&[], 1000), Err(InflateError::Truncated));
        assert_eq!(inflate_raw_bounded(&d, 100), Ok(100));
        assert_eq!(inflate_raw_bounded(&d, 99), Err(InflateError::TooLarge));
        let mut trailing = d.clone();
        trailing.extend_from_slice(b"garbage");
        assert_eq!(inflate_raw_bounded(&trailing, 1000), Ok(100));
        assert_eq!(inflate_raw_bounded(&d[..d.len() - 1], 1000), Err(InflateError::Truncated));
        assert_eq!(inflate_raw_bounded(&[0x01, 0x00, 0x00, 0xff, 0xff], 1000), Ok(0));
        assert_eq!(inflate_raw_bounded(&[0x07], 1000), Err(InflateError::Invalid));
        let zeros = deflate_raw(&[0; 70_000]);
        assert_eq!(inflate_raw_bounded(&zeros, 70_000), Ok(70_000));
        assert_eq!(inflate_raw_bounded(&zeros, 69_999), Err(InflateError::TooLarge));
        assert_eq!(inflate_raw_bounded(&[0x63, 0x00, 0x02, 0x00], 1000), Ok(4));
        assert_eq!(inflate_raw_to_vec(&d, 100).unwrap(), vec![0x41; 100]);
    }
}
