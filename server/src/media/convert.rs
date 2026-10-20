//! Port of `lib/media/convert.ts`: the libvips conversion engine - an image
//! to another image format, or wrapped in a one-page PDF - and the budget
//! that decides whether a conversion may run at all.
//!
//! Upstream re-exports the target tables and the probe from here; in Rust
//! they stay where they live (`convert_targets`, `probe`) and are re-exported
//! for callers that import them "from convert".

use bytes::Bytes;

use crate::media::abort::AbortSignal;
use crate::media::pdf_lib::{random_image_name, single_jpeg_page_pdf};
use crate::media::transform_cost::{max_pixels_for, peak_bytes_for};
use crate::media::transform_queue::as_error;
use crate::media::vips::sharp::ChromaSubsampling;
use crate::media::vips::{FailOn, HeifOptions, JpegOptions, PngOptions, Sharp, WebpOptions};

pub use crate::media::convert_targets::{
    ConvertOption, ConvertTarget, IMAGE_TARGETS, Motion, can_convert, convert_option_for, convert_targets_for,
    is_convert_target, keeps_animation,
};
pub use crate::media::probe::{ImageProbe, probe_image};

/// `Converted`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converted {
    pub buffer: Vec<u8>,
    pub mime: &'static str,
    pub ext: &'static str,
}

/// `BudgetVerdict`
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BudgetVerdict {
    pub ok: bool,
    pub animated: bool,
    pub pixels: u64,
    pub limit: u64,
    pub peak_bytes: f64,
}

/// `conversionBudget(probe, to)`: decide whether this conversion may run,
/// and whether it runs frame by frame.
///
/// Two things it has to get right, both of them past bugs:
///
///  - pixels are counted per FRAME when the animation is being kept. The old
///    guard measured one frame, so a 1654x762 GIF with 256 frames read as
///    1.26 Mpx and sailed through a 64 Mpx budget while actually costing 322;
///  - the ceiling comes from the TARGET, not from a single number for all of
///    them. 64 Mpx of PNG is 384 MB and 2.7 s; 64 Mpx of AVIF would be 2.6 GB
///    inside a 2 GiB container. See `transform_cost` for the measurements.
pub fn conversion_budget(animated: bool, frame_pixels: u64, total_pixels: u64, to: ConvertTarget) -> BudgetVerdict {
    let animated = animated && keeps_animation(to);
    let pixels = if animated { total_pixels } else { frame_pixels };
    let limit = max_pixels_for(to, animated);
    let peak_bytes = peak_bytes_for(to, animated, pixels);
    BudgetVerdict { ok: pixels <= limit, animated, pixels, limit, peak_bytes }
}

/// [`conversion_budget`] over a probe.
pub fn conversion_budget_for(probe: &ImageProbe, to: ConvertTarget) -> BudgetVerdict {
    conversion_budget(probe.animated, probe.frame_pixels, probe.total_pixels, to)
}

/// `ConvertOptions`
#[derive(Debug, Clone, Default)]
pub struct ConvertOptions {
    /// Must come from `conversion_budget`, never from a guess.
    pub animated: bool,
    /// Asked of libvips, and of us between stages. libvips honours it only
    /// where it evaluates progressively - an animated WebP encode aborts on
    /// time, a single-frame AVIF encode does not - so this is a best-effort
    /// brake, and the pixel budget is the real bound.
    pub timeout_ms: Option<u64>,
    /// The caller's signal. Checked at every stage boundary.
    pub signal: Option<AbortSignal>,
}

/// `checkAborted(signal)`: throws whatever aborted us, so the route can tell
/// 499 from 503.
fn check_aborted(signal: Option<&AbortSignal>) -> anyhow::Result<()> {
    match signal {
        Some(s) if s.aborted() => Err(as_error(s.reason())),
        _ => Ok(()),
    }
}

/// `withDeadline(pipeline, timeoutMs)`: `.timeout({ seconds: max(1, ceil(ms / 1000)) })`.
fn with_deadline(pipeline: Sharp, timeout_ms: Option<u64>) -> Sharp {
    match timeout_ms.filter(|ms| *ms > 0) {
        Some(ms) => pipeline.timeout(ms.div_ceil(1000).max(1) as u32),
        None => pipeline,
    }
}

