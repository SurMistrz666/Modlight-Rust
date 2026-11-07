//! Not upstream: removing metadata from a file while it is being uploaded
//! (SKELETON.md section 9, item 47). One switch per account, `user.strip_metadata`.
//!
//! Runs inside `upload::finalize` on the plaintext, before the size, the
//! checksum, the dimensions and the encryption of the file are taken, so all of
//! them describe the cleaned file.
//!
//! The picture stays the same: whatever a format allows is rewritten without
//! decoding it (`jpeg`, `png`, `webp`, `gif`: segments, chunks and blocks are
//! copied or left out; `heif`: the metadata items are overwritten where they
//! are; `av`: a stream copy; `office`: the other zip entries are copied
//! verbatim). Only TIFF goes through libvips, losslessly. The formats are told
//! by their bytes, never by the name.
//!
//! **Fails open.** The file is never lost: a file that cannot be cleaned is
//! stored as it came, and the reason lands in `uploads.metadata_kept`, which
//! the Settings card lists for the owner ([`KeptReason`]).
//!
//! | Module | Formats | How |
//! |---|---|---|
//! | [`jpeg`] | JPEG | segments, the pixels are not touched |
//! | [`png`] | PNG, APNG | chunks |
//! | [`webp`] | WebP | RIFF chunks |
//! | [`gif`] | GIF | blocks |
//! | [`heif`] | HEIC, AVIF | the metadata items are overwritten in place |
//! | [`raster`] | TIFF; an AVIF [`heif`] cannot follow | libvips, a metadata-free save |
//! | [`av`] | MP4, MOV, M4V, MKV, WebM, AVI, MP3, M4A, FLAC, Ogg, Opus, WAV | ffmpeg, stream copy |
//! | [`pdf`] | PDF | lopdf: the Info dictionary and the XMP streams go |
//! | [`office`] | docx, xlsx, pptx, odt, ods, odp | zip rewrite, neutral property parts |

pub mod av;
#[cfg(test)]
mod fuzz;
pub mod gif;
pub mod heif;
pub mod jpeg;
pub mod office;
pub mod pdf;
pub mod png;
pub mod raster;
pub mod webp;

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::config::Config;
use crate::media::abort::AbortSignal;
use crate::media::av_cost::{AV_COPY_TIMEOUT_MS, AV_WEIGHT};
use crate::media::av_workspace::{AvWorkspace, open_scratch};
use crate::media::transform_cost::{CONVERT_TIMEOUT_MS, CONVERT_WEIGHT};
use crate::media::transform_queue::{RunOptions, image_transforms};
use crate::storage::{ByteRange, StorageDriver};
use crate::upload::disk::has_room_for;

/// How much of a file is read to tell what it is.
pub const HEAD_BYTES: usize = 4096;

/// Why a file kept its metadata: the value of `uploads.metadata_kept`, and the
/// last part of the panel's `metadata.reasons.*` wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeptReason {
    /// Damaged, or a layout this code does not understand.
    Damaged,
    /// The format cannot be cleaned here without losing something (a camera's
    /// raw file, an AVIF sequence whose layout the in-place cleaner does not
    /// follow).
    Format,
    /// Password-protected: the parts that hold the metadata cannot be read.
    Protected,
    /// Digitally signed: any change would break the signature.
    Signed,
    /// Too big, or too slow, to finish within the deadline of a media job.
    TooLarge,
    /// The media queue was full, or the disk had no room for a second copy.
    Busy,
    /// ffmpeg or libvips is missing or stopped for a reason of its own.
    Tool,
}

impl KeptReason {
    pub fn as_str(self) -> &'static str {
        match self {
            KeptReason::Damaged => "damaged",
            KeptReason::Format => "format",
            KeptReason::Protected => "protected",
            KeptReason::Signed => "signed",
            KeptReason::TooLarge => "too-large",
            KeptReason::Busy => "busy",
            KeptReason::Tool => "tool",
        }
    }
}

/// A byte-level rewriter gave up. Nothing it wrote is used.
#[derive(Debug, thiserror::Error)]
pub enum RewriteError {
    #[error("{0}")]
    Malformed(&'static str),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl RewriteError {
    fn reason(&self) -> KeptReason {
        match self {
            RewriteError::Malformed(_) => KeptReason::Damaged,
            RewriteError::Io(_) => KeptReason::Tool,
        }
    }
}

/// `read_exact`, with a file that ends too soon being a malformed file.
pub(crate) fn read_all<R: Read>(input: &mut R, buf: &mut [u8], what: &'static str) -> Result<(), RewriteError> {
    match input.read_exact(buf) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Err(RewriteError::Malformed(what)),
        Err(e) => Err(e.into()),
    }
}

