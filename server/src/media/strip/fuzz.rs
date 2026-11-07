//! Fuzz-style tests of the byte-level rewriters (JPEG, PNG, WebP, GIF, HEIF):
//! every truncation of a sample and a few thousand seeded mutations of it. Each
//! input must end in `Ok` or `Err`, never in a panic; the reader must not be
//! asked again and again past its end (a loop that makes no progress); the
//! output may not run away from the input; and whatever succeeds must be
//! final: cleaning it again changes nothing.

use std::io::{BufRead, Cursor, Read, Seek, SeekFrom, Write};
use std::sync::mpsc;
use std::time::Duration;

use bytes::Bytes;

use super::RewriteError;
use crate::media::vips::{GifOptions, JpegOptions, PngOptions, Sharp, WebpOptions};

/// A reader over a buffer that fails the test when it is read at its end over
/// and over: every loop in the rewriters must consume input to go on.
struct Guarded<'a> {
    inner: Cursor<&'a [u8]>,
    reads_at_end: u32,
}

impl<'a> Guarded<'a> {
    fn new(data: &'a [u8]) -> Guarded<'a> {
        Guarded { inner: Cursor::new(data), reads_at_end: 0 }
    }

    fn note(&mut self, n: usize) {
        if n == 0 {
            self.reads_at_end += 1;
            assert!(self.reads_at_end < 64, "read at the end of the input {} times", self.reads_at_end);
        }
    }
}

impl Read for Guarded<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if !buf.is_empty() {
            self.note(n);
        }
        Ok(n)
    }
}

impl BufRead for Guarded<'_> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        let empty = self.inner.position() as usize >= self.inner.get_ref().len();
        if empty {
            self.note(0);
        }
        self.inner.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.inner.consume(amt)
    }
}

impl Seek for Guarded<'_> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// A writer that fails the test when the output outgrows the input.
struct Bounded {
    out: Cursor<Vec<u8>>,
    limit: usize,
}

impl Write for Bounded {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.out.write(buf)?;
        assert!(self.out.get_ref().len() <= self.limit, "the output outgrew the input: {}", self.out.get_ref().len());
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Seek for Bounded {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.out.seek(pos)
    }
}

type Rewriter = fn(&mut Guarded<'_>, &mut Bounded) -> Result<bool, RewriteError>;

fn rewriters() -> [(&'static str, Rewriter); 5] {
    [
        ("jpeg", |r, w| super::jpeg::strip(r, w)),
        ("png", |r, w| super::png::strip(r, w)),
        ("webp", |r, w| super::webp::strip(r, w)),
        ("gif", |r, w| super::gif::strip(r, w)),
        ("heif", |r, w| super::heif::strip(r, w)),
    ]
}

/// Run one rewriter on one input; `Some(output)` when it changed something.
fn once(rewrite: Rewriter, input: &[u8]) -> Result<Option<Vec<u8>>, RewriteError> {
    let mut reader = Guarded::new(input);
    // A WebP chunk may gain its pad byte; a JPEG never grows.
    let mut writer = Bounded { out: Cursor::new(Vec::new()), limit: input.len() + input.len() / 2 + 64 };
    let changed = rewrite(&mut reader, &mut writer)?;
    Ok(changed.then(|| writer.out.into_inner()))
}

/// The whole property on one input.
fn check(name: &str, rewrite: Rewriter, input: &[u8]) {
    let outcome = std::panic::catch_unwind(|| once(rewrite, input));
    let result = match outcome {
        Ok(result) => result,
        Err(_) => panic!("{name}: panicked on {} bytes: {:02x?}", input.len(), &input[..input.len().min(96)]),
    };
    if let Ok(Some(output)) = result {
        let again = std::panic::catch_unwind(|| once(rewrite, &output))
            .unwrap_or_else(|_| panic!("{name}: panicked on its own output"));
        assert!(
            matches!(again, Ok(None)),
            "{name}: cleaning its own output changed it again ({} bytes in): {again:?}",
            input.len()
        );
    }
}

/// xorshift64*: the same "random" inputs on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// One mutation: a flipped byte, a field of 4 bytes set to an edge value, a
/// run inserted or cut out.
fn mutate(rng: &mut Rng, input: &[u8]) -> Vec<u8> {
    let mut v = input.to_vec();
    for _ in 0..1 + rng.below(4) {
        if v.is_empty() {
            v.push(0xFF);
        }
        let at = rng.below(v.len());
        match rng.below(6) {
            0 => v[at] ^= 1 << rng.below(8),
            1 => v[at] = [0x00, 0xFF, 0x7F, 0x80, 0xD8, 0xD9, 0xDA][rng.below(7)],
            2 => {
                let edge = [0u32, 1, 2, 7, 8, 0xFFFF, 0x7FFF_FFFF, 0xFFFF_FFFF, v.len() as u32][rng.below(9)];
                let end = (at + 4).min(v.len());
                let bytes = if rng.below(2) == 0 { edge.to_be_bytes() } else { edge.to_le_bytes() };
                v[at..end].copy_from_slice(&bytes[..end - at]);
            }
            3 => {
                let run: Vec<u8> = (0..1 + rng.below(16)).map(|_| rng.next() as u8).collect();
                v.splice(at..at, run);
            }
            4 => {
                let end = (at + 1 + rng.below(32)).min(v.len());
                v.drain(at..end);
            }
            _ => {
                // A copy of another part of the file: a valid-looking header in the wrong place.
                let from = rng.below(v.len());
                let len = rng.below(24).min(v.len() - from);
                let piece = v[from..from + len].to_vec();
                v.splice(at..at, piece);
            }
        }
    }
    v
}

// ── Samples ─────────────────────────────────────────────────────────────────

fn pixels(width: u32, height: u32) -> Bytes {
    let mut v = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            v.extend_from_slice(&[(x * 255 / width) as u8, (y * 255 / height) as u8, ((x ^ y) * 9) as u8]);
        }
    }
    Bytes::from(v)
}

