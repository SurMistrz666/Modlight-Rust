//! JPEG: the segments are rewritten and the compressed picture is copied, so
//! the decoded pixels cannot change.
//!
//! Kept: JFIF APP0 (without a thumbnail), Adobe APP14 (the colour transform),
//! the ICC profile chunks of APP2 and every structural segment. Gone: EXIF and
//! XMP (APP1), IPTC/Photoshop (APP13), MPF and every other APPn, comments, and
//! whatever follows the end of the image (the secondary pictures MPF points at,
//! the video of a motion photo).
//!
//! A photo whose EXIF says it is rotated keeps a minimal EXIF holding only the
//! Orientation tag, or phone pictures would turn sideways.

use std::io::{BufRead, Write};

use super::{RewriteError, pass, read_all};

const SOI: u8 = 0xD8;
const EOI: u8 = 0xD9;
const SOS: u8 = 0xDA;
const COM: u8 = 0xFE;
const APP0: u8 = 0xE0;
const APP1: u8 = 0xE1;
const APP2: u8 = 0xE2;
const APP14: u8 = 0xEE;

const EXIF_ID: &[u8] = b"Exif\0\0";
const JFIF_ID: &[u8] = b"JFIF\0";
const ICC_ID: &[u8] = b"ICC_PROFILE\0";
const ADOBE_ID: &[u8] = b"Adobe";

/// `FF` + marker + the length word that counts itself + what is in `body`.
fn write_segment<W: Write>(output: &mut W, marker: u8, body: &[u8]) -> Result<(), RewriteError> {
    let Ok(length) = u16::try_from(body.len() + 2) else {
        return Err(RewriteError::Malformed("JPEG segment too long"));
    };
    output.write_all(&[0xFF, marker])?;
    output.write_all(&length.to_be_bytes())?;
    output.write_all(body)?;
    Ok(())
}

/// The next marker where one must be: `FF`, any fill bytes, the code. `None`
/// when the file ends here.
fn next_marker<R: BufRead>(input: &mut R) -> Result<Option<u8>, RewriteError> {
    let mut byte = [0u8; 1];
    if input.read(&mut byte)? == 0 {
        return Ok(None);
    }
    if byte[0] != 0xFF {
        return Err(RewriteError::Malformed("JPEG: a marker was expected"));
    }
    loop {
        if input.read(&mut byte)? == 0 {
            return Ok(None);
        }
        match byte[0] {
            0xFF => continue,
            0x00 => return Err(RewriteError::Malformed("JPEG: a stuffed byte outside a scan")),
            code => return Ok(Some(code)),
        }
    }
}

/// Copy entropy-coded data up to the next real marker (which is consumed and
/// returned); `None` when the file ends inside the data. `FF 00` and the restart
/// markers are data; a run of `FF` before a marker is fill.
fn copy_scan<R: BufRead, W: Write>(input: &mut R, output: &mut W) -> Result<Option<u8>, RewriteError> {
    // An `FF` read but not yet known to be data or the start of a marker.
    let mut held = false;
    loop {
        let buf = input.fill_buf()?;
        if buf.is_empty() {
            if held {
                output.write_all(&[0xFF])?;
            }
            return Ok(None);
        }
        let mut at = 0;
        while at < buf.len() {
            if held {
                let byte = buf[at];
                at += 1;
                match byte {
                    0xFF => {}
                    0x00 | 0xD0..=0xD7 => {
                        output.write_all(&[0xFF, byte])?;
                        held = false;
                    }
                    marker => {
                        input.consume(at);
                        return Ok(Some(marker));
                    }
                }
                continue;
            }
            match buf[at..].iter().position(|&b| b == 0xFF) {
                Some(found) => {
                    output.write_all(&buf[at..at + found])?;
                    held = true;
                    at += found + 1;
                }
                None => {
                    output.write_all(&buf[at..])?;
                    at = buf.len();
                }
            }
        }
        let used = buf.len();
        input.consume(used);
    }
}

/// The Orientation tag (1-8) of a TIFF structure, as EXIF wraps it. Every read
/// is bounds-checked: the offsets come from the file.
pub fn exif_orientation(tiff: &[u8]) -> Option<u16> {
    let big = match tiff.get(..2)? {
        b"II" => false,
        b"MM" => true,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let b = tiff.get(at..at.checked_add(2)?)?;
        Some(if big { u16::from_be_bytes([b[0], b[1]]) } else { u16::from_le_bytes([b[0], b[1]]) })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b = tiff.get(at..at.checked_add(4)?)?;
        Some(if big {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        })
    };
    if u16_at(2)? != 42 {
        return None;
    }
    let ifd = u32_at(4)? as usize;
    let count = u16_at(ifd)? as usize;
    (0..count).find_map(|i| {
        let entry = ifd.checked_add(2)?.checked_add(i.checked_mul(12)?)?;
        // Tag 0x0112, one SHORT, held in the value field itself.
        if u16_at(entry)? == 0x0112 && u16_at(entry + 2)? == 3 && u32_at(entry + 4)? == 1 {
            u16_at(entry + 8).filter(|o| (1..=8).contains(o))
        } else {
            None
        }
    })
}

