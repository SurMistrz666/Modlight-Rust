/**
 * The MLENC1 container: end-to-end encrypted uploads.
 *
 * ## What this is for
 *
 * An upload marked "encrypted" is turned into ciphertext by the browser BEFORE
 * a single byte leaves the tab, and turned back by the browser of whoever opens
 * the link. The key is generated here, lives in the URL fragment, and is never
 * sent anywhere: browsers do not put a fragment on the wire, do not put it in a
 * `Referer`, and no proxy or access log can record what it was never given.
 * The server stores an opaque blob and can decrypt exactly nothing.
 *
 * ## Why a frame format rather than one `crypto.subtle.encrypt`
 *
 * `crypto.subtle.encrypt` has no streaming form. Encrypting a file in one call
 * means holding the plaintext AND the ciphertext in the tab's memory at once, so
 * the 5 GB screen recording this product exists for would take ~10 GB of RAM and
 * kill the tab before the first byte moved. It also cannot resume: a connection
 * that dies at 90% would have to re-encrypt everything.
 *
 * So the plaintext is cut into fixed chunks and each one is sealed on its own.
 * Because every frame has a length that is a pure function of the file size and
 * the chunk size, the ciphertext offset of frame k is known without encrypting
 * anything before it - which is what lets this ride on tus: a PATCH resuming at
 * ciphertext offset N maps back to a frame index and an offset inside it, so a
 * transfer resumes from the last confirmed byte exactly as an unencrypted one
 * does. See components/upload/encrypted-source.ts for that half.
 *
 * ## Cipher and IV discipline
 *
 * AES-256-GCM (WebCrypto `AES-GCM`, 256-bit key, 128-bit tag - the WebCrypto
 * default and the only tag length we ever ask for).
 *
 * The IV is 12 bytes, the size AES-GCM is specified for, and it is NEVER random
 * per frame: it is `noncePrefix (8 random bytes, once per file) || counter (4
 * bytes, big endian)`. Two frames of one file therefore cannot share an IV,
 * because the counter is part of it and no counter is used twice. Two different
 * files cannot collide in a way that matters either, because each file gets a
 * fresh 256-bit key AND a fresh 64-bit prefix - and IV reuse is only dangerous
 * under the SAME key.
 *
 * The counter is 32 bits, so a file may hold 2^32 - 1 frames; at the 8 MiB
 * default chunk that is 32 EiB, and `buildEnvelope` refuses anything that would
 * overflow it rather than wrapping the counter around, which is the one way this
 * scheme could ever repeat an IV.
 *
 * Counter 0 is reserved for the metadata frame (the real filename and MIME
 * type), so data frames are numbered from 1.
 *
 * ## What the additional data binds
 *
 * Every frame is sealed with AAD = `fixed header (32 bytes) || counter (4) ||
 * last-frame flag (1)`. That makes each frame inseparable from:
 *
 *  - its file (the header carries the nonce prefix, the chunk size and the total
 *    plaintext length), so frames cannot be spliced between two files;
 *  - its position, so frames cannot be reordered;
 *  - whether it is the final one, so the file cannot be truncated silently. A
 *    truncated container fails to open at all, because the decryptor knows from
 *    the header how many frames it must find.
 *
 * ## Byte layout
 *
 * ```
 *  offset  len  field
 *  0       6    magic "MLENC1"
 *  6       1    0x00       - a separator, and deliberately a control byte: it is
 *                            what stops the server's type sniffer from deciding
 *                            an encrypted file is text/plain (lib/media/sniff).
 *  7       1    version    - 1
 *  8       4    plaintext chunk size, uint32 BE
 *  12      8    plaintext total size, uint64 BE
 *  20      8    nonce prefix (random, per file)
 *  28      4    metadata frame length, uint32 BE
 *  32      M    metadata frame  : AES-GCM(counter 0) over the metadata JSON
 *  32+M    ...  data frames k=1..N: AES-GCM(counter k) over plaintext chunk k
 * ```
 *
 * Nothing before offset 32+M is secret; all of it is authenticated.
 *
 * This module is deliberately isomorphic - no `server-only`, no Node imports -
 * because the browser encrypts with it, the browser decrypts with it, and the
 * tests exercise the very same code under Node's WebCrypto.
 */

