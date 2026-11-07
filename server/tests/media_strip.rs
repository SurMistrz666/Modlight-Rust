//! Removing metadata on upload (`media::strip`, SKELETON.md 9.47) with real
//! files: each format gets known metadata planted (GPS, author, comment, camera,
//! dates), is cleaned the way finalize cleans an upload, and is checked for two
//! things: the metadata is gone, and the picture or the streams are not touched
//! (decoded pixels compared exactly, packets hashed, orientation, colour profile
//! and animation kept).
//!
//! Fixtures are made here, with the libvips binding, ffmpeg and ImageMagick;
//! a test whose tool is missing on this machine says so and passes, as the other
//! media tests do.

mod media_common;

use std::path::{Path, PathBuf};
use std::process::Command;

use bytes::Bytes;
use media_common::solid;
use modlight_server::config::Config;
use modlight_server::media::ffmpeg::ffmpeg_available;
use modlight_server::media::strip::{KeptReason, Outcome, Source, jpeg::exif_orientation, strip_upload};
use modlight_server::media::vips::{
    self, Arg, Blob, GifOptions, Image, Operation, PngOptions, Sharp, WebpOptions, image::band_format,
};
use modlight_server::storage::LocalDriver;

const PLANTED: &[&str] = &["PLANTED", "Kowalski", "52.2297", "2021:05:05", "2021-05-05"];

struct Bench {
    dir: tempfile::TempDir,
    config: Config,
    storage: LocalDriver,
}

impl Bench {
    fn new() -> Bench {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().join("tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        let vars = [("UPLOAD_TMP_PATH".to_string(), tmp.to_string_lossy().into_owned())].into_iter().collect();
        let config = Config::from_env_map(&vars).unwrap();
        let storage = LocalDriver::new(dir.path().join("storage").to_str().unwrap());
        Bench { dir, config, storage }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// Clean `bytes` as finalize cleans a ShareX upload: `Ok(Some(cleaned))`,
    /// `Ok(None)` for nothing to do, `Err(reason)` for "kept as it came".
    async fn clean(&self, name: &str, bytes: &[u8]) -> Result<Option<Vec<u8>>, KeptReason> {
        let path = self.path(name);
        std::fs::write(&path, bytes).unwrap();
        let outcome = strip_upload(&self.config, &self.storage, Source::Local(&path), bytes.len() as i64, None).await;
        // The upload's own file is never changed.
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "{name}: the source was modified");
        let result = match outcome {
            Outcome::Untouched => Ok(None),
            Outcome::Kept(reason) => Err(reason),
            Outcome::Cleaned(cleaned) => {
                let out = std::fs::read(&cleaned.path).unwrap();
                assert_eq!(cleaned.size as usize, out.len());
                assert_eq!(cleaned.checksum, sha256(&out));
                Ok(Some(out))
            }
        };
        // The scratch directory is gone with the outcome.
        let left: Vec<_> = std::fs::read_dir(self.dir.path().join("tmp")).unwrap().collect();
        assert!(left.is_empty(), "{name}: scratch left behind: {left:?}");
        result
    }

    async fn cleaned(&self, name: &str, bytes: &[u8]) -> Vec<u8> {
        match self.clean(name, bytes).await {
            Ok(Some(out)) => out,
            other => panic!("{name}: expected a cleaned file, got {:?}", other.map(|o| o.map(|v| v.len()))),
        }
    }
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(bytes))
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn assert_no_planted(name: &str, bytes: &[u8]) {
    for planted in PLANTED {
        assert!(!contains(bytes, planted.as_bytes()), "{name}: {planted} survived");
    }
}

fn assert_planted(name: &str, bytes: &[u8]) {
    assert!(contains(bytes, b"PLANTED"), "{name}: the fixture carries no metadata to begin with");
}

/// Decoded pixels of every page, as libvips gives them (no rotation applied).
fn pixels(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let meta = Sharp::new(Bytes::copy_from_slice(bytes)).animated(true).metadata_blocking().unwrap();
    let raw = Sharp::new(Bytes::copy_from_slice(bytes)).animated(true).raw_output().to_buffer_blocking().unwrap();
    (meta.width, meta.height, raw)
}

fn metadata(bytes: &[u8]) -> vips::Metadata {
    Sharp::new(Bytes::copy_from_slice(bytes)).animated(true).metadata_blocking().unwrap()
}

// ── EXIF with everything a phone writes ─────────────────────────────────────

/// A little-endian TIFF block: IFD0 (Orientation, Make, Model, DateTime, Artist,
/// Exif and GPS pointers), the Exif IFD (DateTimeOriginal) and the GPS IFD.
fn exif_tiff(orientation: u16) -> Vec<u8> {
    fn entry(out: &mut Vec<u8>, tag: u16, kind: u16, count: u32, value: [u8; 4]) {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&value);
    }
    let strings: [(u16, &[u8]); 4] = [
        (0x010F, b"PLANTED-Make\0"),
        (0x0110, b"PLANTED-Model\0"),
        (0x0132, b"2021:05:05 10:00:00\0"),
        (0x013B, b"Jan Kowalski\0"),
    ];
    let ifd0_entries = 1 + strings.len() + 2;
    let ifd0_len = 2 + ifd0_entries * 12 + 4;
    let data_at = 8 + ifd0_len;
    let mut ifd0 = (ifd0_entries as u16).to_le_bytes().to_vec();
    let mut data = Vec::new();
    entry(&mut ifd0, 0x0112, 3, 1, [orientation as u8, 0, 0, 0]);
    for (tag, text) in strings {
        entry(&mut ifd0, tag, 2, text.len() as u32, (data_at as u32 + data.len() as u32).to_le_bytes());
        data.extend_from_slice(text);
    }
    // The Exif IFD and the GPS IFD come after the strings.
    let exif_ifd_at = data_at + data.len();
    let date = b"2021:05:05 10:00:00\0";
    let exif_ifd_len = 2 + 12 + 4;
    let gps_ifd_at = exif_ifd_at + exif_ifd_len + date.len();
    entry(&mut ifd0, 0x8769, 4, 1, (exif_ifd_at as u32).to_le_bytes());
    entry(&mut ifd0, 0x8825, 4, 1, (gps_ifd_at as u32).to_le_bytes());
    ifd0.extend_from_slice(&0u32.to_le_bytes());
    let mut exif_ifd = 1u16.to_le_bytes().to_vec();
    entry(&mut exif_ifd, 0x9003, 2, date.len() as u32, ((exif_ifd_at + exif_ifd_len) as u32).to_le_bytes());
    exif_ifd.extend_from_slice(&0u32.to_le_bytes());
    data.extend(exif_ifd);
    data.extend_from_slice(date);
    let rationals_at = gps_ifd_at + 2 + 2 * 12 + 4;
    let mut gps = 2u16.to_le_bytes().to_vec();
    entry(&mut gps, 0x0001, 2, 2, *b"N\0\0\0");
    entry(&mut gps, 0x0002, 5, 3, (rationals_at as u32).to_le_bytes());
    gps.extend_from_slice(&0u32.to_le_bytes());
    // 52 / 13 / 47.0 seconds ("52.2297" written out for the search).
    for (n, d) in [(52u32, 1u32), (13, 1), (4692, 100)] {
        gps.extend_from_slice(&n.to_le_bytes());
        gps.extend_from_slice(&d.to_le_bytes());
    }
    data.extend(gps);
    data.extend_from_slice(b"52.2297 PLANTED-GPS");
    let mut tiff = b"II*\0\x08\0\0\0".to_vec();
    tiff.extend(ifd0);
    tiff.extend(data);
    tiff
}

