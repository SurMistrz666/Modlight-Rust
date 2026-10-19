/**
 * How big the result of a conversion will be, before it exists.
 *
 * The menu used to say "usually about half the size, same quality" under WebP
 * and AVIF, which is untrue in both directions: measured over 308 conversions of
 * 74 production files, a PNG becomes a WebP about ONE EIGHTH its size (median
 * ratio 0.125) while a JPEG becomes a WebP about three quarters of one (0.74).
 * A sentence that wrong is worse than no sentence, so it is replaced by the
 * number the reader actually wanted.
 *
 * THE HONESTY RULE. Confidence is a property of the PAIR (source format, target),
 * never of the file, and this module refuses to invent precision it does not
 * have:
 *
 *   exact   - a container copy. No encoder runs, the streams are moved as they
 *             are, so the result is the source's size. Measured: 12 of 15 within
 *             2 percent, worst case -8.5 percent (dropped data streams and
 *             metadata). Printed WITHOUT a tilde.
 *   approx  - a measured multiplier exists. JPEG sources predict well from their
 *             own size (p90 error 1.43x); PNG and GIF sources predict only from
 *             PIXEL COUNT, and even then with a p90 error of 2 to 3x, so those
 *             are rounded to a single significant figure - the rounding IS the
 *             error bar.
 *   null    - anything whose spread is wider than the number itself: an
 *             animated GIF to WebP measured between 0.10x and 3.89x its source,
 *             and a six second clip out of a video ran from 226 kB to 15 MB. The
 *             UI says what the operation DOES there instead ("6 s, no sound").
 *
 * Every audio and video encode is null for a second, structural reason: the
 * uploads table has no duration column, and duration is the only meter those
 * scale with. Adding one (filled lazily, the way `checksum` already is) is what
 * would move ->mp3 (16 kB/s), ->ogg (14 kB/s) and ->wav (sample rate x channels
 * x 2) out of this bucket.
 *
 * Browser-safe: arithmetic over columns the row already carries.
 */

import { downloadOptionsFor, type DownloadOption } from "@/lib/media/download-menu";
import { currentFormatOf } from "@/lib/media/convert-plan";
import { humanFileSize } from "@/lib/format";

export type SizeConfidence = "exact" | "approx";

/** A predicted result size, or null when predicting one would be a guess. */
export type SizeEstimate = { confidence: SizeConfidence; bytes: number } | null;

/** The columns an estimate reads. */
export interface EstimateSource {
  mimeType: string | null | undefined;
  ext: string | null | undefined;
  size: number;
  width?: number | null;
  height?: number | null;
  frames?: number | null;
}

const KB = 1024;
const MPX = 1_000_000;

/**
 * JPEG source, multiplied by the source's SIZE. n = 20 production files.
 * Median error 1.07 to 1.15x, p90 no worse than 1.43x; a log-log fit puts the
 * exponent on source size at 1.00 to 1.06 and on pixel count at about zero,
 * which is why this family alone reads the byte count.
 */
const FROM_JPEG: Record<string, number> = {
  webp: 0.74,
  avif: 0.36,
  png: 9.4,
  pdf: 1.17,
};

/**
 * PNG source, in kB per megapixel. n = 24, and the same fit REJECTS the source
 * size outright (it chooses an exponent of 0.00 to 0.09 for it): one 734 kB PNG
 * produced a 25 kB WebP while a 10 kB one produced a 10 kB WebP. Pixels are the
 * only thing that carries. This is the common case on this server, where 7393 of
 * 8110 pictures are PNG screenshots.
 */
const FROM_PNG: Record<string, number> = {
  webp: 123,
  avif: 66,
  jpg: 178,
  pdf: 222,
};

/** GIF source to a STILL target, in kB per megapixel. n = 8, a weaker sample. */
const FROM_GIF: Record<string, number> = {
  png: 496,
  jpg: 174,
  avif: 79,
  pdf: 242,
};

/**
 * Round to `digits` significant figures.
 *
 * For the pixel-based families this is the whole error bar: printing "~123.4 kB"
 * for a prediction that is routinely out by a factor of two claims a precision
 * nobody has. "~100 kB" says the same thing and says it honestly.
 */
