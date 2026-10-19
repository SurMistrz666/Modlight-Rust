/**
 * Turning a query string into a library listing, with nothing in it the
 * database can choke on.
 *
 * Pure, and free of every server import on purpose: this is the half of
 * `GET /api/uploads` a unit test can pin, and the half where a mistake is a 500
 * rather than a wrong answer. `?perPage=abc` is the case that makes the point -
 * `Number("abc")` is NaN, `Math.max(1, NaN)` is NaN, and a `LIMIT NaN` reaches
 * Postgres as `22P02 invalid input syntax`. So every number here is checked for
 * FINITENESS before it is clamped, never after.
 *
 * The vocabularies (sort keys, file kinds) and the page ceiling live here rather
 * than in lib/data/uploads.ts because both modules need them, and the panel and
 * the desktop client must not end up with two different ideas of what "largest"
 * means or how big a page may be. uploads.ts re-exports all of it, so nothing
 * that already imported from there had to change.
 */

export type SortKey = "newest" | "oldest" | "largest" | "smallest" | "name" | "views";

export const SORT_KEYS: SortKey[] = [
  "newest",
  "oldest",
  "largest",
  "smallest",
  "name",
  "views",
];

/**
 * Coarse file kinds, derived from mimeType. No tagging by type: the
 * information lives in the MIME type already, so there is no
 * join table to keep in sync.
 */
export type FileKind = "image" | "video" | "audio" | "document" | "archive" | "paste" | "other";

export const FILE_KINDS: FileKind[] = [
  "image",
  "video",
  "audio",
  "document",
  "archive",
  "paste",
  "other",
];

export const DEFAULT_PER_PAGE = 30;

/**
 * The most rows one request may pull.
 *
 * There is an account on production carrying 8733 uploads, and a client that
 * asked for all of them would get a response measured in megabytes built from
 * 8733 rows held in memory at once - on a container with a 2 GiB ceiling that
 * two concurrent transforms already have designs on (lib/media/transform-cost.ts).
 * A caller asking for more is answered with a full page rather than an error,
 * and the response echoes the perPage that was actually USED so paging arithmetic
 * on the other end stays correct.
 */
export const MAX_PER_PAGE = 120;

/**
 * Highest page number anyone may ask for.
 *
 * A ceiling is needed for the same reason `perPage` has one, and it was missed
 * because `page` looks harmless: it is only ever multiplied. But the product is
 * what reaches Postgres - `OFFSET (page - 1) * perPage` - and node-postgres
 * serialises a parameter by calling `toString()` on it. `?page=1e30` survives
 * every check above (`Number("1e30")` is finite, and flooring it changes
 * nothing), then arrives as the literal string `"3e+31"`, which Postgres cannot
 * read as a bigint: `22P02 invalid input syntax`, thrown from inside the driver
 * and answered as a 500. Exactly the failure the `LIMIT NaN` note describes,
 * reached by a different road - finiteness is necessary and not sufficient, the
 * value also has to still be a value once it has been arithmetic on.
 *
 * A million pages is past the end of any real library (production's largest
 * account is 8733 uploads, i.e. 73 pages), and asking beyond the data simply
 * returns nothing, so the clamp costs a caller nothing it wanted.
 */
export const MAX_PAGE = 1_000_000;

/** Longest search needle we will put inside an ILIKE '%...%'. */
const MAX_SEARCH_LENGTH = 200;

export interface UploadQuery {
  page: number;
  perPage: number;
  search?: string;
  sort: SortKey;
  kind?: FileKind;
  /** List the trash instead of the library. */
  trashed: boolean;
  /**
   * The folder id EXACTLY as the client wrote it, including the "root"
   * sentinel - deliberately not validated here.
   *
   * It names a row in a `uuid` column belonging to a specific account, so the
   * only useful check is the one that needs the database and the caller's id:
   * `resolveFolderTarget` in lib/upload/folder-target.ts, which is also what the
   * two upload paths use. Half-validating it here would just add a second,
   * weaker answer to a question that already has one.
   */
  folder?: string;
}

/**
 * A whole number >= 1, or the fallback.
 *
 * An absent parameter and an empty one mean the same thing ("the client did not
 * say"), which is worth spelling out: `Number("")` is 0, so the obvious version
 * turns `?perPage=` into a one-item page instead of a default one.
 */
function positiveInt(raw: string | null, fallback: number): number {
  if (raw == null || raw.trim() === "") return fallback;
  const n = Number(raw);
  if (!Number.isFinite(n)) return fallback;
  return Math.max(1, Math.floor(n));
}

/** Read a value only when it is one this server actually understands. */
function oneOf<T extends string>(raw: string | null, allowed: T[]): T | undefined {
  return raw && (allowed as string[]).includes(raw) ? (raw as T) : undefined;
}

export function parseUploadQuery(params: URLSearchParams): UploadQuery {
  const search = params.get("search")?.trim().slice(0, MAX_SEARCH_LENGTH) || undefined;

  return {
    page: Math.min(MAX_PAGE, positiveInt(params.get("page"), 1)),
    perPage: Math.min(MAX_PER_PAGE, positiveInt(params.get("perPage"), DEFAULT_PER_PAGE)),
    search,
    // An unknown sort is the default sort, not an error: a newer client asking
    // for a key this deployment has not shipped yet should still see its files.
    sort: oneOf(params.get("sort"), SORT_KEYS) ?? "newest",
    kind: oneOf(params.get("kind"), FILE_KINDS),
    // Strict "1", the same literal the panel's own toggles use (?sub=1).
    trashed: params.get("trashed") === "1",
    folder: params.get("folderId")?.trim() || undefined,
  };
}
