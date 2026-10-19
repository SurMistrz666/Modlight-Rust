/**
 * Per-account avatar: the little round picture that replaces the initials.
 *
 * Framework-agnostic helpers only (no next/*, no db, no sharp) so the serving
 * route, the upload route, the profile card and every place that draws an
 * initials circle agree on the same rules without importing each other. Same
 * shape as lib/background.ts, deliberately: an avatar is the second piece of
 * per-account decoration and the two should not need two mental models.
 *
 * Central decision, and the one that everything else follows from: an avatar is
 * ALWAYS a small square this app re-encoded itself. It is never the file the
 * user picked, even when they picked one of their own uploads. See
 * lib/media/avatar.ts for the encoder and app/actions/avatar.ts for why the
 * "choose one of my files" path derives a copy rather than pointing at the row.
 */

/** Where a derived avatar lives in storage. The token is what makes it cacheable. */
export const AVATAR_PREFIX = "avatars";

/** Re-encoded output, always. Animated avatars are animated WebP, same type. */
export const AVATAR_CONTENT_TYPE = "image/webp";

/**
 * Largest input we will even read, before re-encoding.
 *
 * A third of the wallpaper's allowance (12 MB) because an avatar is drawn at
 * 28-40 CSS pixels: past this the extra bytes cannot reach the screen, they can
 * only reach libvips. Still generous enough for a phone photo or a real
 * animated GIF, and comfortably under MAX_TRANSFORM_BYTES (32 MB), which is the
 * ceiling on anything that has to exist in memory as a whole.
 */
export const AVATAR_MAX_BYTES = 8 * 1024 * 1024;

/**
 * Edge of the square we store.
 *
 * The largest avatar the panel draws today is `size-10` (40 px), and the picker
 * preview is 96 px; at 3x device pixel ratio that is 288. 256 covers every real
 * screen for the sizes in use and keeps a still WebP around 10-20 kB.
 * Deliberately NOT enlarging below this (see withoutEnlargement in the encoder):
 * a 64 px source stays 64 px rather than being upscaled into fake detail.
 */
export const AVATAR_SIZE = 256;

/**
 * Frames we are willing to encode.
 *
 * At the 10 fps a GIF usually runs, 120 frames is twelve seconds. Beyond that
 * it is a video wearing a GIF container, and the thing it will be shown in is a
 * 32 px circle in a sidebar.
 */
export const AVATAR_MAX_FRAMES = 120;

/**
 * Decompression-bomb guard for ONE frame. A 40 kB PNG can declare
 * 100000 x 100000 pixels, which libvips would happily try to allocate.
 * Same number as MAX_THUMBNAIL_PIXELS, for the same reason.
 */
export const AVATAR_MAX_FRAME_PIXELS = 64_000_000;

/**
 * Frame-aware ceiling: width x FRAME height x frame count.
 *
 * This is the guard that a naive `width * height` misses, and the miss is not
 * theoretical - a 1654x762 GIF with 256 frames reads as 1.26 Mpx and costs 322.
 * The animated WebP encoder measures at 7.4 Mpx/s of source
 * (lib/media/transform-cost.ts), so 40 Mpx is about 5.4 s of work: inside the
 * 10 s deadline a thumbnail-weight job is given, with room for a slower core.
 */
export const AVATAR_MAX_TOTAL_PIXELS = 40_000_000;

/**
 * Ceiling on what we STORE, which is the only number the viewer feels.
 *
 * The frame budget bounds the encode, not the result: a busy 120-frame sequence
 * can still come out fat, and this file is fetched on every page of the panel
 * plus every public page its owner owns. When an animated encode lands above
 * this the avatar is re-encoded as a still and the owner is told that it was -
 * silently dropping the animation would be the "ciche Failed" mistake again.
 */
export const AVATAR_MAX_OUTPUT_BYTES = 512 * 1024;

/**
 * Storage key for a fresh avatar.
 *
 * The random token is load-bearing twice, exactly as for wallpapers: it lets the
 * public URL be immutable-cached (a replacement is a different URL, so no cache
 * ever needs invalidating), and it keeps the key unguessable so `/avatar/{tag}`
 * stays the only way in even on a storage backend that serves objects directly.
 */
export function avatarKey(userId: string, token: string): string {
  return `${AVATAR_PREFIX}/${userId}-${token}.webp`;
}

/** True for a value this app wrote, as opposed to anything else in `user.image`. */
export function isAvatarKey(value: string | null | undefined): value is string {
  return typeof value === "string" && value.startsWith(`${AVATAR_PREFIX}/`);
}

/**
 * The cache-busting fragment of a key: `avatars/<id>-<token>.webp` -> token.
 *
 * Returns "" for anything that does not look like one of our keys, which the
 * caller turns into a URL with no `v` - correct but merely uncached, never wrong.
 */