fn segment(marker: u8, body: &[u8]) -> Vec<u8> {
    let mut v = vec![0xFF, marker];
    v.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
    v.extend_from_slice(body);
    v
}

/// A JPEG with a real sRGB profile, encoded by libvips.
fn jpeg_with_profile(width: u32, height: u32) -> Vec<u8> {
    vips::init().unwrap();
    let mut raw = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            raw.extend_from_slice(&[(x * 7) as u8, (y * 11) as u8, ((x + y) * 5) as u8]);
        }
    }
    let image = Image::from_memory(&raw, width as i32, height as i32, 3, band_format::UCHAR)
        .unwrap()
        .copy_with_interpretation("srgb")
        .unwrap();
    let op = Operation::new("jpegsave_buffer")
        .unwrap()
        .set("Q", 90)
        .set("profile", Arg::Str("srgb"))
        .set("keep", Arg::Nick("icc"));
    image.save_buffer(op).unwrap()
}

/// That JPEG with EXIF (rotated, GPS, camera, author, dates), XMP, IPTC, MPF and
/// a comment planted after its own segments, and a second picture after it.
fn phone_jpeg(orientation: u16) -> Vec<u8> {
    let plain = jpeg_with_profile(48, 32);
    let mut exif = b"Exif\0\0".to_vec();
    exif.extend(exif_tiff(orientation));
    let mut out = plain[..2].to_vec();
    out.extend(segment(0xE1, &exif));
    out.extend(segment(
        0xE1,
        b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta><dc:creator>PLANTED Jan Kowalski</dc:creator></x:xmpmeta>",
    ));
    out.extend(segment(0xED, b"Photoshop 3.0\08BIM\x04\x04\0\0\0\0\0\x0fPLANTED-IPTC-by"));
    out.extend(segment(0xE2, b"MPF\0II*\0PLANTED-MPF"));
    out.extend(segment(0xFE, b"PLANTED comment"));
    out.extend_from_slice(&plain[2..]);
    out.extend_from_slice(b"\xFF\xD8PLANTED second picture of a motion photo");
    out
}

