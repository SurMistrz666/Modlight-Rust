/**
 * The bridge between the MLENC1 container and tus resumable upload.
 *
 * ## The problem this solves
 *
 * The whole product exists so a 5 GB file survives a bad line, and that is tus:
 * the browser sends the file as a series of PATCHes and, when the connection
 * dies, asks the server what offset it reached and carries on from there. An
 * encrypted upload must keep every one of those properties, which rules out the
 * obvious implementation - encrypt the file, upload the result - twice over: it
 * needs the whole plaintext and the whole ciphertext in memory at once, and it
 * throws all of that away if the tab reloads.
 *
 * tus-js-client (which Uppy drives) has exactly the seam needed: `fileReader`.
 * Instead of slicing the File itself, it asks a source for the bytes at
 * `[start, end)` of "the upload". So the upload becomes the CONTAINER, the
 * source hands out container bytes, and every resumable property is inherited
 * for free - offsets, retries, pause and resume, progress, the concurrency
 * limit, the whole lot. Nothing about the tus protocol changes, and the server
 * needs no idea that any of this is happening.
 *
 * ## Why arbitrary ranges are answerable at all
 *
 * Because the container's geometry is fixed the moment the header is built:
 * frame k starts at `headerBytes + (k-1) * (chunk + 16)` and is sealed with an
 * IV derived from k. So container offset -> frame index -> "re-encrypt exactly
 * that frame and take the slice you asked for". A resume from an offset in the
 * middle of a frame is therefore just as cheap as one on a boundary, which
 * matters: a PATCH killed mid-body leaves the server holding a partial frame,
 * and that is the ordinary case on a bad line, not the exotic one.
 *
 * ## Memory
 *
 * Two frames of ciphertext, and one plaintext slice while it is being sealed.
 * Two, not one, because the header shifts everything: with tus asking for one
 * frame-width per PATCH, every request straddles two frames, so a one-entry
 * cache would re-encrypt every frame twice. At the 8 MiB default that is a
 * ~32 MiB working set for a file of any size.
 *
 * ## What is deliberately not here
 *
 * No duplicate pre-flight. The unencrypted uploader hashes small files and asks
 * the server "do you already have these bytes?", which is a fine trade there and
 * an unacceptable one here: it would hand the server a plaintext checksum, i.e.
 * a confirmation oracle for any file it already knows. An encrypted upload tells
 * the server nothing about its contents, including whether they are familiar.
 */

import {
  buildEnvelope,
  clampName,
  encryptFrame,
  frameIndexAt,
  frameOffset,
  generateKeyBytes,
  importKey,
  plainBytesInFrame,
  type EnvelopeMetadata,
  type EnvelopePlan,
  DEFAULT_PLAIN_CHUNK_BYTES,
  ENCRYPTED_EXT,
  ENCRYPTED_MIME,
  GCM_TAG_BYTES,
} from "@/lib/crypto/envelope";
import { encodeKey } from "@/lib/crypto/link";

/** One PATCH carries one frame. See the module comment on the 2-frame cache. */
export const ENCRYPTED_TUS_CHUNK_BYTES = DEFAULT_PLAIN_CHUNK_BYTES + GCM_TAG_BYTES;

/** Characters used for the opaque stored name. Unambiguous, lower case. */
const NAME_ALPHABET = "abcdefghijkmnpqrstuvwxyz23456789";

/**
 * The name an encrypted upload is STORED under.
 *
 * Random, because the real one is inside the container and the point is that it
 * never reaches us. It is also what the owner sees in their own file list, which
 * is a genuine cost of encrypting the filename and is called out in the UI.
 */
export function opaqueStoredName(): string {
  const raw = new Uint8Array(10);
  globalThis.crypto.getRandomValues(raw);
  let out = "";
  for (const byte of raw) out += NAME_ALPHABET[byte % NAME_ALPHABET.length];
  return `${out}.${ENCRYPTED_EXT}`;
}

/** Everything the uploader needs to know about one file it is encrypting. */
export interface EncryptionPlan {
  /** The key, base64url, for the link fragment. Never sent anywhere. */
  encodedKey: string;
  key: CryptoKey;
  plan: EnvelopePlan;
  /** Name the server will store and show in the owner's list. */
  storedName: string;
  /** Size of the container: what tus is told, and what counts against quota. */
  cipherTotalBytes: number;
  /** The real name, kept in the tab so the results list can show it. */
  plainName: string;
  plainSize: number;
}

