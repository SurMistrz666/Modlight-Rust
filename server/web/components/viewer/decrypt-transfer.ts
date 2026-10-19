/**
 * Fetching an encrypted container and turning it back into a file, in the tab.
 *
 * ## The two requests, and why there are exactly two
 *
 * 1. `Range: bytes=0-4095` - the container header. This is the request that
 *    COUNTS as a view, and it is the only one that does: `/raw` counts a request
 *    whose range starts at zero and nothing else (see the `isInitialRequest`
 *    rule in app/[userCode]/[mediaCode]/[action]/route.ts). Four kilobytes is
 *    enough by construction, because the encrypted metadata frame has a ceiling
 *    that keeps every header inside it.
 * 2. `Range: bytes=4096-` - the rest. Deliberately not starting at zero, so it
 *    does not count a second view. Without that, opening an encrypted file would
 *    spend two views, and a burn-after-view link would 404 itself halfway
 *    through being read for the first time.
 *
 * The preview page does not count a view for an encrypted file precisely
 * because of this: the page on its own shows nothing but a spinner, so the read
 * that matters is the one made here.
 *
 * ## Memory
 *
 * The body is consumed as a stream and decrypted a frame at a time, so the
 * working set is one frame (8 MiB by default) plus whatever the caller keeps.
 * A save through the File System Access API keeps nothing at all, which is what
 * makes a multi-gigabyte file recoverable in a browser; a preview keeps the
 * whole plaintext, which is why the preview has a size ceiling and the save does
 * not (components/viewer/decrypted-presentation.ts).
 *
 * ## The key
 *
 * It arrives as a `CryptoKey` that was imported from the fragment and is never
 * an argument to anything that builds a URL. `assertNoKeyInUrl` is the tripwire
 * for the refactor that forgets that.
 */

import {
  decryptFrame,
  frameCipherBytes,
  openEnvelope,
  EnvelopeError,
  HEADER_PROBE_BYTES,
  type EnvelopeMetadata,
  type EnvelopeParams,
} from "@/lib/crypto/envelope";
import { urlLeaksKey } from "@/lib/crypto/link";

/** What one ranged read of the raw route gave us. */
export interface ContainerHead {
  /** Bytes 0 .. min(HEADER_PROBE_BYTES, size) of the container. */
  head: Uint8Array;
  /** True when that single read was the entire file. */
  complete: boolean;
}

/** Refuse to request a URL that carries the key. See lib/crypto/link.ts. */
export function assertNoKeyInUrl(url: string, encodedKey: string): void {
  if (urlLeaksKey(url, encodedKey)) {
    throw new Error("Refusing to request a URL that carries the decryption key.");
  }
}

/** Thrown when the file is gone, gated, or its view budget is spent. */
export class TransferError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
    this.name = "TransferError";
  }
}

async function ranged(
  url: string,
  range: string,
  signal?: AbortSignal,
): Promise<Response> {
  const res = await fetch(url, { headers: { Range: range }, signal, cache: "no-store" });
  if (!res.ok) throw new TransferError(`Request failed with ${res.status}.`, res.status);
  return res;
}

/** Read just enough of the container to open it. Spends the one counted view. */
export async function fetchContainerHead(
  url: string,
  signal?: AbortSignal,
): Promise<ContainerHead> {
  const res = await ranged(url, `bytes=0-${HEADER_PROBE_BYTES - 1}`, signal);
  const head = new Uint8Array(await res.arrayBuffer());
  // 206 with a Content-Range whose total equals what we got, or a 200 shorter
  // than the probe, both mean "that was the whole file".
  const total = totalFromContentRange(res.headers.get("content-range"));
  const complete = total != null ? head.byteLength >= total : head.byteLength < HEADER_PROBE_BYTES;
  return { head, complete };
}

function totalFromContentRange(header: string | null): number | null {
  if (!header) return null;
  const m = /\/(\d+)\s*$/.exec(header.trim());
  if (!m) return null;
  const n = Number(m[1]);
  return Number.isFinite(n) ? n : null;
}

/** Parse the header and decrypt the metadata frame. */
export async function openContainer(
  head: Uint8Array,
  key: CryptoKey,
): Promise<{ params: EnvelopeParams; metadata: EnvelopeMetadata }> {
  return openEnvelope(head, key);
}

/** A queue of ciphertext bytes fed from the head plus the streamed remainder. */
class ByteQueue {
  private parts: Uint8Array[] = [];
  private size = 0;

  push(chunk: Uint8Array): void {
    if (chunk.byteLength === 0) return;
    this.parts.push(chunk);
    this.size += chunk.byteLength;
  }