#[tokio::test]
async fn jpeg_loses_exif_xmp_iptc_and_comments_but_keeps_pixels_orientation_and_profile() {
    let bench = Bench::new();
    let photo = phone_jpeg(6);
    assert_planted("jpeg", &photo);
    let before = metadata(&photo);
    assert_eq!(before.orientation, Some(6));
    assert!(before.has_profile);

    let out = bench.cleaned("photo.jpg", &photo).await;
    assert_no_planted("jpeg", &out);
    assert!(!contains(&out, b"MPF\0") && !contains(&out, b"Photoshop") && !contains(&out, b"xap/1.0"));
    let after = metadata(&out);
    assert_eq!(after.orientation, Some(6), "a phone photo must not turn sideways");
    assert!(after.has_profile, "the colour profile stays");
    let exif = after.exif.expect("the minimal EXIF");
    assert_eq!(exif_orientation(&exif[6..]), Some(6));
    assert_eq!(pixels(&photo), pixels(&out), "the decoded picture is the same");
    // The profile is the same bytes.
    let icc_at = |b: &[u8]| b.windows(12).position(|w| w == b"ICC_PROFILE\0").unwrap();
    let (a, b) = (icc_at(&photo), icc_at(&out));
    let len = u16::from_be_bytes([photo[a - 2], photo[a - 1]]) as usize;
    assert_eq!(photo[a..a + len - 2], out[b..b + len - 2]);

    // An upright photo keeps no EXIF at all; a clean one is left alone.
    let upright = bench.cleaned("upright.jpg", &phone_jpeg(1)).await;
    assert!(metadata(&upright).exif.is_none());
    assert_eq!(bench.clean("again.jpg", &upright).await, Ok(None));
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut v = (data.len() as u32).to_be_bytes().to_vec();
    v.extend_from_slice(kind);
    v.extend_from_slice(data);
    let mut crc = crc32fast::Hasher::new();
    crc.update(kind);
    crc.update(data);
    v.extend_from_slice(&crc.finalize().to_be_bytes());
    v
}

fn png_chunks(png: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut at = 8;
    while at + 8 <= png.len() {
        let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        out.push(String::from_utf8_lossy(&png[at + 4..at + 8]).into_owned());
        at += 12 + len;
    }
    out
}

#[tokio::test]
async fn png_loses_text_time_and_exif_but_keeps_pixels_and_colour() {
    let bench = Bench::new();
    let mut raw = Vec::new();
    for y in 0..20u32 {
        for x in 0..30u32 {
            raw.extend_from_slice(&[(x * 8) as u8, (y * 12) as u8, 77, (255 - x * 3) as u8]);
        }
    }
    let plain = Sharp::raw(Bytes::from(raw), 30, 20, 4).png(PngOptions::default()).to_buffer_blocking().unwrap();
    let mut planted = plain[..33].to_vec();
    planted.extend(png_chunk(b"gAMA", &45455u32.to_be_bytes()));
    planted.extend(png_chunk(b"tEXt", b"Author\0Jan Kowalski"));
    planted.extend(png_chunk(b"zTXt", b"Comment\0\0x\x9c\x0b\xf0\xf1\xf4\x0bq\x0d\x01\0\x0b\x8a\x02\xa6"));
    planted.extend(png_chunk(b"iTXt", b"XML:com.adobe.xmp\0\0\0\0\0<x:xmpmeta>PLANTED 52.2297</x:xmpmeta>"));
    planted.extend(png_chunk(b"tIME", &[7, 229, 5, 5, 10, 0, 0]));
    planted.extend(png_chunk(b"eXIf", &exif_tiff(1)));
    planted.extend(png_chunk(b"pHYs", &[0, 0, 11, 19, 0, 0, 11, 19, 1]));
    planted.extend_from_slice(&plain[33..]);
    assert_planted("png", &planted);

    let out = bench.cleaned("shot.png", &planted).await;
    assert_no_planted("png", &out);
    let chunks = png_chunks(&out);
    assert!(chunks.contains(&"gAMA".to_string()) && chunks.contains(&"pHYs".to_string()), "{chunks:?}");
    for gone in ["tEXt", "zTXt", "iTXt", "tIME", "eXIf"] {
        assert!(!chunks.contains(&gone.to_string()), "{gone}: {chunks:?}");
    }
    assert_eq!(pixels(&planted), pixels(&out));
}

