"use client";

import Uppy, { type Meta } from "@uppy/core";
import Tus, { type TusBody } from "@uppy/tus";

/**
 * The single place the browser-side upload engine is configured.
 *
 * Both surfaces that can start a transfer — the Uppy Dashboard on
 * /dashboard/upload and the app-wide drop zone in the shell — share ONE Uppy
 * instance created here. That is deliberate: two engines would mean two queues,
 * two progress models and two sets of tus fingerprints, so a file dropped on the
 * dashboard would be invisible to the floating transfer window.
 *
 * This module is loaded lazily (`await import(...)`) by the transfers provider,
 * which keeps Uppy and tus-js-client out of the bundle of every panel page that
 * never uploads anything.
 */

/**
 * Default chunk size, mirroring `config.uploads.tusChunkSize`. Chunking is a
 * purely client-side decision, so the server value reaches us either as the
 * `chunkSize` argument (a Server Component can read `config` and pass it down)
 * or via NEXT_PUBLIC_TUS_CHUNK_SIZE — `TUS_CHUNK_SIZE` alone is invisible here.
 */
export const DEFAULT_CHUNK_SIZE = 50 * 1024 * 1024;

/**
 * Uppy defaults to 20 files in flight. On the kind of connection this product
 * exists for that means 20 streams fighting over one thin uplink: every chunk
 * crawls, several time out at once, and the whole batch thrashes. Three keeps
 * the pipe busy while leaving each chunk enough bandwidth to actually land.
 */
export const CONCURRENT_UPLOADS = 3;

/**
 * Retry schedule, in ms between attempts. The sum (~2 minutes) is the real
 * number that matters: it is how long a transfer survives a dead connection —
 * a lift ride, a train tunnel, a router reboot — before Uppy gives up. tus
 * resumes from the last confirmed offset, so waiting is nearly free.
 */
export const RETRY_DELAYS = [0, 1_000, 3_000, 5_000, 10_000, 20_000, 30_000, 60_000];

/** Uppy metadata we add on top of the fields Uppy fills in itself. */
export type ModlightMeta = Meta & {
  /** Target folder id, `ROOT_FOLDER_META`, or absent for the account default. */
  folderId?: string;
};

export type ModlightUppy = Uppy<ModlightMeta, TusBody>;

function envChunkSize(): number | undefined {
  const raw = Number(process.env.NEXT_PUBLIC_TUS_CHUNK_SIZE);
  return Number.isFinite(raw) && raw > 0 ? raw : undefined;
}

/**
 * Build the app's Uppy instance. `autoProceed` stays false so the Dashboard
 * keeps its explicit "Upload" button; the drop zone calls `upload()` itself
 * right after adding files, which is what makes a desktop drag feel like a
 * file move rather than a two-step form.
 */
export function createUppy(chunkSize?: number): ModlightUppy {
  return new Uppy<ModlightMeta, TusBody>({
    autoProceed: false,
    restrictions: { maxFileSize: null },
  }).use(Tus, {
    endpoint: "/api/files",
    chunkSize: chunkSize ?? envChunkSize() ?? DEFAULT_CHUNK_SIZE,
    limit: CONCURRENT_UPLOADS,
    retryDelays: RETRY_DELAYS,
    withCredentials: true,
    removeFingerprintOnSuccess: true,
  });
}
