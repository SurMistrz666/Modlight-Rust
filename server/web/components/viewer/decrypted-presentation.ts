/**
 * How decrypted bytes may be shown, and the ceilings that decide whether they
 * are shown at all.
 *
 * ## Why this is a module of its own
 *
 * Decrypting happens in the visitor's tab, so every protection the SERVER
 * applies to an ordinary file is simply not in the path: no allowlist decides
 * the Content-Type, because there is no response; no `X-Content-Type-Options`
 * helps, because nothing was served. The MIME type comes out of the container,
 * where it was put by whoever encrypted the file. It is attacker-controlled.
 *
 * A `blob:` URL inherits the origin of the page that created it. So
 * `URL.createObjectURL(new Blob([html], { type: "text/html" }))` opened in an
 * iframe is a live document ON OUR ORIGIN, with our cookies - the exact stored
 * XSS the raw route has been defending against since day one, reintroduced
 * through the side door. That is the whole reason this file exists as pure,
 * testable logic instead of a few conditionals inside a component.
 *
 * ## The rule
 *
 * The decision is delegated to `serveDecision()`, the SAME allowlist `/raw`
 * uses, so a decrypted file can never be presented in a way the identical
 * unencrypted file would not have been:
 *
 *  - an image only when it is one the browser renders and cannot script
 *    (`isDisplayableImage`, which excludes SVG);
 *  - video/audio only for a container a browser really decodes;
 *  - anything that `/raw` would hand over as `text/plain` - HTML, SVG, XML,
 *    JSON, JavaScript - is rendered as TEXT, escaped, exactly as the source
 *    view does for an uploaded page. It never becomes a document;
 *  - everything else gets a download card.
 *
 * PDF is deliberately stricter here than on the raw route, which serves it
 * inline: a PDF viewer is a large attack surface handed a file no scanner ever
 * saw, on a blob URL that carries our origin. The card costs one click.
 *
 * Rendering an encrypted HTML upload for real is not on offer at all. The
 * sandboxed render route (`Content-Security-Policy: sandbox`, its own opaque
 * origin, its own ticket) works on bytes the server can read, and the server
 * cannot read these. That is a consequence of end-to-end encryption, not a gap.
 */

import {
  effectiveType,
  isDisplayableImage,
  isTextPayload,
  playableAudioType,
  playableVideoType,
} from "@/lib/media/mime";

/**
 * Largest plaintext shown inline.
 *
 * Decrypting for display means the whole plaintext exists in the tab at once
 * (a `<video>` needs a seekable source, and the only seekable source we can
 * build is a Blob). 256 MiB of plaintext peaks at roughly twice that while the
 * ciphertext response is still being consumed, which a desktop tab absorbs and
 * a phone usually does. Past it the viewer offers the streaming save instead of
 * trying and killing the tab, which is the failure this ceiling exists to
 * prevent.
 */
export const MAX_INLINE_BYTES = 256 * 1024 * 1024;

/**
 * Largest file saved WITHOUT the File System Access API.
 *
 * With `showSaveFilePicker` (Chrome, Edge, Opera) the decrypted frames are
 * written straight to the chosen file and there is no ceiling at all - a 5 GB
 * upload comes back down in bounded memory. Without it (Firefox, Safari) the
 * only route is a Blob, and past a couple of gigabytes browsers start refusing
 * one. Refusing with a sentence beats a tab that dies at 80%.
 */
export const MAX_BLOB_SAVE_BYTES = 2 * 1024 * 1024 * 1024;

export type DecryptedMode = "image" | "video" | "audio" | "text" | "download";

export interface DecryptedPresentation {
  mode: DecryptedMode;
  /**
   * Type for the object URL the browser renders. Only set for image/video/audio;
   * `null` everywhere else, because everywhere else there is no object URL.
   */
  renderType: string | null;
  /** Set when `mode` is "download", saying which of the two reasons applies. */
  reason: "too-large" | "not-inline-safe" | null;
}

/**
 * Type to stamp on a Blob that exists only to be SAVED.
 *
 * Always `application/octet-stream`, never the file's own type. A saved blob is
 * reached through `<a download>` or a file handle, where the type changes
 * nothing about how it lands on disk - but a blob URL that carries `text/html`
 * is one address-bar paste away from being a live document on our origin. The
 * extension in the filename is what tells the operating system what the file is.
 */
export const SAVE_BLOB_TYPE = "application/octet-stream";

/** How to present a decrypted file of this type and size. */
export function decryptedPresentation(
  mime: string | null | undefined,
  ext: string | null | undefined,
  size: number,
): DecryptedPresentation {
  if (size > MAX_INLINE_BYTES) {
    return { mode: "download", renderType: null, reason: "too-large" };
  }

  if (isDisplayableImage(mime, ext)) {
    return { mode: "image", renderType: effectiveType(mime, ext), reason: null };
  }

  const video = playableVideoType(mime, ext);
  if (video) return { mode: "video", renderType: video, reason: null };

  const audio = playableAudioType(mime, ext);
  if (audio) return { mode: "audio", renderType: audio, reason: null };

  // Everything /raw would hand over as text/plain: HTML, SVG, XML, JSON,
  // JavaScript, shell scripts. Shown as escaped text, never as a document.
  if (isTextPayload(mime, ext)) {
    return { mode: "text", renderType: null, reason: null };
  }

  return { mode: "download", renderType: null, reason: "not-inline-safe" };
}

/**
 * Filename extension, lower case, for a name that came out of a container.
 *
 * Used only as the second piece of evidence for `effectiveType`, exactly as the
 * serving layer uses `uploads.ext`: plenty of files carry
 * `application/octet-stream` because the operating system had no registry entry
 * for the format, and the name is the better clue for those.
 */
export function extOfName(name: string): string {
  const dot = name.lastIndexOf(".");
  if (dot < 0 || dot === name.length - 1) return "";
  return name.slice(dot + 1).toLowerCase();
}
