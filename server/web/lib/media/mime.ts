/**
 * MIME helpers, governing how files are
 * previewed, embedded, and protected against XSS.
 */

import { isUselessMime, mimeFromExt } from "@/lib/media/sniff";

/** Image types the browser can render inline (preview as <img>). */
const DISPLAYABLE_IMAGES = new Set([
  "image/apng",
  "image/bmp",
  "image/gif",
  "image/x-icon",
  "image/jpeg",
  "image/png",
  "image/tiff",
  "image/webp",
  "image/avif",
]);

/** Types worth rich-embedding (Open Graph) for bots like Discord/Twitter/Telegram. */
const EMBEDDABLE = new Set([
  ...DISPLAYABLE_IMAGES,
  "video/mp4",
  "video/ogg",
  "video/webm",
]);

/**
 * The type to REASON with, given what we stored and what the file is called.
 *
 * A stored type is only evidence when it says something. ShareX and curl send
 * `application/octet-stream` for anything Windows has no registry entry for, and
 * an imported row keeps whatever its old host guessed years ago - so for
 * those rows the name on disk is the better clue, and without consulting it a
 * perfectly ordinary .avif has no preview, no inline display and no conversion.
 *
 * This is the rule `playableVideoType` already followed for video, lifted out so
 * images and everything else obey it too, rather than growing a second
 * mechanism beside it.
 *
 * It changes what we THINK a file is, never what we SERVE it as: `serveDecision`
 * below is an allowlist, so resolving an upload to text/html or image/svg+xml
 * still puts `text/plain` on the wire.
 */
export function effectiveType(
  mime: string | null | undefined,
  ext?: string | null,
): string {
  const m = normalizeMime(mime ?? "");
  if (m && !isUselessMime(m)) return m;
  return mimeFromExt(ext) ?? m;
}

export function isDisplayableImage(
  mime: string | null | undefined,
  ext?: string | null,
): boolean {
  return DISPLAYABLE_IMAGES.has(effectiveType(mime, ext));
}

export function isEmbeddable(mime: string | null | undefined, ext?: string | null): boolean {
  return EMBEDDABLE.has(effectiveType(mime, ext));
}

/**
 * Types that must never be served with their own Content-Type, because the
 * browser would execute them in our origin (stored XSS). Ported from
 * `must_be_escaped()`, but the serving decision below no longer relies on a
 * blocklist — this stays for callers that just want the flag.
 */
export function mustBeEscaped(mime: string | null | undefined): boolean {
  if (!mime) return false;
  const m = normalizeMime(mime);
  return m.startsWith("text/htm") || m.includes("svg") || m.endsWith("+xml") || m === "text/xml";
}

/** Strip parameters and casing: `Text/HTML; charset=UTF-8` → `text/html`. */
function normalizeMime(mime: string): string {
  return mime.split(";")[0].trim().toLowerCase();
}

/**
 * Types the browser may render in our origin as-is. Everything outside this set
 * is either downgraded to plain text or pushed out as a download — an allowlist,
 * because a blocklist only ever covers the markup types someone thought of
 * (`application/xhtml+xml`, `image/svg+xml`, `text/xml`, … all execute script).
 */
const INLINE_SAFE = new Set([
  ...DISPLAYABLE_IMAGES,
  "video/mp4",
  "video/webm",
  "video/ogg",
  "video/quicktime",
  "audio/mpeg",
  "audio/mp4",
  "audio/ogg",
  "audio/wav",
  "audio/x-wav",
  "audio/webm",
  "audio/flac",
  "application/pdf",
  "text/plain",
]);

/**
 * Text-ish payloads worth showing in the browser instead of downloading. They
 * are served as text/plain, which is inert regardless of what they contain, so
 * source-viewing a `.html`/`.svg`/`.json` upload stays possible without handing
 * it script execution on our domain.
 */
function isTextLike(mime: string): boolean {
  return (
    mime.startsWith("text/") ||
    mime.endsWith("+xml") ||
    mime.endsWith("+json") ||
    mime === "application/json" ||
    mime === "application/xml" ||
    mime === "application/javascript" ||
    mime === "application/x-javascript" ||
    mime === "application/x-sh" ||
    mime === "application/sql"
  );
}

