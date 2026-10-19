/**
 * The single "Download as…" list, for every kind of file.
 *
 * There are two conversion engines behind this app - libvips for pictures,
 * ffmpeg for anything with a timeline - and each already owns the question of
 * what it can produce (convert-targets.ts, av-targets.ts). What was missing was
 * one place that asks BOTH, because the dashboard menu, the viewer bar and the
 * route each had to know which engine claimed a file, and three copies of that
 * knowledge is how a menu entry ends up answering 415.
 *
 * The list is also ORDERED and GROUPED here, and that is the part that makes it
 * readable rather than merely correct. A menu that puts "one frame as a picture"
 * next to "the whole animation" and lets the reader work out which is which is a
 * menu that gets a file converted wrongly; anything that throws the motion away
 * is separated out under its own heading, and never appears above something that
 * keeps it.
 *
 * Browser-safe: pure functions over a database row's own columns.
 */

import { convertTargetsFor, type Motion } from "@/lib/media/convert-targets";
import {
  avTargetsFor,
  type AvCost,
  type AvSourceSize,
  type AvYield,
} from "@/lib/media/av-targets";

/**
 * Where an entry sits in the menu.
 *
 *  - `main`  : what the reader came for. Keeps whatever the file has.
 *  - `frame` : produces one frame of something that moves. Real - taking a still
 *              out of a GIF is a thing people do - but it must never be reachable
 *              by accident, so it lives below a heading of its own.
 */
export type DownloadSection = "main" | "frame";

export interface DownloadOption {
  /** Value for `?to=`. */
  to: string;
  /** Which engine will run, which decides how the entry is labelled. */
  engine: "image" | "av";
  section: DownloadSection;
  /** Pictures only: what happens to the animation on the way out. */
  motion?: Motion;
  /** Media only: what comes out, and who may ask for it. */
  yields?: AvYield;
  cost?: AvCost;
}

export interface DownloadMenuOptions extends AvSourceSize {
  /**
   * May this viewer ask for something that runs an encoder?
   *
   * True for the owner and for admins. A public visitor is shown only the stream
   * copies, because the rest would be a menu entry that answers 403 - see the
   * authorization note in av-targets.ts. This is a DISPLAY rule; the route
   * enforces the same thing independently.
   */
  canEncode: boolean;
}

export function downloadOptionsFor(
  mime: string | null | undefined,
  ext: string | null | undefined,
  frames: number | null | undefined,
  { canEncode, width, height }: DownloadMenuOptions,
): DownloadOption[] {
  const claimed = avTargetsFor(mime, ext, frames, { width, height });
  const av = claimed
    .filter((o) => canEncode || o.cost === "copy")
    .map<DownloadOption>((o) => ({
      to: o.to,
      engine: "av",
      // Nothing ffmpeg offers throws motion away any more: the frame grabs were
      // removed from the menu outright (av-targets.ts), so every entry here is
      // something the reader asked for whole.
      section: "main",
      yields: o.yields,
      cost: o.cost,
    }));

  // Video and audio belong to ffmpeg alone; libvips returns nothing for them.
  // An ANIMATION, though, is genuinely claimed by both - sharp keeps a GIF's
  // motion in WebP, ffmpeg turns the same GIF into an MP4 seventy times smaller,
  // and both are worth having. Where the two name the same target, ffmpeg wins:
  // it is the one that reaches every frame of an AVIF sequence, where libvips
  // only ever sees the cover.
  //
  // Note the dedupe runs against everything ffmpeg CLAIMS, not against what this
  // viewer is allowed to run. The route dispatches on `avOptionFor` before it
  // checks permissions, so a name ffmpeg claims never reaches libvips - and
  // handing a public visitor the libvips WebP of an AVIF sequence produced
  // exactly that mismatch: the menu drew the entry, the route gave it to ffmpeg,
  // and ffmpeg answered 403 because animating it is an encode.
  const taken = new Set<string>(claimed.map((o) => o.to));
  const image = convertTargetsFor(mime, ext, frames).filter((o) => !taken.has(o.to));

  // Does anything reach this file's motion? That decides what to do with the
  // uncounted rows - every imported row has no frame count, so "one frame
  // comes out" is a guess there. Where the motion is reachable the guess is
  // worth acting on and the still targets are demoted; with nothing to demote
  // them below (a lone WebP that may or may not move) they stay where they are
  // and say "if" instead, rather than heading a section about a loss that may
  // not be happening. Read from `claimed` rather than from the visible list,
  // because whether the file moves is not a fact about who is looking at it.
  const keepsMotion =
    claimed.some((o) => o.yields === "animation") || image.some((o) => o.motion === "keeps");

  const all: DownloadOption[] = [
    ...av,
    ...image.map<DownloadOption>((o) => ({
      to: o.to,
      engine: "image",
      section:
        o.motion === "single" || (o.motion === "single-if-animated" && keepsMotion)
          ? "frame"
          : "main",
      motion: o.motion,
    })),
  ];

  return [...all.filter((o) => o.section === "main"), ...all.filter((o) => o.section === "frame")];
}

/**
 * The i18n key for the small grey line under a target's name.
 *
 * Returned as a key rather than a string so the caller translates it in its own
 * namespace, and so this module stays free of next-intl. Every one of these has
 * to survive being read by somebody who does not know what a container is - the
 * word "remux" was in here once and it told the reader nothing.
 */
export function downloadHintKey(option: DownloadOption): string | null {
  if (option.engine === "image") {
    if (option.motion === "keeps") return "convert.keepsAnimation";
    if (option.motion === "single") return "convert.singleFrame";
    if (option.motion === "single-if-animated") return "convert.frameIfAnimated";
    // WebP and AVIF used to carry "usually about half the size, same quality".
    // Measured over 308 production conversions that is wrong in both directions
    // (png to webp is 0.125x, jpg to webp is 0.74x), and the menu now prints the
    // predicted size beside the format name instead - see lib/media/convert-size.ts.
    // A number the reader can act on replaces a sentence that had to be vague.
    return null;
  }
  switch (option.yields) {
    case "video":
      return "convert.containerCopy";
    case "webVideo":
      return "convert.webPlayable";
    case "clip":
      return "convert.animationClip";
    case "animation":
      return "convert.keepsMotion";
    case "audio":
      return "convert.audioTrack";
    case "audioFormat":
      return "convert.audioFormat";
    case "audioVideo":
      return "convert.audioAsVideo";
    default:
      return null;
  }
}

/**
 * The name to print for a target, when its own letters do not say it.
 *
 * Returned as a translation key rather than a string, like every other label in
 * this module, and null for the overwhelming majority: a format name is its own
 * best label and "MP4" needs no help. The exception is `webmp4`, which writes an
 * ordinary .mp4 and would otherwise print as "WEBMP4" - a word that tells the
 * reader nothing and looks like a typo for WebM.
 */
export function downloadLabelKey(to: string): string | null {
  return to === "webmp4" ? "convert.webPlayableLabel" : null;
}

/** Does the menu need a heading before this entry? Only the first frame target. */
export function sectionHeadingKey(
  option: DownloadOption,
  previous: DownloadOption | undefined,
): string | null {
  if (option.section !== "frame") return null;
  return previous && previous.section === "frame" ? null : "convert.frameSection";
}
