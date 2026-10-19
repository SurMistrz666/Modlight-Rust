/**
 * "Will this video actually play well for the people I send it to?"
 *
 * Answered from the upload row alone - width, height, size, and a frame total
 * from `frames` or from duration and rate - so it costs nothing to ask and can
 * be asked at the moment the owner is looking at the file, which is the only
 * moment at which the answer is worth anything.
 *
 * WHY THIS EXISTS
 *
 * A user reported that his ShareX recordings "stutter". The file he pointed at
 * is structurally perfect: faststart, constant 30 fps, 33.3 ms between every
 * one of its 403 frames. Nothing in the container is wrong. Two things about
 * its SHAPE are, and the panel had every number needed to say so at upload time
 * and said nothing - so the owner found out from a complaint.
 *
 *   1. It is 5056 pixels across. Hardware H.264 decoders stop at 4096 wide
 *      (Intel QuickSync, NVDEC and AMD alike). Past that the browser decodes in
 *      software, and software decoding manages around 292 Mpx/s - while this
 *      file is 9.73 Mpx at 30 fps, which is 292 Mpx/s. There is no headroom at
 *      all, on one core, while the rest of the page is also being drawn.
 *   2. It holds 0.0044 bits per pixel. At that budget the encoder cannot
 *      refresh the picture; it can only afford to mark most of it unchanged,
 *      which is exactly what "frozen but the clock is running" looks like.
 *
 * Both faults are properties of the file's shape, not of its content, which is
 * what makes them answerable from five integers and what keeps this module pure.
 *
 * WHAT THIS DELIBERATELY IS NOT
 *
 * It is not a quality score. Nothing here opens the file, looks at a frame or
 * asks an encoder anything, so it cannot tell a beautiful low-bitrate clip from
 * an ugly one. It answers one narrow question - "is this file outside the shape
 * a browser plays well?" - and every threshold below is set so that answering
 * "yes" is rare. See the measurements on each constant.
 */

import { mediaKind } from "@/lib/media/mime";
import { needsWebRescue, WEB_MAX_EDGE, type AvSourceSize } from "@/lib/media/av-targets";

/**
 * Widest H.264 picture a hardware decoder will take.
 *
 * Not a guess and not a Modlight number: it is the limit the shipping
 * fixed-function H.264 decoders share - Intel QuickSync, NVIDIA NVDEC and AMD
 * VCN all refuse a picture wider than this and hand the stream back to the CPU.
 * Chrome, Firefox and Safari then decode it in software, which is where the
 * stutter comes from.
 *
 * The row does not store a codec, so the rule cannot be narrowed to H.264 and
 * the sentence the reader sees says "most hardware video decoders" rather than
 * naming one. That hedge is doing real work: an HEVC decoder usually goes to
 * 8192, so a 5000-pixel HEVC file would be overstated. All 7 files past this
 * line on production are H.264, and a picture that size is a heavy decode
 * whatever the codec, so the note stays worth showing - but it must not claim
 * more than it knows.
 *
 * Distinct from WEB_MAX_EDGE (1920), which is what the repair scales DOWN to.
 * The two numbers answer different questions and must not be merged: 4096 is
 * "where it breaks", 1920 is "where it is safe" - see the encode measurements on
 * WEB_MAX_EDGE for why the repair does not simply park the output at 4096.
 */
export const HARDWARE_DECODER_MAX_EDGE = 4096;

/** Pixels in one H.264 macroblock: 16 x 16. The unit an encoder actually spends in. */
export const MACROBLOCK_PIXELS = 256;

/**
 * Bits per macroblock below which a picture cannot be refreshed, only marked.
 *
 * This is the derivation that puts the bitrate line somewhere defensible instead
 * of somewhere that looks tidy. H.264 codes in 16x16 macroblocks. Signalling
 * "this block is unchanged" costs on the order of one bit; actually coding a
 * block with any detail in it costs tens. So the reported file:
 *
 *     9.7 Mpx / 256 = 38 000 macroblocks per frame
 *     1296 kb/s / 30 fps = 43 kbit per frame
 *     43 000 / 38 000 = 1.14 bits per macroblock
 *
 * There is no bitrate left over to refresh anything. The encoder can code a
 * sliver of the picture per frame and must declare the rest unchanged, and that
 * is why the recording looks frozen while its frame timing is perfect.
 *
 * Two bits per macroblock is therefore the line: below it the encoder cannot
 * even afford one coded block per two, whatever is in the file. Measured against
 * all 464 production videos that could be probed (read-only, September 2026):
 *
 *     1   bit  / macroblock   0 videos  (0.0%)   - would miss the reported file
 *     1.5 bits / macroblock   3 videos  (0.6%)
 *     2   bits / macroblock   6 videos  (1.3%)   <- chosen
 *     3   bits / macroblock  14 videos  (3.0%)
 *     4   bits / macroblock  30 videos  (6.5%)
 *     6   bits / macroblock  58 videos  (12.5%)
 *
 * The reported file sits at 1.14, the lowest of all 464. The lowest file the
 * line does NOT flag is at 2.06, and the median production video is at 16.6, so
 * there is a factor of eight between the line and a normal file. Combined with
 * the width rule the note appears on 12 of 464 rows - 2.6% - which is the number
 * that matters, because a warning shown on a large share of a library is a
 * warning nobody reads.
 */
export const MIN_BITS_PER_MACROBLOCK = 2;

/** The same line expressed per pixel, which is what the row's arithmetic yields. */
export const STARVED_BITS_PER_PIXEL = MIN_BITS_PER_MACROBLOCK / MACROBLOCK_PIXELS;

