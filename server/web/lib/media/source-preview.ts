/**
 * "Does this file read as source, and what readings does it have?"
 *
 * One module, because the answer used to be written down twice - once in the
 * preview page (to decide whether to read the bytes) and once in the viewer (to
 * decide what to draw) - in two conditions that were nearly, but not exactly,
 * the same. Neither matched the raw route, which is how a .json opened as a
 * download card whose "Open raw" button then showed the very same bytes as text.
 *
 * Nothing here is a new definition of "is this text". `isTextPayload` derives
 * that from `serveDecision`, the anti-XSS gate itself, and `rowMimeType` is the
 * paste rule from lib/media/paste-type. This module only composes them.
 */

import { isTextPayload } from "@/lib/media/mime";
import { rowMimeType } from "@/lib/media/paste-type";
import { normalizeExt } from "@/lib/media/sniff";
import { LANG_EXT } from "@/lib/paste-languages";

/**
 * How much of a file may be read into the page as text.
 *
 * The preview is server-rendered and the string travels in the payload, so this
 * bounds a public request rather than a private one. Over the ceiling the page
 * says the size out loud and offers the raw file instead of rendering nothing.
 */
export const MAX_INLINE_TEXT = 512 * 1024; // 512 KB

export interface SourceSubject {
  mimeType: string;
  ext?: string | null;
  /** 'file' | 'paste' - a paste's type is declared by its language. */
  kind?: string | null;
  language?: string | null;
}

/**
 * Should this file's preview show its contents rather than a download card?
 *
 * True for exactly the files `/raw` serves as text, which is the only answer
 * that cannot disagree with what the next click shows.
 */
export function showsSource(subject: SourceSubject): boolean {
  return isTextPayload(rowMimeType(subject), subject.ext);
}

/** Read the bytes, or refuse and say why. Split out so the ceiling is testable. */
export function inlineTextPlan(size: number): "read" | "too-large" {
  return size > MAX_INLINE_TEXT ? "too-large" : "read";
}

// ---------------------------------------------------------------------------
// The readings
// ---------------------------------------------------------------------------

/**
 * `source` - the contents, formatted, on this page.
 * `raw`    - the plain bytes, exactly as stored, at the raw URL.
 * `page`   - the file executed as a document, in the sandboxed frame.
 */
export type SourceMode = "source" | "raw" | "page";

export interface SourceReading {
  mode: SourceMode;
  /**
   * Where this reading lives, or null when it is rendered in place.
   *
   * `raw` carries a real URL on purpose. Making it a mode of this page would
   * mean re-printing the same bytes with the decoration turned off, which is a
   * costume rather than the raw file; the point of the button is to reach the
   * actual response, with its own Content-Type and its own headers. It opens in
   * a new tab so the reading it came from is still one click away - the preview
   * page is a public link, and sending a visitor to a text/plain dead end with
   * only the Back button as the way home is how the old "Open raw" link behaved.
   */
  href: string | null;
}

/**
 * Every reading this file has, in the order they are offered.
 *
 * Pure and separate from the component so the URLs can be asserted without a
 * DOM: a toggle that points one of its buttons at the wrong file is exactly the
 * bug a rendering test is worst at catching.
 */
export function sourceReadings(input: {
  rawUrl: string;
  /** Relative URL of the sandboxed render, ticket included. Null when refused. */
  renderPath?: string | null;
}): SourceReading[] {
  // `raw` is last because it is the only one that leaves the page. The two
  // readings that swap in place sit next to each other, so the control reads as
  // a switch with an escape hatch beside it rather than as three peers, one of
  // which silently opens a tab.
  const readings: SourceReading[] = [{ mode: "source", href: null }];
  if (input.renderPath) readings.push({ mode: "page", href: input.renderPath });
  readings.push({ mode: "raw", href: input.rawUrl });
  return readings;
}

// ---------------------------------------------------------------------------
// Naming the language
// ---------------------------------------------------------------------------

/**
 * Display names for the languages this deployment actually sees, keyed by
 * extension because that is what both an upload and a paste end up carrying
 * (`LANG_EXT` maps a paste's language onto one).
 *
 * Presentation only: nothing here decides whether a file is text, how it is
 * served, or what it may be converted to.
 */
const LANGUAGE_NAMES: Record<string, string> = {
  bash: "Shell",
  bat: "Batch",
  c: "C",
  cc: "C++",
  cfg: "INI",
  cjs: "JavaScript",
  cmd: "Batch",
  conf: "INI",
  cpp: "C++",
  cs: "C#",
  css: "CSS",
  csv: "CSV",
  diff: "Diff",
  dockerfile: "Dockerfile",
  env: "Env",
  go: "Go",
  h: "C",
  hpp: "C++",
  htm: "HTML",
  html: "HTML",
  ini: "INI",
  java: "Java",
  js: "JavaScript",
  json: "JSON",
  jsonc: "JSON",
  jsx: "JSX",
  kt: "Kotlin",
  log: "Log",
  lua: "Lua",
  markdown: "Markdown",
  md: "Markdown",
  mjs: "JavaScript",
  patch: "Diff",
  php: "PHP",
  pl: "Perl",
  ps1: "PowerShell",
  py: "Python",
  r: "R",
  rb: "Ruby",
  rs: "Rust",
  scss: "SCSS",
  sh: "Shell",
  sql: "SQL",
  srt: "Subtitles",
  svelte: "Svelte",
  svg: "SVG",
  swift: "Swift",
  sxcu: "JSON",
  tf: "Terraform",
  toml: "TOML",
  ts: "TypeScript",
  tsv: "TSV",
  tsx: "TSX",
  txt: "Text",
  vtt: "Subtitles",
  vue: "Vue",
  xhtml: "XHTML",
  xml: "XML",
  yaml: "YAML",
  yml: "YAML",
  zig: "Zig",
  zsh: "Shell",
};

/**
 * What to call the language on screen, or null when there is nothing to say.
 *
 * The fallback echoes the extension, so it is bounded the way card.ts bounds
 * its token: letters and digits only, at most eight of them. `ext` comes from a
 * filename somebody else chose, and while React escapes it on the way into the
 * page, a label is not the place to render forty characters of someone's
 * punctuation either.
 */
export function codeLanguageLabel(subject: SourceSubject): string | null {
  const fromLanguage = subject.kind === "paste" ? LANG_EXT[(subject.language ?? "").toLowerCase()] : null;
  const ext = normalizeExt(fromLanguage ?? subject.ext);
  if (!ext) return null;
  const named = LANGUAGE_NAMES[ext];
  if (named) return named;
  return /^[a-z0-9]{1,8}$/.test(ext) ? ext.toUpperCase() : null;
}
