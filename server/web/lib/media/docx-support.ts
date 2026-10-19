/**
 * Which documents get a preview, and how big one may be.
 *
 * Split from docx.ts because that module pulls in mammoth and sharp behind a
 * `server-only` guard, and these two answers are needed by places that must stay
 * light: the render ticket (which is framework-agnostic and unit-tested), the
 * viewer deciding whether to show a toggle, and the file menu in the browser.
 */

/**
 * Largest .docx accepted for preview.
 *
 * The 90th percentile of the documents on this deployment is 2.3 MB and the
 * largest is 16 MB. 10 MB previews almost all of them while keeping the archive
 * small enough that the zip guard's verifying decompression stays cheap. The one
 * file above the line gets a clear message rather than an empty page.
 */
export const MAX_DOCX_BYTES = 10 * 1024 * 1024;

export type DocxKind = "docx" | "legacy-doc";

/**
 * Does this file get a document preview at all?
 *
 * `.doc` is the case worth naming rather than lumping in with "no". It is not a
 * ZIP and not XML - it is the OLE2 compound binary format Word used until 2007,
 * and mammoth cannot read a byte of it. There is exactly one such file on this
 * deployment, and the honest thing is for the UI to say the old format is not
 * supported rather than render an empty page and let the reader conclude the
 * document itself is empty.
 *
 * The extension is consulted alongside the type because the stored type is so
 * often useless: ShareX and curl send application/octet-stream for anything
 * Windows has no registry entry for, and an imported row keeps whatever its
 * old host guessed years ago.
 */
export function docxSupport(
  mime: string | null | undefined,
  ext: string | null | undefined,
): DocxKind | null {
  const e = (ext ?? "").replace(/^\./, "").toLowerCase();
  const m = (mime ?? "").split(";")[0].trim().toLowerCase();

  if (
    e === "docx" ||
    m === "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
  ) {
    return "docx";
  }
  if (e === "doc" || m === "application/msword") return "legacy-doc";
  return null;
}
