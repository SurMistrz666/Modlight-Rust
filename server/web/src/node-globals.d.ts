/**
 * Two Node globals that upstream modules shared with the server mention.
 *
 * - `Buffer` appears only in the signatures of lib/media/sniff.ts helpers that
 *   sniff uploaded bytes on the server; the browser never calls them (and, like
 *   upstream's client bundle, has no Buffer at runtime). Declared with exactly the
 *   members those helpers use, so the files type-check unchanged.
 * - `process.env.NEXT_PUBLIC_*` is inlined at build time, as Next does
 *   (vite.config.ts `define`); this declaration only types the reads.
 */
interface Buffer extends Uint8Array {
  readUInt32BE(offset: number): number;
  toString(encoding?: string, start?: number, end?: number): string;
  subarray(start?: number, end?: number): Buffer;
}

declare const process: {
  readonly env: Readonly<Record<string, string | undefined>>;
};