fn riff_chunk(name: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut v = name.to_vec();
    v.extend_from_slice(&(data.len() as u32).to_le_bytes());
    v.extend_from_slice(data);
    if data.len() % 2 == 1 {
        v.push(0);
    }
    v
}

fn riff(chunks: &[Vec<u8>]) -> Vec<u8> {
    let body = chunks.concat();
    let mut v = b"RIFF".to_vec();
    v.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
    v.extend_from_slice(b"WEBP");
    v.extend(body);
    v
}

fn webp_chunk_names(webp: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut at = 12;
    while at + 8 <= webp.len() {
        let len = u32::from_le_bytes(webp[at + 4..at + 8].try_into().unwrap()) as usize;
        out.push(String::from_utf8_lossy(&webp[at..at + 4]).into_owned());
        at += 8 + len + (len & 1);
    }
    out
}

#[tokio::test]
async fn webp_loses_exif_and_xmp_and_keeps_pixels_profile_and_animation() {
    let bench = Bench::new();
    // A still: libvips' extended file (VP8X + ICCP + VP8), then EXIF and XMP added.
    vips::init().unwrap();
    let image = Image::from_memory(&solid(40, 24, [30, 120, 210]), 40, 24, 3, band_format::UCHAR)
        .unwrap()
        .copy_with_interpretation("srgb")
        .unwrap();
    let op = Operation::new("webpsave_buffer")
        .unwrap()
        .set("Q", 80)
        .set("profile", Arg::Str("srgb"))
        .set("keep", Arg::Nick("icc"));
    let with_profile = image.save_buffer(op).unwrap();
    let names = webp_chunk_names(&with_profile);
    assert_eq!(names.first().map(String::as_str), Some("VP8X"), "{names:?}");
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    let mut at = 12;
    while at + 8 <= with_profile.len() {
        let len = u32::from_le_bytes(with_profile[at + 4..at + 8].try_into().unwrap()) as usize;
        let mut chunk = with_profile[at..at + 8 + len + (len & 1)].to_vec();
        if &chunk[..4] == b"VP8X" {
            chunk[8] |= 0x08 | 0x04; // EXIF and XMP flags
        }
        chunks.push(chunk);
        at += 8 + len + (len & 1);
    }
    let mut exif = b"Exif\0\0".to_vec();
    exif.extend(exif_tiff(1));
    chunks.push(riff_chunk(b"EXIF", &exif));
    chunks.push(riff_chunk(b"XMP ", b"<x:xmpmeta>PLANTED Jan Kowalski</x:xmpmeta>"));
    let planted = riff(&chunks);
    assert_planted("webp", &planted);
    let out = bench.cleaned("still.webp", &planted).await;
    assert_no_planted("webp", &out);
    let names = webp_chunk_names(&out);
    assert!(names.contains(&"ICCP".to_string()) && !names.contains(&"EXIF".to_string()), "{names:?}");
    assert_eq!(out[20] & 0x0C, 0, "the VP8X flags no longer promise EXIF or XMP");
    assert!(metadata(&out).has_profile);
    assert_eq!(pixels(&planted), pixels(&out));

    // An animation keeps every frame, its timing and its loop.
    let frames: Vec<u8> = (0..3).flat_map(|i| solid(16, 12, [i * 80, 40, 200 - i * 60])).collect();
    let animated = Sharp::raw(Bytes::from(frames), 16, 36, 3)
        .raw_page_height(12)
        .webp(WebpOptions { loop_count: Some(0), delay: Some(vec![120]), ..WebpOptions::default() })
        .to_buffer_blocking()
        .unwrap();
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    let mut at = 12;
    while at + 8 <= animated.len() {
        let len = u32::from_le_bytes(animated[at + 4..at + 8].try_into().unwrap()) as usize;
        let mut chunk = animated[at..at + 8 + len + (len & 1)].to_vec();
        if &chunk[..4] == b"VP8X" {
            chunk[8] |= 0x08;
        }
        chunks.push(chunk);
        at += 8 + len + (len & 1);
    }
    chunks.push(riff_chunk(b"EXIF", &exif));
    let planted = riff(&chunks);
    let out = bench.cleaned("loop.webp", &planted).await;
    assert_no_planted("animated webp", &out);
    let (a, b) = (metadata(&planted), metadata(&out));
    assert_eq!((b.pages, b.loop_count, b.delay.clone()), (a.pages, a.loop_count, a.delay.clone()));
    assert_eq!(b.pages, Some(3));
    assert_eq!(pixels(&planted), pixels(&out));
}