// ---------------------------------------------------------------------------
// What a browser will actually play
// ---------------------------------------------------------------------------

/**
 * Container type -> the type to ADVERTISE, or null when no browser can decode it.
 *
 * The distinction the player got wrong: a container is not a codec. QuickTime
 * and MP4 are both ISO base media format, and a .mov off a phone, a Mac screen
 * recorder or OBS carries H.264/AAC - which every browser decodes happily. But
 * Chrome and Firefox reject `video/quicktime` on the LABEL alone, before
 * touching a byte, so the only thing between the user and a working player was
 * that string. Relabelling to video/mp4 is not a lie about the bytes; it is the
 * same bitstream under the name the decoder is registered against.
 *
 * Matroska, AVI, WMV, FLV, MPEG-PS and MPEG-TS are a different case: no browser
 * ships a demuxer for them, so no label helps and pretending otherwise just
 * produces a dead player. Those return null and get the download card instead.
 */
const VIDEO_PLAYS_AS: Record<string, string> = {
  "video/mp4": "video/mp4",
  "video/webm": "video/webm",
  "video/ogg": "video/ogg",
  "application/ogg": "video/ogg",
  "video/quicktime": "video/mp4",
  "video/x-m4v": "video/mp4",
  "video/m4v": "video/mp4",
  "video/3gpp": "video/mp4",
  "video/3gpp2": "video/mp4",
};

/** Same idea for audio: the container lies, the codec is what plays. */
const AUDIO_PLAYS_AS: Record<string, string> = {
  "audio/mpeg": "audio/mpeg",
  "audio/mp3": "audio/mpeg",
  "audio/mp4": "audio/mp4",
  "audio/x-m4a": "audio/mp4",
  "audio/m4a": "audio/mp4",
  "audio/aac": "audio/aac",
  "audio/ogg": "audio/ogg",
  "audio/opus": "audio/ogg",
  "audio/webm": "audio/webm",
  "audio/wav": "audio/wav",
  "audio/x-wav": "audio/wav",
  "audio/wave": "audio/wav",
  "audio/flac": "audio/flac",
  "audio/x-flac": "audio/flac",
};

/**
 * The type to put on a <source> and on the Content-Type of an inline video, or
 * null when the browser cannot play this file at all.
 *
 * The extension fallback that used to live in a second table here is gone:
 * `effectiveType` performs it for every helper in this module, and
 * `mimeFromExt("mov")` is `video/quicktime`, which the table below already maps
 * to video/mp4. One mechanism instead of two, with the same answers.
 */
export function playableVideoType(
  mime: string | null | undefined,
  ext?: string | null,
): string | null {
  // Note the deliberate asymmetry: a stored `video/*` type is trusted even when
  // it is bad news, so a .mkv renamed to .mp4 does not get to claim it plays.
  return VIDEO_PLAYS_AS[effectiveType(mime, ext)] ?? null;
}

export function playableAudioType(
  mime: string | null | undefined,
  ext?: string | null,
): string | null {
  return AUDIO_PLAYS_AS[effectiveType(mime, ext)] ?? null;
}

export interface ServeDecision {
  disposition: "inline" | "attachment";
  contentType: string;
}

/** How a raw file may be handed to the browser (see INLINE_SAFE / isTextLike). */
export function serveDecision(mime: string | null | undefined, ext?: string | null): ServeDecision {
  // The extension rescues the rows whose stored type is useless - see
  // effectiveType. The allowlist below is unchanged, so this can only ever move
  // a file from "unopenable download" towards "shown safely", never towards
  // executing in our origin.
  const m = effectiveType(mime, ext);
  if (!m || isUselessMime(m)) {
    return { disposition: "attachment", contentType: "application/octet-stream" };
  }

  // A <video> takes its cue from this header as much as from the <source type>,
  // so a .mov handed over as video/quicktime is refused by the player no matter
  // what the markup says. Both ends have to agree on the name the decoder knows.
  if (m.startsWith("video/")) {
    const playable = playableVideoType(m, ext);
    if (playable) return { disposition: "inline", contentType: playable };
  }
  if (m.startsWith("audio/")) {
    const playable = playableAudioType(m, ext);
    if (playable) return { disposition: "inline", contentType: playable };
  }

  if (INLINE_SAFE.has(m)) {
    return {
      disposition: "inline",
      contentType: m === "text/plain" ? "text/plain; charset=utf-8" : m,
    };
  }
  if (isTextLike(m)) {
    return { disposition: "inline", contentType: "text/plain; charset=utf-8" };
  }
  return { disposition: "attachment", contentType: "application/octet-stream" };
}

