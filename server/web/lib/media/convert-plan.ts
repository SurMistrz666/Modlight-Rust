/**
 * Deciding WHAT an in-place conversion would do, before anything is done.
 *
 * Every judgement here is a pure function over a database row, so the whole
 * decision surface - which files a bulk target applies to, which are already in
 * that format, what a conversion does to the quota, which columns are rewritten
 * - is a unit test rather than something discovered on production data.
 *
 * The separation matters for a second reason. lib/media/convert-in-place.ts is
 * the only module that WRITES, and it is deliberately kept small enough to read
 * in one sitting; the arithmetic and the taxonomy live here so that file can be
 * about ordering and failure recovery and nothing else.
 *
 * Browser-safe: no storage, no database, no sharp, no ffmpeg.
 */

import {
  avOptionFor,
  AV_OUTPUT,
  type AvSourceSize,
  type AvTarget,
} from "@/lib/media/av-targets";
import { convertOptionFor, type ConvertTarget } from "@/lib/media/convert-targets";
import { downloadOptionsFor, type DownloadOption } from "@/lib/media/download-menu";
import { effectiveType } from "@/lib/media/mime";
import { swapFilenameExt } from "@/lib/media/paths";

/**
 * `copy` leaves the source untouched and produces a second file with its own
 * code; `replace` rewrites the bytes behind the file that already exists, so its
 * link keeps working and nothing new appears in the dashboard.
 */
export type ConvertMode = "replace" | "copy";

/** Which engine produces the result. Decided exactly as the /convert route does. */
export type ConvertEngine = "image" | "av";

/**
 * A file that will not be converted, and it is not an error.
 *
 * `already-in-format` is the one the reported bug turns on: select five JPEGs
 * and three PNGs, ask for PNG, and three of them have nothing to do. The target
 * tables never OFFER a file the format it already is (SELF_TARGET in
 * convert-targets.ts, AUDIO_SELF in av-targets.ts), so without this check those
 * three would be reported as "cannot be converted", which is both false and
 * alarming.
 */
export type ConvertSkipReason =
  | "already-in-format"
  | "unsupported-target"
  | "paste"
  | "trashed"
  | "expired";

/** A conversion that was attempted and did not happen. */
export type ConvertFailure =
  | "not-found"
  | "forbidden"
  | "unsupported-target"
  | "too-large"
  | "quota"
  | "no-space"
  | "busy"
  | "timeout"
  | "aborted"
  | "conflict"
  | "storage"
  | "encoder";

export type ConvertOutcome =
  | {
      status: "converted";
      uploadId: string;
      code: string;
      ext: string;
      mimeType: string;
      size: number;
      previousSize: number;
      previousExt: string;
      /** Something true about the result nobody asked for ("clipped", "audio-dropped"). */
      note?: string;
    }
  | {
      status: "created";
      uploadId: string;
      sourceId: string;
      code: string;
      ext: string;
      mimeType: string;
      size: number;
      note?: string;
    }
  | { status: "skipped"; uploadId: string; reason: ConvertSkipReason }
  | { status: "failed"; uploadId: string; reason: ConvertFailure };

/** The report a bulk run hands back. Counts first, because that is the sentence. */
export interface ConvertReport {
  to: string;
  mode: ConvertMode;
  converted: number;
  skipped: number;
  failed: number;
  results: ConvertOutcome[];
}

/**
 * The columns any of this needs. Declared structurally so a caller may hand over
 * a narrow `select()` of a thousand rows instead of pulling password hashes and
 * blurhashes into memory to answer "does PNG apply to this?".
 */
export interface ConvertCandidate {
  id: string;
  ext: string | null;
  mimeType: string | null;
  frames: number | null;
  /**
   * Picture size, for the one target that is offered only to files too big for
   * a hardware decoder (WEB_MAX_EDGE in lib/media/av-targets.ts). Null on every
   * row uploaded before the columns existed, and that reads as "do not offer
   * it" rather than as "small".
   */
  width?: number | null;
  height?: number | null;
  size: number;
  kind: string;
  deletedAt: Date | null;
  expiresAt: Date | null;
}

// ---------------------------------------------------------------------------
// What a target produces
// ---------------------------------------------------------------------------

/** MIME and extension the libvips engine writes. Mirrors convert.ts. */
const IMAGE_OUTPUT: Record<ConvertTarget, { mime: string; ext: string }> = {
  pdf: { mime: "application/pdf", ext: "pdf" },
  png: { mime: "image/png", ext: "png" },
  jpg: { mime: "image/jpeg", ext: "jpg" },
  webp: { mime: "image/webp", ext: "webp" },
  avif: { mime: "image/avif", ext: "avif" },
};

