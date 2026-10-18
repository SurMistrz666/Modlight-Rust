/**
 * The API-token scope lattice: pure data and pure functions, no database.
 *
 * Separate from lib/api-token.ts because both a "use client" component (the
 * token picker) needs these constants, and lib/api-token.ts reaches
 * @/lib/db - importing it there would open a Postgres pool in a browser bundle.
 */

/* ────────────────────────── scopes ──────────────────────────
 *
 * Until this existed, every API token was the whole account. A token handed to
 * somebody so they could LIST a library could also empty it: the same string
 * that authenticates `GET /api/uploads` authenticates `DELETE /api/uploads`,
 * `POST /api/folders`, and `/{tag}/{code}/delete/{token}`, which deletes for
 * good. The owner could neither see what a token was allowed to do nor narrow
 * one without minting a replacement and re-configuring every client that held
 * the old one.
 *
 * Four scopes, and four is the number because each one answers a question
 * somebody actually asks, in one line:
 *
 *   upload  Put files in. Cannot see, change or delete anything.   (ShareX)
 *   read    See files, folders and links. Cannot change anything.  (a viewer)
 *   write   Everything above, plus changing and deleting.          (the app)
 *   full    Everything, including administrator endpoints.
 *
 * A fifth would have to be explained twice, and a scope nobody can explain gets
 * set wrong.
 *
 * They are deliberately NOT a rank. `upload` and `read` are incomparable —
 * neither contains the other — so every comparison below is a SUBSET test over
 * the capability sets, never `a >= b`. A rank would have had to put one of the
 * two first, and whichever it was, "narrow this token" would have silently
 * granted the other one's power. */

export const TOKEN_SCOPES = ["upload", "read", "write", "full"] as const;
export type TokenScope = (typeof TOKEN_SCOPES)[number];

/**
 * What a call needs, as opposed to what a token has.
 *
 * Every surface that accepts a token names one of these, and it is a REQUIRED
 * argument to `authenticateApiToken` on purpose: a new endpoint cannot be
 * written that forgets to say what it needs, because it would not compile.
 *
 * `identify` is the exception that proves it: every scope grants it, and exactly
 * one surface asks for it (`GET /api/me`). It exists so "any valid token" is
 * something a route STATES rather than something it achieves by leaving the
 * argument off. Answering "who am I and how much room is left" is how a client
 * verifies its own config and preflights a queue, so refusing it to an
 * upload-only token would break the test-connection button on precisely the
 * tokens we most want people to choose - and it discloses nothing the holder
 * could not already learn by uploading a byte and reading the 507.
 */
export type TokenCapability = "identify" | "upload" | "read" | "write" | "admin";

const GRANTS: Record<TokenScope, readonly TokenCapability[]> = {
  upload: ["identify", "upload"],
  read: ["identify", "read"],
  write: ["identify", "upload", "read", "write"],
  full: ["identify", "upload", "read", "write", "admin"],
};

/**
 * The narrowest scope that grants a capability — what a 403 tells the caller to
 * go and create. Answering "you need more" without saying how much more is how
 * people end up minting `full` for everything.
 */
export const SCOPE_FOR: Record<TokenCapability, TokenScope> = {
  // Every scope grants it, so `upload` is simply the narrowest of them. This
  // entry can never actually be printed in a refusal.
  identify: "upload",
  upload: "upload",
  read: "read",
  write: "write",
  admin: "full",
};

/**
 * The scope legacy rows are backfilled to, and the one nothing may be created
 * with by accident. See drizzle/0012_api_token_scopes.sql: the column default
 * exists for the ALTER and is dropped immediately afterwards, so a row that
 * fails to name a scope is a constraint violation rather than a silent `full`.
 */
export const LEGACY_TOKEN_SCOPE: TokenScope = "full";

export function isTokenScope(value: unknown): value is TokenScope {
  return typeof value === "string" && (TOKEN_SCOPES as readonly string[]).includes(value);
}

export function capabilitiesOf(scope: TokenScope): readonly TokenCapability[] {
  return GRANTS[scope];
}

export function scopeGrants(scope: TokenScope, capability: TokenCapability): boolean {
  return GRANTS[scope].includes(capability);
}

/**
 * May a token holding `current` be changed to `next` without creating a new one?
 *
 * Only when `next` is a PROPER SUBSET of `current`. Anything else is a widening,
 * and a widening performed on an existing token is a privilege escalation with
 * no new secret involved: whoever holds a leaked read-only token would otherwise
 * promote it to `write` and delete the library it was only ever meant to show.
 * Equal is refused too — it is not a narrowing, and letting it through would
 * make "the scope changed" a claim the audit trail could not rely on.
 */
export function isNarrowing(current: TokenScope, next: TokenScope): boolean {
  if (current === next) return false;
  return GRANTS[next].every((cap) => GRANTS[current].includes(cap));
}

/** Every scope a token at `current` may be narrowed to, for the UI to offer. */
export function narrowerScopes(current: TokenScope): TokenScope[] {
  return TOKEN_SCOPES.filter((s) => isNarrowing(current, s));
}