/** Magic bytes every container starts with. */
export const CONTAINER_MAGIC = "MLENC1";

/** Format version. Bumped only for a change a v1 reader could not survive. */
export const CONTAINER_VERSION = 1;

/** Bytes before the metadata frame. */
export const FIXED_HEADER_BYTES = 32;

/** AES-GCM authentication tag, appended to every frame by WebCrypto. */
export const GCM_TAG_BYTES = 16;

/** AES-256. */
export const KEY_BYTES = 32;

/** IV length AES-GCM is specified for. */
export const IV_BYTES = 12;

/** Random part of the IV, fixed for one file. */
export const NONCE_PREFIX_BYTES = 8;

/** Counter part of the IV, in bytes. 32 bits, so 2^32 - 1 usable frames. */
export const COUNTER_BYTES = 4;

/** Highest frame index the counter can express. */
export const MAX_FRAME_INDEX = 0xffffffff;

/** IV counter reserved for the metadata frame. Data frames start at 1. */
export const METADATA_COUNTER = 0;

/**
 * Plaintext bytes per frame.
 *
 * 8 MiB is a memory decision, not a throughput one: encrypting a frame needs the
 * plaintext slice and its ciphertext live at the same time, and the upload
 * source keeps two frames cached so a tus PATCH straddling a boundary does not
 * encrypt anything twice. That is a ~32 MiB working set for a file of any size,
 * which a phone can hold. Larger frames would waste memory; much smaller ones
 * would add a 16-byte tag more often and make the frame table longer for no gain.
 */
export const DEFAULT_PLAIN_CHUNK_BYTES = 8 * 1024 * 1024;

/**
 * Ceiling on the encrypted metadata frame.
 *
 * The point is that ONE ranged read of `HEADER_PROBE_BYTES` is always enough to
 * open a container. The viewer's first request is that read and it is the only
 * request that spends a view, so a header that could overflow it would cost a
 * second counted request and break burn-after-view links.
 */
export const MAX_METADATA_BYTES = 3072;

/** How much of a container the viewer reads before it can name the file. */
export const HEADER_PROBE_BYTES = 4096;

/**
 * Longest filename kept inside the container.
 *
 * `uploads.filename` is varchar(512) and a name is UTF-8, so an unbounded one
 * could push the header past the probe. Truncating the visible name is a far
 * better failure than a container that needs two reads to open.
 */
export const MAX_NAME_CHARS = 220;

/** Extension every encrypted upload is stored under. */
export const ENCRYPTED_EXT = "mlenc";

/**
 * MIME the upload is declared as.
 *
 * It has to be a type the server has never heard of, and that is the whole
 * point: `mediaKind()` answers "other" for it, so `sharp` is never asked to
 * decode it, `ffmpeg` is never asked to seek in it, no thumbnail or motion
 * preview or "download as" is offered, and `serveDecision()` hands it over as
 * `application/octet-stream; attachment`. Random bytes must never be fed to a
 * decoder, and the cheapest way to guarantee that is to look like nothing.
 */
export const ENCRYPTED_MIME = "application/vnd.modlight.encrypted";

/** Everything wrong with a container reads as one error type. */
export class EnvelopeError extends Error {
  constructor(
    message: string,
    /** Machine-readable reason, so the viewer can pick a sentence. */
    readonly reason:
      | "not-a-container"
      | "unsupported-version"
      | "malformed"
      | "bad-key"
      | "truncated",
  ) {
    super(message);
    this.name = "EnvelopeError";
  }
}

/** What a container says about itself before any key is applied. */
export interface EnvelopeParams {
  version: number;
  plainChunkBytes: number;
  plainTotalBytes: number;
  noncePrefix: Uint8Array;
  metadataCipherBytes: number;
  /** Bytes before the first data frame: FIXED_HEADER_BYTES + metadata frame. */
  headerBytes: number;
  /** Number of data frames. Zero for an empty file. */
  frameCount: number;
  /** Total size of the stored container. */
  cipherTotalBytes: number;
  /** The 32 fixed header bytes, kept because they are every frame's AAD. */
  fixedHeader: Uint8Array;
}

