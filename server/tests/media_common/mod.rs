//! Picture fixtures for the media tests, made with the libvips binding the way
//! upstream's tests make them with sharp (`create`, `join: { animated }`,
//! `raw`), so nothing binary is committed.
#![allow(dead_code)]

use bytes::Bytes;
use modlight_server::media::vips::{
    self, Arg, GifOptions, HeifOptions, Image, JpegOptions, Operation, PngOptions, Sharp, image::band_format,
};

/// xorshift32: deterministic "random" bytes.
pub fn noise(len: usize, seed: u32) -> Vec<u8> {
    let mut s = seed.max(1);
    (0..len)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s as u8
        })
        .collect()
}

/// Raw RGB pixels of one colour (sharp's `create: { channels: 3, background }`).
pub fn solid(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    rgb.iter().copied().cycle().take((width * height * 3) as usize).collect()
}

pub async fn png_of(raw: Vec<u8>, width: u32, height: u32, channels: u32) -> Bytes {
    Bytes::from(
        Sharp::raw(Bytes::from(raw), width, height, channels).png(PngOptions::default()).to_buffer().await.unwrap(),
    )
}

pub async fn jpeg_of(raw: Vec<u8>, width: u32, height: u32) -> Bytes {
    Bytes::from(Sharp::raw(Bytes::from(raw), width, height, 3).jpeg(JpegOptions::default()).to_buffer().await.unwrap())
}

/// `still()` of image-meta.test.ts: a flat 24x16 (or other) PNG.
pub async fn still(width: u32, height: u32) -> Bytes {
    png_of(solid(width, height, [200, 40, 90]), width, height, 3).await
}

/// A real animated GIF: `n` frames stacked as pages, `delay: 80, loop: 0`
/// (what `sharp(frames, { join: { animated: true } }).gif(...)` produces).
pub async fn animated_gif_with(n: u32, width: u32, height: u32, colour: impl Fn(u32) -> [u8; 3]) -> Bytes {
    let mut raw = Vec::with_capacity((width * height * 3 * n) as usize);
    for i in 0..n {
        raw.extend(solid(width, height, colour(i)));
    }
    gif_of_pages(raw, width, height, n, 80).await
}

pub async fn gif_of_pages(raw: Vec<u8>, width: u32, height: u32, n: u32, delay: u32) -> Bytes {
    Bytes::from(
        Sharp::raw(Bytes::from(raw), width, height * n, 3)
            .raw_page_height(height)
            .gif(GifOptions { delay: Some(vec![delay]), loop_count: Some(0), ..GifOptions::default() })
            .to_buffer()
            .await
            .unwrap(),
    )
}

/// image-meta.test.ts `animatedGif(n)`: 12x10 pages in distinct colours.
pub async fn animated_gif(n: u32) -> Bytes {
    animated_gif_with(n, 12, 10, |i| [(20 + i * 30) as u8, 10, 200]).await
}

/// An AVIF whose MAJOR brand is the sequence brand libvips refuses on sight.
pub async fn avif_sequence_brand() -> Bytes {
    let avif = Sharp::raw(Bytes::from(solid(16, 16, [10, 200, 90])), 16, 16, 3)
        .avif(HeifOptions { quality: 40, effort: 0, ..HeifOptions::default() })
        .to_buffer()
        .await
        .unwrap();
    let mut patched = avif.clone();
    patched[8..12].copy_from_slice(b"avis");
    Bytes::from(patched)
}

/// Pages in a buffer, counted the way a browser would see them.
pub async fn pages_of(buf: &Bytes) -> u32 {
    let meta = Sharp::new(buf.clone()).animated(true).metadata().await.unwrap();
    meta.pages.filter(|&p| p > 0).unwrap_or(1)
}

/// A JPEG carrying EXIF (`withExif`), and optionally an orientation tag.
pub fn jpeg_with_exif(width: u32, height: u32, orientation: Option<i32>) -> Bytes {
    vips::init().unwrap();
    let raw = solid(width, height, [90, 90, 90]);
    let image = Image::from_memory(&raw, width as i32, height as i32, 3, band_format::UCHAR)
        .unwrap()
        .copy_with_interpretation("srgb")
        .unwrap()
        .mutate(|m| {
            m.set_string("exif-ifd0-Copyright", "somebody");
            m.set_string("exif-ifd0-Software", "a camera");
            if let Some(o) = orientation {
                m.set_int("orientation", o);
            }
        })
        .unwrap();
    let op = Operation::new("jpegsave_buffer").unwrap().set("keep", Arg::Nick("exif"));
    Bytes::from(image.save_buffer(op).unwrap())
}

/// A minimal 24-bit BMP: a format this libvips may load through ImageMagick
/// but production's never could.
pub fn bmp(width: u32, height: u32) -> Bytes {
    let row = (width * 3).div_ceil(4) * 4;
    let size = 54 + row * height;
    let mut b = Vec::with_capacity(size as usize);
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&size.to_le_bytes());
    b.extend_from_slice(&[0, 0, 0, 0]);
    b.extend_from_slice(&54u32.to_le_bytes());
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(width as i32).to_le_bytes());
    b.extend_from_slice(&(height as i32).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24u16.to_le_bytes());
    b.extend_from_slice(&[0u8; 24]);
    for _ in 0..height {
        let mut r: Vec<u8> = (0..width).flat_map(|_| [40u8, 80, 120]).collect();
        r.resize(row as usize, 0);
        b.extend(r);
    }
    Bytes::from(b)
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// The classic decompression bomb: a few bytes of PNG whose header DECLARES
/// `width` x `height`.
pub fn png_claiming(width: u32, height: u32) -> Bytes {
    let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = b"IHDR".to_vec();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    b.extend_from_slice(&13u32.to_be_bytes());
    b.extend_from_slice(&ihdr);
    b.extend_from_slice(&crc32(&ihdr).to_be_bytes());
    let idat = b"IDAT\x78\x9c\x03\x00\x00\x00\x00\x01".to_vec();
    b.extend_from_slice(&((idat.len() - 4) as u32).to_be_bytes());
    b.extend_from_slice(&idat);
    b.extend_from_slice(&crc32(&idat).to_be_bytes());
    let iend = b"IEND".to_vec();
    b.extend_from_slice(&0u32.to_be_bytes());
    b.extend_from_slice(&iend);
    b.extend_from_slice(&crc32(&iend).to_be_bytes());
    Bytes::from(b)
}
