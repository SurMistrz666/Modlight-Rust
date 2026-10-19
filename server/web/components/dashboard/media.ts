import { createElement } from "react";
import {
  AlignLeft,
  AppWindow,
  Archive,
  Binary,
  BookText,
  ClipboardType,
  Code,
  File,
  FileArchive,
  FileAudio,
  FileCode,
  FileDigit,
  FileImage,
  FileSpreadsheet,
  FileText,
  FileType2,
  FileVideo,
  Film,
  Image as ImageIcon,
  Music,
  Package,
  Table2,
  Type,
  type LucideIcon,
} from "lucide-react";
import { effectiveType, isDisplayableImage, playableVideoType } from "@/lib/media/mime";
import type { FileKind } from "./types";

/** Client-side mirror of the proven media URL scheme (no server imports). */
export function buildPublicUrl(appUrl: string, userCode: string, code: string, ext: string) {
  return ext ? `${appUrl}/${userCode}/${code}.${ext}` : `${appUrl}/${userCode}/${code}`;
}

/**
 * Where "Open" and a double click send the owner: the SAME origin the panel is
 * being read on, deliberately relative.
 *
 * This is not a cosmetic difference from `buildPublicUrl`. The deployment
 * answers on several hostnames and an account may hand out links on any of them
 * (`preferredDomain`), but the session cookie is host-only - Better Auth sets no
 * Domain attribute, which is what keeps it off every other app on the registrable
 * domain. Sending the owner to their preferred host therefore arrived as an
 * anonymous visitor: no session, so `isPrivilegedViewer` was false, so every
 * encoder-backed target (a video's GIF and MP3, an AVIF sequence's GIF) vanished
 * from the page's menu while the right-click menu in the panel still showed them.
 * That was the reported mismatch, and a relative link is its whole cure.
 *
 * Copied links keep using `buildPublicUrl` with the owner's chosen hostname:
 * that is what the clipboard and ShareX should read.
 */
export function buildOpenUrl(userCode: string, code: string, ext: string) {
  return ext ? `/${userCode}/${code}.${ext}` : `/${userCode}/${code}`;
}

export function buildRawUrl(appUrl: string, userCode: string, code: string, ext: string) {
  return ext ? `${appUrl}/${userCode}/${code}/raw.${ext}` : `${appUrl}/${userCode}/${code}/raw`;
}

export function buildDownloadUrl(appUrl: string, userCode: string, code: string) {
  return `${appUrl}/${userCode}/${code}/download`;
}

/**
 * Every type question takes the extension too.
 *
 * A tile whose row says `application/octet-stream` (ShareX on Windows never
 * learned what a .avif is) would otherwise get an icon instead of the picture it
 * is. `effectiveType` only consults the name when the stored type says nothing.
 */
export function isImage(mime: string, ext?: string | null) {
  return effectiveType(mime, ext).startsWith("image/");
}

export function isVideo(mime: string, ext?: string | null) {
  return effectiveType(mime, ext).startsWith("video/");
}

export function isAudio(mime: string, ext?: string | null) {
  return effectiveType(mime, ext).startsWith("audio/");
}

export function isGif(mime: string, ext?: string | null) {
  return effectiveType(mime, ext) === "image/gif";
}

/**
 * Can this tile actually draw a picture of the file, or only a mark for it?
 *
 * Asked of the two helpers that decide the real answer, not of `isImage` /
 * `isVideo`. The difference is the whole point of the type badge: `image/heic`
 * and `image/vnd.adobe.photoshop` pass `isImage`, but `mediaKind` refuses them a
 * thumbnail, so the tile fetched the original and got an <img> the browser
 * cannot decode. A .mkv passes `isVideo`, but no browser ships a demuxer for
 * Matroska, so the <video> painted nothing. Both produced the same blank grey
 * rectangle the owner complained about.
 */
export function hasVisualPreview(mime: string, ext?: string | null) {
  return canShowImage(mime, ext) || canPlayVideo(mime, ext);
}

/** True when the thumbnail endpoint will rasterize this (see `mediaKind`). */
export function canShowImage(mime: string, ext?: string | null) {
  return isDisplayableImage(mime, ext);
}

/** True when a browser can decode this container at all (see `VIDEO_PLAYS_AS`). */
export function canPlayVideo(mime: string, ext?: string | null) {
  return playableVideoType(mime, ext) !== null;
}