fn encoded(make: impl FnOnce(Sharp) -> Sharp) -> Vec<u8> {
    make(Sharp::raw(pixels(24, 16), 24, 16, 3)).to_buffer_blocking().expect("libvips encodes the sample")
}

fn jpeg_sample() -> Vec<u8> {
    let plain = encoded(|s| s.jpeg(JpegOptions { quality: 70, ..JpegOptions::default() }));
    let mut exif = b"Exif\0\0".to_vec();
    exif.extend_from_slice(b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0");
    let segment = |marker: u8, body: &[u8]| {
        let mut v = vec![0xFF, marker];
        v.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
        v.extend_from_slice(body);
        v
    };
    let mut out = plain[..2].to_vec();
    out.extend(segment(0xE1, &exif));
    out.extend(segment(0xE1, b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta>author</x:xmpmeta>"));
    out.extend(segment(0xED, b"Photoshop 3.0\08BIM\x04\x04"));
    out.extend(segment(0xFE, b"a comment"));
    out.extend_from_slice(&plain[2..]);
    out.extend_from_slice(b"trailing");
    out
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

fn png_sample() -> Vec<u8> {
    let plain = encoded(|s| s.png(PngOptions::default()));
    // After the signature and IHDR (8 + 25 bytes).
    let mut out = plain[..33].to_vec();
    out.extend(png_chunk(b"tEXt", b"Author\0Jan"));
    out.extend(png_chunk(b"tIME", &[7, 230, 1, 1, 1, 1, 1]));
    out.extend(png_chunk(b"eXIf", b"II*\0\x08\0\0\0\0\0"));
    out.extend_from_slice(&plain[33..]);
    out
}

fn webp_sample() -> Vec<u8> {
    let plain = encoded(|s| s.webp(WebpOptions { quality: 60, ..WebpOptions::default() }));
    // RIFF header, then the VP8 chunk: wrap it with VP8X, EXIF and XMP.
    let vp8 = &plain[12..];
    let chunk = |name: &[u8; 4], data: &[u8]| {
        let mut v = name.to_vec();
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(data);
        if data.len() % 2 == 1 {
            v.push(0);
        }
        v
    };
    let vp8x = chunk(b"VP8X", &[0x0C, 0, 0, 0, 23, 0, 0, 15, 0, 0]);
    let body = [vp8x, vp8.to_vec(), chunk(b"EXIF", b"II*\0\x08\0\0\0\0\0"), chunk(b"XMP ", b"<x:xmpmeta/>")].concat();
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend(body);
    out
}

fn gif_sample() -> Vec<u8> {
    let plain = encoded(|s| s.gif(GifOptions { loop_count: Some(0), ..GifOptions::default() }));
    let flags = plain[10];
    let table = if flags & 0x80 != 0 { 3usize << ((flags & 7) + 1) } else { 0 };
    let at = 13 + table;
    let mut out = plain[..at].to_vec();
    out.extend_from_slice(&[0x21, 0xFE, 9]);
    out.extend_from_slice(b"a comment");
    out.push(0);
    out.extend_from_slice(&[0x21, 0xFF, 11]);
    out.extend_from_slice(b"XMP DataXMP");
    out.extend_from_slice(&[4, b'<', b'x', b'/', b'>', 0]);
    out.extend_from_slice(&plain[at..]);
    out
}

fn samples() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("jpeg", jpeg_sample()),
        ("png", png_sample()),
        ("webp", webp_sample()),
        ("gif", gif_sample()),
        ("heif", super::heif::tests::heif(b"heic", false, 6)),
        ("heif", super::heif::tests::heif(b"avif", true, 1)),
    ]
}

/// Run `work` on its own thread and fail the test when it does not finish in
/// time: a rewriter that loops would hang the whole suite otherwise.
fn within(seconds: u64, work: impl FnOnce() + Send + 'static) {
    let (done, finished) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        work();
        let _ = done.send(());
    });
    match finished.recv_timeout(Duration::from_secs(seconds)) {
        Ok(()) => worker.join().unwrap(),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            if let Err(panic) = worker.join() {
                std::panic::resume_unwind(panic);
            }
        }
        Err(mpsc::RecvTimeoutError::Timeout) => panic!("the rewriters did not finish within {seconds} s"),
    }
}

