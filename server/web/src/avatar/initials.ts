/**
 * The letters of an initials circle (the avatar fallback): the first two
 * characters of a name or address, upper-cased. Not upstream, which took
 * `text.slice(0, 2)`. That counts UTF-16 code units, so a name starting with a
 * letter and an emoji ("B😀b") kept half of the emoji and the circle showed a
 * replacement character, and a flag, a skin tone or a family emoji was cut out
 * of its sequence (bug hunt 2026-11-11, WEB-09). Characters here are grapheme
 * clusters (Intl.Segmenter), or code points where the browser has no Segmenter.
 */
export function initials(text: string, count = 2): string {
  return leading(text, count).toUpperCase();
}

function leading(text: string, count: number): string {
  const Segmenter = (Intl as { Segmenter?: typeof Intl.Segmenter }).Segmenter;
  if (typeof Segmenter === "function") {
    const out: string[] = [];
    for (const { segment } of new Segmenter(undefined, { granularity: "grapheme" }).segment(text)) {
      if (out.length === count) break;
      out.push(segment);
    }
    return out.join("");
  }
  return Array.from(text).slice(0, count).join("");
}