/** The bits of a file that would otherwise leak, sealed inside the container. */
export interface EnvelopeMetadata {
  /** Original filename. */
  name: string;
  /** Original MIME type, as the browser reported it. */
  type: string;
}

/** A built container: its header bytes plus everything needed to fill it in. */
export interface EnvelopePlan extends EnvelopeParams {
  /** Bytes 0 .. headerBytes of the container, ready to send. */
  header: Uint8Array;
}

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });

function subtle(): SubtleCrypto {
  const c = globalThis.crypto;
  if (!c?.subtle) {
    throw new EnvelopeError(
      "WebCrypto is unavailable; encryption needs a secure context (https).",
      "malformed",
    );
  }
  return c.subtle;
}

/** 32 random bytes: one file's key. Never leaves the browser except in a fragment. */
export function generateKeyBytes(): Uint8Array {
  const raw = new Uint8Array(KEY_BYTES);
  globalThis.crypto.getRandomValues(raw);
  return raw;
}

/** 8 random bytes: the fixed half of every IV in one file. */
export function generateNoncePrefix(): Uint8Array {
  const raw = new Uint8Array(NONCE_PREFIX_BYTES);
  globalThis.crypto.getRandomValues(raw);
  return raw;
}

/** Turn raw key bytes into a non-extractable AES-GCM key. */
export async function importKey(raw: Uint8Array): Promise<CryptoKey> {
  if (raw.length !== KEY_BYTES) {
    throw new EnvelopeError(`Key must be ${KEY_BYTES} bytes.`, "bad-key");
  }
  // extractable:false so nothing downstream - a stray log, a devtools snapshot,
  // a well-meaning "let me just serialise this" - can read the key back out.
  return subtle().importKey("raw", toBufferSource(raw), "AES-GCM", false, [
    "encrypt",
    "decrypt",
  ]);
}

/**
 * `Uint8Array` from a WebCrypto call, as a plain ArrayBuffer view.
 *
 * TypeScript's DOM lib types `BufferSource` as `ArrayBuffer | ArrayBufferView`,
 * and a Uint8Array over a SharedArrayBuffer is neither in strict mode. Copying
 * is not the point; narrowing is.
 */
function toBufferSource(view: Uint8Array): ArrayBuffer {
  const copy = new ArrayBuffer(view.byteLength);
  new Uint8Array(copy).set(view);
  return copy;
}

/** IV for one frame: nonce prefix, then the counter, big endian. */
export function ivFor(noncePrefix: Uint8Array, counter: number): Uint8Array {
  if (noncePrefix.length !== NONCE_PREFIX_BYTES) {
    throw new EnvelopeError("Bad nonce prefix length.", "malformed");
  }
  if (!Number.isInteger(counter) || counter < 0 || counter > MAX_FRAME_INDEX) {
    throw new EnvelopeError("Frame counter out of range.", "malformed");
  }
  const iv = new Uint8Array(IV_BYTES);
  iv.set(noncePrefix, 0);
  new DataView(iv.buffer).setUint32(NONCE_PREFIX_BYTES, counter, false);
  return iv;
}

/**
 * Additional authenticated data for one frame.
 *
 * The whole fixed header goes in, so a frame is bound to the file's identity
 * (nonce prefix), its shape (chunk size, total length) and its own metadata
 * length. Then the counter, so frames cannot be reordered, and the last-frame
 * flag, so the file cannot be cut short without the final frame failing to open.
 */
export function aadFor(fixedHeader: Uint8Array, counter: number, last: boolean): Uint8Array {
  const aad = new Uint8Array(FIXED_HEADER_BYTES + COUNTER_BYTES + 1);
  aad.set(fixedHeader.subarray(0, FIXED_HEADER_BYTES), 0);
  new DataView(aad.buffer).setUint32(FIXED_HEADER_BYTES, counter, false);
  aad[FIXED_HEADER_BYTES + COUNTER_BYTES] = last ? 1 : 0;
  return aad;
}

/** Frames of plaintext in a file of `total` bytes cut into `chunk`-byte pieces. */
export function frameCountFor(total: number, chunk: number): number {
  if (total <= 0) return 0;
  return Math.ceil(total / chunk);
}

