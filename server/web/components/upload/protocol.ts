/**
 * The tiny contract shared between the browser and the tus server, kept in a
 * module of its own so a consumer can import a constant without dragging the
 * whole Uppy runtime (~90 kB) into its bundle.
 *
 * The server half lives in `lib/upload/tus.ts`, which cannot import this file:
 * it is a server module and this one sits under `components/`. The two string
 * literals are therefore mirrored there, with a comment pointing back here —
 * changing one means changing both.
 */

/**
 * Sentinel meaning "put this file in no folder at all".
 *
 * Folder ids are UUIDs, so this can never collide with one. It has to exist
 * because an ABSENT `folderId` already means something different: fall back to
 * the account's default folder, exactly like a ShareX upload does.
 */
export const ROOT_FOLDER_META = "root";

/** Metadata key carrying the drop target from the browser to `onUploadCreate`. */
export const FOLDER_META_KEY = "folderId";

/** Response header the tus finalize step uses to hand back the public URL. */
export const MEDIA_URL_HEADER = "X-Media-Url";

/**
 * Response header carrying the link a client should put on the clipboard,
 * already resolved from the account's `copy_raw` preference.
 *
 * Unused by the browser, which navigates rather than copies, but mirrored here
 * because this file is the contract: the desktop companions read it off the
 * resumable path, and its single-POST twin is the `copy_url` field. Both routes
 * must answer alike, or one file copies two different ways depending on which
 * endpoint its size happened to select.
 */
export const MEDIA_COPY_URL_HEADER = "X-Media-Copy-Url";