#[test]
fn the_samples_are_cleaned_and_their_metadata_found() {
    for (name, sample) in samples() {
        let rewrite = rewriters().iter().find(|(n, _)| *n == name).unwrap().1;
        let cleaned = once(rewrite, &sample).unwrap_or_else(|e| panic!("{name}: {e}")).expect("something to remove");
        check(name, rewrite, &sample);
        assert!(cleaned.len() <= sample.len(), "{name}");
    }
}

#[test]
fn every_truncation_ends_without_a_panic() {
    within(120, || {
        for (name, sample) in samples() {
            let rewrite = rewriters().iter().find(|(n, _)| *n == name).unwrap().1;
            for cut in 0..sample.len() {
                check(name, rewrite, &sample[..cut]);
            }
        }
    });
}

#[test]
fn mutated_files_end_without_a_panic() {
    within(240, || {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for (name, sample) in samples() {
            let rewrite = rewriters().iter().find(|(n, _)| *n == name).unwrap().1;
            for _ in 0..3000 {
                let input = mutate(&mut rng, &sample);
                check(name, rewrite, &input);
            }
        }
    });
}

#[test]
fn foreign_and_random_bytes_end_without_a_panic() {
    within(120, || {
        let mut rng = Rng(42);
        let all: Vec<Vec<u8>> = samples().into_iter().map(|(_, s)| s).collect();
        for (name, rewrite) in rewriters() {
            // Every sample given to every rewriter.
            for sample in &all {
                check(name, rewrite, sample);
            }
            for len in [0usize, 1, 2, 3, 7, 8, 12, 13, 31, 64, 1000] {
                for _ in 0..50 {
                    let noise: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
                    check(name, rewrite, &noise);
                    // The right magic, then noise.
                    let mut magic = match name {
                        "jpeg" => vec![0xFF, 0xD8],
                        "png" => super::png::SIGNATURE.to_vec(),
                        "webp" => b"RIFF\x10\0\0\0WEBP".to_vec(),
                        "gif" => b"GIF89a".to_vec(),
                        _ => b"\0\0\0\x10ftypheic\0\0\0\0".to_vec(),
                    };
                    magic.extend_from_slice(&noise);
                    check(name, rewrite, &magic);
                }
            }
        }
    });
}