  get length(): number {
    return this.size;
  }

  /** Remove and return exactly `n` bytes. Caller must check `length` first. */
  take(n: number): Uint8Array {
    const out = new Uint8Array(n);
    let filled = 0;
    while (filled < n) {
      const part = this.parts[0];
      const need = n - filled;
      if (part.byteLength <= need) {
        out.set(part, filled);
        filled += part.byteLength;
        this.parts.shift();
      } else {
        out.set(part.subarray(0, need), filled);
        this.parts[0] = part.subarray(need);
        filled = n;
      }
    }
    this.size -= n;
    return out;
  }
}

export interface DecryptOptions {
  url: string;
  key: CryptoKey;
  params: EnvelopeParams;
  head: Uint8Array;
  /** True when `head` already holds the whole container. */
  complete: boolean;
  signal?: AbortSignal;
  /** Called with plaintext bytes recovered so far. */
  onProgress?: (done: number, total: number) => void;
}

/**
 * Plaintext frames, in order.
 *
 * An async generator rather than a callback because the two consumers want
 * opposite things: the preview collects every frame, the save writes each one
 * away and forgets it. Both get bounded memory out of the same code.
 */
export async function* decryptedFrames(opts: DecryptOptions): AsyncGenerator<Uint8Array> {
  const { params, key, head } = opts;
  const queue = new ByteQueue();

  // Everything the probe already gave us beyond the header.
  if (head.byteLength > params.headerBytes) {
    queue.push(head.subarray(params.headerBytes));
  }

  let reader: ReadableStreamDefaultReader<Uint8Array> | null = null;
  if (!opts.complete && params.cipherTotalBytes > head.byteLength) {
    const from = head.byteLength;
    const res = await ranged(opts.url, `bytes=${from}-`, opts.signal);
    if (!res.body) throw new TransferError("The response carried no body.", res.status);
    reader = res.body.getReader();

    // A proxy that dropped the Range header answers 200 with the whole file.
    // Then the head we already hold is a duplicate of the first bytes of this
    // stream, so start over from this stream alone rather than concatenating
    // the same bytes twice.
    if (res.status !== 206) {
      let toDrop = params.headerBytes;
      const fresh = new ByteQueue();
      while (toDrop > 0) {
        const { done, value } = await reader.read();
        if (done) throw new TransferError("The file ended before its header did.", 200);
        const chunk = value as Uint8Array;
        if (chunk.byteLength <= toDrop) {
          toDrop -= chunk.byteLength;
        } else {
          fresh.push(chunk.subarray(toDrop));
          toDrop = 0;
        }
      }
      // Replace the queue contents wholesale.
      while (queue.length > 0) queue.take(queue.length);
      queue.push(fresh.take(fresh.length));
    }
  }

  let done = 0;
  for (let index = 1; index <= params.frameCount; index++) {
    const need = frameCipherBytes(params, index);
    while (queue.length < need) {
      if (!reader) throw new EnvelopeError("The file is shorter than its header claims.", "truncated");
      const next = await reader.read();
      if (next.done) {
        throw new EnvelopeError("The file is shorter than its header claims.", "truncated");
      }
      queue.push(next.value as Uint8Array);
    }
    const plain = await decryptFrame(key, params, index, queue.take(need));
    done += plain.byteLength;
    opts.onProgress?.(done, params.plainTotalBytes);
    yield plain;
  }

  await reader?.cancel().catch(() => {});
}

/**
 * The whole plaintext as a Blob.
 *
 * Only ever called under `MAX_INLINE_BYTES` (or `MAX_BLOB_SAVE_BYTES` for a
 * save on a browser with no file picker), because this is the one path that
 * holds everything at once.
 */
export async function decryptToBlob(opts: DecryptOptions, type: string): Promise<Blob> {
  const parts: Uint8Array[] = [];
  for await (const frame of decryptedFrames(opts)) parts.push(frame);
  return new Blob(parts as BlobPart[], { type });
}

/** Minimal shape of a File System Access writable, so no DOM lib version is required. */
export interface WritableSink {
  write(data: Uint8Array): Promise<void>;
  close(): Promise<void>;
  abort?(reason?: unknown): Promise<void>;
}

/** Stream the plaintext into a file handle. Bounded memory, any size. */
export async function decryptToSink(opts: DecryptOptions, sink: WritableSink): Promise<void> {
  try {
    for await (const frame of decryptedFrames(opts)) await sink.write(frame);
    await sink.close();
  } catch (err) {
    // Leaving a half-written file behind, silently, is how somebody ends up
    // trusting a truncated backup.
    await sink.abort?.(err).catch(() => {});
    throw err;
  }
}
