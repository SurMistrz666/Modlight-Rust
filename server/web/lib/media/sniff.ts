/**
 * What kind of file is this, really?
 *
 * The stored `mimeType` used to be whatever the client claimed, which is the
 * weakest evidence available: on Windows both ShareX and the file picker read
 * the type out of `HKCR\.<ext>\Content Type`, and Windows does not register the
 * modern formats at all, so a perfectly ordinary .avif/.heic/.opus arrives as
 * `application/octet-stream`. A row with that type has no preview, no inline
 * display and no conversion, which is exactly the bug this module exists for.
 *
 * Three sources of truth, deliberately ranked (see `resolveUploadType`):
 * signature, then extension, then the client's claim. No dependencies and no
 * I/O, so the upload paths, the migration script and the tests all share one
 * implementation.
 */

// ---------------------------------------------------------------------------
// Extension -> MIME
// ---------------------------------------------------------------------------

/**
 * Extension -> canonical MIME, including the formats Windows is most likely to
 * shrug at.
 *
 * `application/octet-stream` entries are not omissions: they record a format we
 * recognise and for which "unknown binary" is the honest answer.
 */
export const MIME_BY_EXT: Record<string, string> = {
  // images
  png: "image/png",
  apng: "image/apng",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  jfif: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  avif: "image/avif",
  avifs: "image/avif",
  jxl: "image/jxl",
  bmp: "image/bmp",
  ico: "image/x-icon",
  cur: "image/x-icon",
  tif: "image/tiff",
  tiff: "image/tiff",
  svg: "image/svg+xml",
  heic: "image/heic",
  heif: "image/heif",
  psd: "image/vnd.adobe.photoshop",
  // video
  mp4: "video/mp4",
  m4v: "video/x-m4v",
  mov: "video/quicktime",
  qt: "video/quicktime",
  webm: "video/webm",
  ogv: "video/ogg",
  mkv: "video/x-matroska",
  avi: "video/x-msvideo",
  wmv: "video/x-ms-wmv",
  flv: "video/x-flv",
  mpg: "video/mpeg",
  mpeg: "video/mpeg",
  ts: "video/mp2t",
  "3gp": "video/3gpp",
  "3g2": "video/3gpp2",
  // audio
  mp3: "audio/mpeg",
  ogg: "audio/ogg",
  oga: "audio/ogg",
  opus: "audio/opus",
  wav: "audio/wav",
  flac: "audio/flac",
  m4a: "audio/mp4",
  mka: "audio/x-matroska",
  aac: "audio/aac",
  aif: "audio/aiff",
  aiff: "audio/aiff",
  wma: "audio/x-ms-wma",
  mid: "audio/midi",
  midi: "audio/midi",
  aup3: "audio/x-audacity-project",
  // text / code
  txt: "text/plain",
  md: "text/markdown",
  markdown: "text/markdown",
  csv: "text/csv",
  tsv: "text/tab-separated-values",
  log: "text/plain",
  ini: "text/plain",
  cfg: "text/plain",
  conf: "text/plain",
  html: "text/html",
  htm: "text/html",
  xhtml: "application/xhtml+xml",
  css: "text/css",
  js: "text/javascript",
  mjs: "text/javascript",
  cjs: "text/javascript",
  jsx: "text/plain",
  tsx: "text/plain",
  lua: "text/x-lua",
  py: "text/x-python",
  rb: "text/x-ruby",
  sh: "application/x-sh",
  bat: "text/plain",
  ps1: "text/plain",
  c: "text/x-c",
  h: "text/x-c",
  cpp: "text/x-c",
  cs: "text/plain",
  java: "text/x-java-source",
  go: "text/plain",
  rs: "text/plain",
  php: "text/x-php",
  sql: "application/sql",
  json: "application/json",
  jsonc: "application/json",
  jwt: "application/jwt",
  xml: "application/xml",
  yml: "text/plain",
  yaml: "text/plain",
  toml: "text/plain",
  srt: "application/x-subrip",
  vtt: "text/vtt",
  ics: "text/calendar",
  sxcu: "application/json", // ShareX custom-uploader config
  // documents
  pdf: "application/pdf",
  rtf: "application/rtf",
  doc: "application/msword",
  docx: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
  xls: "application/vnd.ms-excel",
  xlsx: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
  ppt: "application/vnd.ms-powerpoint",
  pptx: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
  odt: "application/vnd.oasis.opendocument.text",
  ods: "application/vnd.oasis.opendocument.spreadsheet",
  odp: "application/vnd.oasis.opendocument.presentation",
  epub: "application/epub+zip",
  // archives
  zip: "application/zip",
  rar: "application/vnd.rar",
  "7z": "application/x-7z-compressed",
  gz: "application/gzip",
  bz2: "application/x-bzip2",
  xz: "application/x-xz",
  zst: "application/zstd",
  tar: "application/x-tar",
  tgz: "application/gzip",
  // executables / installers / fonts
  exe: "application/vnd.microsoft.portable-executable",
  msi: "application/x-msi",
  apk: "application/vnd.android.package-archive",
  jar: "application/java-archive",
  wasm: "application/wasm",
  ttf: "font/ttf",
  otf: "font/otf",
  woff: "font/woff",
  woff2: "font/woff2",
  // 3D / game assets
  "3mf": "model/3mf",
  stl: "model/stl",
  obj: "model/obj",
  gr2: "application/octet-stream", // Granny2 mesh - no registered type
};