const KINDS = new Set<string>([
  "image",
  "video",
  "audio",
  "document",
  "archive",
  "paste",
  "other",
]);

const ARCHIVE_MIMES = new Set([
  "application/zip",
  "application/x-7z-compressed",
  "application/vnd.rar",
  "application/x-rar-compressed",
  "application/gzip",
  "application/x-tar",
  "application/x-bzip2",
  "application/x-xz",
  "application/zstd",
]);

const DOCUMENT_MIMES = new Set([
  "application/pdf",
  "application/msword",
  "application/vnd.ms-excel",
  "application/vnd.ms-powerpoint",
  "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
  "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
  "application/vnd.openxmlformats-officedocument.presentationml.presentation",
  "application/vnd.oasis.opendocument.text",
  "application/vnd.oasis.opendocument.presentation",
  "application/rtf",
  "application/epub+zip",
]);

/**
 * Rows and columns: the files a spreadsheet or a query opens, as opposed to the
 * files you read. Claimed BEFORE `DOCUMENT_MIMES`, which also lists the two
 * Excel types - a spreadsheet is a document, but it is the more specific answer.
 */
const DATA_MIMES = new Set([
  "text/csv",
  "text/tab-separated-values",
  "application/vnd.ms-excel",
  "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
  "application/vnd.oasis.opendocument.spreadsheet",
  "application/vnd.sqlite3",
]);

/**
 * Source and markup. The `text/x-*` prefix carries most of it (that is where
 * sniff.ts files .py/.lua/.php/.c), so only the types outside that namespace
 * are spelled out here.
 */
const CODE_MIMES = new Set([
  "text/html",
  "text/css",
  "text/javascript",
  "text/xml",
  "application/xhtml+xml",
  "application/javascript",
  "application/x-javascript",
  "application/json",
  "application/xml",
  "application/x-sh",
  "application/sql",
  "application/jwt",
  "application/wasm",
]);

/**
 * Programs, not documents. `.apk` and `.jar` are zip containers underneath, but
 * "an app" is the answer a person is looking for when they see the tile.
 */
const APP_MIMES = new Set([
  "application/vnd.microsoft.portable-executable",
  "application/x-msdownload",
  "application/x-msi",
  "application/vnd.android.package-archive",
  "application/java-archive",
  "application/vnd.debian.binary-package",
  "application/x-rpm",
  "application/x-apple-diskimage",
]);

// ---------------------------------------------------------------------------
// Type families - what the tile's badge is made of
// ---------------------------------------------------------------------------

/**
 * The families a file can belong to, finer than `FileKind`.
 *
 * `FileKind` exists to drive a SQL filter, so it is deliberately coarse: a PDF,
 * a spreadsheet, a shell script and a README are one bucket called "document".
 * That is fine for a dropdown and useless on a tile, where the whole job is
 * telling those four apart at a glance. This is the display axis, and it is the
 * only one: `kindIcon` and `kindTint` below resolve through it too, so a PDF is
 * the same red in the grid, in the list, in the trash and in the duplicate
 * finder rather than three different greys.
 */
export type TypeFamily =
  | "image"
  | "video"
  | "audio"
  | "pdf"
  | "document"
  | "data"
  | "code"
  | "archive"
  | "font"
  | "app"
  | "binary";

/**
 * The family a file belongs to.
 *
 * Everything is decided by `effectiveType`, which consults the extension only
 * when the stored type says nothing - so a .avif that ShareX filed as
 * `application/octet-stream` is an image here, while a .txt renamed to .zip is
 * still whatever its stored type says it is. There is no extension table in this
 * module on purpose: lib/media/sniff.ts owns the one table, and a second copy
 * would drift the day someone adds a format to only one of them.
 */
export function typeFamily(mime: string | null | undefined, ext?: string | null): TypeFamily {
  const m = effectiveType(mime, ext);
  if (!m) return "binary";
  const [top] = m.split("/");

  if (top === "image") return "image";
  if (top === "video") return "video";
  if (top === "audio") return "audio";
  if (top === "font") return "font";
  if (m === "application/pdf") return "pdf";

  // Before DOCUMENT_MIMES, which also lists the two Excel types: a spreadsheet
  // is a document, but "rows and columns" is the more specific answer.
  if (DATA_MIMES.has(m)) return "data";
  if (CODE_MIMES.has(m) || m.startsWith("text/x-") || m.endsWith("+xml") || m.endsWith("+json")) {
    return "code";
  }
  if (ARCHIVE_MIMES.has(m)) return "archive";
  if (APP_MIMES.has(m)) return "app";
  // `application/x-subrip` is subtitles: text a person reads, filed nowhere else.
  if (top === "text" || DOCUMENT_MIMES.has(m) || m === "application/x-subrip") return "document";
  return "binary";
}

