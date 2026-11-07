//! GIF: a walk over the blocks. Comment extensions, application extensions
//! other than the loop one (XMP, ICC and the like) and extensions this file
//! does not know are left out, and so is whatever follows the trailer. Frames,
//! their timing and the colour tables are copied as they are.

use std::io::{BufRead, Read, Write};

use super::{RewriteError, pass, read_all};

const IMAGE: u8 = 0x2C;
const EXTENSION: u8 = 0x21;
const TRAILER: u8 = 0x3B;

const GRAPHIC_CONTROL: u8 = 0xF9;
const PLAIN_TEXT: u8 = 0x01;
const APPLICATION: u8 = 0xFF;

/// The application extensions that make an animation loop.
const LOOP_IDS: [&[u8; 11]; 2] = [b"NETSCAPE2.0", b"ANIMEXTS1.0"];

/// Move a chain of sub-blocks, up to and including its terminator.
fn sub_blocks<R: Read, W: Write>(input: &mut R, mut output: Option<&mut W>) -> Result<(), RewriteError> {
    loop {
        let mut size = [0u8; 1];
        read_all(input, &mut size, "GIF: a block chain is cut off")?;
        if let Some(out) = output.as_deref_mut() {
            out.write_all(&size)?;
        }
        if size[0] == 0 {
            return Ok(());
        }
        pass(input, output.as_deref_mut(), u64::from(size[0]), "GIF: a block is cut off")?;
    }
}

/// A colour table of `flags`' size, when it has one.
fn colour_table<R: Read, W: Write>(input: &mut R, output: &mut W, flags: u8) -> Result<(), RewriteError> {
    if flags & 0x80 != 0 {
        pass(input, Some(output), 3u64 << ((flags & 0x07) + 1), "GIF: a colour table is cut off")?;
    }
    Ok(())
}

