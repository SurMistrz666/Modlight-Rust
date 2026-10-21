//! The picture on the clipboard as encoded bytes, for the app to decode under its own
//! limits (`modlight_core::image_decode`): the "PNG" format when a program put one
//! there, else the device-independent bitmap as a BMP file.
//!
//! `CF_DIBV5` is asked for before `CF_DIB` (the system offers both whenever either, or a
//! plain `CF_BITMAP`, is there), because only the version 5 header can say a picture has
//! transparency. A clipboard bitmap is a BMP file without its 14-byte file header; the
//! header is written here, with the offset of the pixels after the masks and colour
//! table. File lists and text are arboard's (it reads them correctly on Windows).

use std::time::Duration;

use windows_sys::Win32::Foundation::HGLOBAL;
use windows_sys::Win32::Graphics::Gdi::{BI_BITFIELDS, BI_RGB, BI_RLE4, BI_RLE8};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW,
};
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::System::Ole::{CF_DIB, CF_DIBV5};

use crate::wide::wide;

/// `BI_ALPHABITFIELDS`: four masks after a plain `BITMAPINFOHEADER`.
const BI_ALPHABITFIELDS: u32 = 6;
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
/// The largest PNG taken from the clipboard; a still picture may not decode to more
/// than 512 MiB anyway (`image_decode`).
const MAX_PNG_BYTES: usize = 512 * 1024 * 1024;

/// The PNG or BMP bytes on the clipboard, when it holds a picture.
pub fn image_bytes() -> Option<Vec<u8>> {
    let _open = OpenClipboardGuard::open()?;
    let png = wide("PNG");
    // SAFETY: `png` is NUL-terminated.
    let png = unsafe { RegisterClipboardFormatW(png.as_ptr()) };
    if png != 0
        && let Some(bytes) = clipboard_bytes(png, MAX_PNG_BYTES)
        && bytes.starts_with(PNG_SIGNATURE)
    {
        return Some(bytes);
    }
    for format in [CF_DIBV5, CF_DIB] {
        if let Some(dib) = clipboard_dib(format as u32)
            && let Some(bmp) = dib_to_bmp(&dib)
        {
            return Some(bmp);
        }
    }
    None
}

/// The clipboard, open for reading while this lives. Another program may hold it for a
/// moment, so opening is tried a few times; it is always closed again.
struct OpenClipboardGuard;

impl OpenClipboardGuard {
    fn open() -> Option<OpenClipboardGuard> {
        for attempt in 0..10 {
            // SAFETY: opens the clipboard for this thread; closed in `drop`.
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return Some(OpenClipboardGuard);
            }
            if attempt < 9 {
                std::thread::sleep(Duration::from_millis(15));
            }
        }
        log::warn!("Modlight: the clipboard is held by another program");
        None
    }
}

impl Drop for OpenClipboardGuard {
    fn drop(&mut self) {
        // SAFETY: the clipboard was opened by `open` on this thread.
        unsafe { CloseClipboard() };
    }
}

/// A copy of the clipboard's `format` (the clipboard is open), up to `limit` bytes.
fn clipboard_bytes(format: u32, limit: usize) -> Option<Vec<u8>> {
    // SAFETY: the clipboard is open (the caller holds the guard).
    if unsafe { IsClipboardFormatAvailable(format) } == 0 {
        return None;
    }
    // SAFETY: as above; the handle stays the clipboard's and is not freed here.
    let handle: HGLOBAL = unsafe { GetClipboardData(format) };
    if handle.is_null() {
        return None;
    }
    with_global(handle, |bytes| {
        (bytes.len() <= limit).then(|| bytes.to_vec())
    })
    .flatten()
}

