/**
 * The plain, serializable row a public gallery tile needs.
 *
 * Deliberately narrow: public pages hand these straight to a client component,
 * so nothing that gates access (password hash, expiry, view budget, storage key)
 * may travel with them.
 */
export interface PublicTileItem {
  id: string;
  /** Short public code, the `{mediaCode}` segment of the link. */
  code: string;
  /** Lowercase extension without the dot ("png"), or "" when unknown. */
  ext: string;
  filename: string;
  mimeType: string;
  size: number;
  /** Blurhash placeholder for image tiles (null when it was never computed). */
  blurhash: string | null;
}