/// Rewrite a GIF. `Ok(false)` means there was nothing to remove. Constant memory.
///
/// A file that ends at a block boundary without a trailer is complete as far as
/// every viewer goes; a block that is cut off is a broken file.
pub fn strip<R: BufRead, W: Write>(input: &mut R, output: &mut W) -> Result<bool, RewriteError> {
    let mut header = [0u8; 13];
    read_all(input, &mut header, "GIF: too short")?;
    if &header[..6] != b"GIF87a" && &header[..6] != b"GIF89a" {
        return Err(RewriteError::Malformed("not a GIF"));
    }
    output.write_all(&header)?;
    colour_table(input, output, header[10])?;

    let mut changed = false;
    let mut frames = 0u32;
    loop {
        let mut byte = [0u8; 1];
        if input.read(&mut byte)? == 0 {
            if frames == 0 {
                return Err(RewriteError::Malformed("GIF: no picture"));
            }
            return Ok(changed);
        }
        match byte[0] {
            TRAILER => {
                if frames == 0 {
                    return Err(RewriteError::Malformed("GIF: no picture"));
                }
                output.write_all(&byte)?;
                return Ok(changed || !input.fill_buf()?.is_empty());
            }
            IMAGE => {
                let mut descriptor = [0u8; 9];
                read_all(input, &mut descriptor, "GIF: an image descriptor is cut off")?;
                output.write_all(&byte)?;
                output.write_all(&descriptor)?;
                colour_table(input, output, descriptor[8])?;
                pass(input, Some(&mut *output), 1, "GIF: an image is cut off")?;
                sub_blocks(input, Some(&mut *output))?;
                frames += 1;
            }
            EXTENSION => {
                let mut label = [0u8; 1];
                read_all(input, &mut label, "GIF: an extension is cut off")?;
                match label[0] {
                    GRAPHIC_CONTROL | PLAIN_TEXT => {
                        output.write_all(&[EXTENSION, label[0]])?;
                        sub_blocks(input, Some(&mut *output))?;
                    }
                    APPLICATION => {
                        let mut size = [0u8; 1];
                        read_all(input, &mut size, "GIF: an extension is cut off")?;
                        let mut id = vec![0u8; usize::from(size[0])];
                        read_all(input, &mut id, "GIF: an extension is cut off")?;
                        if LOOP_IDS.iter().any(|known| known.as_slice() == id.as_slice()) {
                            output.write_all(&[EXTENSION, APPLICATION, size[0]])?;
                            output.write_all(&id)?;
                            sub_blocks(input, Some(&mut *output))?;
                        } else {
                            sub_blocks(input, None::<&mut W>)?;
                            changed = true;
                        }
                    }
                    // The comment extension, and any extension this file has no
                    // name for: nothing a viewer draws.
                    _ => {
                        sub_blocks(input, None::<&mut W>)?;
                        changed = true;
                    }
                }
            }
            _ => return Err(RewriteError::Malformed("GIF: an unknown block")),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn gif(blocks: &[Vec<u8>]) -> Vec<u8> {
        let mut v = b"GIF89a".to_vec();
        v.extend_from_slice(&[1, 0, 1, 0, 0x80, 0, 0]); // a two-colour global table
        v.extend_from_slice(&[0, 0, 0, 255, 255, 255]);
        for b in blocks {
            v.extend_from_slice(b);
        }
        v
    }

    fn frame() -> Vec<u8> {
        let mut v = vec![IMAGE, 0, 0, 0, 0, 1, 0, 1, 0, 0];
        v.extend_from_slice(&[2, 2, 0x44, 0x01, 0]);
        v
    }

    fn with_local_table() -> Vec<u8> {
        let mut v = vec![IMAGE, 0, 0, 0, 0, 1, 0, 1, 0, 0x80];
        v.extend_from_slice(&[9, 9, 9, 8, 8, 8]);
        v.extend_from_slice(&[2, 2, 0x44, 0x01, 0]);
        v
    }

    fn control() -> Vec<u8> {
        vec![EXTENSION, GRAPHIC_CONTROL, 4, 0, 10, 0, 0, 0]
    }

    fn app(id: &[u8; 11], data: &[u8]) -> Vec<u8> {
        let mut v = vec![EXTENSION, APPLICATION, 11];
        v.extend_from_slice(id);
        v.push(data.len() as u8);
        v.extend_from_slice(data);
        v.push(0);
        v
    }

    fn comment(text: &[u8]) -> Vec<u8> {
        let mut v = vec![EXTENSION, 0xFE, text.len() as u8];
        v.extend_from_slice(text);
        v.push(0);
        v
    }

    fn run(input: &[u8]) -> Result<(bool, Vec<u8>), RewriteError> {
        let mut out = Vec::new();
        let changed = strip(&mut Cursor::new(input), &mut out)?;
        Ok((changed, out))
    }

    #[test]
    fn comments_and_foreign_application_extensions_go() {
        let keep = vec![app(b"NETSCAPE2.0", &[1, 0, 0]), control(), frame(), vec![TRAILER]];
        let input = gif(&[
            comment(b"made by Jan"),
            app(b"NETSCAPE2.0", &[1, 0, 0]),
            app(b"XMP DataXMP", b"<x:xmpmeta/>"),
            app(b"ICCRGBG1012", b"profile"),
            vec![EXTENSION, 0x77, 1, 2, 0],
            control(),
            frame(),
            vec![TRAILER],
        ]);
        let (changed, out) = run(&input).unwrap();
        assert!(changed);
        assert_eq!(out, gif(&keep));
    }

    #[test]
    fn the_loop_timing_and_colour_tables_stay() {
        let input = gif(&[
            app(b"ANIMEXTS1.0", &[1, 2, 0]),
            control(),
            frame(),
            control(),
            with_local_table(),
            vec![EXTENSION, PLAIN_TEXT, 1, 7, 0],
            vec![TRAILER],
        ]);
        let (changed, out) = run(&input).unwrap();
        assert!(!changed);
        assert_eq!(out, input);
    }

    #[test]
    fn what_follows_the_trailer_goes() {
        let clean = gif(&[frame(), vec![TRAILER]]);
        let mut input = clean.clone();
        input.extend_from_slice(b"a zip, say");
        let (changed, out) = run(&input).unwrap();
        assert!(changed);
        assert_eq!(out, clean);
    }

    #[test]
    fn a_missing_trailer_is_tolerated_at_a_block_boundary() {
        let input = gif(&[comment(b"x"), frame()]);
        let (changed, out) = run(&input).unwrap();
        assert!(changed);
        assert_eq!(out, gif(&[frame()]));
    }

    #[test]
    fn broken_layouts_are_refused() {
        let full = gif(&[comment(b"text"), frame(), vec![TRAILER]]);
        for cut in [0, 5, 12, 15, 21, 23, 25, 27, 29, 33] {
            assert!(run(&full[..cut]).is_err(), "cut at {cut}: {:?}", &full[..cut]);
        }
        assert!(run(&gif(&[vec![0x00]])).is_err());
        assert!(run(&gif(&[vec![TRAILER]])).is_err());
        assert!(run(&gif(&[])).is_err());
        assert!(run(&gif(&[frame(), vec![0x42]])).is_err());
        assert!(run(b"GIF90a1234567").is_err());
    }
}