/**
 * What comes out of a target, without running it.
 *
 * Both engines can be asked for `webp`, and they write the same MIME and the
 * same extension, so one table answers for both. Which engine actually runs is
 * `conversionEngineFor`'s business.
 */
export function outputFor(to: string): { mime: string; ext: string } | null {
  return (
    IMAGE_OUTPUT[to as ConvertTarget] ?? AV_OUTPUT[to as AvTarget] ?? null
  );
}

// ---------------------------------------------------------------------------
// "It already is that"
// ---------------------------------------------------------------------------

/**
 * Extensions that name the same format under two spellings.
 *
 * Kept deliberately short. Every entry here is a claim that converting between
 * the two would be a no-op, and a wrong entry silently refuses a conversion the
 * user actually wanted.
 */
const EXT_ALIASES: Record<string, string> = {
  jpeg: "jpg",
  jpe: "jpg",
  apng: "png",
};

/**
 * Types that ARE one of our targets. The row's stored MIME is the stronger
 * signal - a file named ".txt" holding a PNG is a PNG - so it is consulted
 * first, with the extension as the fallback for the many rows that carry
 * application/octet-stream.
 */
const MIME_FORMAT: Record<string, string> = {
  "application/pdf": "pdf",
  "image/png": "png",
  "image/apng": "png",
  "image/jpeg": "jpg",
  "image/jpg": "jpg",
  "image/webp": "webp",
  "image/avif": "avif",
  "image/gif": "gif",
  "audio/mpeg": "mp3",
  "audio/mp3": "mp3",
  "audio/ogg": "ogg",
  "audio/vorbis": "ogg",
  "audio/wav": "wav",
  "audio/x-wav": "wav",
  "audio/wave": "wav",
  "video/mp4": "mp4",
  "video/quicktime": "mov",
  "video/x-matroska": "mkv",
  "video/webm": "webm",
};

function normaliseExt(ext: string | null | undefined): string {
  const e = (ext ?? "").replace(/^\./, "").toLowerCase();
  return EXT_ALIASES[e] ?? e;
}

/** The target name this file already answers to, or "" when it is none of them. */
export function currentFormatOf(
  mime: string | null | undefined,
  ext: string | null | undefined,
): string {
  const byMime = MIME_FORMAT[effectiveType(mime, ext)];
  if (byMime) return byMime;
  const e = normaliseExt(ext);
  return outputFor(e) ? e : "";
}

/** Would converting to `to` produce the format this file already is? */
export function isAlreadyInFormat(
  mime: string | null | undefined,
  ext: string | null | undefined,
  to: string,
): boolean {
  const target = normaliseExt(to);
  if (!target) return false;
  return currentFormatOf(mime, ext) === target || normaliseExt(ext) === target;
}

// ---------------------------------------------------------------------------
// Which engine, and is it offered at all
// ---------------------------------------------------------------------------

/**
 * Which engine would answer `to` for this file, or null when nothing would.
 *
 * The order is not incidental: it is the same order the /convert route
 * dispatches in (ffmpeg first), because both engines answer to `webp` and they
 * mean different things by it - all 43 frames of an AVIF sequence to ffmpeg, the
 * cover frame to libvips. Asking the same pair of functions in the same sequence
 * is what keeps "what the menu offered" and "what the server does" identical.
 */
export function conversionEngineFor(
  mime: string | null | undefined,
  ext: string | null | undefined,
  frames: number | null | undefined,
  to: string | null | undefined,
  size?: AvSourceSize | null,
): ConvertEngine | null {
  if (avOptionFor(mime, ext, frames, to, size)) return "av";
  if (convertOptionFor(mime, ext, frames, to)) return "image";
  return null;
}

/**
 * Everything this file can be converted INTO, in menu order.
 *
 * The very list the "Download as…" menu renders (`canEncode: true`, because an
 * in-place conversion is only ever run by the owner or an admin), so the convert
 * submenu cannot offer a target the writer would then refuse.
 */
export function convertTargetsForRow(row: {
  mimeType: string | null;
  ext: string | null;
  frames: number | null;
  width?: number | null;
  height?: number | null;
}): DownloadOption[] {
  return downloadOptionsFor(row.mimeType, row.ext, row.frames, {
    canEncode: true,
    width: row.width,
    height: row.height,
  });
}