/** Build the container header and the key for one file, before anything moves. */
export async function planEncryption(
  file: { name: string; type: string; size: number },
  options: { chunkBytes?: number } = {},
): Promise<EncryptionPlan> {
  const keyBytes = generateKeyBytes();
  const key = await importKey(keyBytes);
  const metadata: EnvelopeMetadata = {
    name: file.name || "file",
    type: file.type || "application/octet-stream",
  };
  const plan = await buildEnvelope({
    key,
    metadata,
    plainTotalBytes: file.size,
    plainChunkBytes: options.chunkBytes,
  });

  const encodedKey = encodeKey(keyBytes);
  // The raw key has served its purpose. Zeroing it is not a guarantee (the
  // string above is immutable and the GC decides when it dies) but it costs
  // nothing and removes the one copy we can actually reach.
  keyBytes.fill(0);

  return {
    encodedKey,
    key,
    plan,
    storedName: opaqueStoredName(),
    cipherTotalBytes: plan.cipherTotalBytes,
    // The CLAMPED name, so what this page shows the uploader is the same name
    // the recipient will see. `buildEnvelope` applies the same clamp to what it
    // seals, and a very long name is the one case where the two could differ.
    plainName: clampName(metadata.name),
    plainSize: file.size,
  };
}

/** The shape tus-js-client expects back from `fileReader.openFile`. */
interface TusFileSource {
  size: number;
  slice(start: number, end: number): Promise<{ value: Blob | null; done: boolean }>;
  close(): void;
}

/** Two-entry cache of sealed frames, keyed by frame index. */
class FrameCache {
  private entries = new Map<number, Uint8Array>();

  constructor(private readonly limit = 2) {}

  get(index: number): Uint8Array | undefined {
    const hit = this.entries.get(index);
    if (hit) {
      // Re-insert so the map's iteration order is least-recent-first.
      this.entries.delete(index);
      this.entries.set(index, hit);
    }
    return hit;
  }

  set(index: number, bytes: Uint8Array): void {
    this.entries.set(index, bytes);
    while (this.entries.size > this.limit) {
      const oldest = this.entries.keys().next();
      if (oldest.done) break;
      this.entries.delete(oldest.value);
    }
  }

  clear(): void {
    this.entries.clear();
  }
}

/**
 * A container-shaped view of a File, for tus-js-client.
 *
 * Every method is pure with respect to the upload: asking for the same range
 * twice produces the same bytes, which is what makes a retry safe.
 */
export function encryptedSource(file: Blob, plan: EncryptionPlan): TusFileSource {
  const { key } = plan;
  const params = plan.plan;
  const cache = new FrameCache();

  async function frame(index: number): Promise<Uint8Array> {
    const cached = cache.get(index);
    if (cached) return cached;

    const start = (index - 1) * params.plainChunkBytes;
    const length = plainBytesInFrame(params, index);
    const plain = new Uint8Array(await file.slice(start, start + length).arrayBuffer());
    if (plain.byteLength !== length) {
      // The File changed on disk mid-upload (renamed, truncated, a synced
      // folder). Continuing would produce a container that decrypts to garbage
      // at exactly one frame, which is far worse than a failed upload.
      throw new Error("The file changed while it was being uploaded.");
    }
    const sealed = await encryptFrame(key, params, index, plain);
    plain.fill(0);
    cache.set(index, sealed);
    return sealed;
  }

  return {
    size: params.cipherTotalBytes,

    async slice(start: number, end: number) {
      const from = Math.max(0, Math.min(start, params.cipherTotalBytes));
      const to = Math.max(from, Math.min(end, params.cipherTotalBytes));
      const out = new Uint8Array(to - from);

      let cursor = from;
      while (cursor < to) {
        if (cursor < params.headerBytes) {
          const take = Math.min(params.headerBytes, to) - cursor;
          out.set(params.header.subarray(cursor, cursor + take), cursor - from);
          cursor += take;
          continue;
        }
        const index = frameIndexAt(params, cursor);
        if (index < 1) break; // empty file: nothing after the header
        const base = frameOffset(params, index);
        const bytes = await frame(index);
        const inner = cursor - base;
        const take = Math.min(bytes.byteLength - inner, to - cursor);
        if (take <= 0) break;
        out.set(bytes.subarray(inner, inner + take), cursor - from);
        cursor += take;
      }

      return { value: new Blob([out]), done: to >= params.cipherTotalBytes };
    },

    close() {
      cache.clear();
    },
  };
}

/**
 * tus-js-client `fileReader` that encrypts on the way out.
 *
 * `openFile` is handed the same `File` object Uppy holds, so the plan is looked
 * up by object identity. A WeakMap rather than a Map keyed by Uppy's file id:
 * the reader is called by tus, which knows nothing about Uppy ids, and a plan
 * that outlived its File would keep a key alive for no reason.
 */
export function encryptingFileReader(plans: WeakMap<Blob, EncryptionPlan>) {
  return {
    async openFile(input: Blob): Promise<TusFileSource> {
      const plan = plans.get(input);
      // Refusing beats uploading: a file with no plan would go up in the clear
      // under a name that says it is encrypted, which is the worst outcome
      // available here.
      if (!plan) throw new Error("This file has no encryption plan; refusing to upload it.");
      return encryptedSource(input, plan);
    },
  };
}

/** Metadata the SERVER is allowed to see for an encrypted upload. */
export function serverVisibleMeta(plan: EncryptionPlan): { name: string; type: string } {
  return { name: plan.storedName, type: ENCRYPTED_MIME };
}