/// Move exactly `count` bytes along, into `output` when one is given. Constant
/// memory whatever `count` claims: a length field is only ever a promise.
pub(crate) fn pass<R: Read, W: Write>(
    input: &mut R,
    output: Option<&mut W>,
    count: u64,
    what: &'static str,
) -> Result<(), RewriteError> {
    let mut limited = input.take(count);
    let moved = match output {
        Some(out) => std::io::copy(&mut limited, out)?,
        None => std::io::copy(&mut limited, &mut std::io::sink())?,
    };
    if moved != count {
        return Err(RewriteError::Malformed(what));
    }
    Ok(())
}

/// What a file is, as far as this module cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Jpeg,
    Png,
    Webp,
    Gif,
    Heic,
    Avif,
    Tiff,
    /// Audio or video. `weak` is a signature that also occurs by chance (a bare
    /// MPEG frame sync): when ffprobe disagrees, the file is simply not media.
    Av {
        weak: bool,
    },
    Pdf,
    /// Any zip; [`office`] decides whether it is a document.
    Zip,
}

/// The `ftyp` box of an ISO media file: major brand and compatible brands.
fn ftyp_brands(head: &[u8]) -> Option<(&[u8], Vec<&[u8]>)> {
    if head.len() < 16 || &head[4..8] != b"ftyp" {
        return None;
    }
    let size = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
    let end = size.clamp(16, head.len());
    Some((&head[8..12], head[16..end].as_chunks::<4>().0.iter().map(<[u8; 4]>::as_slice).collect()))
}

const HEIC_BRANDS: [&[u8]; 8] = [b"heic", b"heix", b"hevc", b"hevx", b"heim", b"heis", b"hevm", b"hevs"];
/// HEIF's own brands, which name no codec.
const HEIF_BRANDS: [&[u8]; 2] = [b"mif1", b"msf1"];

/// What the first bytes say the file is; `None` for everything this module
/// leaves alone.
pub fn kind_of(head: &[u8]) -> Option<Kind> {
    if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(Kind::Jpeg);
    }
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(Kind::Png);
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        return Some(Kind::Gif);
    }
    if head.len() >= 12 && head.starts_with(b"RIFF") {
        return match &head[8..12] {
            b"WEBP" => Some(Kind::Webp),
            b"WAVE" | b"AVI " => Some(Kind::Av { weak: false }),
            _ => None,
        };
    }
    if head.starts_with(b"II*\0")
        || head.starts_with(b"MM\0*")
        || head.starts_with(b"II+\0")
        || head.starts_with(b"MM\0+")
    {
        return Some(Kind::Tiff);
    }
    if head.starts_with(b"%PDF-") {
        return Some(Kind::Pdf);
    }
    if head.starts_with(b"PK\x03\x04") {
        return Some(Kind::Zip);
    }
    if let Some((major, compatible)) = ftyp_brands(head) {
        if major == b"avif" || major == b"avis" {
            return Some(Kind::Avif);
        }
        if HEIC_BRANDS.contains(&major) {
            return Some(Kind::Heic);
        }
        if HEIF_BRANDS.contains(&major) {
            let avif = compatible.iter().any(|b| *b == b"avif" || *b == b"avis");
            return Some(if avif { Kind::Avif } else { Kind::Heic });
        }
        return Some(Kind::Av { weak: false });
    }
    if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) || head.starts_with(b"fLaC") || head.starts_with(b"OggS") {
        return Some(Kind::Av { weak: false });
    }
    // QuickTime from before `ftyp`.
    if head.get(4..8).is_some_and(|atom| [&b"moov"[..], b"mdat", b"wide", b"free", b"skip"].contains(&atom)) {
        return Some(Kind::Av { weak: true });
    }
    if head.starts_with(b"ID3") {
        return Some(Kind::Av { weak: true });
    }
    // An MPEG audio frame: sync bits, a real layer, a real bitrate index.
    if let [0xFF, b1, b2, ..] = *head
        && b1 & 0xE0 == 0xE0
        && b1 & 0x06 != 0
        && b2 & 0xF0 != 0xF0
    {
        return Some(Kind::Av { weak: true });
    }
    None
}

