//! TIFF, and an AVIF whose layout `heif` cannot follow: saved again by libvips
//! with `keep=icc` (the ICC profile is the one piece of metadata that changes
//! how the picture looks).
//!
//! - **TIFF** is lossless: the pixels are copied in their own depth, the
//!   Orientation tag is carried over by the saver (no pixel moves), and the
//!   compression stays what it was or, for a lossy one, becomes deflate. File to
//!   file, sequentially, so memory does not grow with the picture. Camera raws are
//!   TIFF on the outside and refused: libvips would read their small preview and
//!   throw the sensor data away.
//! - **AVIF** (only when the in-place blanking of `heif` refused the file) is
//!   saved at high quality, 4:4:4, with any orientation applied: a second
//!   generation of a lossy codec, the one quality cost of this feature, paid only
//!   for an unusual file. Refused, because the saver would lose them: sequences and
//!   multi-picture files, and more than 8 bits (the image's libaom has no high bit
//!   depth). A HEIC never comes here: the server's libvips has no HEVC codec
//!   ([`HEVC_DECODE`]).

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::{KeptReason, Kind};
use crate::media::convert::{ConvertTarget, conversion_budget};
use crate::media::vips::operation::{Arg, Operation};
use crate::media::vips::sharp::{DEFAULT_LIMIT_INPUT_PIXELS, HEVC_DECODE};
use crate::media::vips::{self, Image, VipsError};

/// AVIF: visually lossless at this setting.
const AVIF_QUALITY: i32 = 90;
/// `transform_cost`: effort 2 is 4.9 times faster than 4 for a 2% larger file.
const AVIF_EFFORT: i32 = 2;

/// Seconds libvips may spend on one picture before it is asked to stop.
const VIPS_TIMEOUT_SECONDS: u32 = 25;

/// What IFD0 of a TIFF says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TiffLook {
    /// `Compression` (259), 1 when absent.
    pub compression: u32,
    /// A camera raw: sensor data under a preview.
    pub raw: bool,
}

/// Read the first directory of a TIFF. `None` for a layout this does not follow
/// (a BigTIFF: libvips reads those, and raw files never are one).
pub fn tiff_look(head: &[u8]) -> Option<TiffLook> {
    let big = match head.get(..2)? {
        b"II" => false,
        b"MM" => true,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u32> {
        let b = head.get(at..at.checked_add(2)?)?;
        Some(u32::from(if big { u16::from_be_bytes([b[0], b[1]]) } else { u16::from_le_bytes([b[0], b[1]]) }))
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b = head.get(at..at.checked_add(4)?)?;
        Some(if big {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        })
    };
    if u16_at(2)? != 42 {
        return None;
    }
    let mut look = TiffLook { compression: 1, raw: head.get(8..10) == Some(b"CR") };
    let ifd = u32_at(4)? as usize;
    for i in 0..u16_at(ifd)? as usize {
        let entry = ifd + 2 + i * 12;
        let Some(tag) = u16_at(entry) else { break };
        match tag {
            // NewSubfileType: IFD0 being a reduced version of something else.
            254 => look.raw |= u32_at(entry + 8).is_some_and(|v| v & 1 != 0),
            259 => look.compression = u16_at(entry + 8).unwrap_or(1),
            // SubIFDs, DNGVersion.
            330 | 0xC612 => look.raw = true,
            _ => {}
        }
    }
    Some(look)
}

/// The libvips word for a TIFF compression; a lossy or an exotic one becomes
/// deflate, which loses nothing.
pub fn tiff_compression(code: u32) -> &'static str {
    match code {
        1 => "none",
        5 => "lzw",
        32773 => "packbits",
        50000 => "zstd",
        _ => "deflate",
    }
}

