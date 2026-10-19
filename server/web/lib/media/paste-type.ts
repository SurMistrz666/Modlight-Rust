/**
 * What type is an in-app paste, really?
 *
 * A paste was stored with a hardcoded `text/plain; charset=utf-8` whatever
 * language its author picked, and the language itself went into a column
 * (`uploads.language`) that the whole preview layer ignored. The extension on
 * disk was the only trace of it that ever reached a browser. So a paste created
 * as HTML looked, to every decision downstream, exactly like a paste created as
 * plain text - which is why it got no source/page toggle while an uploaded
 * `page.html` did.
 *
 * The fix is to make the stored type honest rather than to teach the preview
 * layer a second way of guessing. `language` is not a filename: it is written
 * only by `createPaste`, only from the closed `LANG_EXT` list, and only because
 * a signed-in author picked it from a menu. That is the same act as uploading a
 * .html file, no more and no less, so it earns the same treatment and no new
 * capability appears.
 *
 * NONE of this touches how bytes are served. `serveDecision()` in mime.ts is an
 * allowlist: a paste resolved to text/html still leaves the server as
 * `text/plain; charset=utf-8`, and `pasteMimeType` below refuses to return any
 * type that would not. That refusal is asserted in tests/unit/source-preview.
 */

import { LANG_EXT } from "@/lib/paste-languages";
import { mimeFromExt, normalizeMime } from "@/lib/media/sniff";
import { isTextPayload } from "@/lib/media/mime";

/**
 * The MIME a paste declares by its language, or null when the language says
 * nothing usable.
 *
 * Two guards, both load-bearing:
 *
 *  - A language NAME is not a file extension. `LANG_EXT.ts` is "ts", and the
 *    extension table reads `ts` as `video/mp2t` (MPEG transport stream), so a
 *    TypeScript paste would have become a video row: a dead player instead of
 *    code, on the single most likely language in this repo's own audience.
 *  - The result must be something `/raw` would hand over as text. That is the
 *    anti-XSS gate answering the question, rather than a second list beside it,
 *    and it is what makes "a paste can never be stored as a type that gets its
 *    own Content-Type" a fact rather than an intention.
 */
export function pasteMimeType(language: string | null | undefined): string | null {
  const ext = LANG_EXT[(language ?? "").trim().toLowerCase()];
  if (!ext) return null;
  const mime = mimeFromExt(ext);
  if (!mime) return null;
  return isTextPayload(mime) ? normalizeMime(mime) : null;
}

/**
 * The type to reason with for a row.
 *
 * For an ordinary upload this is the stored type, untouched. For a paste it is
 * what its language declares, which also repairs every paste created before
 * this module existed without a migration: those rows carry the placeholder
 * type but the right `language`, and this reads the language.
 *
 * Idempotent, so it is safe to apply more than once along a request.
 */
export function rowMimeType(row: {
  kind?: string | null;
  language?: string | null;
  mimeType: string;
}): string {
  if (row.kind !== "paste") return row.mimeType;
  return pasteMimeType(row.language) ?? row.mimeType;
}
