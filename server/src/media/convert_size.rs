//! Port of `lib/media/convert-size.ts`: how big the result of a conversion
//! will be, before it exists.
//!
//! The menu used to say "usually about half the size, same quality" under WebP
//! and AVIF, which is untrue in both directions: measured over 308 conversions
//! of 74 production files, a PNG becomes a WebP about ONE EIGHTH its size
//! (median ratio 0.125) while a JPEG becomes a WebP about three quarters of one
//! (0.74). A sentence that wrong is worse than no sentence, so it is replaced
//! by the number the reader actually wanted.
//!
//! THE HONESTY RULE. Confidence is a property of the PAIR (source format,
//! target), never of the file, and this module refuses to invent precision it
//! does not have:
//!
//! ```text
//!   exact   - a container copy. No encoder runs, the streams are moved as they
//!             are, so the result is the source's size. Printed WITHOUT a tilde.
//!   approx  - a measured multiplier exists. JPEG sources predict well from their
//!             own size (p90 error 1.43x); PNG and GIF sources predict only from
//!             PIXEL COUNT, and even then with a p90 error of 2 to 3x, so those
//!             are rounded to a single significant figure - the rounding IS the
//!             error bar.
//!   null    - anything whose spread is wider than the number itself: an
//!             animated GIF to WebP measured between 0.10x and 3.89x its source,
//!             and a six second clip out of a video ran from 226 kB to 15 MB.
//! ```
//!
//! Every audio and video encode is null for a second, structural reason:
//! duration is the only meter those scale with.

use crate::i18n::format::js_number_to_string;
use crate::media::av_targets::{AvCost, AvSourceSize};
use crate::media::convert_plan::current_format_of;
use crate::media::convert_targets::Motion;
use crate::media::download_menu::{DownloadMenuOptions, DownloadOption, Engine, download_options_for};

/// `SizeConfidence`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeConfidence {
    Exact,
    Approx,
}

impl SizeConfidence {
    pub fn as_str(self) -> &'static str {
        match self {
            SizeConfidence::Exact => "exact",
            SizeConfidence::Approx => "approx",
        }
    }
}

/// A predicted result size; `None` when predicting one would be a guess.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SizeEstimate {
    pub confidence: SizeConfidence,
    pub bytes: f64,
}

/// `EstimateSource`: the columns an estimate reads.
#[derive(Debug, Clone, Default)]
pub struct EstimateSource {
    pub mime_type: Option<String>,
    pub ext: Option<String>,
    pub size: f64,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub frames: Option<f64>,
}

const KB: f64 = 1024.0;
const MPX: f64 = 1_000_000.0;

/// JPEG source, multiplied by the source's SIZE. n = 20 production files.
fn from_jpeg(to: &str) -> Option<f64> {
    Some(match to {
        "webp" => 0.74,
        "avif" => 0.36,
        "png" => 9.4,
        "pdf" => 1.17,
        _ => return None,
    })
}

/// PNG source, in kB per megapixel. n = 24, and the same fit REJECTS the
/// source size outright: pixels are the only thing that carries.
fn from_png(to: &str) -> Option<f64> {
    Some(match to {
        "webp" => 123.0,
        "avif" => 66.0,
        "jpg" => 178.0,
        "pdf" => 222.0,
        _ => return None,
    })
}

/// GIF source to a STILL target, in kB per megapixel. n = 8, a weaker sample.
fn from_gif(to: &str) -> Option<f64> {
    Some(match to {
        "png" => 496.0,
        "jpg" => 174.0,
        "avif" => 79.0,
        "pdf" => 242.0,
        _ => return None,
    })
}

/// `Math.round(x)`: halves go towards +Infinity.
fn js_round(x: f64) -> f64 {
    let r = x.round();
    if x < 0.0 && r - x == -0.5 { r + 1.0 } else { r }
}

/// `roundSignificant(value, digits)`: round to `digits` significant figures.
/// For the pixel-based families this is the whole error bar.
pub fn round_significant(value: f64, digits: i32) -> f64 {
    if !value.is_finite() || value <= 0.0 {
        return 0.0;
    }
    let magnitude = value.log10().floor() - f64::from(digits - 1);
    let step = 10f64.powf(magnitude);
    js_round(value / step) * step
}

