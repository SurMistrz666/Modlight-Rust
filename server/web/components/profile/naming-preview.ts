/**
 * Client-side mirror of the code generator, used ONLY to render the live
 * preview in the profile.
 *
 * lib/naming.ts cannot be imported here: it reaches `node:crypto` (through
 * lib/ids) and would drag it into the browser bundle. The duplication is
 * deliberate and bounded — this file never allocates a real code, so it is free
 * to use `Math.random`, and the scheme list is the only thing that has to be
 * kept in step with `NAMING_SCHEMES`.
 *
 * The vocabulary itself is NOT duplicated: lib/naming-words.ts is pure data
 * with no Node dependencies, so both sides import the same words and the
 * preview cannot drift from what the server will actually produce.
 */

import {
  ADJECTIVES,
  NOUNS,
  UNAMBIGUOUS_ALPHABET,
  poolSize,
  WORD_PAIR_POOL,
} from "@/lib/naming-words";

/** Keep in step with NAMING_SCHEMES in lib/naming.ts. */
export const NAMING_SCHEME_IDS = [
  "random",
  "words",
  "random-chars",
  "filename",
  "filename-random",
  "date-filename",
  "custom",
] as const;

export type NamingSchemeId = (typeof NAMING_SCHEME_IDS)[number];

export function isNamingSchemeId(v: string | null | undefined): v is NamingSchemeId {
  return !!v && (NAMING_SCHEME_IDS as readonly string[]).includes(v);
}

/** Offered word counts / code lengths. Mirrors the clamps in lib/naming.ts. */
export const WORD_COUNTS = [2, 3, 4] as const;
export const RANDOM_LENGTHS = [5, 6, 7, 8, 10, 12] as const;

function pick<T>(list: readonly T[]): T {
  return list[Math.floor(Math.random() * list.length)];
}

function words(count: number): string {
  const n = Math.min(4, Math.max(2, Math.round(count) || 2));
  const adjectives: string[] = [];
  for (let guard = 0; adjectives.length < n - 1 && guard < 100; guard++) {
    const word = pick(ADJECTIVES);
    if (!adjectives.includes(word)) adjectives.push(word);
  }
  return [...adjectives, pick(NOUNS)].join("-");
}

function chars(length: number): string {
  const n = Math.min(32, Math.max(1, Math.round(length) || 6));
  let out = "";
  for (let i = 0; i < n; i++) out += pick(UNAMBIGUOUS_ALPHABET.split(""));
  return out;
}

/** Pronounceable filler, standing in for `humanCode` in previews. */
function human(length = 10): string {
  const consonants = "bcdfghjklmnpqrstvwxyz";
  const vowels = "aeiou";
  const digits = Math.max(0, Math.round(length * 0.2));
  let out = "";
  for (let i = 0; i < length - digits; i++) {
    out += i % 2 === 1 ? pick(vowels.split("")) : pick(consonants.split(""));
  }
  for (let i = 0; i < digits; i++) out += pick("0123456789".split(""));
  return out;
}

function two(n: number): string {
  return String(n).padStart(2, "0");
}

/** Slug of the example filename ("Screenshot 2026-07-31.png") previews use. */
const SAMPLE_SLUG = "screenshot-2026-07-31";

function expand(pattern: string, now: Date): string {
  return pattern.replace(/\{(\w+)(?::(\d+))?\}/g, (whole, token: string, arg?: string) => {
    const sized = /^(words|rand)(\d+)$/.exec(token);
    if (sized) return sized[1] === "words" ? words(Number(sized[2])) : chars(Number(sized[2]));

    switch (token) {
      case "name":
        return SAMPLE_SLUG;
      case "ext":
        return "png";
      case "date":
        return `${now.getFullYear()}-${two(now.getMonth() + 1)}-${two(now.getDate())}`;
      case "time":
        return `${two(now.getHours())}${two(now.getMinutes())}${two(now.getSeconds())}`;
      case "year":
        return String(now.getFullYear());
      case "month":
        return two(now.getMonth() + 1);
      case "day":
        return two(now.getDate());
      case "words":
        return words(arg ? Number(arg) : 2);
      case "rand":
        return chars(arg ? Number(arg) : 6);
      case "random":
        return human(arg ? Number(arg) : 6).toLowerCase();
      default:
        return whole;
    }
  });
}

/** Mirrors `sanitizeCode` in lib/naming.ts: keeps case, `_` and `.`. */
function sanitize(input: string): string {
  return input
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/[^A-Za-z0-9._-]+/g, "-")
    .replace(/-{2,}/g, "-")
    .replace(/^[-.]+|[-.]+$/g, "")
    .slice(0, 100);
}

export interface PreviewInput {
  scheme: NamingSchemeId;
  pattern: string;
  wordCount: number;
  randomLength: number;
}

/** One example code, shaped exactly like the server would build it. */
export function previewCode({ scheme, pattern, wordCount, randomLength }: PreviewInput): string {
  const now = new Date();
  const date = `${now.getFullYear()}-${two(now.getMonth() + 1)}-${two(now.getDate())}`;

  switch (scheme) {
    case "words":
      return `${words(wordCount)}.png`;
    case "random-chars":
      return `${chars(randomLength)}.png`;
    case "filename":
      return `${SAMPLE_SLUG}.png`;
    case "filename-random":
      return `${SAMPLE_SLUG}-${human(4).toLowerCase()}.png`;
    case "date-filename":
      return `${date}-${SAMPLE_SLUG}.png`;
    case "custom":
      return `${sanitize(expand(pattern, now)) || human(10)}.png`;
    case "random":
    default:
      return `${human(10)}.png`;
  }
}

/**
 * How many distinct codes the chosen settings can produce. Only the randomised
 * schemes have a finite pool worth quoting; everything else returns 0.
 *
 * Returned as a raw number rather than a formatted string so the caller can run
 * it through `Intl.NumberFormat` in the *viewer's* language — "551 mln" for a
 * Polish reader, "551M" for an English one.
 */
export function poolFor(
  scheme: NamingSchemeId,
  opts: { wordCount?: number; randomLength?: number },
): number {
  if (scheme === "words") return wordPool(opts.wordCount ?? 2);
  if (scheme === "random-chars") return poolSize(opts.randomLength ?? 6);
  return 0;
}

/** Adjectives are drawn without replacement, hence the falling factorial. */
function wordPool(count: number): number {
  const n = Math.min(4, Math.max(2, Math.round(count) || 2));
  if (n === 2) return WORD_PAIR_POOL;
  let adjectives = 1;
  for (let i = 0; i < n - 1; i++) adjectives *= ADJECTIVES.length - i;
  return adjectives * NOUNS.length;
}