/// The bitmap of `format`, copied only when its header names a size the app may decode.
fn clipboard_dib(format: u32) -> Option<Vec<u8>> {
    // SAFETY: the clipboard is open (the caller holds the guard).
    if unsafe { IsClipboardFormatAvailable(format) } == 0 {
        return None;
    }
    // SAFETY: as above.
    let handle: HGLOBAL = unsafe { GetClipboardData(format) };
    if handle.is_null() {
        return None;
    }
    with_global(handle, |bytes| {
        let header = DibHeader::read(bytes)?;
        modlight_core::image_decode::fits(header.width as u64, header.height.unsigned_abs() as u64)
            .then(|| bytes.to_vec())
    })
    .flatten()
}

/// `work` on the bytes of a global memory block, locked for the call.
fn with_global<T>(handle: HGLOBAL, work: impl FnOnce(&[u8]) -> T) -> Option<T> {
    // SAFETY: `handle` is a live global memory handle; it is unlocked below.
    let pointer = unsafe { GlobalLock(handle) };
    if pointer.is_null() {
        return None;
    }
    // SAFETY: as above.
    let size = unsafe { GlobalSize(handle) };
    // SAFETY: the locked block holds `size` bytes until it is unlocked.
    let answer = work(unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), size) });
    // SAFETY: balances the GlobalLock above.
    unsafe { GlobalUnlock(handle) };
    Some(answer)
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// The fields of a `BITMAPINFOHEADER` (or a later version) that place the pixels.
struct DibHeader {
    size: usize,
    width: i32,
    height: i32,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    colors_used: u32,
}

impl DibHeader {
    fn read(dib: &[u8]) -> Option<DibHeader> {
        let header = DibHeader {
            size: u32_at(dib, 0)? as usize,
            width: u32_at(dib, 4)? as i32,
            height: u32_at(dib, 8)? as i32,
            bit_count: u16_at(dib, 14)?,
            compression: u32_at(dib, 16)?,
            size_image: u32_at(dib, 20)?,
            colors_used: u32_at(dib, 32)?,
        };
        // The clipboard carries the 40-byte header or a later, longer one.
        (header.size >= 40 && header.size <= dib.len() && header.width > 0 && header.height != 0)
            .then_some(header)
    }

    /// Bytes of pixel data.
    fn pixel_bytes(&self) -> Option<usize> {
        if self.compression == BI_RLE8 || self.compression == BI_RLE4 {
            return Some(self.size_image as usize);
        }
        let stride = (self.width as u64 * self.bit_count as u64).div_ceil(32) * 4;
        usize::try_from(stride.checked_mul(self.height.unsigned_abs() as u64)?).ok()
    }
}