fn megapixels(src: &EstimateSource) -> Option<f64> {
    let w = src.width.unwrap_or(0.0);
    let h = src.height.unwrap_or(0.0);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    Some((w * h) / MPX)
}

/// `estimateConvertedSize(src, option)`: the predicted size of `option`
/// applied to this file. The option comes from the very list the menu
/// renders, so an entry can never be labelled with a number belonging to a
/// different engine.
pub fn estimate_converted_size(src: &EstimateSource, option: &DownloadOption) -> Option<SizeEstimate> {
    if option.engine == Engine::Av {
        // A stream copy moves the same samples into another box. Everything
        // else on this engine runs an encoder whose output scales with
        // duration, which the row does not know.
        return (option.cost == Some(AvCost::Copy) && src.size > 0.0)
            .then_some(SizeEstimate { confidence: SizeConfidence::Exact, bytes: src.size });
    }

    let from = current_format_of(src.mime_type.as_deref(), src.ext.as_deref());
    if from == "jpg" {
        let ratio = from_jpeg(option.to)?;
        if src.size <= 0.0 {
            return None;
        }
        return Some(SizeEstimate {
            confidence: SizeConfidence::Approx,
            bytes: round_significant(src.size * ratio, 2),
        });
    }

    let per_mpx = match from.as_str() {
        "png" => from_png(option.to),
        "gif" => from_gif(option.to),
        _ => None,
    }?;

    // An animated GIF kept animated (WebP) is the pair with the widest spread
    // of all, so it is never given a number even though its source IS a GIF.
    if option.motion == Some(Motion::Keeps) {
        return None;
    }
    let mpx = megapixels(src)?;
    Some(SizeEstimate { confidence: SizeConfidence::Approx, bytes: round_significant(per_mpx * KB * mpx, 1) })
}

/// `estimateTargetSize(src, to)`: same, when the caller holds a target name
/// rather than a menu entry.
pub fn estimate_target_size(src: &EstimateSource, to: &str) -> Option<SizeEstimate> {
    let options = download_options_for(
        src.mime_type.as_deref(),
        src.ext.as_deref(),
        src.frames,
        DownloadMenuOptions { can_encode: true, size: AvSourceSize { width: src.width, height: src.height } },
    );
    let option = options.iter().find(|o| o.to == to)?;
    estimate_converted_size(src, option)
}

/// `formatEstimate(bytes)`: a size for a menu row, rounded in the unit it is
/// DISPLAYED in, which is the only place the rounding survives.
pub fn format_estimate(bytes: f64) -> String {
    if !bytes.is_finite() || bytes <= 0.0 {
        return crate::format::human_file_size(0.0, 1);
    }
    const UNITS: [(f64, &str, usize); 4] =
        [(1_073_741_824.0, "GB", 1), (1_048_576.0, "MB", 1), (1024.0, "KB", 0), (1.0, "B", 0)];
    let (scale, unit, decimals) = UNITS.iter().copied().find(|(s, _, _)| bytes >= *s).unwrap_or(UNITS[3]);
    let value = bytes / scale;
    // Two significant figures in the displayed unit: 390 KB, 38 KB, 1.2 MB.
    let shown = if value >= 100.0 {
        js_round(value)
    } else {
        crate::format::js_to_fixed(value, decimals).parse::<f64>().unwrap_or(value)
    };
    format!("{} {unit}", js_number_to_string(shown))
}

/// `EstimateTotal`
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EstimateTotal {
    /// Bytes the files with a prediction are expected to produce.
    pub bytes: f64,
    /// How many of them had one.
    pub counted: u32,
    /// ...and how many could not be predicted, so the total is a floor.
    pub unknown: u32,
}

/// `estimateTotalSize(sources, to)`: add up a selection, keeping the
/// unpredictable ones VISIBLE rather than folding a made-up number into the sum.
pub fn estimate_total_size(sources: &[EstimateSource], to: &str) -> EstimateTotal {
    let mut total = EstimateTotal { bytes: 0.0, counted: 0, unknown: 0 };
    for src in sources {
        match estimate_target_size(src, to) {
            Some(e) => {
                total.bytes += e.bytes;
                total.counted += 1;
            }
            None => total.unknown += 1,
        }
    }
    total
}

