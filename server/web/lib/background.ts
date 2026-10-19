/**
 * Per-account wallpaper: the image a user puts behind their panel, and behind
 * every public page they own.
 *
 * Framework-agnostic helpers only (no next/*, no db) so the serving route, the
 * upload route, the profile form and the theme applier all agree on the same
 * rules without importing each other.
 */

/** Where a wallpaper lives in storage. The token is what makes the URL cacheable. */
export const BACKGROUND_PREFIX = "backgrounds";

/** Re-encoded output, always. */
export const BACKGROUND_CONTENT_TYPE = "image/webp";

/**
 * Largest upload we will even read, before re-encoding.
 *
 * Generous enough for a phone photo or a 4K wallpaper, small enough that a
 * request can be buffered without turning this into an upload endpoint. Real
 * uploads go through tus and busboy; this is a settings form.
 */
export const BACKGROUND_MAX_BYTES = 12 * 1024 * 1024;

/** Longest edge kept after re-encoding. Beyond this nobody can see the detail. */
export const BACKGROUND_MAX_EDGE = 2560;

/** Veil strength bounds, in percent of the skin's own background colour. */
export const DIM_MIN = 0;
export const DIM_MAX = 95;

/**
 * Weakest veil at which body text still clears WCAG AA (4.5:1) over the WORST
 * image anyone could pick - measured against pure white and pure black across
 * all twelve skins in both modes. The floor is terminal/dark, which lands at
 * 4.85:1 here and 4.35:1 one notch lower.
 *
 * Not enforced: it is the owner's panel and a vivid wallpaper is a legitimate
 * choice. It is what the slider warns below.
 */
export const DIM_SAFE = 68;

/**
 * Chosen for the same measurement with margin (5.31:1 worst case) rather than
 * for looks. Someone setting a wallpaper is not thinking about contrast, so the
 * value they get without touching anything has to be legible over any photo;
 * they can see the effect of lowering it immediately.
 */
export const DIM_DEFAULT = 70;

export function clampDim(value: unknown): number {
  // null, undefined and "" all coerce to 0 through Number(), and 0 is a
  // meaningful setting here (no veil at all) - so an absent value would silently
  // become "photograph at full strength behind the text" rather than the
  // default. They have to be rejected before any coercion happens.
  if (value === null || value === undefined || value === "") return DIM_DEFAULT;

  const n = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(n)) return DIM_DEFAULT;
  return Math.min(DIM_MAX, Math.max(DIM_MIN, Math.round(n)));
}

/**
 * Storage key for a fresh wallpaper.
 *
 * The random token is load-bearing twice over: it lets the public URL be
 * immutable-cached (a replacement is a different URL, so no cache ever has to be
 * invalidated), and it keeps the key unguessable, so `/bg/{tag}` stays the only
 * way in even on a storage backend that serves objects directly.
 */
export function backgroundKey(userId: string, token: string): string {
  return `${BACKGROUND_PREFIX}/${userId}-${token}.webp`;
}

/**
 * The cache-busting fragment of a key: `backgrounds/<id>-<token>.webp` -> token.
 *
 * Returns "" for anything that does not look like one of our keys, which the
 * caller turns into a URL with no `v` - correct but merely uncached, never wrong.
 */
export function backgroundToken(key: string | null | undefined): string {
  if (!key) return "";
  const base = key.slice(key.lastIndexOf("/") + 1).replace(/\.webp$/, "");
  const dash = base.lastIndexOf("-");
  return dash < 0 ? "" : base.slice(dash + 1);
}

/**
 * Public URL of an account's wallpaper, or null when it has none.
 *
 * Keyed by tag rather than by user id: the tag is already public (it is the
 * first segment of every link this account hands out), whereas the internal id
 * is not, and a wallpaper is served to anyone who can open the page.
 *
 * Root-relative on purpose. The app answers on several domains, and a wallpaper
 * pinned to one of them would be a cross-origin image request on the others -
 * which the Content-Security-Policy would then have to allow for no reason.
 */
export function backgroundUrl(
  userCode: string | null | undefined,
  key: string | null | undefined,
): string | null {
  if (!userCode || !key) return null;
  const token = backgroundToken(key);
  return `/bg/${encodeURIComponent(userCode)}${token ? `?v=${encodeURIComponent(token)}` : ""}`;
}

/** Everything the theme applier needs to paint one. */
export interface BackgroundStyle {
  url: string;
  /** 0-1, ready for CSS. */
  dim: number;
}

export function backgroundStyle(
  userCode: string | null | undefined,
  key: string | null | undefined,
  dim: number | null | undefined,
): BackgroundStyle | null {
  const url = backgroundUrl(userCode, key);
  if (!url) return null;
  return { url, dim: clampDim(dim ?? DIM_DEFAULT) / 100 };
}

/**
 * Image types we accept for re-encoding.
 *
 * SVG is excluded deliberately. It is a script-bearing document, and although
 * sharp would rasterise it, its loader has been a source of file-read and
 * network-fetch bugs - and this is the one image path where the bytes arrive
 * from a form rather than the upload pipeline.
 */
const ACCEPTED = new Set(["image/png", "image/jpeg", "image/webp", "image/avif", "image/gif"]);

export function isAcceptedBackgroundType(type: string | null | undefined): boolean {
  return !!type && ACCEPTED.has(type.split(";")[0].trim().toLowerCase());
}

/** For the file picker's `accept` attribute. */
export const BACKGROUND_ACCEPT = [...ACCEPTED].join(",");