/// `convertImageBuffer(input, to, opts)`: convert an image buffer to another
/// image format, or wrap it in a PDF.
///
/// `animated` must come from `conversion_budget`: passing it for a target
/// that cannot hold motion produces a giant sprite sheet (a 135-frame GIF
/// becomes a 640x86400 PNG).
pub async fn convert_image_buffer(input: Bytes, to: ConvertTarget, opts: ConvertOptions) -> anyhow::Result<Converted> {
    check_aborted(opts.signal.as_ref())?;
    let animated = opts.animated && keeps_animation(to);
    if to == ConvertTarget::Pdf {
        return image_to_pdf(input, &opts).await;
    }

    let img = with_deadline(
        if animated {
            // .rotate() is refused for multi-page input, and an animation has
            // no EXIF orientation to bake in anyway.
            Sharp::new(input).fail_on(FailOn::None).animated(true)
        } else {
            // bake EXIF orientation
            Sharp::new(input).fail_on(FailOn::None).rotate()
        },
        opts.timeout_ms,
    );

    Ok(match to {
        ConvertTarget::Png => {
            Converted { buffer: img.png(PngOptions::default()).to_buffer().await?, mime: "image/png", ext: "png" }
        }
        ConvertTarget::Jpg => Converted {
            buffer: img.jpeg(JpegOptions { quality: 90, ..Default::default() }).to_buffer().await?,
            mime: "image/jpeg",
            ext: "jpg",
        },
        ConvertTarget::Webp => Converted {
            buffer: img
                .webp(WebpOptions { quality: if animated { 80 } else { 90 }, effort: 4, ..Default::default() })
                .to_buffer()
                .await?,
            mime: "image/webp",
            ext: "webp",
        },
        // Effort 2, not 4. Measured on 12 Mpx of incompressible noise: effort
        // 4 takes 22.5 s, effort 2 takes 4.6 s, and the output is 2% larger.
        ConvertTarget::Avif => Converted {
            buffer: img.avif(HeifOptions { quality: 60, effort: 2, ..Default::default() }).to_buffer().await?,
            mime: "image/avif",
            ext: "avif",
        },
        ConvertTarget::Pdf => unreachable!("handled above"),
    })
}

/// `imageToPdf(input, opts)`: wrap one picture in a one-page PDF.
///
/// Via JPEG, not PNG, and that is the whole point of this function: pdf-lib
/// embeds a PNG by decoding and re-deflating it in JavaScript on the main
/// thread (9.4 s and 1.4 GB at 64 Mpx), while `embedJpg` parses the SOF and
/// hands the bytes over verbatim under `DCTDecode`. The page is JPEG, so it is
/// lossy and transparency is flattened onto white - a PDF page is a sheet of
/// paper - and 4:4:4 chroma keeps screenshot text from fringing.
async fn image_to_pdf(input: Bytes, opts: &ConvertOptions) -> anyhow::Result<Converted> {
    // Never animated: a PDF page holds one picture, so this is frame one by design.
    let (data, info) =
        with_deadline(Sharp::new(input).fail_on(FailOn::None).rotate().flatten([255.0, 255.0, 255.0]), opts.timeout_ms)
            .jpeg(JpegOptions { quality: 90, chroma_subsampling: ChromaSubsampling::Yuv444, ..Default::default() })
            .to_buffer_with_info()
            .await?;

    check_aborted(opts.signal.as_ref())?;

    let bytes = single_jpeg_page_pdf(&data, info.width, info.height, &random_image_name(), chrono::Utc::now())?;
    Ok(Converted { buffer: bytes, mime: "application/pdf", ext: "pdf" })
}

#[cfg(test)]
mod tests {
    //! The budget parts of tests/unit/convert-targets.test.ts.
    use super::*;

    use crate::media::transform_cost::max_pixels_for;

    // 1654x762, 256 frames: the Round 7 shape.
    const BIG_FRAME: u64 = 1654 * 762;

    #[test]
    fn refuses_an_animated_reencode_whose_every_frame_is_counted() {
        let v = conversion_budget(true, BIG_FRAME, BIG_FRAME * 256, ConvertTarget::Webp);
        assert!(!v.ok);
        assert_eq!(v.pixels, BIG_FRAME * 256);
        assert!(v.pixels > v.limit);
        let still = conversion_budget(true, BIG_FRAME, BIG_FRAME * 256, ConvertTarget::Png);
        assert!(still.ok && !still.animated);
        assert_eq!(still.pixels, BIG_FRAME);
        let bomb = max_pixels_for(ConvertTarget::Png, false) + 1;
        assert!(!conversion_budget(false, bomb, bomb, ConvertTarget::Png).ok);
        let gif = conversion_budget(true, 358 * 640, 358 * 640 * 63, ConvertTarget::Webp);
        assert!(gif.ok && gif.animated);
        assert_eq!(gif.pixels, 358 * 640 * 63);
        assert!(conversion_budget(false, 30_000_000, 30_000_000, ConvertTarget::Png).ok);
        assert!(!conversion_budget(false, 30_000_000, 30_000_000, ConvertTarget::Avif).ok);
    }

    #[test]
    fn counts_frames_only_when_the_animation_is_kept() {
        let frame = 1654 * 762;
        let webp = conversion_budget(true, frame, frame * 16, ConvertTarget::Webp);
        assert!(webp.animated);
        assert_eq!(webp.pixels, frame * 16);
        let png = conversion_budget(true, frame, frame * 256, ConvertTarget::Png);
        assert!(!png.animated);
        assert_eq!(png.pixels, frame);
        assert!(png.ok);
    }
}