/// A packed DIB from the clipboard as a BMP file, or `None` when it is not a complete
/// one. Two habits of the clipboard are smoothed over, as other readers do: a 32-bit
/// picture that says `BI_RGB` while its version 5 header names an alpha mask (Chromium
/// writes those) keeps its transparency, and a 32-bit picture whose alpha is zero all
/// over (what the system's own conversions give) is opaque rather than invisible.
fn dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    let header = DibHeader::read(dib)?;
    let mut dib = dib.to_vec();
    let pixels = header.pixel_bytes()?;
    let has_masks = header.size >= 52;
    let has_alpha_mask = header.size >= 56;
    let mut compression = header.compression;
    if header.bit_count == 32
        && compression == BI_RGB
        && has_alpha_mask
        && u32_at(&dib, 52)? == 0xff00_0000
    {
        compression = BI_BITFIELDS;
        dib[16..20].copy_from_slice(&BI_BITFIELDS.to_le_bytes());
        if u32_at(&dib, 40)? | u32_at(&dib, 44)? | u32_at(&dib, 48)? == 0 {
            dib[40..44].copy_from_slice(&0x00ff_0000u32.to_le_bytes());
            dib[44..48].copy_from_slice(&0x0000_ff00u32.to_le_bytes());
            dib[48..52].copy_from_slice(&0x0000_00ffu32.to_le_bytes());
        }
    }
    // The masks after the header: always for a plain header with bit fields; after a
    // longer header (which holds them itself) only when the system wrote them again,
    // which its own conversion to `CF_DIBV5` does.
    let masks = match compression {
        BI_BITFIELDS if !has_masks => 12,
        BI_ALPHABITFIELDS if !has_masks => 16,
        BI_BITFIELDS
            if dib.len() >= header.size + 12 + pixels
                && dib.get(header.size..header.size + 12) == dib.get(40..52) =>
        {
            12
        }
        _ => 0,
    };
    let colors = match header.colors_used {
        0 if header.bit_count <= 8 => 1usize << header.bit_count,
        0 => 0,
        used if used <= 256 => used as usize,
        _ => return None,
    };
    let offset = header.size + masks + colors * 4;
    if offset.checked_add(pixels)? > dib.len() {
        return None;
    }
    if header.bit_count == 32 && compression == BI_BITFIELDS && has_alpha_mask {
        let alpha_mask = u32_at(&dib, 52)?;
        let transparent_all_over = alpha_mask != 0
            && dib[offset..offset + pixels]
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| u32::from_le_bytes(*p) & alpha_mask == 0);
        if transparent_all_over {
            dib[52..56].copy_from_slice(&0u32.to_le_bytes());
        }
    }
    let file_size = u32::try_from(14 + dib.len()).ok()?;
    let mut bmp = Vec::with_capacity(14 + dib.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&file_size.to_le_bytes());
    bmp.extend_from_slice(&[0, 0, 0, 0]);
    bmp.extend_from_slice(&u32::try_from(14 + offset).ok()?.to_le_bytes());
    bmp.extend_from_slice(&dib);
    Some(bmp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use modlight_core::image_decode;

    fn info_header(size: u32, width: i32, height: i32, bits: u16, compression: u32) -> Vec<u8> {
        let mut h = Vec::new();
        h.extend_from_slice(&size.to_le_bytes());
        h.extend_from_slice(&width.to_le_bytes());
        h.extend_from_slice(&height.to_le_bytes());
        h.extend_from_slice(&1u16.to_le_bytes());
        h.extend_from_slice(&bits.to_le_bytes());
        h.extend_from_slice(&compression.to_le_bytes());
        h.extend_from_slice(&[0; 20]);
        h
    }

    /// A version 5 header with the given masks.
    fn v5_header(width: i32, height: i32, compression: u32, masks: [u32; 4]) -> Vec<u8> {
        let mut h = info_header(124, width, height, 32, compression);
        for mask in masks {
            h.extend_from_slice(&mask.to_le_bytes());
        }
        h.extend_from_slice(b"BGRs");
        h.resize(124, 0);
        h
    }

    const MASKS: [u32; 4] = [0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0xff00_0000];

    fn decode(dib: &[u8]) -> image::RgbaImage {
        image_decode::from_bytes(&dib_to_bmp(dib).unwrap())
            .unwrap()
            .into_rgba8()
    }

    #[test]
    fn a_24_bit_bitmap_with_padded_rows() {
        // 2 x 2, bottom-up: the first row in memory is the bottom one. Rows are 8 bytes.
        let mut dib = info_header(40, 2, 2, 24, BI_RGB);
        dib.extend_from_slice(&[0, 0, 255, 0, 255, 0, 0, 0]); // bottom: red, green
        dib.extend_from_slice(&[255, 0, 0, 255, 255, 255, 0, 0]); // top: blue, white
        let picture = decode(&dib);
        assert_eq!(picture.get_pixel(0, 1).0, [255, 0, 0, 255]);
        assert_eq!(picture.get_pixel(1, 1).0, [0, 255, 0, 255]);
        assert_eq!(picture.get_pixel(0, 0).0, [0, 0, 255, 255]);
        assert_eq!(picture.get_pixel(1, 0).0, [255, 255, 255, 255]);
    }

    #[test]
    fn transparency_of_a_version_5_bitmap_is_kept() {
        let mut dib = v5_header(1, -1, BI_BITFIELDS, MASKS);
        dib.extend_from_slice(&[10, 20, 30, 128]);
        assert_eq!(decode(&dib).get_pixel(0, 0).0, [30, 20, 10, 128]);
    }

    #[test]
    fn masks_the_system_repeats_after_the_header_are_skipped() {
        let mut dib = v5_header(
            2,
            -1,
            BI_BITFIELDS,
            [0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0],
        );
        for mask in &MASKS[..3] {
            dib.extend_from_slice(&mask.to_le_bytes());
        }
        dib.extend_from_slice(&[1, 2, 3, 0, 4, 5, 6, 0]);
        let picture = decode(&dib);
        assert_eq!(picture.get_pixel(0, 0).0, [3, 2, 1, 255]);
        assert_eq!(picture.get_pixel(1, 0).0, [6, 5, 4, 255]);
    }

    #[test]
    fn chromium_style_rgb_with_an_alpha_mask_keeps_its_alpha() {
        let mut dib = v5_header(1, 1, BI_RGB, [0, 0, 0, 0xff00_0000]);
        dib.extend_from_slice(&[0, 0, 200, 64]);
        assert_eq!(decode(&dib).get_pixel(0, 0).0, [200, 0, 0, 64]);
    }

    #[test]
    fn alpha_that_is_zero_everywhere_means_opaque() {
        let mut dib = v5_header(2, 1, BI_BITFIELDS, MASKS);
        dib.extend_from_slice(&[9, 8, 7, 0, 6, 5, 4, 0]);
        let picture = decode(&dib);
        assert_eq!(picture.get_pixel(0, 0).0, [7, 8, 9, 255]);
        assert_eq!(picture.get_pixel(1, 0).0, [4, 5, 6, 255]);
    }

    #[test]
    fn a_plain_header_with_bit_fields_and_a_palette_picture() {
        let mut dib = info_header(40, 1, 1, 32, BI_BITFIELDS);
        for mask in &MASKS[..3] {
            dib.extend_from_slice(&mask.to_le_bytes());
        }
        dib.extend_from_slice(&[50, 60, 70, 0]);
        assert_eq!(decode(&dib).get_pixel(0, 0).0, [70, 60, 50, 255]);

        // 8 bits a pixel, two colours in the table.
        let mut dib = info_header(40, 2, 1, 8, BI_RGB);
        dib[32..36].copy_from_slice(&2u32.to_le_bytes());
        dib.extend_from_slice(&[0, 0, 0, 0, 255, 255, 255, 0]);
        dib.extend_from_slice(&[1, 0, 0, 0]);
        let picture = decode(&dib);
        assert_eq!(picture.get_pixel(0, 0).0, [255, 255, 255, 255]);
        assert_eq!(picture.get_pixel(1, 0).0, [0, 0, 0, 255]);
    }

    #[test]
    fn incomplete_or_odd_bitmaps_are_not_pictures() {
        let mut dib = info_header(40, 4, 4, 24, BI_RGB);
        dib.extend_from_slice(&[0; 10]);
        assert!(dib_to_bmp(&dib).is_none(), "too few pixels");
        assert!(dib_to_bmp(&info_header(40, 0, 4, 24, BI_RGB)).is_none());
        assert!(dib_to_bmp(&[40, 0, 0]).is_none());
        let mut dib = info_header(40, 1, 1, 8, BI_RGB);
        dib[32..36].copy_from_slice(&100_000u32.to_le_bytes());
        assert!(dib_to_bmp(&dib).is_none(), "colour table too long");
    }

    #[test]
    fn reading_the_clipboard_answers() {
        // Reads only; whatever is on this machine's clipboard, the call answers.
        if let Some(bytes) = image_bytes() {
            assert!(bytes.starts_with(PNG_SIGNATURE) || bytes.starts_with(b"BM"));
        }
    }
}
