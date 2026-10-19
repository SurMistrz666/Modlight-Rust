import { config } from "@/lib/config";

/** Build the storage key for a media file: "{userCode}/{code}.{ext}". */
export function storageKeyFor(userCode: string, code: string, ext: string): string {
  return ext ? `${userCode}/${code}.${ext}` : `${userCode}/${code}`;
}

/**
 * Which origin a given user's links are generated with.
 *
 * The deployment answers on several names (host.example.com, h.example.com, s.example.com —
 * see `config.appDomains`) and every one of them serves the same files, so this
 * only decides what a NEW link *reads* as. Anything already shared keeps
 * resolving on whichever host it was created with, forever.
 *
 * An unknown value falls back to `config.appUrl` rather than being echoed back:
 * the result ends up in ShareX configs, Open Graph tags and copied links, so a
 * stored origin the operator never configured must never be trusted. That also
 * makes the column self-healing — drop a domain from APP_DOMAINS and the
 * accounts pointing at it quietly return to the default.
 */
export function userBaseUrl(preferred?: string | null): string {
  if (!preferred) return config.appUrl;
  const wanted = preferred.trim().replace(/\/+$/, "").toLowerCase();
  if (!wanted) return config.appUrl;
  return config.appDomains.find((d) => d.toLowerCase() === wanted) ?? config.appUrl;
}

/** Is this origin one the operator configured? Used to validate user input. */
export function isKnownDomain(origin: string): boolean {
  const wanted = origin.trim().replace(/\/+$/, "").toLowerCase();
  return config.appDomains.some((d) => d.toLowerCase() === wanted);
}

/**
 * Public preview page URL: /{userCode}/{code}.{ext}  (the shape ShareX configs expect).
 *
 * `base` defaults to `config.appUrl` so every existing call site keeps its
 * behaviour; pass `userBaseUrl(owner.preferredDomain)` to honour the owner's
 * chosen hostname.
 */
export function publicUrl(userCode: string, code: string, ext: string, base = config.appUrl): string {
  return ext ? `${base}/${userCode}/${code}.${ext}` : `${base}/${userCode}/${code}`;
}

/**
 * Preview URL without the tag segment: /{code}.{ext}
 *
 * Admin-only (user.skipTagInUrl) because a tagless code sits in the root
 * namespace shared with every top-level route — see RESERVED_TAGS in
 * lib/naming.ts, which is what keeps a code from shadowing /login.
 */
export function taglessUrl(code: string, ext: string, base = config.appUrl): string {
  return ext ? `${base}/${code}.${ext}` : `${base}/${code}`;
}

/** Raw file URL without the tag segment: /{code}/raw.{ext} */
export function taglessRawUrl(code: string, ext: string, base = config.appUrl): string {
  return ext ? `${base}/${code}/raw.${ext}` : `${base}/${code}/raw`;
}

/** Raw file URL: /{userCode}/{code}/raw.{ext}  (the shape ShareX configs expect). */
export function rawUrl(userCode: string, code: string, ext: string, base = config.appUrl): string {
  return ext ? `${base}/${userCode}/${code}/raw.${ext}` : `${base}/${userCode}/${code}/raw`;
}

/**
 * The link-shaping facts that belong to an ACCOUNT rather than to a file.
 * Structural on purpose, so a caller may hand over a whole user row or just the
 * three columns it bothered to select.
 */
export interface LinkOwner {
  userCode?: string | null;
  preferredDomain?: string | null;
  skipTagInUrl?: boolean | null;
}

/**
 * The pair of public links for one upload, on the hostname and in the shape its
 * owner chose.
 *
 * Two account settings are folded in here and neither is cosmetic:
 * `preferredDomain` decides which configured hostname a NEW link reads as, and
 * `skipTagInUrl` drops the /{tag}/ segment entirely for the accounts allowed to.
 * Both were being spelled out again at every call site - the ShareX finalize
 * path, the release manifest, and now the listing API the desktop client pages
 * through - and the second one has already been forgotten once here: uploads
 * kept coming back as /{tag}/{code} while the profile switch said otherwise,
 * because only the profile page consulted the column. One function, so the next
 * caller cannot forget it and the panel, ShareX and the desktop client cannot
 * hand the same user three different spellings of the same file.
 */
export function mediaUrls(
  owner: LinkOwner,
  code: string,
  ext: string,
): { url: string; rawUrl: string } {
  const base = userBaseUrl(owner.preferredDomain);
  const tag = owner.userCode ?? "u";

  if (owner.skipTagInUrl === true) {
    return { url: taglessUrl(code, ext, base), rawUrl: taglessRawUrl(code, ext, base) };
  }
  return { url: publicUrl(tag, code, ext, base), rawUrl: rawUrl(tag, code, ext, base) };
}

/**
 * Sandboxed-render URL for an uploaded page: /{userCode}/{code}/render?t=…
 *
 * Deliberately RELATIVE. The frame has to be same-origin with the page that
 * embeds it (the app CSP's frame-src is our own origins, and the owner's
 * preferred hostname is not necessarily the one the visitor is on), and a
 * relative path is also the one form that cannot be pasted somewhere else and
 * opened as a top-level document by accident. Keep every caller on this helper:
 * moving the sandbox to a separate registrable domain later is then one edit.
 */
export function renderPath(userCode: string, code: string, ticket: string): string {
  return `/${userCode}/${code}/render?t=${encodeURIComponent(ticket)}`;
}

/**
 * One action on a file as a PATH: /{userCode}/{code}/{action}.
 *
 * Never carries an origin, and that is the point. The actions a PAGE fetches
 * for itself - the scrub-bar preview track and its sprite sheet, the video
 * poster - have to be same-origin with the page that fetches them: a text
 * track is fetched under CORS rules, the sheet is pulled by an `<img
 * crossorigin>` inside the player's shadow root, and this deployment answers
 * on three hostnames with no CORS headers between them. Same-origin is also
 * what carries the unlock cookie of a password-protected file, which exists
 * only on the hostname the visitor unlocked.
 *
 * The tagged form always resolves, including for an owner whose links are
 * spelled without the tag: /{tag}/{code}/... is the canonical route and the
 * tagless page is rendered by it (app/[userCode]/page.tsx).
 */
export function mediaActionPath(userCode: string, code: string, action: string): string {
  return `/${userCode}/${code}/${action}`;
}

/** Force-download URL. */
export function downloadUrl(userCode: string, code: string, base = config.appUrl): string {
  return `${base}/${userCode}/${code}/download`;
}

/** Deletion URL used by ShareX: /{userCode}/{code}/delete/{token}. */
export function deleteUrl(userCode: string, code: string, token: string, base = config.appUrl): string {
  return `${base}/${userCode}/${code}/delete/${token}`;
}

/** Extract a lowercase extension (no dot) from a filename. */
export function extOf(filename: string): string {
  const m = /\.([a-z0-9]+)$/i.exec(filename);
  return m ? m[1].toLowerCase() : "";
}

/**
 * Taking those URLs and keys back APART lives in lib/media/paths.ts, which has
 * no imports at all - not even this module's `config` - so the browser side of
 * the conversion menu can use it. Re-exported here because this is where callers
 * already look.
 */
export {
  codeFromMediaCode,
  suffixKey,
  swapFilenameExt,
  swapKeyExt,
} from "@/lib/media/paths";