/** The family a coarse `FileKind` implies, for callers that hold no MIME type. */
function familyOfKind(kind: FileKind): TypeFamily {
  switch (kind) {
    case "image":
      return "image";
    case "video":
      return "video";
    case "audio":
      return "audio";
    case "archive":
      return "archive";
    // A paste is text somebody typed; without its MIME type that is all we know.
    case "paste":
    case "document":
      return "document";
    default:
      return "binary";
  }
}

/**
 * Plate colour per family, as an opaque hex.
 *
 * Opaque is the entire trick, and it is why this table is hardcoded rather than
 * token-driven. A badge must stay legible on fourteen skins whose `--card`
 * ranges from white through Windows 98 silver (#d4d0c8) to near-black - and a
 * tinted LABEL cannot: amber-600 text reads 2.2:1 on win98's silver card and
 * 12:1 on Late Night's. An opaque plate removes the skin from the equation
 * entirely, so the only ratio left to guarantee is white-on-plate, which is a
 * constant. Every value below is a Tailwind 600/700 shade measured at 5.0:1 or
 * better against white, and tests/unit/type-badge.test.ts recomputes them.
 *
 * The plate is the ONLY hardcoded colour in the badge. Everything around it -
 * the tile, the caption under the mark, the chip laid over an animated preview -
 * uses tokens, so the mark reads as a logo sitting on the skin rather than as a
 * hole punched through it.
 */
export const FAMILY_PLATE: Record<TypeFamily, string> = {
  image: "#6d28d9", // violet-700
  video: "#be123c", // rose-700
  audio: "#b45309", // amber-700
  pdf: "#b91c1c", // red-700
  document: "#0369a1", // sky-700
  data: "#047857", // emerald-700
  code: "#4338ca", // indigo-700
  archive: "#c2410c", // orange-700
  font: "#a21caf", // fuchsia-700
  app: "#475569", // slate-600
  binary: "#52525b", // zinc-600
};

/**
 * Glyph per family for a STANDALONE icon: a list row, the details drawer, the
 * duplicate finder. These are the file-shaped lucide icons, because in those
 * places the icon is the only thing saying "this is a file".
 */
export const FAMILY_ICON: Record<TypeFamily, LucideIcon> = {
  image: FileImage,
  video: FileVideo,
  audio: FileAudio,
  pdf: FileType2,
  document: FileText,
  data: FileSpreadsheet,
  code: FileCode,
  archive: FileArchive,
  font: Type,
  app: Package,
  binary: FileDigit,
};

/**
 * Glyph per family for the badge, whose PLATE is already a sheet of paper.
 *
 * A second table for the same axis, and deliberately so: putting `FileImage`
 * inside the badge draws a small file outline inside a large file outline, which
 * reads as a rendering mistake rather than as a type. Inside the plate the
 * emblem has to be the subject alone - a frame, a note, a bracket - the way a
 * real file-type icon carries a symbol on the sheet rather than another sheet.
 * Both tables are keyed by the same family, so they cannot disagree about what
 * a file IS, only about how it is drawn.
 */
export const FAMILY_EMBLEM: Record<TypeFamily, LucideIcon> = {
  image: ImageIcon,
  video: Film,
  audio: Music,
  pdf: BookText,
  document: AlignLeft,
  data: Table2,
  code: Code,
  archive: Archive,
  font: Type,
  app: AppWindow,
  binary: Binary,
};

/**
 * Soft tint per family, for the small icon plates in list rows and the drawer.
 *
 * Fixed hues at 12% alpha rather than tokens, for the same reason the tints they
 * replace always used them: these are category colours, like a folder label, and
 * a wash that thin sits legibly on any surface. Only a GLYPH is drawn in the
 * full-strength colour, never running text, so the bar is the 3:1 asked of
 * non-text rather than the 4.5:1 asked of a label. `binary` stays on tokens
 * because "we have nothing to say about this file" has no category colour.
 */