/** Normalise ".PNG" / "png" / "" to a bare lowercase key. */
export function normalizeExt(ext: string | null | undefined): string {
  return (ext ?? "").trim().toLowerCase().replace(/^\./, "");
}

/** Extension of a filename, without the dot ("" when there is none). */
export function extOfName(filename: string | null | undefined): string {
  const m = /\.([a-z0-9]+)$/i.exec(filename ?? "");
  return m ? m[1].toLowerCase() : "";
}

/**
 * MIME for an extension, or null when we have nothing to say about it.
 *
 * Null rather than `application/octet-stream` on purpose: every caller wants to
 * distinguish "no opinion" from "definitely an unknown binary", and only the
 * caller knows which default belongs in its place.
 */
export function mimeFromExt(ext: string | null | undefined): string | null {
  return MIME_BY_EXT[normalizeExt(ext)] ?? null;
}

/** Types that mean "I did not look": treat them as absent evidence. */
const USELESS_MIMES = new Set([
  "",
  "*/*",
  "application/octet-stream",
  "binary/octet-stream",
  "application/octetstream",
  "application/x-download",
  "application/download",
  "application/force-download",
  "application/unknown",
  "unknown/unknown",
]);

/** Strip parameters and casing: `Image/PNG; charset=x` -> `image/png`. */
export function normalizeMime(mime: string | null | undefined): string {
  return (mime ?? "").split(";")[0].trim().toLowerCase();
}

/** Does this MIME carry any information at all? */
export function isUselessMime(mime: string | null | undefined): boolean {
  return USELESS_MIMES.has(normalizeMime(mime));
}

// ---------------------------------------------------------------------------
// Byte signatures
// ---------------------------------------------------------------------------

interface Signature {
  mime: string;
  /** Bytes to match, as a hex string. */
  hex: string;
  /** Offset the pattern starts at (default 0). */
  at?: number;
}

/**
 * Ordered longest-evidence-first. Two-byte patterns (BM, MZ) sit at the bottom
 * because they collide with far more than they identify.
 */