/// Where the upload's bytes are.
#[derive(Debug, Clone, Copy)]
pub enum Source<'a> {
    /// The temp file of a finished upload (ShareX, local tus).
    Local(&'a Path),
    /// The object an S3 tus upload left in the bucket.
    Stored(&'a str),
}

/// A cleaned copy of an upload, in a scratch directory of the upload temp
/// folder that goes away with this value.
#[derive(Debug)]
pub struct Cleaned {
    pub path: PathBuf,
    pub size: i64,
    /// SHA-256 of the cleaned bytes, lower-case hex.
    pub checksum: String,
    pub head: Bytes,
    _workspace: AvWorkspace,
}

/// How the attempt ended.
#[derive(Debug)]
pub enum Outcome {
    /// Not a format with metadata to remove, or nothing in it to remove: the
    /// upload goes on as if nothing had happened.
    Untouched,
    Cleaned(Cleaned),
    /// It could not be cleaned, and it is stored as it came.
    Kept(KeptReason),
}

/// The name of the result inside a job's scratch directory.
const OUTPUT_NAME: &str = "cleaned";

/// Deadline of a libvips or lopdf job, which holds a conversion's slot.
const REWRITE_TIMEOUT_MS: u64 = CONVERT_TIMEOUT_MS * 2;

/// The largest audio or video file that is remuxed. A stream copy runs at about
/// 140 MB/s with both cores (`av_cost`), half that with a second job beside it, so
/// 2 GiB fits the 45 s copy deadline with room; a larger file is stored as it
/// came at once rather than after the deadline has been spent on it.
pub const AV_STRIP_MAX_BYTES: i64 = 2 * 1024 * 1024 * 1024;

/// Read the first bytes of the source.
async fn read_source_head(storage: &dyn StorageDriver, source: &Source<'_>) -> Bytes {
    match source {
        Source::Stored(key) => crate::storage::read_head(storage, key, HEAD_BYTES as u64).await,
        Source::Local(path) => {
            let path = path.to_path_buf();
            tokio::task::spawn_blocking(move || {
                let Ok(file) = File::open(path) else { return Bytes::new() };
                let mut buf = Vec::with_capacity(HEAD_BYTES);
                match file.take(HEAD_BYTES as u64).read_to_end(&mut buf) {
                    Ok(_) => Bytes::from(buf),
                    Err(_) => Bytes::new(),
                }
            })
            .await
            .unwrap_or_default()
        }
    }
}

/// Copy a stored object into the scratch directory, within `deadline`.
async fn stage(storage: &dyn StorageDriver, key: &str, to: &Path, deadline: Duration) -> Result<(), KeptReason> {
    let copy = async {
        let mut file = tokio::fs::File::create(to).await?;
        let mut stream = storage.read_stream(key, None).await?;
        while let Some(chunk) = stream.next().await {
            file.write_all(&chunk?).await?;
        }
        file.flush().await?;
        Ok::<_, anyhow::Error>(())
    };
    match tokio::time::timeout(deadline, copy).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => {
            tracing::warn!("[strip] could not read {key} back: {err:#}");
            Err(KeptReason::Tool)
        }
        Err(_) => Err(KeptReason::TooLarge),
    }
}

/// Run a job on the shared media limiter (`transform_queue`), where everything
/// that may cost real CPU or memory goes.
async fn on_limiter<F, Fut>(weight: u32, timeout_ms: u64, job: F) -> Result<bool, KeptReason>
where
    F: FnOnce(AbortSignal) -> Fut,
    Fut: Future<Output = anyhow::Result<Result<bool, KeptReason>>> + Send + 'static,
{
    match image_transforms()
        .run(job, RunOptions { weight: Some(weight), timeout_ms: Some(timeout_ms), signal: None })
        .await
    {
        Ok(result) => result,
        Err(err) if err.is::<crate::media::transform_queue::TimeoutError>() => Err(KeptReason::TooLarge),
        Err(err) if err.is::<crate::media::transform_queue::BusyError>() => Err(KeptReason::Busy),
        Err(err) => {
            tracing::warn!("[strip] job failed: {err:#}");
            Err(KeptReason::Tool)
        }
    }
}