/**
 * What a video that plays well on this server actually carries.
 *
 * The median of the same 464 production videos: 0.0649 bits per pixel, or 16.6
 * bits per macroblock. It is used for ONE thing - turning "0.0044" into "about
 * fifteen times less than normal", because a reader cannot place a bare bits-per
 * -pixel figure and can place a ratio instantly. Nothing branches on it.
 */
export const TYPICAL_BITS_PER_PIXEL = 0.065;

/**
 * The columns this reasons over.
 *
 * Every one of them is optional and every one of them may be null, because that
 * is the truthful state of an upload that predates the column. Null means
 * "nobody measured", never "small" and never "fine", and each rule below fails
 * CLOSED on it: an unmeasured file is never warned about. That is the only safe
 * direction for a notice whose entire value is that it is rare.
 */
export interface PlaybackSource {
  mimeType?: string | null;
  ext?: string | null;
  size?: number | null;
  width?: number | null;
  height?: number | null;
  /** Frames ffprobe counted. Null on roughly one production video in twenty. */
  frames?: number | null;
  durationSeconds?: number | null;
  fps?: number | null;
}

export type PlaybackFault =
  /** Wider than any hardware decoder will take. */
  | "oversized"
  /** Too few bits for the number of pixels to be refreshed. */
  | "starved";

export interface PlaybackDiagnosis {
  /** Ordered, never empty. `oversized` first: it is the one with a repair. */
  faults: PlaybackFault[];
  /** Longest edge in pixels, for the sentence. */
  edge: number;
  /** Bits per pixel, when it could be computed at all. */
  bitsPerPixel: number | null;
  /**
   * How much more a normal video here carries, rounded, for the sentence.
   *
   * Set only when `starved` is among the faults. A field that carried a number
   * in states where the number means nothing is a field that eventually gets
   * printed in one of them: an oversized but well encoded recording would be
   * told it holds "about 1 times less" than normal.
   */
  starvationFactor: number | null;
  /**
   * Does the repair this app already offers actually apply to this file?
   *
   * Deliberately `needsWebRescue` and not a rule of its own: that is the exact
   * test lib/media/av-targets.ts uses to decide whether `webmp4` appears in the
   * menu at all, so the button can never offer a conversion the route would
   * refuse. A starved file that is already small gets the sentence and no
   * button, which is honest - downscaling cannot put back detail the recorder
   * never wrote.
   */
  repairable: boolean;
  /** Longest edge the repair would produce. Only meaningful when repairable. */
  repairEdge: number;
}

/**
 * How many picture frames the file holds, counted if possible, derived if not.
 *
 * `uploads.frames` means "somebody counted" and is absent on about one video in
 * twenty - an MPEG-TS stream wearing an .mp4 name, a WebM, a damaged MP4. Those
 * still carry a duration and a rate, and for a TOTAL (which is all bits-per-
 * pixel needs) the product is as good as the count. The derived number is never
 * written back to the column; see the note on uploads.frames for why that
 * distinction is worth keeping.
 *
 * This is the one implementation. lib/media/av-meta.ts exports `frameCount` as a
 * thin wrapper over it so the upload path, the backfill script and this module
 * cannot drift; it lives HERE rather than there because av-meta reaches for
 * ffmpeg and storage, and this module is imported by client components.
 */
export function frameTotal(source: PlaybackSource): number | null {
  if (source.frames != null && Number.isFinite(source.frames)) {
    return source.frames > 0 ? Math.round(source.frames) : null;
  }
  const { durationSeconds: duration, fps } = source;
  if (duration == null || fps == null) return null;
  if (!Number.isFinite(duration) || !Number.isFinite(fps)) return null;
  const estimate = Math.round(duration * fps);
  return estimate > 0 ? estimate : null;
}

/**
 * Bits of file per pixel of picture: size x 8 / (width x height x frames).
 *
 * The whole file's size, including its container overhead and its audio track,
 * which overstates the picture's share slightly - and overstating is the right
 * direction here, because it can only ever make the verdict kinder.
 */
export function bitsPerPixel(source: PlaybackSource): number | null {
  const { width, height, size } = source;
  if (!width || !height || !size || size <= 0) return null;
  if (!Number.isFinite(width) || !Number.isFinite(height) || !Number.isFinite(size)) return null;
  const frames = frameTotal(source);
  if (frames == null) return null;
  const pixels = width * height * frames;
  if (pixels <= 0) return null;
  return (size * 8) / pixels;
}

/** Longest edge, so a portrait recording and a rotated one are judged alike. */
function longestEdge(size: AvSourceSize): number {
  return Math.max(size.width ?? 0, size.height ?? 0);
}

/**
 * What is wrong with this video's shape, or null when nothing is.
 *
 * Null is the answer for everything that is not a measured video, which is the
 * overwhelming majority of rows: a picture, an audio file, a paste, and every
 * video whose dimensions nobody ever filled in.
 */
export function diagnosePlayback(source: PlaybackSource): PlaybackDiagnosis | null {
  if (mediaKind(source.mimeType, source.ext) !== "video") return null;

  const edge = longestEdge(source);
  if (edge <= 0) return null;

  const faults: PlaybackFault[] = [];
  if (edge > HARDWARE_DECODER_MAX_EDGE) faults.push("oversized");

  const bpp = bitsPerPixel(source);
  const starved = bpp != null && bpp > 0 && bpp < STARVED_BITS_PER_PIXEL;
  if (starved) faults.push("starved");

  if (faults.length === 0) return null;

  return {
    faults,
    edge,
    bitsPerPixel: bpp,
    starvationFactor: starved ? Math.round(TYPICAL_BITS_PER_PIXEL / bpp!) : null,
    repairable: needsWebRescue(source),
    repairEdge: WEB_MAX_EDGE,
  };
}