const SIGNATURES: Signature[] = [
  // images
  { mime: "image/png", hex: "89504e470d0a1a0a" },
  { mime: "image/gif", hex: "474946383761" },
  { mime: "image/gif", hex: "474946383961" },
  { mime: "image/jpeg", hex: "ffd8ff" },
  { mime: "image/tiff", hex: "49492a00" },
  { mime: "image/tiff", hex: "4d4d002a" },
  { mime: "image/vnd.adobe.photoshop", hex: "38425053" },
  { mime: "image/jxl", hex: "0000000c4a584c200d0a870a" },
  { mime: "image/jxl", hex: "ff0a" },
  { mime: "image/x-icon", hex: "00000100" },
  { mime: "image/svg+xml", hex: "3c737667" }, // "<svg"
  // documents / archives
  { mime: "application/pdf", hex: "255044462d" },
  { mime: "application/zip", hex: "504b0304" },
  { mime: "application/zip", hex: "504b0506" },
  { mime: "application/zip", hex: "504b0708" },
  { mime: "application/vnd.rar", hex: "526172211a07" },
  { mime: "application/x-7z-compressed", hex: "377abcaf271c" },
  { mime: "application/x-xz", hex: "fd377a585a00" },
  { mime: "application/zstd", hex: "28b52ffd" },
  { mime: "application/x-bzip2", hex: "425a68" },
  { mime: "application/gzip", hex: "1f8b" },
  { mime: "application/x-tar", hex: "7573746172", at: 257 }, // "ustar"
  { mime: "application/vnd.sqlite3", hex: "53514c69746520666f726d61742033" },
  { mime: "application/rtf", hex: "7b5c727466" },
  { mime: "application/xml", hex: "3c3f786d6c" }, // "<?xml"
  // media containers
  { mime: "application/ogg", hex: "4f676753" },
  { mime: "audio/flac", hex: "664c6143" },
  { mime: "video/x-matroska", hex: "1a45dfa3" },
  { mime: "audio/mpeg", hex: "494433" }, // ID3
  { mime: "audio/mpeg", hex: "fffb" },
  { mime: "audio/mpeg", hex: "fff3" },
  { mime: "audio/mpeg", hex: "fff2" },
  { mime: "audio/aiff", hex: "464f524d" },
  { mime: "audio/midi", hex: "4d546864" },
  // fonts / code objects
  { mime: "font/woff2", hex: "774f4632" },
  { mime: "font/woff", hex: "774f4646" },
  { mime: "font/ttf", hex: "0001000000" },
  { mime: "font/otf", hex: "4f54544f" },
  { mime: "application/wasm", hex: "0061736d" },
  { mime: "application/java-vm", hex: "cafebabe" },
  // weak, last resort
  { mime: "application/vnd.microsoft.portable-executable", hex: "4d5a" },
  { mime: "image/bmp", hex: "424d" },
];

/** RIFF is one container for three unrelated formats; the tag at 8 decides. */
const RIFF_FORMS: Record<string, string> = {
  WEBP: "image/webp",
  WAVE: "audio/wav",
  AVI: "video/x-msvideo", // the tag is "AVI " on disk; `ascii()` trims it
};

/**
 * ISO base media brands.
 *
 * `specific` marks a brand that names the actual format ("this is AVIF"), as
 * opposed to one that only names the container family ("this is some MP4"). The
 * distinction is what lets the extension refine `isom` into `audio/mp4` for an
 * .m4a while never letting it argue with `avis`.
 *
 * The bug that started all this lives here: the feral upload's MAJOR brand is
 * `avis` (an AVIF image SEQUENCE), and `avif` appears only further down the
 * compatible-brands list. Comparing a fixed 12-byte prefix would have missed it.
 */
const ISO_BRANDS: Record<string, { mime: string; specific: boolean }> = {
  avif: { mime: "image/avif", specific: true },
  avis: { mime: "image/avif", specific: true },
  heic: { mime: "image/heic", specific: true },
  heix: { mime: "image/heic", specific: true },
  hevc: { mime: "image/heic", specific: true },
  heim: { mime: "image/heic", specific: true },
  mif1: { mime: "image/heic", specific: false },
  msf1: { mime: "image/heic", specific: false },
  qt: { mime: "video/quicktime", specific: true },
  M4A: { mime: "audio/mp4", specific: true },
  M4B: { mime: "audio/mp4", specific: true },
  M4V: { mime: "video/x-m4v", specific: true },
  M4P: { mime: "video/mp4", specific: true },
  isom: { mime: "video/mp4", specific: false },
  iso2: { mime: "video/mp4", specific: false },
  iso4: { mime: "video/mp4", specific: false },
  iso5: { mime: "video/mp4", specific: false },
  iso6: { mime: "video/mp4", specific: false },
  mp41: { mime: "video/mp4", specific: false },
  mp42: { mime: "video/mp4", specific: false },
  dash: { mime: "video/mp4", specific: false },
  "3gp4": { mime: "video/3gpp", specific: true },
  "3gp5": { mime: "video/3gpp", specific: true },
  "3g2a": { mime: "video/3gpp2", specific: true },
  crx: { mime: "image/x-canon-cr3", specific: true },
};

/** Everything the ISO base media format can be, for the refinement rule. */
const ISO_FAMILY = new Set([
  "video/mp4",
  "video/x-m4v",
  "video/quicktime",
  "video/3gpp",
  "video/3gpp2",
  "audio/mp4",
  "image/avif",
  "image/heic",
  "image/heif",
]);

