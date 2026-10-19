//! Port of `lib/media/image.ts`: image dimensions, frame count and blurhash
//! placeholder for a stored picture, and the thumbnail encoder.

use bytes::Bytes;

use crate::media::blurhash;
use crate::media::probe::probe_image_blocking;
use crate::media::vips::{self, FailOn, Fit, HeifOptions, Position, Resize, Sharp, VipsError, WebpOptions};

/// What `imageMetaFromBuffer` learns. Every field is optional upstream
/// (`undefined` becomes NULL in the column).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageMeta {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub blurhash: Option<String>,
    /// Frames in the file (1 for a still).
    ///
    /// Stored on the row so the "download as…" menu can describe THIS file instead
    /// of guessing from its container: a still WebP is not "possibly animated", and
    /// telling its owner that PNG will give them "a single frame" is a warning
    /// about a problem they do not have.
    pub frames: Option<u32>,
}

/// Best-effort image dimensions, frame count and blurhash placeholder. Never fails.
///
/// Goes through `probeImage`, which is also what the conversion path uses, so the
/// `avis` brand workaround applies here too. Without it the one AVIF sequence in
/// production converted perfectly and yet had no dimensions, no blurhash and no
/// thumbnail, because this function opened the buffer with a plain `sharp()` that
/// libvips refuses on the major brand alone.
pub async fn image_meta_from_buffer(buf: Bytes) -> ImageMeta {
    vips::run_blocking(move || Ok::<_, VipsError>(image_meta_from_buffer_blocking(&buf))).await.unwrap_or_default()
}

/// [`image_meta_from_buffer`] for code already on the blocking pool.
pub fn image_meta_from_buffer_blocking(buf: &Bytes) -> ImageMeta {
    let Some(probe) = probe_image_blocking(buf) else {
        return ImageMeta::default();
    };

    let blurhash = Sharp::new(probe.input.clone())
        .fail_on(FailOn::None)
        .rotate() // bake EXIF orientation before sampling
        .raw_output()
        .ensure_alpha(1.0)
        .resize(Resize::to(Some(32), Some(32)).fit(Fit::Inside))
        .to_buffer_with_info_blocking()
        .ok()
        // ignore blurhash failures
        .and_then(|(data, info)| blurhash::encode(&data, info.width, info.height, 4, 4).ok());

    ImageMeta {
        width: Some(probe.width),
        height: Some(probe.height),
        // `coverOnly` is set in exactly one situation: an AVIF SEQUENCE whose frames
        // libvips refuses to enumerate, reachable only through the brand-swap
        // workaround that yields its cover. Recording 1 there was true of libvips
        // and false of the file, and it stopped mattering the moment ffmpeg arrived:
        // ffmpeg reads all 43 frames of exactly this file, so a hard 1 would hide
        // the only conversion capable of reading it.
        //
        // None here becomes NULL in the column, which is the established word
        // for "unknown" (every legacy row carries it), and unknown is the honest
        // answer - it leaves the frame-reading targets on the menu instead of
        // ruling them out on the strength of a number we did not measure.
        frames: if probe.cover_only { None } else { Some(probe.pages) },
        blurhash,
    }
}

/// `fit` of [`make_thumbnail`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThumbnailFit {
    #[default]
    Cover,
    Inside,
}

/// `format` of [`make_thumbnail`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThumbnailFormat {
    #[default]
    Webp,
    Avif,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ThumbnailOptions {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fit: ThumbnailFit,
    pub format: ThumbnailFormat,
    /// Asked of libvips. Honoured here: a resize evaluates progressively.
    pub timeout_ms: Option<u64>,
}

/// Generate a thumbnail (defaults to a cover crop) as WebP. EXIF is stripped by
/// sharp's default; `.rotate()` bakes orientation first so portraits aren't sideways.
pub async fn make_thumbnail(input: Bytes, opts: ThumbnailOptions) -> Result<Vec<u8>, VipsError> {
    vips::run_blocking(move || make_thumbnail_blocking(input, opts)).await
}

/// [`make_thumbnail`] for code already on the blocking pool.
pub fn make_thumbnail_blocking(input: Bytes, opts: ThumbnailOptions) -> Result<Vec<u8>, VipsError> {
    let mut pipe = Sharp::new(input).fail_on(FailOn::None);
    if let Some(ms) = opts.timeout_ms.filter(|&ms| ms > 0) {
        pipe = pipe.timeout(ms.div_ceil(1000).max(1) as u32);
    }
    let fit = match opts.fit {
        ThumbnailFit::Cover => Fit::Cover,
        ThumbnailFit::Inside => Fit::Inside,
    };
    pipe = pipe
        .rotate()
        .resize(Resize::to(opts.width, opts.height).fit(fit).position(Position::Attention).without_enlargement(true));
    pipe = match opts.format {
        ThumbnailFormat::Avif => pipe.avif(HeifOptions { quality: 55, effort: 3, ..HeifOptions::default() }),
        ThumbnailFormat::Webp => pipe.webp(WebpOptions { quality: 80, ..WebpOptions::default() }),
    };
    pipe.to_buffer_blocking()
}
