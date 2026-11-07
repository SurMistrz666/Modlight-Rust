//! PNG and APNG: a walk over the chunks. What affects the picture stays
//! byte for byte (with its CRC); text, the time stamp, EXIF and every ancillary
//! chunk this file does not know are left out, and so is whatever follows IEND.
//!
//! A chunk that is critical (upper-case first letter) and unknown makes the file
//! unknown: dropping it could break the picture, so the file is refused.

use std::io::{Read, Write};

use super::{RewriteError, pass, read_all};

pub const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// Chunks that stay. `bKGD` and the HDR chunks (`cICP`, `mDCV`, `cLLI` and their
/// spellings from before the third edition) say how to show the picture; a
/// chunk for the camera's clock does not.
const KEPT: [&[u8; 4]; 20] = [
    b"IHDR", b"PLTE", b"IDAT", b"IEND", b"tRNS", b"gAMA", b"cHRM", b"sRGB", b"iCCP", b"sBIT", b"pHYs", b"acTL",
    b"fcTL", b"fdAT", b"bKGD", b"cICP", b"mDCV", b"mDCv", b"cLLI", b"cLLi",
];

/// Length of one chunk's data, by the specification's own limit.
const MAX_CHUNK: u32 = 0x7FFF_FFFF;

/// Rewrite a PNG. `Ok(false)` means there was nothing to remove. Constant
/// memory: chunks are streamed, a length field is never allocated.
pub fn strip<R: Read, W: Write>(input: &mut R, output: &mut W) -> Result<bool, RewriteError> {
    let mut signature = [0u8; 8];
    read_all(input, &mut signature, "PNG: too short")?;
    if &signature != SIGNATURE {
        return Err(RewriteError::Malformed("not a PNG"));
    }
    output.write_all(&signature)?;

    let mut changed = false;
    let mut first = true;
    loop {
        let mut header = [0u8; 8];
        read_all(input, &mut header, "PNG: the file ends before IEND")?;
        let length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        let name: [u8; 4] = [header[4], header[5], header[6], header[7]];
        if length > MAX_CHUNK || !name.iter().all(u8::is_ascii_alphabetic) {
            return Err(RewriteError::Malformed("PNG: not a chunk"));
        }
        if first && &name != b"IHDR" {
            return Err(RewriteError::Malformed("PNG: IHDR must come first"));
        }
        first = false;
        // The data and the four bytes of its CRC.
        let span = u64::from(length) + 4;
        if KEPT.contains(&&name) {
            output.write_all(&header)?;
            pass(input, Some(&mut *output), span, "PNG: a chunk is cut off")?;
        } else if name[0].is_ascii_uppercase() {
            return Err(RewriteError::Malformed("PNG: an unknown critical chunk"));
        } else {
            pass(input, None::<&mut W>, span, "PNG: a chunk is cut off")?;
            changed = true;
        }
        if &name == b"IEND" {
            let mut one = [0u8; 1];
            return Ok(changed || input.read(&mut one)? != 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn chunk(name: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = (data.len() as u32).to_be_bytes().to_vec();
        v.extend_from_slice(name);
        v.extend_from_slice(data);
        let mut crc = crc32fast::Hasher::new();
        crc.update(name);
        crc.update(data);
        v.extend_from_slice(&crc.finalize().to_be_bytes());
        v
    }

    fn png(chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut v = SIGNATURE.to_vec();
        v.extend(chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0]));
        for c in chunks {
            v.extend_from_slice(c);
        }
        v.extend(chunk(b"IDAT", &[1, 2, 3]));
        v.extend(chunk(b"IEND", &[]));
        v
    }

    fn run(input: &[u8]) -> Result<(bool, Vec<u8>), RewriteError> {
        let mut out = Vec::new();
        let changed = strip(&mut Cursor::new(input), &mut out)?;
        Ok((changed, out))
    }

    #[test]
    fn text_time_exif_and_unknown_chunks_go() {
        let input = png(&[
            chunk(b"tEXt", b"Author\0Jan"),
            chunk(b"zTXt", b"Comment\0\0xx"),
            chunk(b"iTXt", b"XML:com.adobe.xmp\0\0\0\0\0<x/>"),
            chunk(b"tIME", &[7, 230, 1, 1, 1, 1, 1]),
            chunk(b"eXIf", b"II*\0"),
            chunk(b"prVt", b"private"),
            chunk(b"gAMA", &[0, 0, 177, 143]),
        ]);
        let (changed, out) = run(&input).unwrap();
        assert!(changed);
        assert_eq!(out, png(&[chunk(b"gAMA", &[0, 0, 177, 143])]));
    }

    #[test]
    fn colour_and_animation_chunks_stay_byte_for_byte() {
        let kept = [
            chunk(b"PLTE", &[1, 2, 3]),
            chunk(b"tRNS", &[0]),
            chunk(b"cHRM", &[0; 32]),
            chunk(b"sRGB", &[0]),
            chunk(b"iCCP", b"icc\0\0data"),
            chunk(b"sBIT", &[8, 8, 8]),
            chunk(b"pHYs", &[0, 0, 11, 19, 0, 0, 11, 19, 1]),
            chunk(b"acTL", &[0, 0, 0, 2, 0, 0, 0, 0]),
            chunk(b"fcTL", &[0; 26]),
            chunk(b"fdAT", &[0, 0, 0, 1, 9, 9]),
            chunk(b"bKGD", &[0, 0]),
            chunk(b"cICP", &[1, 13, 0, 1]),
        ];
        let input = png(&kept);
        let (changed, out) = run(&input).unwrap();
        assert!(!changed);
        assert_eq!(out, input);
    }

    #[test]
    fn what_follows_iend_goes() {
        let clean = png(&[]);
        let mut input = clean.clone();
        input.extend_from_slice(b"appended by something");
        let (changed, out) = run(&input).unwrap();
        assert!(changed);
        assert_eq!(out, clean);
    }

    #[test]
    fn an_unknown_critical_chunk_makes_the_file_unknown() {
        assert!(run(&png(&[chunk(b"CgBI", &[0, 1, 2, 3])])).is_err());
    }

    #[test]
    fn broken_layouts_are_refused() {
        let full = png(&[chunk(b"tEXt", b"a\0b")]);
        for cut in [0, 7, 8, 12, 15, 20, 33, full.len() - 13, full.len() - 1] {
            assert!(run(&full[..cut]).is_err(), "cut at {cut}");
        }
        let mut wrong_order = SIGNATURE.to_vec();
        wrong_order.extend(chunk(b"IDAT", &[1]));
        assert!(run(&wrong_order).is_err());
        let mut huge = SIGNATURE.to_vec();
        huge.extend_from_slice(&u32::MAX.to_be_bytes());
        huge.extend_from_slice(b"IHDR");
        assert!(run(&huge).is_err());
        let mut not_letters = SIGNATURE.to_vec();
        not_letters.extend_from_slice(&[0, 0, 0, 0, 0xFF, 0x00, b'a', b'b']);
        assert!(run(&not_letters).is_err());
    }
}