export function roundSignificant(value: number, digits: number): number {
  if (!Number.isFinite(value) || value <= 0) return 0;
  const magnitude = Math.floor(Math.log10(value)) - (digits - 1);
  const step = Math.pow(10, magnitude);
  return Math.round(value / step) * step;
}

function megapixels(src: EstimateSource): number | null {
  const w = src.width ?? 0;
  const h = src.height ?? 0;
  if (w <= 0 || h <= 0) return null;
  return (w * h) / MPX;
}

/**
 * The predicted size of `option` applied to this file.
 *
 * The option comes from the very list the menu renders, so an entry can never be
 * labelled with a number belonging to a different engine.
 */
export function estimateConvertedSize(
  src: EstimateSource,
  option: DownloadOption,
): SizeEstimate {
  if (option.engine === "av") {
    // A stream copy moves the same samples into another box. Everything else on
    // this engine runs an encoder whose output scales with duration, which the
    // row does not know.
    return option.cost === "copy" && src.size > 0
      ? { confidence: "exact", bytes: src.size }
      : null;
  }

  const from = currentFormatOf(src.mimeType, src.ext);

  if (from === "jpg") {
    const ratio = FROM_JPEG[option.to];
    if (!ratio || src.size <= 0) return null;
    return { confidence: "approx", bytes: roundSignificant(src.size * ratio, 2) };
  }

  const perMpx = from === "png" ? FROM_PNG[option.to] : from === "gif" ? FROM_GIF[option.to] : null;
  if (!perMpx) return null;

  // An animated GIF kept animated (WebP) is the pair with the widest spread of
  // all, so it is never given a number even though its source IS a GIF.
  if (option.motion === "keeps") return null;

  const mpx = megapixels(src);
  if (mpx === null) return null;

  return { confidence: "approx", bytes: roundSignificant(perMpx * KB * mpx, 1) };
}

/** Same, when the caller holds a target name rather than a menu entry. */
export function estimateTargetSize(src: EstimateSource, to: string): SizeEstimate {
  const option = downloadOptionsFor(src.mimeType, src.ext, src.frames, {
    canEncode: true,
    width: src.width,
    height: src.height,
  }).find((o) => o.to === to);
  return option ? estimateConvertedSize(src, option) : null;
}

/**
 * A size for a menu row.
 *
 * Rounded in the unit it is DISPLAYED in, which is the only place the rounding
 * survives. Rounding to two significant figures in base 10 first and then
 * dividing by 1024 to print undoes itself: 400000 is a tidy two-figure number
 * and "390.6 KB" is not, and the decimal claims the estimate is good to a
 * hundred bytes when it can be out by half. These are guesses printed next to a
 * tilde, so they should look like guesses.
 */
export function formatEstimate(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return humanFileSize(0);
  const units: Array<[number, string, number]> = [
    [1024 ** 3, "GB", 1],
    [1024 ** 2, "MB", 1],
    [1024, "KB", 0],
    [1, "B", 0],
  ];
  const [scale, unit, decimals] = units.find(([s]) => bytes >= s) ?? units[units.length - 1];
  const value = bytes / scale;
  // Two significant figures in the displayed unit: 390 KB, 38 KB, 1.2 MB.
  const shown = value >= 100 ? Math.round(value) : Number(value.toFixed(decimals));
  return `${shown} ${unit}`;
}

export interface EstimateTotal {
  /** Bytes the files with a prediction are expected to produce. */
  bytes: number;
  /** How many of them had one. */
  counted: number;
  /** ...and how many could not be predicted, so the total is a floor. */
  unknown: number;
}

/**
 * Add up a selection, keeping the unpredictable ones VISIBLE rather than folding
 * a made-up number into the sum. A total that quietly includes three guesses is
 * a total nobody can act on.
 */
export function estimateTotalSize(sources: EstimateSource[], to: string): EstimateTotal {
  let bytes = 0;
  let counted = 0;
  let unknown = 0;

  for (const src of sources) {
    const estimate = estimateTargetSize(src, to);
    if (estimate) {
      bytes += estimate.bytes;
      counted++;
    } else {
      unknown++;
    }
  }

  return { bytes, counted, unknown };
}
