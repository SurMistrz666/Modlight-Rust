import { MAX_CHECKSUMS, type CheckMatch, type CheckResponse } from "@/lib/upload/duplicate-check";
import { canHash, isHashable, sha256Hex, HASH_MAX_BATCH_BYTES } from "@/components/upload/checksum";

/**
 * "Do I already have this?", asked from the browser before the upload starts.
 *
 * Hash what is cheap to hash (see components/upload/checksum.ts for the
 * thresholds and why they are where they are), ask the server in one request,
 * and hand back the files that already exist. Everything else - the decision,
 * the dialog, removing files from the queue - belongs to the caller.
 *
 * ## This must never be able to stop an upload
 *
 * Every failure path returns "no hits". A digest that is unavailable, a request
 * that 500s, a network that drops, a body that does not parse: none of them are
 * evidence that a file is new, and none of them are a reason to refuse to send
 * it. The worst outcome this function may produce is the behaviour the panel had
 * before it existed. That is why there is no `throw` anywhere below and why the
 * fetch carries its own abort timer - a preflight that hangs would hang the
 * upload behind it, which is a far worse bug than the one it fixes.
 */

/** A file about to be uploaded, in the shape Uppy holds it. */
export interface PreflightInput {
  /** Uppy's file id, which is how the caller maps a hit back to its file. */
  id: string;
  name: string;
  size: number;
  data: Blob;
}

/** A file the caller already has a live copy of. */
export interface PreflightHit {
  id: string;
  name: string;
  size: number;
  checksum: string;
  /** The oldest existing copy: the one worth linking to. */
  existing: CheckMatch;
  /** Live copies in total, when the library holds more than one. */
  copies: number;
}

export interface PreflightOutcome {
  hits: PreflightHit[];
  /** Files whose bytes were actually compared. */
  checked: number;
  /**
   * Files nothing could be said about: too large to hash, past the batch
   * budget, or answered `unknown` because the library holds files with no
   * stored checksum. Counted so the caller can be honest about coverage rather
   * than implying the whole drop was cleared.
   */
  unchecked: number;
}

const EMPTY: PreflightOutcome = { hits: [], checked: 0, unchecked: 0 };

/**
 * How long the panel waits for an answer before giving up and uploading.
 *
 * The check is an optimisation; the upload is the job. Ten seconds is long
 * enough for one indexed lookup on any connection worth uploading over, and
 * short enough that a stalled request is a two-line pause rather than a dead
 * dropzone.
 */
const REQUEST_TIMEOUT_MS = 10_000;

export async function preflightDuplicates(
  files: readonly PreflightInput[],
): Promise<PreflightOutcome> {
  if (files.length === 0) return EMPTY;
  if (!canHash()) return { ...EMPTY, unchecked: files.length };

  /* -- pick what is worth hashing ----------------------------------------- */

  const eligible: PreflightInput[] = [];
  let budget = HASH_MAX_BATCH_BYTES;
  for (const file of files) {
    // Ordered as the user dropped them, and the budget is spent in that order:
    // the first files in a drop are the ones whose result is looked at first.
    if (eligible.length >= MAX_CHECKSUMS) break;
    if (!isHashable(file.size) || file.size > budget) continue;
    budget -= file.size;
    eligible.push(file);
  }
  if (eligible.length === 0) return { ...EMPTY, unchecked: files.length };

  /* -- hash, one at a time ------------------------------------------------ */

  // Sequential on purpose. Each digest holds a whole file in memory, so hashing
  // twenty in parallel would peak at twenty copies for no gain: this is
  // CPU-and-disk bound, not latency bound.
  const byFile: { file: PreflightInput; checksum: string }[] = [];
  for (const file of eligible) {
    try {
      byFile.push({ file, checksum: await sha256Hex(file.data) });
    } catch {
      // A file the browser cannot read (moved or unmounted since the drop) is
      // simply not checked. The upload will produce its own, better error.
    }
  }
  if (byFile.length === 0) return { ...EMPTY, unchecked: files.length };

  /* -- ask ---------------------------------------------------------------- */

  const distinct = [...new Set(byFile.map((entry) => entry.checksum))];
  const answer = await ask(distinct);
  if (!answer) return { ...EMPTY, unchecked: files.length };

  const results = new Map(answer.results.map((r) => [r.checksum, r] as const));

  const hits: PreflightHit[] = [];
  let checked = 0;
  for (const { file, checksum } of byFile) {
    const result = results.get(checksum);
    if (!result || result.status === "unknown") continue;
    checked++;
    // `matches` is oldest first, so [0] is the original.
    const existing = result.matches[0];
    if (result.status !== "duplicate" || !existing) continue;
    hits.push({
      id: file.id,
      name: file.name,
      size: file.size,
      checksum,
      existing,
      copies: result.matches.length,
    });
  }

  return { hits, checked, unchecked: files.length - checked };
}

/** One POST, with its own deadline. Returns null for every kind of failure. */
async function ask(checksums: string[]): Promise<CheckResponse | null> {
  const abort = new AbortController();
  const timer = setTimeout(() => abort.abort(), REQUEST_TIMEOUT_MS);
  try {
    const res = await fetch("/api/uploads/check", {
      method: "POST",
      // Not decoration: the route refuses anything else, because
      // `application/json` is not a CORS-safelisted content type and is
      // therefore what stops a cross-origin POST riding the session cookie.
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ checksums }),
      // The panel authenticates with a cookie; a desktop client sends a token.
      credentials: "same-origin",
      signal: abort.signal,
    });
    if (!res.ok) return null;
    return (await res.json()) as CheckResponse;
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}
