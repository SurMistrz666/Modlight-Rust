/**
 * Which formats a file can be turned into, and what that does to its animation.
 *
 * Split out from `convert.ts` because this half has to run in the browser: the
 * dashboard menu and the viewer bar decide what to offer, and they cannot import
 * sharp. It used to be kept as three hand-maintained copies of a target list
 * plus a regex over the URL, which is four answers to one question.
 *
 * It is also the ONLY answer the route accepts. Not "the route checks that the
 * target is a known format" - that let `/convert?to=avif` through on files whose
 * menu never offered it, and let five targets be offered for an SVG that the
 * route then answered 415. Menu and route call this function, compare against
 * the SAME list, and there is no third opinion to drift.
 */

import { effectiveType, mediaKind } from "@/lib/media/mime";

export type ConvertTarget = "pdf" | "png" | "jpg" | "webp" | "avif";

/**
 * What happens to motion on the way to a target.
 *
 *  - `keeps`             : the animation survives (GIF/APNG -> animated WebP);
 *  - `single`            : this file HOLDS several frames and one comes out;
 *  - `single-if-animated`: the format can animate, nobody has counted THIS file,
 *                          and if it turns out to move then one frame comes out;
 *  - `null`              : nothing to say - either the format cannot animate, or
 *                          we know this file holds exactly one frame.
 *
 * The UI renders this next to the format name, and the third case is why the
 * list has four entries instead of three. "single frame" used to be printed
 * under every still target of any animation-CAPABLE format, so a perfectly
 * ordinary one-frame WebP or AVIF was warned about losing an animation it never
 * had - which reads as "this conversion is lossy, do not bother" and is the
 * reason a user reported the AVIF entries as nonsense. A warning must describe
 * the file in front of the reader, not its container's abilities.
 */
export type Motion = "keeps" | "single" | "single-if-animated" | null;

export interface ConvertOption {
  to: ConvertTarget;
  motion: Motion;
}

/**
 * Every target, in menu order. There is no "jpeg" alias: it was in the type but
 * never in this list, so `isConvertTarget("jpeg")` was false and the encoder's
 * `case "jpeg"` was unreachable code pretending to be a supported format.
 */
export const IMAGE_TARGETS: ConvertTarget[] = ["pdf", "png", "jpg", "webp", "avif"];

/**
 * Containers that can hold an animation.
 *
 * Membership decides what to say when we do NOT know the frame count - see
 * `convertTargetsFor`. Once `frames` is known this set stops mattering, which is
 * the point: a still WebP is not "possibly animated", it is a still.
 */
const ANIMATION_CAPABLE = new Set(["image/gif", "image/apng", "image/webp", "image/avif"]);

/**
 * ...and the subset sharp can re-encode AS an animation.
 *
 * AVIF is absent in both directions, measured rather than assumed. Reading:
 * libheif hands libvips only the primary item of an AVIF sequence, so the 43
 * frames in the file that started this work are invisible and the cover frame is
 * all there is. Writing: sharp does not produce animated AVIF at all - it stacks
 * the frames into one tall strip (a 15-frame 144x256 GIF comes out 144x3840) and
 * fails outright past roughly 16k pixels of stack.
 */
const ANIMATION_READABLE = new Set(["image/gif", "image/apng", "image/webp"]);

/**
 * The target a file already IS, which is therefore not a conversion.
 *
 * "Download this PNG as PNG" spends a decode and an encode to hand back the file
 * the reader already has, and for JPEG it hands back a visibly worse one. The
 * entry was pure noise in a menu whose whole problem was noise.
 */
const SELF_TARGET: Record<string, ConvertTarget> = {
  "image/png": "png",
  "image/apng": "png",
  "image/jpeg": "jpg",
  "image/webp": "webp",
  "image/avif": "avif",
};

export function isConvertTarget(v: string | null | undefined): v is ConvertTarget {
  return !!v && (IMAGE_TARGETS as string[]).includes(v);
}

/** WebP is the only animated container sharp writes. */
export function keepsAnimation(to: ConvertTarget): boolean {
  return to === "webp";
}

/**
 * The single source of truth for "download as…".
 *
 * `frames` is what the stored row knows about THIS file (uploads.frames), not
 * about its format: 1 means a still, >1 an animation, null "nobody has counted"
 * - which is every imported row and every upload made before the column
 * existed.
 *
 * The gate is `mediaKind`, the same predicate the route uses, and it is stricter
 * than the old `startsWith("image/")` on purpose. That test admitted three
 * families libvips cannot open in our build - SVG, HEIC/HEIF and JXL - so the
 * menu offered five conversions and the route answered 415 to all five. SVG
 * would in fact rasterise through librsvg, and is still excluded: rendering
 * attacker-supplied markup with a full SVG engine is a much larger attack
 * surface than decoding a raster, and we deliberately serve those bytes as inert
 * text/plain everywhere else.
 */
export function convertTargetsFor(
  mime: string | null | undefined,
  ext?: string | null,
  frames?: number | null,
): ConvertOption[] {
  if (mediaKind(mime, ext) !== "image") return [];

  const m = effectiveType(mime, ext);
  const known = typeof frames === "number" && frames > 0 ? frames : null;
  // Never offer the format the file already is - see SELF_TARGET.
  const targets = IMAGE_TARGETS.filter((to) => to !== SELF_TARGET[m]);

  // Either the format cannot animate, or this particular file demonstrably does
  // not. Both mean the same thing to a reader: there is no motion to lose.
  if (known === 1 || !ANIMATION_CAPABLE.has(m)) {
    return targets.map((to) => ({ to, motion: null }));
  }

  // Something that moves, or might. AVIF is offered here too, as a still. It was
  // withheld once on the grounds that it "does not work", which was untrue -
  // production converts a GIF to AVIF fine (200, 11568 bytes, 144x256, one
  // frame). The real objection was cost, and cost is now charged where it
  // belongs: lib/media/transform-cost.ts gives AVIF a 13 Mpx ceiling against
  // PNG's 64, measured from its 35 bytes of RSS per source pixel.
  const lossOfMotion: Motion = known ? "single" : "single-if-animated";
  const keepsMotion = ANIMATION_READABLE.has(m);

  return targets
    .map<ConvertOption>((to) => ({
      to,
      motion: to === "webp" && keepsMotion ? "keeps" : lossOfMotion,
    }))
    // The one that survives intact goes first; the rest keep their listed order.
    .sort((a, b) => Number(b.motion === "keeps") - Number(a.motion === "keeps"));
}

/** Does this file offer any conversion at all? */
export function canConvert(
  mime: string | null | undefined,
  ext?: string | null,
  frames?: number | null,
): boolean {
  return convertTargetsFor(mime, ext, frames).length > 0;
}

/**
 * Is `to` something this file really offers?
 *
 * The route's admission test, and it is the same call the menu renders from, so
 * "visible in the UI" and "accepted by the server" cannot come apart.
 */
export function convertOptionFor(
  mime: string | null | undefined,
  ext: string | null | undefined,
  frames: number | null | undefined,
  to: string | null | undefined,
): ConvertOption | null {
  if (!isConvertTarget(to)) return null;
  return convertTargetsFor(mime, ext, frames).find((o) => o.to === to) ?? null;
}
