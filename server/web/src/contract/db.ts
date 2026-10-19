/**
 * Row types of the upstream tables that cross the wire whole.
 *
 * Upstream returns these from a few server actions as `db.select().from(table)`
 * results, i.e. drizzle's `typeof table.$inferSelect`. drizzle is server-only, so
 * the shapes are written out here from upstream lib/db/schema/app-schema.ts.
 * Timestamps arrive as Date (tagged JSON `{"$date": ...}`).
 *
 * @upstream lib/db/schema/app-schema.ts
 */

/** `typeof folders.$inferSelect` (lib/folders.ts `FolderRecord`). */
export interface FoldersRow {
  id: string;
  ownerId: string;
  parentId: string | null;
  name: string;
  slug: string;
  description: string | null;
  isPublic: boolean;
  coverUploadId: string | null;
  sortOrder: number;
  createdAt: Date;
}

/** `typeof shortLinks.$inferSelect`. */
export interface ShortLinksRow {
  id: string;
  ownerId: string;
  code: string;
  target: string;
  passwordHash: string | null;
  clicks: number;
  lastUsedAt: Date | null;
  expiresAt: Date | null;
  createdAt: Date;
}

/** `typeof apiTokens.$inferSelect`. */
export interface ApiTokensRow {
  id: string;
  userId: string;
  name: string;
  scope: string;
  tokenHash: string;
  preview: string;
  lastUsedAt: Date | null;
  revokedAt: Date | null;
  createdAt: Date;
}