/// A rewriter that reads a file once and writes a file once, on the blocking pool.
async fn rewrite_files(
    input: PathBuf,
    output: PathBuf,
    rewrite: fn(&mut BufReader<File>, &mut BufWriter<File>) -> Result<bool, RewriteError>,
) -> Result<bool, KeptReason> {
    let joined = tokio::task::spawn_blocking(move || {
        let mut reader = BufReader::with_capacity(256 * 1024, File::open(input)?);
        let mut writer = BufWriter::with_capacity(256 * 1024, File::create(output)?);
        let changed = rewrite(&mut reader, &mut writer)?;
        writer.flush()?;
        Ok::<bool, RewriteError>(changed)
    })
    .await;
    match joined {
        Ok(Ok(changed)) => Ok(changed),
        Ok(Err(err)) => {
            tracing::debug!("[strip] rewrite refused: {err}");
            Err(err.reason())
        }
        // A panic in a parser must not take the upload with it.
        Err(err) => {
            tracing::error!("[strip] rewrite task failed: {err}");
            Err(KeptReason::Tool)
        }
    }
}

/// Size, SHA-256 and first bytes of a finished file, in one pass.
fn describe_blocking(path: &Path) -> std::io::Result<(i64, String, Bytes)> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut head = Vec::with_capacity(HEAD_BYTES);
    let mut size: u64 = 0;
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        if head.len() < HEAD_BYTES {
            head.extend_from_slice(&buf[..n.min(HEAD_BYTES - head.len())]);
        }
        size += n as u64;
    }
    Ok((size as i64, hex::encode(hash.finalize()), Bytes::from(head)))
}

/// The room a cleaned copy needs is the size of the upload, again; a stored
/// object staged out of the bucket needs that once more.
async fn room_for(config: &Config, size: i64, source: &Source<'_>) -> bool {
    let copies = if matches!(source, Source::Stored(_)) { 2.0 } else { 1.0 };
    has_room_for(config, size as f64 * copies, &config.uploads.tmp_path).await
}

/// Clean one upload. Never fails: see [`Outcome`].
pub async fn strip_upload(
    config: &Config,
    storage: &dyn StorageDriver,
    source: Source<'_>,
    size: i64,
    known_head: Option<&Bytes>,
) -> Outcome {
    let head = match known_head.filter(|h| !h.is_empty()) {
        Some(head) => head.clone(),
        None => read_source_head(storage, &source).await,
    };
    let Some(kind) = kind_of(&head) else {
        return Outcome::Untouched;
    };
    // A zip is only looked at when the end of its directory says it may be a
    // document: an archive of any size is none of this module's business.
    if kind == Kind::Zip && !zip_may_be_document(storage, &source, size).await {
        return Outcome::Untouched;
    }
    if matches!(kind, Kind::Av { .. }) && size > AV_STRIP_MAX_BYTES {
        return Outcome::Kept(KeptReason::TooLarge);
    }
    if !room_for(config, size, &source).await {
        return Outcome::Kept(KeptReason::Busy);
    }
    let mut workspace = match open_scratch(config, "strip-").await {
        Ok(workspace) => workspace,
        Err(err) => {
            tracing::warn!("[strip] no scratch directory: {err:#}");
            return Outcome::Kept(KeptReason::Tool);
        }
    };
    let Ok(output) = workspace.output(OUTPUT_NAME) else {
        return Outcome::Kept(KeptReason::Tool);
    };
    match source {
        Source::Local(path) => workspace.input = path.to_path_buf(),
        Source::Stored(key) => {
            let Ok(staged) = workspace.output("input") else {
                return Outcome::Kept(KeptReason::Tool);
            };
            if let Err(reason) = stage(storage, key, &staged, Duration::from_millis(AV_COPY_TIMEOUT_MS)).await {
                return Outcome::Kept(reason);
            }
            workspace.input = staged;
        }
    }

    let result = clean(kind, workspace.input.clone(), output.clone(), head).await;
    match result {
        Ok(false) => Outcome::Untouched,
        Err(reason) => Outcome::Kept(reason),
        Ok(true) => {
            let path = output.clone();
            let described = tokio::task::spawn_blocking(move || describe_blocking(&path)).await;
            match described {
                Ok(Ok((size, checksum, head))) if size > 0 => {
                    Outcome::Cleaned(Cleaned { path: output, size, checksum, head, _workspace: workspace })
                }
                _ => {
                    tracing::error!("[strip] a cleaned file cannot be read back");
                    Outcome::Kept(KeptReason::Tool)
                }
            }
        }
    }
}