// ---------------------------------------------------------------------------
// The per-file decision
// ---------------------------------------------------------------------------

export type ConvertPlan =
  | { ok: true; engine: ConvertEngine; ext: string; mime: string }
  | { ok: false; reason: ConvertSkipReason };

/**
 * May this row be converted to `to`, and by which engine?
 *
 * Cheapest and most specific first, because the ORDER is what the user reads in
 * the report. A trashed file is not "unsupported", and a file that is already a
 * PNG is not "unsupported" either - collapsing all three into one refusal is
 * what makes a bulk result unreadable.
 */
export function planConversion(
  row: ConvertCandidate,
  to: string,
  mode: ConvertMode,
  now: Date = new Date(),
): ConvertPlan {
  if (row.deletedAt) return { ok: false, reason: "trashed" };

  // A paste's bytes ARE its text: replacing them with a PDF leaves a row that
  // still claims kind='paste' and renders as source code. A copy is fine, and
  // lands as an ordinary file.
  if (row.kind === "paste" && mode === "replace") return { ok: false, reason: "paste" };

  if (row.expiresAt && row.expiresAt.getTime() <= now.getTime()) {
    return { ok: false, reason: "expired" };
  }

  if (isAlreadyInFormat(row.mimeType, row.ext, to)) {
    return { ok: false, reason: "already-in-format" };
  }

  const engine = conversionEngineFor(row.mimeType, row.ext, row.frames, to, row);
  const out = outputFor(to);
  if (!engine || !out) return { ok: false, reason: "unsupported-target" };

  return { ok: true, engine, ext: out.ext, mime: out.mime };
}

export interface BulkPlan {
  to: string;
  mode: ConvertMode;
  convert: { id: string; engine: ConvertEngine }[];
  skipped: { id: string; reason: ConvertSkipReason }[];
}

/**
 * Split a selection into "these will change" and "these will not, and why".
 *
 * The answer to the mixed-selection report: five JPEGs and three PNGs asked for
 * PNG is five conversions and three `already-in-format`, never eight attempts
 * and three failures.
 */
export function planBulkConversion(
  rows: ConvertCandidate[],
  to: string,
  mode: ConvertMode,
  now: Date = new Date(),
): BulkPlan {
  const convert: { id: string; engine: ConvertEngine }[] = [];
  const skipped: { id: string; reason: ConvertSkipReason }[] = [];

  for (const row of rows) {
    const plan = planConversion(row, to, mode, now);
    if (plan.ok) convert.push({ id: row.id, engine: plan.engine });
    else skipped.push({ id: row.id, reason: plan.reason });
  }

  return { to, mode, convert, skipped };
}

export interface TargetSummary {
  to: string;
  /**
   * The engine the FIRST file offering this target would use, for labelling
   * only. Both engines answer to `webp`, so a selection holding a PNG and an
   * AVIF sequence really does convert through two different ones; which runs for
   * a given file is decided per file, by `conversionEngineFor`.
   */
  engine: ConvertEngine;
  /** Files the conversion would actually run on. */
  eligible: number;
  /** Files already in that format - nothing to do, and not a problem. */
  alreadyInFormat: number;
  /** Files whose format cannot go there, plus anything trashed or expired. */
  unsupported: number;
}

/**
 * Which targets to offer for a MIXED selection, and how many files each covers.
 *
 * A target earns its place if it applies to at least one selected file, and it
 * carries its own counts so the menu can say "PNG - 5 of 8" before anything is
 * clicked rather than reporting the shortfall afterwards. Ordered by reach, then
 * by the order the single-file menu uses, so the entry that does the most work
 * is the one under the cursor.
 */
export function bulkTargetSummary(
  rows: ConvertCandidate[],
  mode: ConvertMode,
  now: Date = new Date(),
): TargetSummary[] {
  const order: string[] = [];
  const seen = new Map<string, TargetSummary>();

  const touch = (to: string, engine: ConvertEngine): TargetSummary => {
    let entry = seen.get(to);
    if (!entry) {
      entry = { to, engine, eligible: 0, alreadyInFormat: 0, unsupported: 0 };
      seen.set(to, entry);
      order.push(to);
    }
    return entry;
  };

  // Every target ANY selected file offers, so the menu is the union rather than
  // the intersection - the intersection of a mixed selection is usually empty,
  // which would be a menu that does nothing.
  for (const row of rows) {
    for (const option of convertTargetsForRow(row)) {
      touch(option.to, option.engine);
    }
  }

  for (const row of rows) {
    for (const to of order) {
      const entry = seen.get(to)!;
      const plan = planConversion(row, to, mode, now);
      if (plan.ok) entry.eligible++;
      else if (plan.reason === "already-in-format") entry.alreadyInFormat++;
      else entry.unsupported++;
    }
  }

  return order
    .map((to) => seen.get(to)!)
    .filter((entry) => entry.eligible > 0)
    .sort((a, b) => b.eligible - a.eligible || order.indexOf(a.to) - order.indexOf(b.to));
}