/** Stride between the start of one data frame and the next. */
export function frameStride(plainChunkBytes: number): number {
  return plainChunkBytes + GCM_TAG_BYTES;
}

/** Plaintext bytes carried by data frame `index` (1-based). */
export function plainBytesInFrame(params: EnvelopeParams, index: number): number {
  const start = (index - 1) * params.plainChunkBytes;
  return Math.min(params.plainChunkBytes, params.plainTotalBytes - start);
}

/** Ciphertext bytes of data frame `index` (1-based). */
export function frameCipherBytes(params: EnvelopeParams, index: number): number {
  return plainBytesInFrame(params, index) + GCM_TAG_BYTES;
}

/**
 * Where data frame `index` starts in the container.
 *
 * A plain multiplication is correct even though the final frame may be short:
 * only the LAST frame can be short, and nothing comes after it, so every frame
 * before position k is exactly one stride wide.
 */
export function frameOffset(params: EnvelopeParams, index: number): number {
  return params.headerBytes + (index - 1) * frameStride(params.plainChunkBytes);
}

/** Which data frame covers container offset `offset`. */
export function frameIndexAt(params: EnvelopeParams, offset: number): number {
  if (offset < params.headerBytes) return 0;
  const index =
    Math.floor((offset - params.headerBytes) / frameStride(params.plainChunkBytes)) + 1;
  return Math.min(index, params.frameCount);
}

/** Size of the container that will hold `plainTotalBytes`. */
export function cipherTotalFor(
  plainTotalBytes: number,
  plainChunkBytes: number,
  headerBytes: number,
): number {
  const frames = frameCountFor(plainTotalBytes, plainChunkBytes);
  return headerBytes + plainTotalBytes + frames * GCM_TAG_BYTES;
}

/** Shorten a name to something the header can always hold. */
export function clampName(name: string): string {
  const trimmed = name.trim() || "file";
  return trimmed.length <= MAX_NAME_CHARS ? trimmed : trimmed.slice(0, MAX_NAME_CHARS);
}

function writeFixedHeader(input: {
  plainChunkBytes: number;
  plainTotalBytes: number;
  noncePrefix: Uint8Array;
  metadataCipherBytes: number;
}): Uint8Array {
  const header = new Uint8Array(FIXED_HEADER_BYTES);
  header.set(encoder.encode(CONTAINER_MAGIC), 0);
  header[6] = 0x00;
  header[7] = CONTAINER_VERSION;
  const view = new DataView(header.buffer);
  view.setUint32(8, input.plainChunkBytes, false);
  view.setBigUint64(12, BigInt(input.plainTotalBytes), false);
  header.set(input.noncePrefix, 20);
  view.setUint32(28, input.metadataCipherBytes, false);
  return header;
}

export interface BuildEnvelopeInput {
  key: CryptoKey;
  metadata: EnvelopeMetadata;
  plainTotalBytes: number;
  plainChunkBytes?: number;
  /** Injectable only so a test can pin the IVs; production always randomises. */
  noncePrefix?: Uint8Array;
}

/**
 * Build a container header for a file that is about to be encrypted.
 *
 * The header is finished before any payload byte is touched, which is what lets
 * the uploader declare an exact `Upload-Length` to tus (and therefore be
 * quota-checked, and therefore be resumable) without reading the file first.
 */
