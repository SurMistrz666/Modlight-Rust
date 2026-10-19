/**
 * String surgery on public codes, filenames and storage keys.
 *
 * Deliberately dependency-free - not even `@/lib/config`. Everything here is
 * needed on BOTH sides: the serving routes take URLs apart, the in-place
 * converter rewrites keys and names, and the dashboard menu that offers a
 * conversion runs in the browser. A module with no imports is one that can be
 * pulled into any of those without dragging environment parsing along, and one
 * whose invariants can be unit-tested without a database.
 */

/**
 * Strip a trailing extension to recover the bare code (codes never contain dots).
 *
 * This is what makes the extension in a public URL DECORATIVE: the lookup key is
 * the code alone, so /{tag}/{code}.mp4, /{tag}/{code}.gif and /{tag}/{code} all
 * address the same row. In-place conversion rests entirely on it - the file
 * changes format, the code does not, so every link already in circulation keeps
 * working with no alias table to maintain, and the code can never be handed to
 * some other file because it belongs to this row for as long as the row exists.
 */
export function codeFromMediaCode(mediaCode: string): string {
  return decodeURIComponent(mediaCode).split(".")[0];
}

/**
 * Replace the extension of a storage key, keeping its directory prefix.
 *
 * Derived from the STORED key rather than rebuilt with `storageKeyFor`, and that
 * is the whole point: the prefix is the owner's tag AS IT WAS when the file was
 * uploaded, and a user may have changed their tag since (user_code_aliases).
 * Rebuilding from the current tag would quietly move the object to a different
 * directory on disk; swapping the extension leaves it exactly where it is.
 */
export function swapKeyExt(key: string, ext: string): string {
  const { dir, name } = splitKey(key);
  const base = stripExt(name);
  return ext ? `${dir}${base}.${ext}` : `${dir}${base}`;
}

/**
 * Replace the extension of a display filename.
 *
 * "wakacje.2024.mp4" becomes "wakacje.2024.gif": only the LAST segment is an
 * extension, and a name with no extension at all simply gains one.
 */
export function swapFilenameExt(filename: string, ext: string): string {
  const base = stripExt(filename) || filename;
  return ext ? `${base}.${ext}` : base;
}

/**
 * Insert a short suffix before the extension: "a/b.png" -> "a/b-x1y2.png".
 *
 * Used only when the key a conversion wants is already occupied - the leftover
 * of an earlier attempt that wrote its bytes and then failed before the commit.
 * Writing beside it is always safe; overwriting it would not be, because that
 * object may still be the one a row points at.
 */
export function suffixKey(key: string, suffix: string): string {
  const { dir, name } = splitKey(key);
  const dot = name.lastIndexOf(".");
  return dot > 0
    ? `${dir}${name.slice(0, dot)}-${suffix}${name.slice(dot)}`
    : `${dir}${name}-${suffix}`;
}

function splitKey(key: string): { dir: string; name: string } {
  const slash = key.lastIndexOf("/");
  return slash >= 0
    ? { dir: key.slice(0, slash + 1), name: key.slice(slash + 1) }
    : { dir: "", name: key };
}

/** Drop a trailing ".ext" (never a leading dot, so ".gitignore" survives whole). */
function stripExt(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(0, dot) : name;
}