// ---------------------------------------------------------------------------
// Quota
// ---------------------------------------------------------------------------

/** The quota-relevant slice of a user row. Same conventions as the upload path. */
export interface QuotaHolder {
  usedQuota?: number | null;
  maxQuota?: number | null;
}

/**
 * Bytes a conversion adds to the owner's usage.
 *
 * A copy is a second file, so it costs its whole size. A replace hands the old
 * bytes back, so it costs the DIFFERENCE - which is negative far more often than
 * not, because most conversions shrink the file.
 */
export function quotaDeltaFor(mode: ConvertMode, oldSize: number, newSize: number): number {
  return mode === "copy" ? newSize : newSize - oldSize;
}

/**
 * May this delta be charged?
 *
 * `maxQuota` null or negative is unlimited, the convention every account
 * carries. A delta of zero or less is ALWAYS allowed, and
 * that is a deliberate asymmetry: refusing to shrink a file because the account
 * is already over its limit (an admin lowered it, say) would trap the owner in
 * the state the limit exists to get them out of.
 */
export function quotaAllowsDelta(owner: QuotaHolder, delta: number): boolean {
  if (delta <= 0) return true;
  const max = owner.maxQuota ?? -1;
  if (max < 0) return true;
  return (owner.usedQuota ?? 0) + delta <= max;
}

// ---------------------------------------------------------------------------
// What the row becomes
// ---------------------------------------------------------------------------

/** Everything an in-place conversion learns about the new bytes. */
export interface ConvertedBytes {
  storageKey: string;
  size: number;
  checksum: string;
  mime: string;
  ext: string;
  width: number | null;
  height: number | null;
  frames: number | null;
  /**
   * Playing time and frame rate of the RESULT, in the same columns an upload
   * fills (lib/media/av-meta.ts). They are listed here for the same reason
   * `width` is: the patch below writes them unconditionally, so a stale value
   * cannot survive a conversion. An MP3 extracted from a 30 fps video is not a
   * 30 fps anything, and a GIF cut from a two minute recording is not two
   * minutes long.
   */
  durationSeconds: number | null;
  fps: number | null;
  blurhash: string | null;
}

/**
 * The exact column patch an in-place conversion applies.
 *
 * Its most important property is what it does NOT contain. `code` stays, so
 * every link already shared keeps resolving - the extension in a URL is
 * decorative (lib/media/urls.ts) and the code belongs to this row forever, which
 * is also why no alias table is needed and why the code can never be handed to a
 * different file. `delete_token` stays, so the DeletionURL in an old ShareX
 * config keeps working. The counters, the folder, the published flag and the
 * whole access gate (password, expiry, view cap) stay, because they describe the
 * LINK and the link has not changed - it is still the same file, in a different
 * format.
 *
 * `thumbKey` is cleared and `blurhash` recomputed for the opposite reason: those
 * describe the BYTES, and the bytes are new.
 */
export function conversionRowPatch(
  before: { ext: string | null; filename: string; originalExt: string | null },
  bytes: ConvertedBytes,
  now: Date = new Date(),
) {
  return {
    ext: bytes.ext,
    mimeType: bytes.mime,
    size: bytes.size,
    storageKey: bytes.storageKey,
    checksum: bytes.checksum,
    width: bytes.width,
    height: bytes.height,
    frames: bytes.frames,
    durationSeconds: bytes.durationSeconds,
    fps: bytes.fps,
    blurhash: bytes.blurhash,
    // A thumbnail of the old bytes is a picture of a file that no longer exists.
    thumbKey: null as string | null,
    filename: swapFilenameExt(before.filename, bytes.ext),
    convertedAt: now,
    // Only the FIRST conversion records where the file came from; converting a
    // PNG to WebP and then to AVIF still started as a PNG.
    originalExt: before.originalExt ?? before.ext ?? null,
    updatedAt: now,
  };
}

export { swapFilenameExt, swapKeyExt } from "@/lib/media/paths";