export function avatarToken(key: string | null | undefined): string {
  if (!key) return "";
  const base = key.slice(key.lastIndexOf("/") + 1).replace(/\.webp$/, "");
  const dash = base.lastIndexOf("-");
  return dash < 0 ? "" : base.slice(dash + 1);
}

/**
 * Public URL of an account's avatar, or null when it has none.
 *
 * Keyed by tag rather than by user id, like the wallpaper: an avatar is shown to
 * OTHER people (the admin user list, and any public page of that account), and
 * the tag is already the first segment of every link this account hands out,
 * whereas the internal id is not published anywhere. So the URL leaks the tag
 * and a random token, and nothing else - no id, no address, no file name, and
 * nothing about which upload it was made from.
 *
 * Root-relative on purpose: the app answers on several domains, and an avatar
 * pinned to one of them would be a cross-origin image request on the others.
 *
 * `image` is Better Auth's own column. Nothing writes an http(s) URL into it in
 * this deployment (there are no social providers), but a future one would, so an
 * absolute value is passed through untouched rather than mangled into a key.
 */
export function avatarUrl(
  userCode: string | null | undefined,
  image: string | null | undefined,
): string | null {
  if (!image) return null;
  if (/^https?:\/\//i.test(image) || image.startsWith("/")) return image;
  if (!userCode || !isAvatarKey(image)) return null;

  const token = avatarToken(image);
  return `/avatar/${encodeURIComponent(userCode)}${token ? `?v=${encodeURIComponent(token)}` : ""}`;
}

/**
 * Image types we accept for re-encoding.
 *
 * SVG is excluded for the same reason as on the wallpaper path: it is a
 * script-bearing document, and although libvips would rasterise it, its loader
 * has been a source of file-read and network-fetch bugs.
 */
const ACCEPTED = new Set(["image/png", "image/jpeg", "image/webp", "image/avif", "image/gif"]);

export function isAcceptedAvatarType(type: string | null | undefined): boolean {
  return !!type && ACCEPTED.has(type.split(";")[0].trim().toLowerCase());
}

/** For the file picker's `accept` attribute. */
export const AVATAR_ACCEPT = [...ACCEPTED].join(",");

/** Why an avatar came out still, or was refused outright. */
export type AvatarPlanReason =
  /** The source had one frame; nothing was lost. */
  | "still"
  /** More frames than AVATAR_MAX_FRAMES. */
  | "frames"
  /** width x frame height x frames over AVATAR_MAX_TOTAL_PIXELS. */
  | "pixels"
  /** A single frame over AVATAR_MAX_FRAME_PIXELS: refused, not degraded. */
  | "frame-too-large"
  /** Within every budget; the avatar keeps its animation. */
  | "animate";

export interface AvatarSource {
  width: number;
  /** Height of ONE frame (sharp's `pageHeight`), never the stacked strip. */
  height: number;
  /** Frames in the source. 1 for a still. */
  pages: number;
}

export interface AvatarPlan {
  /** false = refuse the file entirely; there is no cheaper thing to encode. */
  ok: boolean;
  /** Encode as an animated WebP. */
  animate: boolean;
  /** Frames the output will hold. */
  frames: number;
  reason: AvatarPlanReason;
}

/**
 * What to encode, decided from the source's shape alone.
 *
 * Two ceilings, and they answer different questions. The per-frame one is the
 * decompression-bomb guard and it REFUSES: a single frame of 64 Mpx is not a
 * picture of a person's face, and there is nothing sensible to fall back to.
 * The total one is the cost budget and it DEGRADES to a still, because "your
 * GIF is too long to animate at this size" leaves the user with a working
 * avatar, and a 415 leaves them with nothing.
 *
 * Pure arithmetic over the constants above, so the budget is a unit test rather
 * than a production incident - the same discipline as lib/media/transform-cost.ts.
 */
export function planAvatar(src: AvatarSource): AvatarPlan {
  const width = Math.max(0, Math.floor(src.width) || 0);
  const height = Math.max(0, Math.floor(src.height) || 0);
  const pages = Number.isFinite(src.pages) && src.pages > 0 ? Math.floor(src.pages) : 1;

  const framePixels = width * height;
  if (!Number.isFinite(framePixels) || framePixels > AVATAR_MAX_FRAME_PIXELS) {
    return { ok: false, animate: false, frames: 0, reason: "frame-too-large" };
  }

  if (pages <= 1) return { ok: true, animate: false, frames: 1, reason: "still" };
  if (pages > AVATAR_MAX_FRAMES) {
    return { ok: true, animate: false, frames: 1, reason: "frames" };
  }

  // The multiplication a per-frame guard forgets. Charging one frame here is
  // what let a 1654x762x256 GIF read as 1.26 Mpx while costing 322.
  const totalPixels = framePixels * pages;
  if (totalPixels > AVATAR_MAX_TOTAL_PIXELS) {
    return { ok: true, animate: false, frames: 1, reason: "pixels" };
  }

  return { ok: true, animate: true, frames: pages, reason: "animate" };
}