#[cfg(test)]
mod tests {
    //! tests/unit/convert-size.test.ts
    use super::*;

    const MB: f64 = 1024.0 * 1024.0;

    fn src(mime: &str, ext: &str, size: f64) -> EstimateSource {
        EstimateSource { mime_type: Some(mime.into()), ext: Some(ext.into()), size, ..Default::default() }
    }

    fn option(mime: &str, ext: &str, frames: Option<f64>, to: &str) -> DownloadOption {
        download_options_for(
            Some(mime),
            Some(ext),
            frames,
            DownloadMenuOptions { can_encode: true, ..Default::default() },
        )
        .into_iter()
        .find(|o| o.to == to)
        .unwrap_or_else(|| panic!("{mime} does not offer {to}"))
    }

    #[test]
    fn what_a_conversion_will_weigh() {
        let estimate = estimate_converted_size(
            &src("video/quicktime", "mov", 19.3 * MB),
            &option("video/quicktime", "mov", None, "mp4"),
        );
        assert_eq!(estimate, Some(SizeEstimate { confidence: SizeConfidence::Exact, bytes: 19.3 * MB }));
        assert_eq!(
            estimate_converted_size(&src("video/mp4", "mp4", 40.0 * MB), &option("video/mp4", "mp4", None, "gif")),
            None
        );
        assert_eq!(
            estimate_converted_size(&src("video/mp4", "mp4", 40.0 * MB), &option("video/mp4", "mp4", None, "mp3")),
            None
        );

        let jpeg =
            EstimateSource { width: Some(1920.0), height: Some(1080.0), ..src("image/jpeg", "jpg", 1000.0 * 1024.0) };
        let webp = estimate_converted_size(&jpeg, &option("image/jpeg", "jpg", Some(1.0), "webp")).unwrap();
        assert_eq!(webp.confidence, SizeConfidence::Approx);
        assert!(webp.bytes > 0.6 * jpeg.size && webp.bytes < 0.85 * jpeg.size);
        let png = estimate_converted_size(&jpeg, &option("image/jpeg", "jpg", Some(1.0), "png")).unwrap();
        assert!(png.bytes > jpeg.size * 5.0);
    }

    #[test]
    fn pixels_not_bytes_for_png() {
        let small =
            EstimateSource { width: Some(1920.0), height: Some(1080.0), ..src("image/png", "png", 10.0 * 1024.0) };
        let large = EstimateSource { width: Some(1920.0), height: Some(1080.0), ..src("image/png", "png", 5.0 * MB) };
        let a = estimate_target_size(&small, "webp");
        assert_eq!(a, estimate_target_size(&large, "webp"));
        assert_eq!(a.unwrap().confidence, SizeConfidence::Approx);
        assert_eq!(estimate_target_size(&src("image/png", "png", 2.0 * MB), "webp"), None);

        let gif = EstimateSource {
            width: Some(480.0),
            height: Some(270.0),
            frames: Some(40.0),
            ..src("image/gif", "gif", 2.0 * MB)
        };
        assert_eq!(estimate_target_size(&gif, "webp"), None);
        assert!(estimate_target_size(&gif, "png").is_some());
    }

    #[test]
    fn totals_keep_the_unknown_visible() {
        let sized = EstimateSource { width: Some(1000.0), height: Some(1000.0), ..src("image/png", "png", MB) };
        let total = estimate_total_size(&[sized.clone(), sized, src("image/png", "png", MB)], "webp");
        assert_eq!(total.counted, 2);
        assert_eq!(total.unknown, 1);
        assert!(total.bytes > 0.0);
    }

    #[test]
    fn rounding_and_printing() {
        assert_eq!(round_significant(123_456.0, 1), 100_000.0);
        assert_eq!(round_significant(123_456.0, 2), 120_000.0);
        assert_eq!(round_significant(0.0, 1), 0.0);
        assert_eq!(round_significant(-5.0, 1), 0.0);
        assert_eq!(format_estimate(100.0 * 1024.0), "100 KB");
        assert_eq!(format_estimate(2.7 * MB), "2.7 MB");
        assert_eq!(format_estimate(0.0), "0 B");
        assert_eq!(format_estimate(400_000.0), "391 KB");
        assert_eq!(format_estimate(39_000.0), "38 KB");
    }
}