/// The right cleaner for `kind`: `Ok(true)` left a cleaned file at `output`,
/// `Ok(false)` found nothing to do.
async fn clean(kind: Kind, input: PathBuf, output: PathBuf, head: Bytes) -> Result<bool, KeptReason> {
    match kind {
        Kind::Jpeg => rewrite_files(input, output, jpeg::strip).await,
        Kind::Png => rewrite_files(input, output, png::strip).await,
        Kind::Gif => rewrite_files(input, output, gif::strip).await,
        Kind::Webp => rewrite_files(input, output, webp::strip).await,
        Kind::Zip => rewrite_zip(input, output).await,
        Kind::Heic | Kind::Avif => {
            let in_place = rewrite_files(input.clone(), output.clone(), heif::strip).await;
            match in_place {
                // libvips can still save an AVIF whose layout is beyond this
                // module (HEIC needs a codec the server does not have).
                Err(KeptReason::Damaged) if kind == Kind::Avif => {
                    on_limiter(CONVERT_WEIGHT, REWRITE_TIMEOUT_MS, move |_signal| async move {
                        Ok(raster::strip(kind, input, output).await)
                    })
                    .await
                }
                other => other,
            }
        }
        Kind::Tiff => {
            on_limiter(CONVERT_WEIGHT, REWRITE_TIMEOUT_MS, move |_signal| async move {
                Ok(raster::strip(kind, input, output).await)
            })
            .await
        }
        Kind::Pdf => {
            on_limiter(CONVERT_WEIGHT, REWRITE_TIMEOUT_MS, move |_signal| async move {
                let joined = tokio::task::spawn_blocking(move || pdf::strip_file(&input, &output)).await;
                Ok(joined.unwrap_or_else(|err| {
                    tracing::error!("[strip] PDF task failed: {err}");
                    Err(KeptReason::Tool)
                }))
            })
            .await
        }
        Kind::Av { weak } => {
            on_limiter(AV_WEIGHT, AV_COPY_TIMEOUT_MS, move |signal| async move {
                Ok(av::strip(&input, &output, &head, weak, signal).await)
            })
            .await
        }
    }
}

async fn rewrite_zip(input: PathBuf, output: PathBuf) -> Result<bool, KeptReason> {
    let joined = tokio::task::spawn_blocking(move || office::strip_file(&input, &output)).await;
    joined.unwrap_or_else(|err| {
        tracing::error!("[strip] zip task failed: {err}");
        Err(KeptReason::Tool)
    })
}

/// Ask the end of a zip whether it can be a document, without reading (or, from
/// a bucket, copying) the rest of it.
async fn zip_may_be_document(storage: &dyn StorageDriver, source: &Source<'_>, size: i64) -> bool {
    let size = size.max(0) as u64;
    if size == 0 {
        return false;
    }
    let start = size.saturating_sub(office::TAIL_BYTES);
    let tail = match source {
        Source::Local(path) => {
            let path = path.to_path_buf();
            let read = tokio::task::spawn_blocking(move || {
                let mut file = File::open(path)?;
                file.seek(SeekFrom::Start(start))?;
                let mut tail = Vec::new();
                file.take(office::TAIL_BYTES).read_to_end(&mut tail)?;
                Ok::<_, std::io::Error>(tail)
            })
            .await;
            match read {
                Ok(Ok(tail)) => tail,
                _ => return true,
            }
        }
        Source::Stored(key) => {
            let Ok(mut stream) = storage.read_stream(key, Some(ByteRange { start, end: size - 1 })).await else {
                return true;
            };
            let mut tail = Vec::new();
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(chunk) => tail.extend_from_slice(&chunk),
                    Err(_) => return true,
                }
            }
            tail
        }
    };
    office::may_be_document(&tail, start)
}

