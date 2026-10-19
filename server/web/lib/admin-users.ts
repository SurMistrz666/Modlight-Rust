/**
 * Shape of a row in Admin → Users.
 *
 * Lives outside the server-action module so the client table can `import type`
 * it without dragging a `"use server"` boundary into the bundle.
 */
export interface AdminUserRow {
  id: string;
  name: string;
  email: string;
  role: string | null;
  banned: boolean;
  /** ISO timestamp — server actions may only hand plain data to the client. */
  createdAt: string | null;
  /** `user.userCode` — the first segment of every public URL. */
  tag: string | null;
  /**
   * Resolved avatar URL, or null when this account still shows initials.
   *
   * A URL rather than the raw `user.image` column, which holds a storage key:
   * resolving it needs the tag, and doing it here means the table never has to
   * know how avatars are stored.
   */
  avatar: string | null;
  /** Bytes accounted against the quota (trash included — it still occupies disk). */
  usedQuota: number;
  /** Byte limit; `-1` = unlimited. */
  maxQuota: number;
  /** Live uploads (trash excluded). */
  files: number;
  /**
   * May this account mint short links on the bare domain (`/{code}`) instead of
   * under its own tag? Off by default — the root namespace is shared with every
   * top-level route.
   */
  allowRootShortLinks: boolean;
  /** Has this address ever been confirmed? Drives the "Unconfirmed" badge. */
  emailVerified: boolean;
  /**
   * A mailed invitation is outstanding: the account exists but has never had a
   * password set. Distinct from "no password", which is also true for accounts
   * imported without a usable hash.
   */
  invitePending: boolean;
}

/**
 * Everything the edit form needs, on top of what the table shows.
 *
 * Deliberately a separate type loaded per account rather than extra columns on
 * every listed row: the dialog reads it fresh when it opens (see
 * `loadUserDetail` in app/actions/admin.ts), so an operator is never editing a
 * snapshot taken when the tab was opened half an hour ago.
 *
 * Note what is NOT here: the password. There is nothing to show — only a scrypt
 * hash exists, and a form that could display a password would mean the server
 * was storing one.
 */
export interface AdminUserDetail extends AdminUserRow {
  /** Login name, usable instead of the email at sign-in. May be unset. */
  username: string | null;
  twoFactorEnabled: boolean;
  /** Opt-in public gallery at /u/{tag}. */
  publicProfile: boolean;
  /** Generate links for NEW uploads without the tag segment. */
  skipTagInUrl: boolean;
  /** Whether a credential (email + password) login exists at all. */
  hasPassword: boolean;
}

/**
 * Login names accepted at sign-in. Same shape as a tag minus the reserved-word
 * list: a username is never a URL segment, so it cannot shadow a route.
 */
export const USERNAME_PATTERN = /^[a-zA-Z0-9][a-zA-Z0-9._-]{1,31}$/;

/** Loose enough to accept every address that exists, strict enough to catch typos. */
export const EMAIL_PATTERN = /^[^\s@]+@[^\s@]+\.[^\s@]{2,}$/;

/** Better Auth's default bounds (`emailAndPassword.minPasswordLength` = 8). */
export const PASSWORD_MIN = 8;
export const PASSWORD_MAX = 128;