#[tokio::test]
async fn gif_loses_comments_and_foreign_extensions_and_keeps_every_frame() {
    let bench = Bench::new();
    let frames: Vec<u8> = (0..4).flat_map(|i| solid(20, 10, [i * 60, 255 - i * 50, 90])).collect();
    let plain = Sharp::raw(Bytes::from(frames), 20, 40, 3)
        .raw_page_height(10)
        .gif(GifOptions { loop_count: Some(0), delay: Some(vec![90]), ..GifOptions::default() })
        .to_buffer_blocking()
        .unwrap();
    let flags = plain[10];
    let table = if flags & 0x80 != 0 { 3usize << ((flags & 7) + 1) } else { 0 };
    let at = 13 + table;
    let mut planted = plain[..at].to_vec();
    planted.extend_from_slice(&[0x21, 0xFE, 15]);
    planted.extend_from_slice(b"PLANTED comment");
    planted.push(0);
    planted.extend_from_slice(&[0x21, 0xFF, 11]);
    planted.extend_from_slice(b"XMP DataXMP");
    let xmp = b"<x:xmpmeta>Jan Kowalski</x:xmpmeta>";
    planted.push(xmp.len() as u8);
    planted.extend_from_slice(xmp);
    planted.push(0);
    planted.extend_from_slice(&plain[at..]);
    assert_planted("gif", &planted);

    let out = bench.cleaned("loop.gif", &planted).await;
    assert_no_planted("gif", &out);
    assert!(contains(&out, b"NETSCAPE2.0") || metadata(&plain).loop_count == metadata(&out).loop_count);
    let (a, b) = (metadata(&planted), metadata(&out));
    assert_eq!((b.pages, b.loop_count, b.delay.clone()), (a.pages, a.loop_count, a.delay.clone()));
    assert_eq!(b.pages, Some(4));
    assert_eq!(pixels(&planted), pixels(&out));
}

/// A HEIF picture with EXIF (rotation, camera, GPS) saved by libvips.
fn heif_with_exif(compression: &str) -> Option<Vec<u8>> {
    vips::init().unwrap();
    let image = Image::from_memory(&solid(64, 48, [200, 90, 30]), 64, 48, 3, band_format::UCHAR)
        .ok()?
        .copy_with_interpretation("srgb")
        .ok()?
        .mutate(|m| {
            m.set_string("exif-ifd0-Make", "PLANTED-Make");
            m.set_string("exif-ifd0-Artist", "Jan Kowalski");
            m.set_string("exif-ifd0-DateTime", "2021:05:05 10:00:00");
            m.set_int("orientation", 6);
        })
        .ok()?;
    let op = Operation::new("heifsave_buffer")
        .ok()?
        .set("Q", 60)
        .set("compression", Arg::Nick(compression))
        .set("effort", 0)
        .set("keep", Arg::Nick("exif"));
    image.save_buffer(op).ok()
}

/// Decode a HEIF file's first picture through libvips directly (HEVC too,
/// where this libheif can).
fn heif_pixels(bytes: &[u8]) -> Option<Vec<u8>> {
    let blob = Blob::from_bytes(&Bytes::copy_from_slice(bytes)).ok()?;
    let image = Operation::new("heifload_buffer").ok()?.set("buffer", &blob).call().ok()?.image("out").ok()?;
    image.write_to_memory().ok()
}

#[tokio::test]
async fn avif_and_heic_lose_their_exif_in_place_and_keep_every_coded_byte() {
    let bench = Bench::new();
    for (compression, name) in [("av1", "photo.avif"), ("hevc", "photo.heic")] {
        let Some(planted) = heif_with_exif(compression) else {
            eprintln!("this libvips cannot save {compression} HEIF; {name} skipped");
            continue;
        };
        assert_planted(name, &planted);
        let out = bench.cleaned(name, &planted).await;
        assert_no_planted(name, &out);
        assert_eq!(out.len(), planted.len(), "{name}: nothing moves");
        // Every byte that changed was inside the EXIF item.
        let first = planted.iter().zip(&out).position(|(a, b)| a != b).unwrap();
        let last = planted.iter().zip(&out).rposition(|(a, b)| a != b).unwrap();
        let exif = planted.windows(6).rposition(|w| w == b"Exif\0\0").unwrap();
        assert!(first >= exif.saturating_sub(4) && last < exif + 512, "{name}: {first}..{last} vs EXIF at {exif}");
        // The rotation is kept in the minimal EXIF.
        let at = out.windows(9).position(|w| w == b"Exif\0\0II*").expect("minimal EXIF");
        assert_eq!(exif_orientation(&out[at + 6..]), Some(6), "{name}");
        match (heif_pixels(&planted), heif_pixels(&out)) {
            (Some(a), Some(b)) => assert_eq!(a, b, "{name}: the decoded picture is the same"),
            _ => eprintln!("this libheif cannot decode {compression}; {name}: pixels not compared"),
        }
        assert_eq!(bench.clean(&format!("again-{name}"), &out).await, Ok(None));
    }
}

