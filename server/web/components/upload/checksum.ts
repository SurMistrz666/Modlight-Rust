/**
 * sha256 in the browser, and the rules for when it is worth paying for.
 *
 * ## Why there is a threshold at all
 *
 * `crypto.subtle.digest` has no streaming form. Hashing a file means holding
 * the whole thing in an ArrayBuffer first, so the cost is not just CPU time, it
 * is a full copy of the file in the tab's memory. That makes the trade obvious
 * at the two ends and only debatable in the middle:
 *
 *  - A screenshot is 0.2-5 MB. Reading and hashing it is single-digit
 *    milliseconds, and it is EXACTLY the file that gets uploaded four times
 *    because nothing said it was already there. Hashing it is free money.
 *  - A 6 GB screen recording cannot be hashed at all without a 6 GB allocation,
 *    and even if it could, the user would wait a minute staring at a dropzone
 *    before a single byte moved - on the one product whose entire promise is
 *    that big files start immediately and survive a bad line. Nobody
 *    accidentally re-drops a 6 GB capture; they do accidentally re-drop a
 *    screenshot.
 *
 * ## The numbers, and where they come from
 *
 * `HASH_MAX_FILE_BYTES` is 64 MiB, which is not a fresh guess: it is the ceiling
 * `lib/upload/finalize.ts` already applies before reading an upload back into
 * memory for sharp. The panel therefore has ONE answer to "how large a file may
 * we hold in memory at once" rather than two that drift apart. At the ~200-500
 * MB/s WebCrypto manages on current hardware that is at most a couple of hundred
 * milliseconds per file, spent while Uppy is still setting up, and it covers
 * images, documents and short clips - everything that actually gets re-sent.
 *
 * `HASH_MAX_BATCH_BYTES` is the ceiling on ONE drop. Without it, dropping a
 * folder of 400 medium files would be 400 x 64 MiB of reading before the first
 * chunk left, which turns a bounded per-file cost into an unbounded batch cost.
 * At 512 MiB the worst case is a second or two, and everything past the budget
 * is reported as "not checked" rather than as "not a duplicate".
 *
 * Anything skipped for either reason is `unknown`, never `new`. That is the same
 * rule the server applies to rows with no stored checksum - see
 * lib/upload/duplicate-check.ts.
 */

/** Largest single file hashed before uploading. See the module comment. */
export const HASH_MAX_FILE_BYTES = 64 * 1024 * 1024;

/** Bytes one drop may spend on hashing before the rest is left unchecked. */
export const HASH_MAX_BATCH_BYTES = 512 * 1024 * 1024;

/**
 * Is WebCrypto available at all?
 *
 * `crypto.subtle` is undefined outside a secure context, so a panel reached over
 * plain http on a LAN address has no digest. That is a reason to skip the check
 * silently, never a reason to fail an upload: the answer becomes "unknown" for
 * everything, which is exactly what it is.
 */
export function canHash(): boolean {
  return typeof globalThis.crypto?.subtle?.digest === "function";
}

/** Whether this file is small enough to be worth hashing on its own. */
export function isHashable(size: number): boolean {
  return size > 0 && size <= HASH_MAX_FILE_BYTES;
}

const HEX = Array.from({ length: 256 }, (_, i) => i.toString(16).padStart(2, "0"));

/**
 * sha256 of a Blob, lower-case hex.
 *
 * Lower case because that is what `createHash(...).digest("hex")` writes into
 * `uploads.checksum` on every server path, and the comparison is a string
 * equality in Postgres.
 */
export async function sha256Hex(blob: Blob): Promise<string> {
  const buffer = await blob.arrayBuffer();
  const digest = await crypto.subtle.digest("SHA-256", buffer);
  const bytes = new Uint8Array(digest);
  let out = "";
  for (let i = 0; i < bytes.length; i++) out += HEX[bytes[i]];
  return out;
}