/// An APP1 body holding exactly one tag: Orientation.
pub fn minimal_exif(orientation: u16) -> Vec<u8> {
    let mut body = EXIF_ID.to_vec();
    body.extend_from_slice(b"II*\0");
    body.extend_from_slice(&8u32.to_le_bytes()); // IFD0 follows the header
    body.extend_from_slice(&1u16.to_le_bytes()); // one entry
    body.extend_from_slice(&0x0112u16.to_le_bytes());
    body.extend_from_slice(&3u16.to_le_bytes()); // SHORT
    body.extend_from_slice(&1u32.to_le_bytes()); // one value
    body.extend_from_slice(&orientation.to_le_bytes());
    body.extend_from_slice(&[0, 0]); // the rest of the value field
    body.extend_from_slice(&0u32.to_le_bytes()); // no further IFD
    body
}

/// A JFIF APP0 without its embedded thumbnail: a small picture of the original
/// is the kind of thing this feature exists to remove.
fn without_thumbnail(body: &[u8]) -> Option<Vec<u8>> {
    if body.len() < 14 {
        return None;
    }
    if body.len() == 14 && body[12] == 0 && body[13] == 0 {
        return None;
    }
    let mut lean = body[..14].to_vec();
    lean[12] = 0;
    lean[13] = 0;
    Some(lean)
}

/// One APPn or COM segment: what goes to the output instead of it.
enum Verdict {
    Keep,
    Drop,
    Replace(Vec<u8>),
}

fn judge(marker: u8, body: &[u8], orientation_done: &mut bool) -> Verdict {
    match marker {
        APP0 if body.starts_with(JFIF_ID) => match without_thumbnail(body) {
            Some(lean) => Verdict::Replace(lean),
            None => Verdict::Keep,
        },
        APP1 if body.starts_with(EXIF_ID) && !*orientation_done => {
            match exif_orientation(&body[EXIF_ID.len()..]).filter(|&o| o != 1) {
                Some(orientation) => {
                    *orientation_done = true;
                    let minimal = minimal_exif(orientation);
                    if minimal == body { Verdict::Keep } else { Verdict::Replace(minimal) }
                }
                None => Verdict::Drop,
            }
        }
        APP2 if body.starts_with(ICC_ID) => Verdict::Keep,
        APP14 if body.starts_with(ADOBE_ID) => Verdict::Keep,
        _ => Verdict::Drop,
    }
}

