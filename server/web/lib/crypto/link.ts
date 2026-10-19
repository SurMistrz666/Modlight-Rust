/**
 * Where the key lives: the URL fragment, and nowhere else.
 *
 * ## The one rule
 *
 * A fragment is the only part of a URL a browser does not send. It is absent
 * from the request line, absent from `Referer`, absent from every access log,
 * every proxy, every CDN and every error report that captures a URL. That is the
 * entire security argument for end-to-end encrypted links, and it survives
 * exactly as long as nobody moves the key into a query parameter "because it was
 * easier to read on the server".
 *
 * So this module is the only place that writes a key into a URL and the only
 * place that reads one out, and it can do neither through a query string:
 *
 *  - `encryptedLink()` refuses to build a link whose base already carries a
 *    query or a fragment, and puts the key after `#`;
 *  - `keyFromLocation()` reads `location.hash` and NOTHING else, so a key that
 *    someone later "helpfully" passes as `?k=...` simply does not arrive - the
 *    viewer reports a missing key rather than quietly working while leaking it
 *    to every log on the path.
 *
 * tests/unit/encrypted-link.test.ts pins all of that, including a scan of the
 * encryption code for query-shaped key parameters.
 */

import { KEY_BYTES } from "@/lib/crypto/envelope";

/** Fragment parameter carrying the key. */
export const KEY_FRAGMENT_PARAM = "k";

/** Base64url alphabet, unpadded: safe in a fragment with no escaping. */
const B64URL = /^[A-Za-z0-9_-]+$/;

/**
 * Bytes to unpadded base64url.
 *
 * `btoa`/`atob` rather than `Buffer`: this module is imported by the browser
 * bundle as well as by Node, and pulling `node:buffer` into a client chunk to
 * encode 32 bytes would be absurd. Both have been global in Node since v16.
 */
export function toBase64Url(bytes: Uint8Array): string {
  let binary = "";
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** Unpadded base64url back to bytes. Returns null for anything malformed. */
export function fromBase64Url(text: string): Uint8Array | null {
  if (!text || !B64URL.test(text)) return null;
  const b64 = text.replace(/-/g, "+").replace(/_/g, "/") + "=".repeat((4 - (text.length % 4)) % 4);
  try {
    const binary = atob(b64);
    const out = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
    return out;
  } catch {
    return null;
  }
}

/** The key as it appears in a link. */
export function encodeKey(key: Uint8Array): string {
  if (key.length !== KEY_BYTES) throw new Error(`Key must be ${KEY_BYTES} bytes.`);
  return toBase64Url(key);
}

/** A key read back from a link, or null when it is missing or the wrong size. */
export function decodeKey(encoded: string | null | undefined): Uint8Array | null {
  if (!encoded) return null;
  const raw = fromBase64Url(encoded.trim());
  return raw && raw.length === KEY_BYTES ? raw : null;
}

/**
 * The shareable link for an encrypted upload.
 *
 * `pageUrl` is the ordinary preview URL the server handed back. The key is
 * appended as a fragment, so what the browser will later request is exactly
 * `pageUrl` - byte for byte the URL the server already knows.
 *
 * A base that already has a query or a fragment is refused rather than merged.
 * Merging is how a key ends up somewhere it should not be: today the caller
 * appends `?w=1`, tomorrow the fragment builder has to reason about it, and the
 * day after someone flattens the two.
 */
export function encryptedLink(pageUrl: string, encodedKey: string): string {
  if (!encodedKey || !B64URL.test(encodedKey)) throw new Error("Key is not base64url.");
  if (pageUrl.includes("#")) throw new Error("Base URL already carries a fragment.");
  if (pageUrl.includes("?")) throw new Error("Base URL already carries a query string.");
  return `${pageUrl}#${KEY_FRAGMENT_PARAM}=${encodedKey}`;
}

/**
 * Read the key out of a fragment.
 *
 * Takes the raw `location.hash` (with or without its leading `#`). Deliberately
 * NOT a whole URL and deliberately not a `URL` object: handing this function a
 * full URL is the shape that would let a query string reach it.
 */
export function keyFromFragment(hash: string | null | undefined): Uint8Array | null {
  if (!hash) return null;
  const body = hash.startsWith("#") ? hash.slice(1) : hash;
  if (!body) return null;
  // Base64url shares no characters with `&` or `=`, so ordinary parsing is safe.
  const params = new URLSearchParams(body);
  return decodeKey(params.get(KEY_FRAGMENT_PARAM));
}

/** The key for the page currently open, or null. Browser only. */
export function keyFromLocation(): Uint8Array | null {
  if (typeof window === "undefined") return null;
  return keyFromFragment(window.location.hash);
}

/**
 * Would requesting this URL put THIS key on the wire?
 *
 * Exact rather than heuristic: it looks for the encoded key itself in the part
 * of the URL a browser transmits (everything before `#`). A "looks like a key"
 * pattern would have been shorter to write and wrong in production, where a
 * vanity code may legitimately be 60 characters of the same alphabet.
 *
 * Used by the tests and as the runtime assertion on the one request the viewer
 * makes, so a future refactor that threads the key into a query string fails
 * loudly instead of leaking quietly.
 */
export function urlLeaksKey(url: string, encodedKey: string): boolean {
  if (!encodedKey) return false;
  const [beforeHash] = url.split("#");
  if (beforeHash.includes(encodedKey)) return true;
  try {
    return decodeURIComponent(beforeHash).includes(encodedKey);
  } catch {
    // A malformed escape is not evidence of a leak; the literal check above
    // already answered the question that matters.
    return false;
  }
}