export async function buildEnvelope(input: BuildEnvelopeInput): Promise<EnvelopePlan> {
  const plainChunkBytes = input.plainChunkBytes ?? DEFAULT_PLAIN_CHUNK_BYTES;
  if (!Number.isInteger(plainChunkBytes) || plainChunkBytes <= 0) {
    throw new EnvelopeError("Chunk size must be a positive integer.", "malformed");
  }
  const plainTotalBytes = input.plainTotalBytes;
  if (!Number.isInteger(plainTotalBytes) || plainTotalBytes < 0) {
    throw new EnvelopeError("File size must be a non-negative integer.", "malformed");
  }

  const frameCount = frameCountFor(plainTotalBytes, plainChunkBytes);
  if (frameCount > MAX_FRAME_INDEX) {
    // The one condition that could make this scheme repeat an IV. Refusing is
    // the only acceptable answer; wrapping the counter is not.
    throw new EnvelopeError("File needs more frames than the counter can address.", "malformed");
  }

  const noncePrefix = input.noncePrefix ?? generateNoncePrefix();
  const metadataPlain = encoder.encode(
    JSON.stringify({
      n: clampName(input.metadata.name),
      t: input.metadata.type || "application/octet-stream",
    }),
  );
  const metadataCipherBytes = metadataPlain.byteLength + GCM_TAG_BYTES;
  if (metadataCipherBytes > MAX_METADATA_BYTES) {
    throw new EnvelopeError("Encrypted metadata does not fit the header.", "malformed");
  }

  const fixedHeader = writeFixedHeader({
    plainChunkBytes,
    plainTotalBytes,
    noncePrefix,
    metadataCipherBytes,
  });

  const metadataCipher = new Uint8Array(
    await subtle().encrypt(
      {
        name: "AES-GCM",
        iv: toBufferSource(ivFor(noncePrefix, METADATA_COUNTER)),
        additionalData: toBufferSource(aadFor(fixedHeader, METADATA_COUNTER, false)),
        tagLength: GCM_TAG_BYTES * 8,
      },
      input.key,
      toBufferSource(metadataPlain),
    ),
  );

  const headerBytes = FIXED_HEADER_BYTES + metadataCipher.byteLength;
  const header = new Uint8Array(headerBytes);
  header.set(fixedHeader, 0);
  header.set(metadataCipher, FIXED_HEADER_BYTES);

  return {
    version: CONTAINER_VERSION,
    plainChunkBytes,
    plainTotalBytes,
    noncePrefix,
    metadataCipherBytes,
    headerBytes,
    frameCount,
    cipherTotalBytes: cipherTotalFor(plainTotalBytes, plainChunkBytes, headerBytes),
    fixedHeader,
    header,
  };
}

/** Read a container's parameters from its first bytes. No key needed. */
export function parseEnvelopeHeader(head: Uint8Array): EnvelopeParams {
  if (head.byteLength < FIXED_HEADER_BYTES) {
    throw new EnvelopeError("Too short to be a container.", "not-a-container");
  }
  for (let i = 0; i < CONTAINER_MAGIC.length; i++) {
    if (head[i] !== CONTAINER_MAGIC.charCodeAt(i)) {
      throw new EnvelopeError("Not an encrypted container.", "not-a-container");
    }
  }
  if (head[6] !== 0x00) throw new EnvelopeError("Not an encrypted container.", "not-a-container");

  const version = head[7];
  if (version !== CONTAINER_VERSION) {
    throw new EnvelopeError(`Container version ${version} is not supported.`, "unsupported-version");
  }

  const view = new DataView(head.buffer, head.byteOffset, head.byteLength);
  const plainChunkBytes = view.getUint32(8, false);
  const plainTotalRaw = view.getBigUint64(12, false);
  const metadataCipherBytes = view.getUint32(28, false);

  if (plainChunkBytes <= 0) throw new EnvelopeError("Bad chunk size.", "malformed");
  if (plainTotalRaw > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new EnvelopeError("Declared size is out of range.", "malformed");
  }
  if (metadataCipherBytes <= GCM_TAG_BYTES || metadataCipherBytes > MAX_METADATA_BYTES) {
    throw new EnvelopeError("Bad metadata length.", "malformed");
  }

  const plainTotalBytes = Number(plainTotalRaw);
  const frameCount = frameCountFor(plainTotalBytes, plainChunkBytes);
  if (frameCount > MAX_FRAME_INDEX) throw new EnvelopeError("Too many frames.", "malformed");

  const headerBytes = FIXED_HEADER_BYTES + metadataCipherBytes;
  return {
    version,
    plainChunkBytes,
    plainTotalBytes,
    noncePrefix: head.slice(20, 28),
    metadataCipherBytes,
    headerBytes,
    frameCount,
    cipherTotalBytes: cipherTotalFor(plainTotalBytes, plainChunkBytes, headerBytes),
    fixedHeader: head.slice(0, FIXED_HEADER_BYTES),
  };
}

/**
 * Open a container: parse the header and decrypt the metadata frame.
 *
 * `head` must hold at least `headerBytes`; `HEADER_PROBE_BYTES` always does,
 * which is why the metadata frame has a ceiling.
 */