/** Formats that ride inside a zip and would otherwise all read as a zip. */
const ZIP_FAMILY = new Set([
  "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
  "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
  "application/vnd.openxmlformats-officedocument.presentationml.presentation",
  "application/vnd.oasis.opendocument.text",
  "application/vnd.oasis.opendocument.spreadsheet",
  "application/vnd.oasis.opendocument.presentation",
  "application/epub+zip",
  "application/java-archive",
  "application/vnd.android.package-archive",
]);

/** Text-ish types an extension may pin down once the bytes read as text. */
function isTextual(mime: string): boolean {
  return (
    mime.startsWith("text/") ||
    mime.endsWith("+xml") ||
    mime.endsWith("+json") ||
    mime === "application/json" ||
    mime === "application/xml" ||
    mime === "application/sql" ||
    mime === "application/x-sh" ||
    mime === "application/jwt" ||
    mime === "application/x-subrip" ||
    mime === "image/svg+xml"
  );
}

/**
 * May `byExt` overrule `sniffed`?
 *
 * Only ever downward, within one family: a signature identifies the container,
 * and for container formats the extension is the only thing that names the
 * payload. Without this every .docx would be stored as application/zip and
 * every .m4a as video/mp4.
 */
function refines(sniffed: string, byExt: string): boolean {
  if (sniffed === byExt) return false;
  switch (sniffed) {
    // A generic ISO brand (isom/mp42/mif1) names the container and nothing else,
    // so .m4a is the only thing that distinguishes an audio file from a video
    // one. The specific brands never reach this point: they resolve to
    // image/avif, video/quicktime and friends, which have no case here.
    case "video/mp4":
    case "image/heic":
      return ISO_FAMILY.has(byExt);
    case "application/zip":
      return ZIP_FAMILY.has(byExt);
    case "application/ogg":
      return byExt === "video/ogg" || byExt === "audio/ogg" || byExt === "audio/opus";
    case "video/x-matroska":
      return byExt === "video/webm" || byExt === "audio/webm" || byExt === "audio/x-matroska";
    case "image/png":
      return byExt === "image/apng";
    case "application/xml":
      return isTextual(byExt);
    case "text/plain":
      return isTextual(byExt);
    case "font/ttf":
      // The TrueType signature is also what a bare OpenType/TTC file starts with.
      return byExt === "font/otf" || byExt === "font/collection";
    default:
      return false;
  }
}

function matches(head: Buffer, sig: Signature): boolean {
  const at = sig.at ?? 0;
  const bytes = sig.hex.length / 2;
  if (head.length < at + bytes) return false;
  for (let i = 0; i < bytes; i++) {
    if (head[at + i] !== parseInt(sig.hex.slice(i * 2, i * 2 + 2), 16)) return false;
  }
  return true;
}

/** Read a 32-bit big-endian value, or null when the buffer is too short. */
function u32(head: Buffer, at: number): number | null {
  return head.length >= at + 4 ? head.readUInt32BE(at) : null;
}

function ascii(head: Buffer, from: number, to: number): string {
  return head.length >= to ? head.toString("latin1", from, to).replace(/\0/g, "").trim() : "";
}

/**
 * Type of an ISO base media file, from its `ftyp` box.
 *
 * The major brand is tried first and the compatible-brands list is the fallback,
 * because that is the order the specification assigns authority in - and because
 * a specific brand anywhere in the list beats a generic one at the front.
 */
function sniffIsoBmff(head: Buffer): { mime: string; specific: boolean } | null {
  if (ascii(head, 4, 8) !== "ftyp") return null;

  const major = ISO_BRANDS[ascii(head, 8, 12)];
  if (major?.specific) return major;

  // Compatible brands run from offset 16 to the end of the ftyp box.
  const boxSize = u32(head, 0) ?? 0;
  const end = Math.min(boxSize > 16 ? boxSize : head.length, head.length);
  for (let at = 16; at + 4 <= end; at += 4) {
    const brand = ISO_BRANDS[ascii(head, at, at + 4)];
    if (brand?.specific) return brand;
  }

  return major ?? { mime: "video/mp4", specific: false };
}

/**
 * Do the first bytes read as text?
 *
 * Used only as a last resort, and deliberately strict: valid UTF-8 with no
 * control characters beyond tab/newline/carriage return. That is what rescues
 * the log file sitting in production as an unopenable binary download.
 */