/// Rewrite a JPEG. `Ok(false)` means there was nothing to remove (the output is
/// then the same file, and the caller keeps the original).
///
/// Memory: one segment (64 KiB at most) and the reader's buffer, whatever the
/// size of the file. A file that ends inside the compressed data is treated as
/// finished, as a viewer treats it: everything a metadata segment could be has
/// been seen by then.
pub fn strip<R: BufRead, W: Write>(input: &mut R, output: &mut W) -> Result<bool, RewriteError> {
    let mut soi = [0u8; 2];
    read_all(input, &mut soi, "JPEG: too short")?;
    if soi != [0xFF, SOI] {
        return Err(RewriteError::Malformed("not a JPEG"));
    }
    output.write_all(&soi)?;

    let mut changed = false;
    let mut scanned = false;
    let mut orientation_done = false;
    let mut pending: Option<u8> = None;
    loop {
        let marker = match pending.take() {
            Some(marker) => Some(marker),
            None => next_marker(input)?,
        };
        let Some(marker) = marker else {
            if scanned {
                return Ok(changed);
            }
            return Err(RewriteError::Malformed("JPEG: no picture"));
        };
        match marker {
            EOI => {
                if !scanned {
                    return Err(RewriteError::Malformed("JPEG: no picture"));
                }
                output.write_all(&[0xFF, EOI])?;
                // Whatever follows is not part of this picture.
                return Ok(changed || !input.fill_buf()?.is_empty());
            }
            SOI => return Err(RewriteError::Malformed("JPEG: a second start of image")),
            0x01 | 0xD0..=0xD7 => output.write_all(&[0xFF, marker])?,
            _ => {
                let mut length = [0u8; 2];
                read_all(input, &mut length, "JPEG: a segment header is cut off")?;
                let length = u16::from_be_bytes(length) as usize;
                if length < 2 {
                    return Err(RewriteError::Malformed("JPEG: a segment shorter than its header"));
                }
                let body_len = length - 2;
                if marker == COM || (APP0..=0xEF).contains(&marker) {
                    let mut body = vec![0u8; body_len];
                    read_all(input, &mut body, "JPEG: a segment is cut off")?;
                    match judge(marker, &body, &mut orientation_done) {
                        Verdict::Keep => write_segment(output, marker, &body)?,
                        Verdict::Drop => changed = true,
                        Verdict::Replace(new) => {
                            write_segment(output, marker, &new)?;
                            changed = true;
                        }
                    }
                } else {
                    output.write_all(&[0xFF, marker])?;
                    output.write_all(&(length as u16).to_be_bytes())?;
                    pass(input, Some(&mut *output), body_len as u64, "JPEG: a segment is cut off")?;
                    if marker == SOS {
                        scanned = true;
                        match copy_scan(input, output)? {
                            Some(next) => pending = Some(next),
                            None => return Ok(changed),
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn segment(marker: u8, body: &[u8]) -> Vec<u8> {
        let mut v = vec![0xFF, marker];
        v.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
        v.extend_from_slice(body);
        v
    }

    /// A structurally complete JPEG; the scan bytes are not a real picture, which
    /// these tests do not need (`tests/media_strip.rs` decodes real ones).
    fn jpeg(before_scan: &[Vec<u8>]) -> Vec<u8> {
        let mut v = vec![0xFF, SOI];
        for s in before_scan {
            v.extend_from_slice(s);
        }
        v.extend(segment(0xDB, &[0; 65])); // DQT
        v.extend(segment(0xC0, &[8, 0, 16, 0, 16, 1, 1, 0x11, 0])); // SOF0
        v.extend(segment(0xC4, &[0; 20])); // DHT
        v.extend(segment(SOS, &[1, 1, 0, 0, 63, 0]));
        v.extend_from_slice(&[0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD0, 0x56]);
        v.extend_from_slice(&[0xFF, EOI]);
        v
    }

    fn run(input: &[u8]) -> Result<(bool, Vec<u8>), RewriteError> {
        let mut out = Vec::new();
        let changed = strip(&mut Cursor::new(input), &mut out)?;
        Ok((changed, out))
    }

    fn exif_with(orientation: u16, extra: &[u8]) -> Vec<u8> {
        let mut body = minimal_exif(orientation);
        body.extend_from_slice(extra);
        body
    }

    const JFIF: &[u8] = b"JFIF\0\x01\x01\0\0\x48\0\x48\0\0";

    #[test]
    fn comments_exif_xmp_iptc_and_other_app_segments_go() {
        let input = jpeg(&[
            segment(APP0, JFIF),
            segment(APP1, &exif_with(1, b"GPS-AND-CAMERA")),
            segment(APP1, b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta>author</x:xmpmeta>"),
            segment(APP2, b"MPF\0II*\0"),
            segment(0xED, b"Photoshop 3.0\08BIM"),
            segment(0xE5, b"anything"),
            segment(COM, b"a comment"),
        ]);
        let (changed, out) = run(&input).unwrap();
        assert!(changed);
        assert_eq!(out, jpeg(&[segment(APP0, JFIF)]));
    }

    #[test]
    fn jfif_adobe_and_icc_stay_in_their_places() {
        let kept = [
            segment(APP0, JFIF),
            segment(APP2, b"ICC_PROFILE\0\x01\x02part one"),
            segment(APP2, b"ICC_PROFILE\0\x02\x02part two"),
            segment(APP14, b"Adobe\0\x64\0\0\0\0\x01"),
        ];
        let mut with_more = kept.to_vec();
        with_more.insert(1, segment(COM, b"gone"));
        let (changed, out) = run(&jpeg(&with_more)).unwrap();
        assert!(changed);
        assert_eq!(out, jpeg(&kept));
    }

    #[test]
    fn a_rotated_photo_keeps_only_the_orientation() {
        let input = jpeg(&[segment(APP0, JFIF), segment(APP1, &exif_with(6, b"GPS"))]);
        let (changed, out) = run(&input).unwrap();
        assert!(changed);
        assert_eq!(out, jpeg(&[segment(APP0, JFIF), segment(APP1, &minimal_exif(6))]));
        assert_eq!(exif_orientation(&minimal_exif(6)[EXIF_ID.len()..]), Some(6));
        // Cleaning twice changes nothing: the same upload gets the same bytes.
        let (changed, again) = run(&out).unwrap();
        assert!(!changed);
        assert_eq!(again, out);
    }

    #[test]
    fn big_endian_exif_is_read_too() {
        let tiff = b"MM\0*\0\0\0\x08\0\x02\x01\x0f\0\x02\0\0\0\x04\0\0\0\x26\x01\x12\0\x03\0\0\0\x01\0\x08\0\0\0\0\0\0";
        assert_eq!(exif_orientation(tiff), Some(8));
    }

    #[test]
    fn a_nonsense_orientation_is_no_orientation() {
        let mut body = minimal_exif(9);
        assert_eq!(exif_orientation(&body[EXIF_ID.len()..]), None);
        body.truncate(EXIF_ID.len() + 12);
        assert_eq!(exif_orientation(&body[EXIF_ID.len()..]), None);
        let (_, out) = run(&jpeg(&[segment(APP1, &minimal_exif(9))])).unwrap();
        assert_eq!(out, jpeg(&[]));
    }

    #[test]
    fn a_jfif_thumbnail_goes_with_the_rest() {
        let mut body = JFIF.to_vec();
        body[12] = 2;
        body[13] = 1;
        body.extend_from_slice(&[9; 6]);
        let (changed, out) = run(&jpeg(&[segment(APP0, &body)])).unwrap();
        assert!(changed);
        assert_eq!(out, jpeg(&[segment(APP0, JFIF)]));
    }

    #[test]
    fn what_follows_the_picture_is_dropped() {
        let mut input = jpeg(&[segment(APP0, JFIF)]);
        let clean = input.clone();
        input.extend_from_slice(b"\xFF\xD8 a second picture, or the video of a motion photo");
        let (changed, out) = run(&input).unwrap();
        assert!(changed);
        assert_eq!(out, clean);
    }

    #[test]
    fn a_clean_file_is_not_changed() {
        let input = jpeg(&[segment(APP0, JFIF), segment(APP14, b"Adobe\0\x64\0\0\0\0\x01")]);
        let (changed, out) = run(&input).unwrap();
        assert!(!changed);
        assert_eq!(out, input);
    }

    #[test]
    fn segments_between_scans_are_seen_too() {
        // A progressive file: a second scan, with a comment in between.
        let mut input = jpeg(&[]);
        input.truncate(input.len() - 2);
        input.extend(segment(COM, b"between scans"));
        input.extend(segment(0xC4, &[1; 20]));
        input.extend(segment(SOS, &[1, 1, 0, 0, 63, 0]));
        input.extend_from_slice(&[0x77, 0xFF, 0xD9]);
        let (changed, out) = run(&input).unwrap();
        assert!(changed);
        assert!(!out.windows(13).any(|w| w == b"between scans"));
        assert!(out.ends_with(&[0x77, 0xFF, 0xD9]));
    }

    #[test]
    fn a_file_cut_off_inside_the_picture_still_loses_what_came_before() {
        let mut input = jpeg(&[segment(COM, b"gone")]);
        input.truncate(input.len() - 4);
        let (changed, out) = run(&input).unwrap();
        assert!(changed);
        assert!(!out.windows(4).any(|w| w == b"gone"));
        // A lone FF at the very end is kept as data.
        let mut ending_in_ff = jpeg(&[]);
        ending_in_ff.truncate(ending_in_ff.len() - 1);
        assert_eq!(run(&ending_in_ff).unwrap().1, ending_in_ff);
    }

    #[test]
    fn broken_layouts_are_refused() {
        let cases: Vec<Vec<u8>> = vec![
            vec![],
            vec![0xFF],
            vec![0xFF, 0xD9],
            vec![0xFF, SOI],
            vec![0xFF, SOI, 0xFF],
            vec![0xFF, SOI, 0xFF, EOI],
            vec![0xFF, SOI, 0x00, 0x00],
            vec![0xFF, SOI, 0xFF, 0x00],
            vec![0xFF, SOI, 0xFF, SOI],
            vec![0xFF, SOI, 0xFF, APP1],
            vec![0xFF, SOI, 0xFF, APP1, 0x00],
            vec![0xFF, SOI, 0xFF, APP1, 0x00, 0x00],
            vec![0xFF, SOI, 0xFF, APP1, 0x00, 0x01],
            vec![0xFF, SOI, 0xFF, APP1, 0xFF, 0xFF, b'E', b'x'],
            vec![0xFF, SOI, 0xFF, 0xDB, 0xFF, 0xFF, 1, 2, 3],
        ];
        for case in cases {
            assert!(run(&case).is_err(), "{case:?}");
        }
    }

    #[test]
    fn a_marker_split_across_two_reads_is_found() {
        // A reader that hands over one byte at a time.
        let input = jpeg(&[segment(COM, b"gone")]);
        let mut slow = std::io::BufReader::with_capacity(1, Cursor::new(input));
        let mut out = Vec::new();
        assert!(strip(&mut slow, &mut out).unwrap());
        assert_eq!(out, jpeg(&[]));
    }
}