export async function openEnvelope(
  head: Uint8Array,
  key: CryptoKey,
): Promise<{ params: EnvelopeParams; metadata: EnvelopeMetadata }> {
  const params = parseEnvelopeHeader(head);
  if (head.byteLength < params.headerBytes) {
    throw new EnvelopeError("Header is incomplete.", "truncated");
  }

  let plain: Uint8Array;
  try {
    plain = new Uint8Array(
      await subtle().decrypt(
        {
          name: "AES-GCM",
          iv: toBufferSource(ivFor(params.noncePrefix, METADATA_COUNTER)),
          additionalData: toBufferSource(aadFor(params.fixedHeader, METADATA_COUNTER, false)),
          tagLength: GCM_TAG_BYTES * 8,
        },
        key,
        toBufferSource(head.subarray(FIXED_HEADER_BYTES, params.headerBytes)),
      ),
    );
  } catch {
    // GCM does not distinguish "wrong key" from "tampered bytes" and neither
    // should we: both mean these bytes are not this key's.
    throw new EnvelopeError("Wrong key, or the file has been altered.", "bad-key");
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(decoder.decode(plain));
  } catch {
    throw new EnvelopeError("Metadata is not readable.", "malformed");
  }
  const record = (parsed ?? {}) as Record<string, unknown>;
  return {
    params,
    metadata: {
      name: typeof record.n === "string" && record.n ? record.n : "file",
      type: typeof record.t === "string" && record.t ? record.t : "application/octet-stream",
    },
  };
}

/** Seal one data frame. `index` is 1-based; `plain` is that frame's plaintext. */
export async function encryptFrame(
  key: CryptoKey,
  params: EnvelopeParams,
  index: number,
  plain: Uint8Array,
): Promise<Uint8Array> {
  if (index < 1 || index > params.frameCount) {
    throw new EnvelopeError("Frame index out of range.", "malformed");
  }
  return new Uint8Array(
    await subtle().encrypt(
      {
        name: "AES-GCM",
        iv: toBufferSource(ivFor(params.noncePrefix, index)),
        additionalData: toBufferSource(
          aadFor(params.fixedHeader, index, index === params.frameCount),
        ),
        tagLength: GCM_TAG_BYTES * 8,
      },
      key,
      toBufferSource(plain),
    ),
  );
}

/** Open one data frame. Throws `EnvelopeError("bad-key")` on any mismatch. */
export async function decryptFrame(
  key: CryptoKey,
  params: EnvelopeParams,
  index: number,
  cipher: Uint8Array,
): Promise<Uint8Array> {
  if (index < 1 || index > params.frameCount) {
    throw new EnvelopeError("Frame index out of range.", "malformed");
  }
  if (cipher.byteLength !== frameCipherBytes(params, index)) {
    throw new EnvelopeError("Frame is the wrong length.", "truncated");
  }
  try {
    return new Uint8Array(
      await subtle().decrypt(
        {
          name: "AES-GCM",
          iv: toBufferSource(ivFor(params.noncePrefix, index)),
          additionalData: toBufferSource(
            aadFor(params.fixedHeader, index, index === params.frameCount),
          ),
          tagLength: GCM_TAG_BYTES * 8,
        },
        key,
        toBufferSource(cipher),
      ),
    );
  } catch {
    throw new EnvelopeError("Wrong key, or the file has been altered.", "bad-key");
  }
}

/**
 * Does this stored row hold an encrypted container?
 *
 * Asked of the ROW, never of the bytes: the serving layer must not read a file
 * to decide how to present it, and the two facts it needs are already columns.
 * Both are checked because either one alone is guessable - a user can name a
 * plain file `.mlenc`, and a client can claim any MIME - and the answer only
 * decides which viewer renders, never whether anything is decrypted (that fails
 * on its own if the bytes are not a container).
 */
export function isEncryptedUpload(
  mimeType: string | null | undefined,
  ext: string | null | undefined,
): boolean {
  return (
    (mimeType ?? "").split(";")[0].trim().toLowerCase() === ENCRYPTED_MIME &&
    (ext ?? "").toLowerCase() === ENCRYPTED_EXT
  );
}