const FAMILY_TINT: Record<TypeFamily, string> = {
  image: "bg-violet-500/12 text-violet-600 dark:text-violet-400",
  video: "bg-rose-500/12 text-rose-600 dark:text-rose-400",
  audio: "bg-amber-500/12 text-amber-700 dark:text-amber-400",
  pdf: "bg-red-500/12 text-red-600 dark:text-red-400",
  document: "bg-sky-500/12 text-sky-700 dark:text-sky-400",
  data: "bg-emerald-500/12 text-emerald-700 dark:text-emerald-400",
  code: "bg-indigo-500/12 text-indigo-600 dark:text-indigo-400",
  archive: "bg-orange-500/12 text-orange-700 dark:text-orange-400",
  font: "bg-fuchsia-500/12 text-fuchsia-700 dark:text-fuchsia-400",
  app: "bg-slate-500/12 text-slate-700 dark:text-slate-300",
  binary: "bg-muted text-muted-foreground",
};

/**
 * Client-side mirror of `kindOf` in lib/data/uploads (which is server-only).
 * Rows carry `kind` already; this covers surfaces that only have a MIME type.
 */
export function kindFromMime(mime: string, ext?: string | null): FileKind {
  const m = effectiveType(mime, ext);
  if (m.startsWith("image/")) return "image";
  if (m.startsWith("video/")) return "video";
  if (m.startsWith("audio/")) return "audio";
  if (m.startsWith("text/") || DOCUMENT_MIMES.has(m)) return "document";
  if (ARCHIVE_MIMES.has(m)) return "archive";
  return "other";
}

/**
 * Accepts either a `FileKind` or a raw MIME type, so callers that only hold one
 * of the two don't have to classify first.
 */
function resolveKind(kindOrMime: string): FileKind {
  if (KINDS.has(kindOrMime)) return kindOrMime as FileKind;
  return kindFromMime(kindOrMime);
}

/**
 * The family behind a call that may carry a kind, a MIME type, or both.
 *
 * A MIME type always wins, because it is the finer fact: `kind` cannot tell a
 * PDF from a shell script. When only a kind is on hand, `familyOfKind` degrades
 * to what that kind implies rather than inventing a family.
 */
function resolveFamily(kindOrMime: string, mime = ""): TypeFamily {
  const hint = mime || (kindOrMime.includes("/") ? kindOrMime : "");
  if (hint) return typeFamily(hint);
  return familyOfKind(resolveKind(kindOrMime));
}

/**
 * Icon per file, resolved through the family axis.
 *
 * The chain of regexes this replaced ("does the MIME contain the word
 * spreadsheet") answered the same question `typeFamily` answers, in a second
 * place and with different results - it had no idea a .woff2 or an .exe was
 * anything but a nameless `File`. `paste` is checked first and keeps its own
 * glyph: a snippet is a thing a person made here, not a format.
 */
export function kindIcon(kindOrMime: FileKind | string, mime = ""): LucideIcon {
  if (resolveKind(kindOrMime) === "paste") return ClipboardType;
  return FAMILY_ICON[resolveFamily(kindOrMime, mime)] ?? File;
}

/**
 * Tint per file. Same family axis as the icon and the badge plate, so one file
 * carries one colour everywhere it appears.
 */
export function kindTint(kindOrMime: FileKind | string, mime = ""): string {
  // A paste keeps a colour of its own for the same reason it keeps its glyph.
  if (resolveKind(kindOrMime) === "paste") {
    return "bg-emerald-500/12 text-emerald-700 dark:text-emerald-400";
  }
  return FAMILY_TINT[resolveFamily(kindOrMime, mime)] ?? FAMILY_TINT.binary;
}

/**
 * Renders the icon for a kind. A component rather than a bare `kindIcon(...)`
 * lookup assigned to a local `Icon` variable: binding a component inside a
 * render body makes React treat it as a freshly created type on every pass.
 */
export function KindIcon({
  kind,
  mime,
  className,
}: {
  kind: FileKind | string;
  mime?: string;
  className?: string;
}) {
  return createElement(kindIcon(kind, mime ?? ""), { className });
}

/** Short uppercase type label for tiles/rows ("PNG", "MP4", "PDF"). */
export function extLabel(item: { ext: string; mimeType: string }): string {
  if (item.ext) return item.ext.toUpperCase();
  const sub = item.mimeType.split("/")[1] ?? "";
  return sub.split(/[.+;]/)[0]?.toUpperCase() || "FILE";
}