/**
 * Would `/raw` hand these bytes to a browser as readable text?
 *
 * Derived from `serveDecision`, never from a list of its own. That matters
 * because the preview page and the raw route had drifted apart: `.json`, `.sh`,
 * `.sql`, `.xml` and a ShareX `.sxcu` are all streamed inline as text/plain,
 * while `mediaKind` called them "other" and the page drew a download card for
 * files the very next click showed as text. Asking the serving decision itself
 * keeps the two answers identical by construction.
 *
 * It is also the guard on what a paste may be stored as (lib/media/paste-type),
 * which is why it is phrased as a question about the WIRE and not about a set
 * of type names: markup resolves to text/plain here, so it passes, and it
 * passes precisely because it can never be executed.
 */
export function isTextPayload(mime: string | null | undefined, ext?: string | null): boolean {
  const decision = serveDecision(mime, ext);
  return decision.disposition === "inline" && decision.contentType.startsWith("text/plain");
}

export type MediaKind = "image" | "video" | "audio" | "text" | "pdf" | "archive" | "other";

export function mediaKind(mime: string | null | undefined, ext?: string | null): MediaKind {
  const m = effectiveType(mime, ext);
  if (!m) return "other";
  const [type] = m.split("/");
  if (type === "image") return DISPLAYABLE_IMAGES.has(m) ? "image" : "other";
  if (type === "video") return "video";
  if (type === "audio") return "audio";
  if (type === "text") return "text";
  if (m === "application/pdf") return "pdf";
  if (/zip|gzip|tar|rar|7z-compressed/.test(m)) return "archive";
  return "other";
}

/**
 * lucide-react icon name for a given MIME (modern replacement for mime2font).
 *
 * The top-level type is asked as a PREFIX, not as a substring anywhere in the
 * string. That was not a tidy-up: `includes("image")` was the first rule in the
 * list, so `application/x-apple-diskimage` - a .dmg, a disk full of software -
 * came back as a photo. Substring matching against a taxonomy whose names
 * contain each other's words will always eventually do that.
 *
 * The names returned here are looked up in a table on the calling side, so a
 * name that table does not carry degrades to a plain file icon rather than
 * throwing. `FileSpreadsheet` is such a name today: see the note in the
 * viewer's ICONS map.
 */
export function mimeToIcon(mime: string | null | undefined, ext?: string | null): string {
  const m = effectiveType(mime, ext);
  if (!m) return "File";

  const [top] = m.split("/");
  if (top === "image") return "FileImage";
  if (top === "audio") return "FileAudio";
  if (top === "video") return "FileVideo";

  const map: [string, string][] = [
    ["application/pdf", "FileText"],
    ["msword", "FileText"],
    ["wordprocessing", "FileText"],
    ["spreadsheet", "FileSpreadsheet"],
    ["ms-excel", "FileSpreadsheet"],
    // A deck is something you read, not something you look at: it used to
    // answer `FileImage`, which put a photo icon on every .pptx. The row icons
    // file presentations under "document" too (see FAMILY_ICON), and one file
    // should not be two different things in two places.
    ["presentation", "FileText"],
    ["powerpoint", "FileText"],
    ["text/html", "FileCode"],
    ["text/x-", "FileCode"],
    ["application/json", "FileCode"],
    ["javascript", "FileCode"],
    ["text/plain", "FileText"],
    ["epub", "FileText"],
    // After the document types on purpose: .docx, .xlsx, .pptx and .epub are
    // all zip containers, and "an archive" is the least useful true thing that
    // can be said about them.
    ["zip", "FileArchive"],
    ["gzip", "FileArchive"],
    ["tar", "FileArchive"],
    ["rar", "FileArchive"],
  ];
  for (const [needle, icon] of map) {
    if (m.includes(needle)) return icon;
  }
  return "File";
}