/// The directory of a TIFF may sit anywhere in the file: read what its header
/// points at.
fn read_tiff_look(path: &Path) -> Result<Option<TiffLook>, KeptReason> {
    let mut file = File::open(path).map_err(|_| KeptReason::Tool)?;
    let mut header = [0u8; 10];
    file.read_exact(&mut header).map_err(|_| KeptReason::Damaged)?;
    let big = match &header[..2] {
        b"II" => false,
        b"MM" => true,
        _ => return Err(KeptReason::Damaged),
    };
    let magic =
        if big { u16::from_be_bytes([header[2], header[3]]) } else { u16::from_le_bytes([header[2], header[3]]) };
    if magic == 43 {
        return Ok(None);
    }
    let bytes = [header[4], header[5], header[6], header[7]];
    let ifd = u64::from(if big { u32::from_be_bytes(bytes) } else { u32::from_le_bytes(bytes) });
    // A directory of 4096 entries at most is more than any TIFF has. Up to there
    // is all that is read: an offset in the middle of a big scan costs nothing.
    let mut window = Vec::new();
    {
        use std::io::{Seek, SeekFrom};
        file.seek(SeekFrom::Start(ifd)).map_err(|_| KeptReason::Damaged)?;
        file.take(2 + 4096 * 12).read_to_end(&mut window).map_err(|_| KeptReason::Tool)?;
    }
    // The same parser, over a header that says the directory comes right after it.
    let mut view = header[..4].to_vec();
    let eight = if big { 8u32.to_be_bytes() } else { 8u32.to_le_bytes() };
    view.extend_from_slice(&eight);
    view.extend_from_slice(&window);
    let Some(mut look) = tiff_look(&view) else {
        return Err(KeptReason::Damaged);
    };
    look.raw |= &header[8..10] == b"CR";
    Ok(Some(look))
}

fn load(loader: &str, path: &Path, access: &str, pages: i32) -> Result<Image, VipsError> {
    let path = path.to_string_lossy();
    Operation::new(loader)?
        .set("filename", Arg::Str(&path))
        .set("access", Arg::Nick(access))
        .set("n", pages)
        .set("fail_on", Arg::Nick("error"))
        .call_uncached()?
        .image("out")
}

fn strip_tiff(input: &Path, output: &Path) -> Result<(), KeptReason> {
    let look = read_tiff_look(input)?;
    if look.is_some_and(|l| l.raw) {
        return Err(KeptReason::Format);
    }
    let compression = tiff_compression(look.map_or(8, |l| l.compression));
    // Every page: `n` of -1. Pages of different sizes are refused by the loader.
    let image = load("tiffload", input, "sequential", -1).map_err(|err| {
        tracing::debug!("[strip] tiffload: {err}");
        KeptReason::Damaged
    })?;
    if image.width() as u64 * image.height() as u64 > DEFAULT_LIMIT_INPUT_PIXELS {
        return Err(KeptReason::TooLarge);
    }
    image.set_timeout(VIPS_TIMEOUT_SECONDS);
    let path = output.to_string_lossy();
    let saved = Operation::new("tiffsave")
        .map(|op| {
            op.set("in", &image)
                .set("filename", Arg::Str(&path))
                .set("compression", Arg::Nick(compression))
                .set("keep", Arg::Nick("icc"))
        })
        .and_then(Operation::call_uncached);
    saved.map(drop).map_err(save_failure)
}

fn save_failure(err: VipsError) -> KeptReason {
    tracing::debug!("[strip] save: {err}");
    if err.is_timeout() { KeptReason::TooLarge } else { KeptReason::Tool }
}

fn strip_heif(input: &Path, output: &Path) -> Result<(), KeptReason> {
    // libvips picks by the major brand and refuses a sequence outright.
    let mut brand = [0u8; 12];
    if File::open(input).and_then(|mut f| f.read_exact(&mut brand)).is_err() {
        return Err(KeptReason::Damaged);
    }
    if &brand[8..12] == b"avis" {
        return Err(KeptReason::Format);
    }
    let image = load("heifload", input, "random", 1).map_err(|err| {
        tracing::debug!("[strip] heifload: {err}");
        KeptReason::Damaged
    })?;
    if image.get_string("heif-compression").as_deref() == Some("hevc") && !HEVC_DECODE {
        return Err(KeptReason::Format);
    }
    if image.get_int_any("n-pages").is_some_and(|pages| pages > 1) || image.format_nick() != "uchar" {
        return Err(KeptReason::Format);
    }
    let pixels = image.width() as u64 * image.height() as u64;
    if !conversion_budget(false, pixels, pixels, ConvertTarget::Avif).ok {
        return Err(KeptReason::TooLarge);
    }
    let rotated = Operation::new("autorot")
        .and_then(|op| op.set("in", &image).call())
        .and_then(|built| built.image("out"))
        .map_err(|err| {
            tracing::debug!("[strip] autorot: {err}");
            KeptReason::Damaged
        })?;
    rotated.set_timeout(VIPS_TIMEOUT_SECONDS);
    let path = output.to_string_lossy();
    let saved = Operation::new("heifsave")
        .map(|op| {
            op.set("in", &rotated)
                .set("filename", Arg::Str(&path))
                .set("Q", AVIF_QUALITY)
                .set("compression", Arg::Nick("av1"))
                .set("effort", AVIF_EFFORT)
                .set("subsample_mode", Arg::Nick("off"))
                .set("keep", Arg::Nick("icc"))
        })
        .and_then(Operation::call_uncached);
    saved.map(drop).map_err(save_failure)
}

