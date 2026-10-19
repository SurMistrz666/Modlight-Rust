/**
 * "Do I already have this file?" - asked BEFORE the bytes are sent.
 *
 * lib/data/duplicates.ts answers the same question retroactively: it scans a
 * library that already holds the copies and offers to clear them out. That is
 * the wrong end of the problem for the case the owner actually meets, which is
 * the same screenshot uploaded for the fourth time because nothing said it was
 * already there.
 *
 * This module holds the half of the contract with NO database in it: the shape
 * of a checksum, the batch ceiling, and - the part that carries the whole
 * feature's honesty - the rule that turns "we found nothing" into an answer.
 * It is deliberately free of imports so the browser can hold it as well as the
 * server, and so the panel and any desktop client share one definition of what
 * a "duplicate" reply means rather than two that drift.
 *
 * ## Why "not found" is not the same as "you do not have it"
 *
 * `uploads.checksum` is NOT populated for every row:
 *
 *  - rows written before checksums were recorded carry none, and never will unless
 *    something reads their bytes end to end;
 *  - the resumable (tus) path deliberately does not compute one, because
 *    `ingestLocalFile` finishes with a `rename` and hashing would mean an extra
 *    full read of a multi-gigabyte file - the exact cost this product exists to
 *    avoid.
 *
 * So a library can hold the very file being asked about and have nothing to
 * match it against. Reporting that as "new" would be a confident lie, told
 * precisely to the accounts most affected by it. `statusFor` therefore only
 * returns "new" when the absence of a match is a FACT about the whole library,
 * and returns "unknown" otherwise. messages/{en,pl}/profile.json already says
 * this about the retroactive scan; the upload-time wording must agree with it.
 */

/** A stored sha256, lower-case hex, exactly as `uploads.checksum` holds it. */
export const SHA256_HEX = /^[0-9a-f]{64}$/;

/**
 * Checksums one request may ask about.
 *
 * The same 500 the write verbs on /api/uploads use for ids (`MAX_IDS`), and for
 * the same reason: one request must not be able to ask for an unbounded
 * `IN (...)` list. Kept identical rather than tuned separately, because a client
 * that may name 500 files in a move is a client that may hash 500 files in a
 * drop, and two different ceilings for "how many things may one request name"
 * is a number nobody can recall while writing a client.
 */
export const MAX_CHECKSUMS = 500;

export function isSha256(value: unknown): value is string {
  return typeof value === "string" && SHA256_HEX.test(value);
}

export type ChecksumListIssue = "empty" | "malformed" | "too-many";

export type ParsedChecksums =
  | { ok: true; checksums: string[] }
  | { ok: false; issue: ChecksumListIssue };

/**
 * Validate the list a client asked about, BEFORE it can reach a query.
 *
 * The shape check is not decoration. `uploads.checksum` is a varchar, so an
 * arbitrary string would not make Postgres raise the way a malformed uuid does -
 * it would simply be compared, which turns this endpoint into a free-form
 * search over a column it has no business being searched on, at 500 terms a
 * request. Refusing anything that is not a sha256 keeps the query a lookup.
 *
 * Case is normalised down, because that is how every writer of this column
 * stores it (`createHash(...).digest("hex")`), and a client hashing in the
 * browser can easily hand back upper case. Duplicates within one request are
 * collapsed so a drop containing the same file twice does not inflate the count
 * reported back - and so the two copies still both resolve, since results are
 * keyed by checksum rather than by position.
 */
export function parseChecksums(input: unknown): ParsedChecksums {
  if (!Array.isArray(input) || input.length === 0) return { ok: false, issue: "empty" };
  if (input.length > MAX_CHECKSUMS) return { ok: false, issue: "too-many" };

  const checksums: string[] = [];
  const seen = new Set<string>();
  for (const raw of input) {
    if (typeof raw !== "string") return { ok: false, issue: "malformed" };
    const value = raw.trim().toLowerCase();
    if (!SHA256_HEX.test(value)) return { ok: false, issue: "malformed" };
    if (seen.has(value)) continue;
    seen.add(value);
    checksums.push(value);
  }

  if (checksums.length === 0) return { ok: false, issue: "empty" };
  return { ok: true, checksums };
}

/**
 * What the caller is told about one checksum.
 *
 *  - `duplicate` - a live file of yours has exactly these bytes.
 *  - `new`       - no file of yours has them, and that is a fact: every live
 *                  file in the library carries a checksum to compare against.
 *  - `unknown`   - no match was found, but the library holds files this lookup
 *                  cannot see, so nothing can be concluded.
 */
export type CheckStatus = "duplicate" | "new" | "unknown";

/**
 * Decide the status for one checksum.
 *
 * `blind` is true when part of the library was invisible to the lookup: either
 * some live files carry no checksum at all, or the row ceiling truncated the
 * answer. A match always wins - finding one proves the file is there, whatever
 * else could not be seen.
 */
export function statusFor(hasMatch: boolean, blind: boolean): CheckStatus {
  if (hasMatch) return "duplicate";
  return blind ? "unknown" : "new";
}

/** One file the caller already has, in the shape a client can act on. */
export interface CheckMatch {
  id: string;
  code: string;
  filename: string;
  ext: string;
  mimeType: string;
  size: number;
  /** ISO 8601, so this survives JSON. */
  createdAt: string;
  folderId: string | null;
  /** Resolved name of that folder, or null for the loose pile. */
  folderName: string | null;
  /** Public preview page, on the hostname and in the shape this owner uses. */
  url: string;
  /** Direct link to the bytes. */
  rawUrl: string;
  /** Whichever of the two the account's `copy_raw` preference selects. */
  copyUrl: string;
}

/** The answer for one requested checksum. */
export interface CheckResult {
  checksum: string;
  status: CheckStatus;
  /** Oldest first. Empty for anything but `duplicate`. */
  matches: CheckMatch[];
}

/** The whole reply from POST /api/uploads/check. */
export interface CheckResponse {
  message: string;
  /** Distinct checksums actually looked up, after normalisation. */
  requested: number;
  /**
   * Live files of this account carrying no checksum, and therefore invisible to
   * the lookup. Reported rather than hidden: on a server whose library was
   * imported this is nearly all of it, and a client that showed "no
   * duplicates" without it would be making a promise the data cannot keep.
   */
  unchecksummed: number;
  /** The row ceiling bit, so no checksum in this reply can be called `new`. */
  truncated: boolean;
  results: CheckResult[];
}