fn magick() -> Option<&'static str> {
    ["magick"].into_iter().find(|tool| Command::new(tool).arg("-version").output().is_ok_and(|o| o.status.success()))
}

fn run(program: &str, args: &[&str]) -> bool {
    Command::new(program).args(args).output().is_ok_and(|o| o.status.success())
}

#[tokio::test]
async fn tiff_loses_its_tags_losslessly_and_keeps_its_orientation() {
    let Some(magick) = magick() else {
        eprintln!("no ImageMagick on this machine; skipped");
        return;
    };
    let bench = Bench::new();
    let base = bench.path("base.png");
    assert!(run(magick, &["-size", "64x48", "gradient:red-blue", base.to_str().unwrap()]));
    for (compression, name) in [("lzw", "scan.tif"), ("jpeg", "scan-jpeg.tif"), ("none", "scan-raw.tif")] {
        let tiff = bench.path(name);
        let ok = run(
            magick,
            &[
                base.to_str().unwrap(),
                "-compress",
                compression,
                "-set",
                "tiff:artist",
                "Jan Kowalski",
                "-set",
                "tiff:make",
                "PLANTED-Make",
                "-set",
                "tiff:model",
                "PLANTED-Model",
                "-set",
                "tiff:software",
                "PLANTED-Software",
                "-set",
                "tiff:timestamp",
                "2021:05:05 10:00:00",
                "-set",
                "comment",
                "PLANTED comment",
                "-orient",
                "RightTop",
                tiff.to_str().unwrap(),
            ],
        );
        if !ok {
            eprintln!("ImageMagick cannot write a {compression} TIFF; skipped");
            continue;
        }
        let planted = std::fs::read(&tiff).unwrap();
        assert_planted(name, &planted);
        let out = bench.cleaned(name, &planted).await;
        assert_no_planted(name, &out);
        assert_eq!(metadata(&out).orientation, Some(6), "{name}");
        assert_eq!(pixels(&planted), pixels(&out), "{name}: the same pixels, losslessly");
    }
}

// ── Audio and video ─────────────────────────────────────────────────────────