/// Write a little-endian word at `at`, and come back to the end.
pub(crate) fn patch_u32_le<W: Write + Seek>(output: &mut W, at: u64, value: u32) -> std::io::Result<()> {
    let end = output.stream_position()?;
    output.seek(SeekFrom::Start(at))?;
    output.write_all(&value.to_le_bytes())?;
    output.seek(SeekFrom::Start(end))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_are_told_by_their_bytes() {
        assert_eq!(kind_of(b"\xFF\xD8\xFF\xE0xxxx"), Some(Kind::Jpeg));
        assert_eq!(kind_of(b"\x89PNG\r\n\x1a\nxxxx"), Some(Kind::Png));
        assert_eq!(kind_of(b"GIF89a"), Some(Kind::Gif));
        assert_eq!(kind_of(b"RIFF\0\0\0\0WEBPVP8 "), Some(Kind::Webp));
        assert_eq!(kind_of(b"RIFF\0\0\0\0WAVEfmt "), Some(Kind::Av { weak: false }));
        assert_eq!(kind_of(b"RIFF\0\0\0\0AVI LIST"), Some(Kind::Av { weak: false }));
        assert_eq!(kind_of(b"II*\0\x08\0\0\0"), Some(Kind::Tiff));
        assert_eq!(kind_of(b"MM\0*\0\0\0\x08"), Some(Kind::Tiff));
        assert_eq!(kind_of(b"%PDF-1.7\n"), Some(Kind::Pdf));
        assert_eq!(kind_of(b"PK\x03\x04rest"), Some(Kind::Zip));
        assert_eq!(kind_of(b"\x1aE\xdf\xa3"), Some(Kind::Av { weak: false }));
        assert_eq!(kind_of(b"fLaC"), Some(Kind::Av { weak: false }));
        assert_eq!(kind_of(b"OggS"), Some(Kind::Av { weak: false }));
        assert_eq!(kind_of(b"ID3\x04"), Some(Kind::Av { weak: true }));
        assert_eq!(kind_of(&[0xFF, 0xFB, 0x90, 0x00]), Some(Kind::Av { weak: true }));
        // Not a frame: a reserved layer, or the forbidden bitrate index.
        assert_eq!(kind_of(&[0xFF, 0xF9 & !0x06, 0x90]), None);
        assert_eq!(kind_of(&[0xFF, 0xFB, 0xF0]), None);
        for other in
            [&b"hello"[..], b"", b"{\"a\":1}", b"<svg xmlns=", b"BM\0\0", b"\xFF", b"RIFF", b"RIFF\0\0\0\0XXXX"]
        {
            assert_eq!(kind_of(other), None, "{other:?}");
        }
    }

    fn ftyp(major: &[u8; 4], compatible: &[&[u8; 4]]) -> Vec<u8> {
        let size = 16 + compatible.len() * 4;
        let mut v = (size as u32).to_be_bytes().to_vec();
        v.extend_from_slice(b"ftyp");
        v.extend_from_slice(major);
        v.extend_from_slice(&[0, 0, 0, 0]);
        for brand in compatible {
            v.extend_from_slice(*brand);
        }
        v.extend_from_slice(b"\0\0\0\x08free");
        v
    }

    #[test]
    fn the_brand_of_an_iso_file_decides() {
        assert_eq!(kind_of(&ftyp(b"heic", &[b"mif1", b"heic"])), Some(Kind::Heic));
        assert_eq!(kind_of(&ftyp(b"mif1", &[b"mif1", b"heic"])), Some(Kind::Heic));
        assert_eq!(kind_of(&ftyp(b"avif", &[b"mif1"])), Some(Kind::Avif));
        assert_eq!(kind_of(&ftyp(b"avis", &[b"avif"])), Some(Kind::Avif));
        assert_eq!(kind_of(&ftyp(b"mif1", &[b"avif", b"miaf"])), Some(Kind::Avif));
        assert_eq!(kind_of(&ftyp(b"isom", &[b"mp41"])), Some(Kind::Av { weak: false }));
        assert_eq!(kind_of(&ftyp(b"qt  ", &[])), Some(Kind::Av { weak: false }));
        assert_eq!(kind_of(&ftyp(b"M4A ", &[])), Some(Kind::Av { weak: false }));
    }

    #[test]
    fn the_reasons_are_the_words_the_panel_knows() {
        let all = [
            (KeptReason::Damaged, "damaged"),
            (KeptReason::Format, "format"),
            (KeptReason::Protected, "protected"),
            (KeptReason::Signed, "signed"),
            (KeptReason::TooLarge, "too-large"),
            (KeptReason::Busy, "busy"),
            (KeptReason::Tool, "tool"),
        ];
        for (reason, word) in all {
            assert_eq!(reason.as_str(), word);
        }
    }

    #[test]
    fn describing_reads_the_whole_file_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        std::fs::write(&path, vec![7u8; 9000]).unwrap();
        let (size, checksum, head) = describe_blocking(&path).unwrap();
        assert_eq!(size, 9000);
        assert_eq!(head.len(), HEAD_BYTES);
        assert_eq!(checksum, hex::encode(Sha256::digest(vec![7u8; 9000])));
    }
}