function looksLikeText(head: Buffer): boolean {
  if (head.length === 0) return false;

  // A truncated read can cut a multi-byte character in half, which is not
  // evidence of anything - drop up to 3 trailing continuation bytes.
  let end = head.length;
  for (let i = 0; i < 3 && end > 0; i++) {
    const b = head[end - 1];
    if (b < 0x80) break;
    end--;
    if ((b & 0xc0) === 0xc0) break; // dropped the lead byte itself
  }
  const slice = head.subarray(0, end);
  if (slice.length === 0) return false;

  try {
    new TextDecoder("utf-8", { fatal: true }).decode(slice);
  } catch {
    return false;
  }
  for (const byte of slice) {
    if (byte === 0x09 || byte === 0x0a || byte === 0x0d) continue;
    if (byte < 0x20 || byte === 0x7f) return false;
  }
  return true;
}

/**
 * The type the bytes themselves claim, or null when they say nothing.
 *
 * `head` only needs the first few hundred bytes; 4 KiB covers every pattern
 * here (the tar magic sits at offset 257) and keeps the read cheap enough to do
 * on a multi-gigabyte upload.
 */
export function sniffMime(head: Buffer | null | undefined): string | null {
  if (!head || head.length === 0) return null;

  const iso = sniffIsoBmff(head);
  if (iso) return iso.mime;

  if (matches(head, { mime: "", hex: "52494646" })) {
    const form = RIFF_FORMS[ascii(head, 8, 12)];
    if (form) return form;
  }

  for (const sig of SIGNATURES) {
    if (matches(head, sig)) return sig.mime;
  }

  return looksLikeText(head) ? "text/plain" : null;
}

// ---------------------------------------------------------------------------
// The decision
// ---------------------------------------------------------------------------

export interface UploadTypeInput {
  /** Content-Type the client sent (multipart part header, or tus metadata). */
  clientMime?: string | null;
  /** Original filename, used for its extension. Ignored when `ext` is given. */
  filename?: string | null;
  /** Extension without the dot, when the caller already has it. */
  ext?: string | null;
  /** First bytes of the payload (4 KiB is plenty). */
  head?: Buffer | null;
}

/**
 * The type to STORE for an upload.
 *
 * Ranked by how hard each source is to be wrong about:
 *
 *  1. the signature, because bytes are the one thing the sender does not get to
 *     narrate - except where the signature only identifies a container, in which
 *     case
 *  2. the extension refines it downward within the same family (zip -> docx,
 *     ftyp -> m4a), and also stands alone when the bytes said nothing;
 *  3. the client's Content-Type, last and only when it is specific, because on
 *     Windows it is a registry lookup that simply has no entry for anything
 *     released this decade - which is how a .avif became an unopenable binary;
 *  4. `application/octet-stream` as an honest "no idea" (a Granny2 mesh really
 *     is an unknown binary, and pretending otherwise would be worse).
 *
 * This never decides how a file is SERVED. `serveDecision()` stays an allowlist,
 * so recognising an upload as text/html or image/svg+xml still results in
 * `text/plain` on the wire.
 */
export function resolveUploadType(input: UploadTypeInput): string {
  const ext = normalizeExt(input.ext ?? extOfName(input.filename));
  const byExt = mimeFromExt(ext);
  const usefulExt = byExt && !isUselessMime(byExt) ? byExt : null;
  const sniffed = sniffMime(input.head);

  if (sniffed) {
    if (usefulExt && refines(sniffed, usefulExt)) return usefulExt;
    return sniffed;
  }
  if (usefulExt) return usefulExt;

  const client = normalizeMime(input.clientMime);
  if (client && !isUselessMime(client)) return client;

  return "application/octet-stream";
}

/**
 * Is `candidate` a better answer than `current` for an already-stored row?
 *
 * The migration script's guard rail: a backfill may sharpen a type, never blunt
 * one. Without it a run over a file whose bytes have gone missing would happily
 * downgrade a good row to octet-stream.
 */
export function isSharperType(current: string | null | undefined, candidate: string): boolean {
  const now = normalizeMime(current);
  const next = normalizeMime(candidate);
  if (!next || next === now) return false;
  if (isUselessMime(next)) return false;
  return isUselessMime(now);
}