fn ffmpeg(args: &[&str]) {
    let out = Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).output().unwrap();
    assert!(out.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn probe(path: &Path) -> serde_json::Value {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_format", "-show_streams", "-show_chapters", "-of", "json"])
        .arg(path)
        .output()
        .unwrap();
    serde_json::from_slice(&out.stdout).unwrap()
}

/// The packets of every audio and picture stream, hashed: the same for a stream
/// copy. (Data streams, where a phone keeps its telemetry and an MP4 its
/// chapter track, are dropped on purpose.)
fn packets(path: &Path) -> String {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-map", "0", "-c", "copy", "-f", "streamhash", "-hash", "sha256", "-"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().lines().filter(|l| l.contains(",v,") || l.contains(",a,")).collect()
}

fn has_data_stream(info: &serde_json::Value) -> bool {
    info["streams"].as_array().into_iter().flatten().any(|s| s["codec_type"] == "data")
}

fn rotation(info: &serde_json::Value) -> Option<i64> {
    info["streams"]
        .as_array()?
        .iter()
        .find_map(|s| s["side_data_list"].as_array()?.iter().find_map(|d| d["rotation"].as_i64()))
}

fn tags(info: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    let holders = std::iter::once(&info["format"])
        .chain(info["streams"].as_array().into_iter().flatten())
        .chain(info["chapters"].as_array().into_iter().flatten());
    for holder in holders {
        if let Some(map) = holder["tags"].as_object() {
            for (k, v) in map {
                out.push(format!("{k}={}", v.as_str().unwrap_or_default()));
            }
        }
    }
    out
}

#[tokio::test]
async fn audio_and_video_lose_tags_and_chapters_by_stream_copy() {
    if !ffmpeg_available().await {
        eprintln!("no ffmpeg on this machine; skipped");
        return;
    }
    let bench = Bench::new();
    let dir = bench.dir.path();
    let p = |n: &str| dir.join(n).to_string_lossy().into_owned();
    let video = ["-f", "lavfi", "-i", "testsrc=size=96x64:rate=10:duration=1"];
    let audio = ["-f", "lavfi", "-i", "sine=frequency=440:duration=1"];
    let tags_args = [
        "-metadata",
        "title=PLANTED title",
        "-metadata",
        "artist=Jan Kowalski",
        "-metadata",
        "comment=PLANTED comment",
        "-metadata",
        "date=2021-05-05",
        "-metadata",
        "creation_time=2021-05-05T10:00:00Z",
        "-metadata",
        "location=+52.2297+021.0122/",
    ];
    let chapters = p("chapters.txt");
    std::fs::write(&chapters, ";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=500\ntitle=PLANTED chapter\n")
        .unwrap();
    // name, encoder arguments, muxer arguments
    let cases: Vec<(&str, Vec<&str>)> = vec![
        ("phone.mp4", vec!["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-movflags", "use_metadata_tags"]),
        ("phone.mov", vec!["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"]),
        ("clip.m4v", vec!["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-f", "mp4", "-brand", "M4V "]),
        ("clip.mkv", vec!["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"]),
        ("clip.webm", vec!["-c:v", "libvpx", "-c:a", "libvorbis"]),
        ("clip.avi", vec!["-c:v", "mpeg4", "-c:a", "libmp3lame"]),
        ("song.m4a", vec!["-vn", "-c:a", "aac", "-f", "ipod"]),
        ("song.mp3", vec!["-vn", "-c:a", "libmp3lame"]),
        ("song.flac", vec!["-vn", "-c:a", "flac"]),
        ("song.ogg", vec!["-vn", "-c:a", "libvorbis"]),
        ("song.opus", vec!["-vn", "-c:a", "libopus"]),
        ("song.wav", vec!["-vn", "-c:a", "pcm_s16le"]),
    ];
    for (name, codec) in cases {
        let out_path = p(&format!("plain-{name}"));
        let rotated = name.ends_with(".mp4") || name.ends_with(".mov");
        // A phone's rotation is side data that only a stream copy keeps (an
        // encode bakes it into the pixels): encode first, then rotate.
        let encoded_path = p(&format!("encoded-{name}"));
        let target = if rotated { &encoded_path } else { &out_path };
        let mut args: Vec<&str> = Vec::new();
        args.extend(video);
        args.extend(audio);
        args.extend(["-i", &chapters, "-map", "0", "-map", "1", "-map_metadata", "2", "-map_chapters", "2"]);
        args.extend(codec.iter().copied());
        args.extend(tags_args);
        args.push(target);
        let status =
            Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-y"]).args(&args).output().unwrap();
        if !status.status.success() {
            eprintln!("this ffmpeg cannot make {name}: {}", String::from_utf8_lossy(&status.stderr).trim());
            continue;
        }
        if rotated {
            let muxer = if name.ends_with(".mov") { "mov" } else { "mp4" };
            ffmpeg(&[
                "-display_rotation",
                "90",
                "-i",
                &encoded_path,
                // The chapters come back from the chapter list, as a new track.
                "-map",
                "0:v",
                "-map",
                "0:a",
                "-c",
                "copy",
                "-movflags",
                "use_metadata_tags",
                "-f",
                muxer,
                &out_path,
            ]);
        }
        let planted = std::fs::read(&out_path).unwrap();
        let before = probe(Path::new(&out_path));
        assert!(
            tags(&before).iter().any(|t| t.contains("PLANTED") || t.contains("Kowalski")),
            "{name}: {:?}",
            tags(&before)
        );

        let out = bench.cleaned(name, &planted).await;
        let cleaned = dir.join(format!("cleaned-{name}"));
        std::fs::write(&cleaned, &out).unwrap();
        let after = probe(&cleaned);
        let left = tags(&after);
        assert!(
            !left.iter().any(|t| ["PLANTED", "Kowalski", "2021", "52.2297", "creation_time", "location"]
                .iter()
                .any(|w| t.contains(w))),
            "{name}: {left:?}"
        );
        assert!(after["chapters"].as_array().is_none_or(|c| c.is_empty()), "{name}: chapters left");
        assert_eq!(rotation(&after), rotation(&before), "{name}: rotation");
        assert!(!has_data_stream(&after), "{name}: a data stream is left");
        assert_eq!(packets(Path::new(&out_path)), packets(&cleaned), "{name}: the packets changed");
        assert_eq!(after["format"]["format_name"], before["format"]["format_name"], "{name}: the container changed");
        if rotated {
            assert!(rotation(&before).is_some(), "{name}: the fixture was not rotated");
        }
    }
}

#[tokio::test]
async fn cover_art_is_a_stream_and_stays_while_the_tags_go() {
    if !ffmpeg_available().await {
        eprintln!("no ffmpeg on this machine; skipped");
        return;
    }
    let bench = Bench::new();
    let dir = bench.dir.path();
    let cover = dir.join("cover.jpg");
    std::fs::write(&cover, jpeg_with_profile(32, 32)).unwrap();
    let song = dir.join("plain.mp3");
    let tone = dir.join("tone.mp3");
    ffmpeg(&["-f", "lavfi", "-i", "sine=frequency=440:duration=1", "-c:a", "libmp3lame", tone.to_str().unwrap()]);
    ffmpeg(&[
        "-i",
        tone.to_str().unwrap(),
        "-i",
        cover.to_str().unwrap(),
        "-map",
        "0",
        "-map",
        "1",
        "-c",
        "copy",
        "-disposition:v",
        "attached_pic",
        "-metadata",
        "artist=Jan Kowalski",
        "-metadata",
        "comment=PLANTED",
        "-id3v2_version",
        "3",
        song.to_str().unwrap(),
    ]);
    let planted = std::fs::read(&song).unwrap();
    let out = bench.cleaned("song.mp3", &planted).await;
    let cleaned = dir.join("cleaned.mp3");
    std::fs::write(&cleaned, &out).unwrap();
    let after = probe(&cleaned);
    assert!(!tags(&after).iter().any(|t| t.contains("Kowalski") || t.contains("PLANTED")), "{:?}", tags(&after));
    assert_eq!(packets(&song), packets(&cleaned));
}

#[tokio::test]
async fn what_is_not_media_or_cannot_be_read_is_said_so() {
    let bench = Bench::new();
    // Text, a zip that is not a document, random bytes: nothing to do.
    assert_eq!(bench.clean("notes.txt", b"just some text").await, Ok(None));
    assert_eq!(bench.clean("noise.bin", &media_common::noise(5000, 7)).await, Ok(None));
    // A JPEG cut short in its headers: kept as it came, said to be damaged.
    let photo = phone_jpeg(6);
    assert_eq!(bench.clean("cut.jpg", &photo[..40]).await, Err(KeptReason::Damaged));
    // A PNG with an unknown critical chunk.
    let mut png = Sharp::raw(Bytes::from(solid(4, 4, [1, 2, 3])), 4, 4, 3)
        .png(PngOptions::default())
        .to_buffer_blocking()
        .unwrap();
    let iend = png.len() - 12;
    png.splice(iend..iend, png_chunk(b"ZZZZ", b"?"));
    assert_eq!(bench.clean("odd.png", &png).await, Err(KeptReason::Damaged));
    if ffmpeg_available().await {
        // A video whose container is broken: ffprobe cannot read it.
        let mut video = b"\0\0\0\x18ftypisom\0\0\x02\0isomiso2".to_vec();
        video.extend_from_slice(&media_common::noise(2000, 3));
        assert_eq!(bench.clean("broken.mp4", &video).await, Err(KeptReason::Damaged));
    }
}

#[tokio::test]
async fn pdf_loses_its_info_and_xmp_and_keeps_its_pages() {
    use lopdf::{Document, Object, Stream, dictionary};
    let bench = Bench::new();
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let mut kids = Vec::new();
    for text in ["first page", "second page"] {
        let contents = doc.add_object(Stream::new(dictionary! {}, format!("BT ({text}) Tj ET").into_bytes()));
        kids.push(Object::from(doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => contents,
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
        })));
    }
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => 2 }));
    let xmp = doc.add_object(Stream::new(
        dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
        b"<x:xmpmeta><dc:creator>Jan Kowalski</dc:creator><xmp:CreateDate>2021-05-05</xmp:CreateDate></x:xmpmeta>"
            .to_vec(),
    ));
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id, "Metadata" => xmp });
    let info = doc.add_object(dictionary! {
        "Author" => Object::string_literal("Jan Kowalski"),
        "Title" => Object::string_literal("PLANTED title"),
        "Creator" => Object::string_literal("PLANTED Word"),
        "CreationDate" => Object::string_literal("D:20210505100000Z"),
    });
    doc.trailer.set("Root", catalog);
    doc.trailer.set("Info", info);
    let mut planted = Vec::new();
    doc.save_to(&mut planted).unwrap();
    assert_planted("pdf", &planted);

    let out = bench.cleaned("contract.pdf", &planted).await;
    assert_no_planted("pdf", &out);
    let again = Document::load_mem(&out).unwrap();
    assert_eq!(again.get_pages().len(), 2);
    assert!(!again.trailer.has(b"Info"));
    let path = bench.path("cleaned.pdf");
    std::fs::write(&path, &out).unwrap();
    if let Ok(info) = Command::new("pdfinfo").arg(&path).output()
        && info.status.success()
    {
        let text = String::from_utf8_lossy(&info.stdout);
        assert!(text.contains("Pages:") && text.contains('2'), "{text}");
        assert!(!text.contains("Kowalski") && !text.contains("PLANTED"), "{text}");
    } else {
        eprintln!("no pdfinfo on this machine; the result was checked with lopdf only");
    }
}