/// Clean one picture on the blocking pool; a file is always rewritten.
pub async fn strip(kind: Kind, input: PathBuf, output: PathBuf) -> Result<bool, KeptReason> {
    let joined = vips::run_blocking(move || {
        Ok(match kind {
            Kind::Tiff => strip_tiff(&input, &output),
            _ => strip_heif(&input, &output),
        })
    })
    .await;
    match joined {
        Ok(Ok(())) => Ok(true),
        Ok(Err(reason)) => Err(reason),
        Err(err) => {
            tracing::warn!("[strip] libvips: {err}");
            Err(KeptReason::Tool)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A little-endian TIFF header and one directory with these tags.
    fn tiff(tags: &[(u16, u16, u32)]) -> Vec<u8> {
        let mut v = b"II*\0\x08\0\0\0".to_vec();
        v.extend_from_slice(&(tags.len() as u16).to_le_bytes());
        for (tag, kind, value) in tags {
            v.extend_from_slice(&tag.to_le_bytes());
            v.extend_from_slice(&kind.to_le_bytes());
            v.extend_from_slice(&1u32.to_le_bytes());
            v.extend_from_slice(&value.to_le_bytes());
        }
        v.extend_from_slice(&0u32.to_le_bytes());
        v
    }

    #[test]
    fn the_first_directory_says_the_compression_and_whether_it_is_a_raw() {
        assert_eq!(tiff_look(&tiff(&[(256, 3, 8)])), Some(TiffLook { compression: 1, raw: false }));
        assert_eq!(tiff_look(&tiff(&[(259, 3, 5)])), Some(TiffLook { compression: 5, raw: false }));
        assert!(tiff_look(&tiff(&[(254, 4, 1)])).unwrap().raw);
        assert!(!tiff_look(&tiff(&[(254, 4, 0)])).unwrap().raw);
        assert!(tiff_look(&tiff(&[(330, 4, 100)])).unwrap().raw);
        assert!(tiff_look(&tiff(&[(0xC612, 1, 1)])).unwrap().raw);
    }

    #[test]
    fn a_canon_raw_says_so_after_the_header() {
        let mut cr2 = b"II*\0\x10\0\0\0CR\x02\0\0\0\0\0".to_vec();
        cr2.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        assert!(tiff_look(&cr2).unwrap().raw);
    }

    #[test]
    fn big_endian_directories_and_nonsense_are_handled() {
        let be = b"MM\0*\0\0\0\x08\0\x01\x01\x03\0\x03\0\0\0\x01\0\x05\0\0\0\0\0\0";
        assert_eq!(tiff_look(be), Some(TiffLook { compression: 5, raw: false }));
        assert_eq!(tiff_look(b""), None);
        assert_eq!(tiff_look(b"XX*\0"), None);
        assert_eq!(tiff_look(b"II+\0\x08\0\0\0"), None);
        assert_eq!(tiff_look(b"II*\0\xFF\xFF\xFF\xFF"), None);
        // A directory that claims more entries than there are bytes ends quietly.
        assert!(tiff_look(b"II*\0\x08\0\0\0\xFF\xFF").is_some());
    }

    #[test]
    fn a_lossy_compression_becomes_a_lossless_one() {
        assert_eq!(tiff_compression(1), "none");
        assert_eq!(tiff_compression(5), "lzw");
        assert_eq!(tiff_compression(8), "deflate");
        assert_eq!(tiff_compression(32946), "deflate");
        assert_eq!(tiff_compression(32773), "packbits");
        assert_eq!(tiff_compression(7), "deflate"); // JPEG
        assert_eq!(tiff_compression(34712), "deflate"); // JPEG 2000
    }
}
