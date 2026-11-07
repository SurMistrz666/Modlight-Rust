//! WebP: a RIFF file. The EXIF and XMP chunks go, and so does any chunk the
//! container does not define (readers must skip those anyway); the VP8X flags
//! and the RIFF size are put right. The compressed picture, the ICC profile and
//! the animation are copied as they are.

use std::io::{Read, Seek, Write};

use super::{RewriteError, pass, patch_u32_le, read_all};

/// Chunks that stay.
const KEPT: [&[u8; 4]; 7] = [b"VP8 ", b"VP8L", b"VP8X", b"ALPH", b"ANIM", b"ANMF", b"ICCP"];

/// Chunks that are a picture: a file without one is broken.
const PICTURES: [&[u8; 4]; 3] = [b"VP8 ", b"VP8L", b"ANMF"];

/// VP8X flag bits (`Rsv|I|L|E|X|A|R`).
const FLAG_EXIF: u8 = 0x08;
const FLAG_XMP: u8 = 0x04;

/// Rewrite a WebP. `Ok(false)` means there was nothing to remove. The output
/// must be seekable: the RIFF size is known only at the end.
pub fn strip<R: Read, W: Write + Seek>(input: &mut R, output: &mut W) -> Result<bool, RewriteError> {
    let mut header = [0u8; 12];
    read_all(input, &mut header, "WebP: too short")?;
    if &header[..4] != b"RIFF" || &header[8..] != b"WEBP" {
        return Err(RewriteError::Malformed("not a WebP"));
    }
    let declared = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if declared < 4 {
        return Err(RewriteError::Malformed("WebP: a RIFF size smaller than its own header"));
    }
    output.write_all(&header)?;

    let mut remaining = u64::from(declared) - 4;
    let mut written: u64 = 4;
    let mut changed = false;
    let mut seen_picture = false;
    while remaining >= 8 {
        let mut chunk = [0u8; 8];
        read_all(input, &mut chunk, "WebP: a chunk header is cut off")?;
        remaining -= 8;
        let name: [u8; 4] = [chunk[0], chunk[1], chunk[2], chunk[3]];
        let size = u64::from(u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]));
        if size > remaining {
            return Err(RewriteError::Malformed("WebP: a chunk is longer than the file says"));
        }
        // Chunks are padded to an even size; a last chunk may lack its pad.
        let pad = (size & 1).min(remaining - size);
        remaining -= size + pad;

        if !KEPT.contains(&&name) {
            pass(input, None::<&mut W>, size + pad, "WebP: a chunk is cut off")?;
            changed = true;
            continue;
        }
        seen_picture |= PICTURES.contains(&&name);
        output.write_all(&chunk)?;
        if &name == b"VP8X" {
            if size < 10 {
                return Err(RewriteError::Malformed("WebP: VP8X is too short"));
            }
            let mut flags = [0u8; 1];
            read_all(input, &mut flags, "WebP: a chunk is cut off")?;
            let clean = flags[0] & !(FLAG_EXIF | FLAG_XMP);
            changed |= clean != flags[0];
            output.write_all(&[clean])?;
            pass(input, Some(&mut *output), size - 1, "WebP: a chunk is cut off")?;
        } else {
            pass(input, Some(&mut *output), size, "WebP: a chunk is cut off")?;
        }
        // The pad byte is written whether or not the file had it.
        if size & 1 == 1 {
            if pad == 1 {
                let mut skipped = [0u8; 1];
                read_all(input, &mut skipped, "WebP: a chunk is cut off")?;
            }
            output.write_all(&[0])?;
        }
        written += 8 + size + (size & 1);
    }
    if !seen_picture {
        return Err(RewriteError::Malformed("WebP: no picture"));
    }
    // Stray bytes inside the RIFF, and anything after it, are not part of the file.
    pass(input, None::<&mut W>, remaining, "WebP: the file is shorter than it says")?;
    if remaining > 0 {
        changed = true;
    }
    let mut one = [0u8; 1];
    changed |= input.read(&mut one)? != 0;

    let Ok(size) = u32::try_from(written) else {
        return Err(RewriteError::Malformed("WebP: too large"));
    };
    changed |= size != declared;
    patch_u32_le(output, 4, size)?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn chunk(name: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = name.to_vec();
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(data);
        if data.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    fn riff(chunks: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = chunks.concat();
        let mut v = b"RIFF".to_vec();
        v.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
        v.extend_from_slice(b"WEBP");
        v.extend(body);
        v
    }

    fn vp8x(flags: u8) -> Vec<u8> {
        chunk(b"VP8X", &[flags, 0, 0, 0, 1, 0, 0, 1, 0, 0])
    }

    fn run(input: &[u8]) -> Result<(bool, Vec<u8>), RewriteError> {
        let mut out = Cursor::new(Vec::new());
        let changed = strip(&mut Cursor::new(input), &mut out)?;
        Ok((changed, out.into_inner()))
    }

    #[test]
    fn exif_and_xmp_go_and_the_header_is_put_right() {
        let picture = chunk(b"VP8 ", &[1, 2, 3, 4, 5]);
        let input = riff(&[
            vp8x(FLAG_EXIF | FLAG_XMP | 0x10),
            picture.clone(),
            chunk(b"EXIF", b"Exif\0\0II*\0"),
            chunk(b"XMP ", b"<x:xmpmeta/>"),
        ]);
        let (changed, out) = run(&input).unwrap();
        assert!(changed);
        assert_eq!(out, riff(&[vp8x(0x10), picture]));
    }

    #[test]
    fn the_profile_and_the_animation_stay() {
        let input = riff(&[
            vp8x(0x20 | 0x02),
            chunk(b"ICCP", b"profile"),
            chunk(b"ANIM", &[0, 0, 0, 0, 0, 0]),
            chunk(b"ANMF", &[9; 17]),
            chunk(b"ANMF", &[8; 18]),
        ]);
        let (changed, out) = run(&input).unwrap();
        assert!(!changed);
        assert_eq!(out, input);
    }

    #[test]
    fn unknown_chunks_and_what_follows_the_riff_go() {
        let picture = chunk(b"VP8L", &[1, 2]);
        let clean = riff(std::slice::from_ref(&picture));
        let (changed, out) = run(&riff(&[chunk(b"JUNK", b"x"), picture.clone()])).unwrap();
        assert!(changed);
        assert_eq!(out, clean);
        let mut trailing = clean.clone();
        trailing.extend_from_slice(b"appended");
        let (changed, out) = run(&trailing).unwrap();
        assert!(changed);
        assert_eq!(out, clean);
    }

    #[test]
    fn a_last_chunk_without_its_pad_is_read_and_mended() {
        let mut input = riff(&[chunk(b"VP8 ", &[1, 2, 3])]);
        input.pop();
        let size = (input.len() - 8) as u32;
        input[4..8].copy_from_slice(&size.to_le_bytes());
        let (changed, out) = run(&input).unwrap();
        assert!(changed);
        assert_eq!(out, riff(&[chunk(b"VP8 ", &[1, 2, 3])]));
    }

    #[test]
    fn broken_layouts_are_refused() {
        let full = riff(&[vp8x(0), chunk(b"VP8 ", &[1, 2, 3, 4])]);
        for cut in [0, 3, 11, 13, 19, 25, full.len() - 1] {
            assert!(run(&full[..cut]).is_err(), "cut at {cut}");
        }
        assert!(run(&riff(&[])).is_err());
        assert!(run(&riff(&[chunk(b"EXIF", b"only")])).is_err());
        assert!(run(&riff(&[chunk(b"VP8X", &[0; 4]), chunk(b"VP8 ", &[1])])).is_err());
        let mut lying = riff(&[chunk(b"VP8 ", &[1, 2])]);
        lying[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(run(&lying).is_err());
        let mut tiny = riff(&[chunk(b"VP8 ", &[1, 2])]);
        tiny[4..8].copy_from_slice(&1u32.to_le_bytes());
        assert!(run(&tiny).is_err());
    }
}
