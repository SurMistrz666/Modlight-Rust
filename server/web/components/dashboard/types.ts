import type { FileKind } from "@/lib/data/uploads";

export type { FileKind };

/**
 * Whose library is on screen.
 *
 * "admin" is an administrator reviewing somebody else's files. It is not a
 * permission level (the server decides that, and it already lets an admin act
 * on any upload) but a statement about which actions are TRUTHFUL here: every
 * folder action in app/actions/folders.ts is scoped to `session.user.id`, so a
 * "Move to folder" button in this view would list the admin's own folders,
 * update zero rows and report success. A button that lies is worse than an
 * absent one, so those entries are dropped instead of disabled.
 */
export type FileViewMode = "owner" | "admin";

/**
 * Plain, serializable shape passed from server components into client UI.
 *
 * Tags are gone: organisation is folders (a file lives in exactly one) and the
 * coarse type comes from `kind`, derived from the MIME type on the server.
 */
export interface FileItem {
  id: string;
  code: string;
  filename: string;
  ext: string;
  mimeType: string;
  size: number;
  width: number | null;
  height: number | null;
  /**
   * Frames in the file (1 = still, >1 = animation, null = never counted).
   * Feeds `convertTargetsFor`, so the "Download as…" labels describe this file
   * rather than what its container is capable of.
   */
  frames: number | null;
  /**
   * Playing time and average frame rate, for audio and video.
   *
   * Carried for one reason: `frames` is a COUNT and ffprobe fails to produce
   * one on roughly one video in twenty (an MPEG-TS stream wearing an .mp4 name,
   * a WebM, a damaged MP4). Duration times rate covers exactly those, and
   * without a frame total there is no bits-per-pixel figure and therefore no
   * verdict on whether the file will play well - see lib/media/playback-health.
   */
  durationSeconds?: number | null;
  fps?: number | null;
  blurhash: string | null;
  published: boolean;
  views: number;
  downloads: number;
  /** ISO string — Dates aren't serializable across the server/client boundary. */
  createdAt: string;
  /** Coarse type derived from `mimeType` (image/video/audio/document/…). */
  kind: FileKind;
  /** Folder this file lives in, or null when it sits loose at the root. */
  folderId: string | null;
  /** Display name of `folderId`, hydrated by the page for the chip/drawer. */
  folderName: string | null;
  /** ISO string for a time-based expiry, or null when the link never expires. */
  expiresAt?: string | null;
  /** Burn-after-N view cap (1 = one-time link), or null for no limit. */
  maxViews?: number | null;
  /** Whether the public link is password-protected. */
  hasPassword?: boolean;
}

// ── Drag & drop contract ────────────────────────────────────────────────────
// Dragging a tile onto a folder in the sidebar rail moves it. The payload is
// written under two MIME types on purpose:
//
//   • UPLOAD_DND_TYPE — a private type, because `dataTransfer.getData()` is
//     blocked during `dragover`; only `dataTransfer.types` is readable, so a
//     drop target needs a distinctive type name to decide whether to light up.
//   • "application/json" — the interchange format asked for, and what any
//     non-Modlight drop target would look at.

export const UPLOAD_DND_TYPE = "application/x-modlight-uploads";
export const UPLOAD_DND_JSON = "application/json";

export interface UploadDragPayload {
  /** Upload ids being dragged — the whole selection when the grab started inside it. */
  ids: string[];
  /** Folder the drag started from, so a drop on the same folder can no-op. */
  fromFolderId: string | null;
  /** Human label for drag ghosts and drop hints ("screenshot.png" / "12 files"). */
  label: string;
}

/** Write the payload onto a `dragstart` event's dataTransfer. */
export function setUploadDragPayload(dt: DataTransfer, payload: UploadDragPayload) {
  const json = JSON.stringify(payload);
  try {
    dt.setData(UPLOAD_DND_TYPE, json);
    dt.setData(UPLOAD_DND_JSON, json);
    // Plain-text fallback so dropping onto a text field does something sane.
    dt.setData("text/plain", payload.label);
  } catch {
    /* Some browsers reject custom types in odd contexts — the drag still works. */
  }
  dt.effectAllowed = "move";
}

/**
 * True when this drag carries Modlight uploads. Safe to call from `dragover`,
 * where only the type list is exposed.
 */
export function hasUploadDrag(dt: DataTransfer | null | undefined): boolean {
  if (!dt) return false;
  return Array.from(dt.types).includes(UPLOAD_DND_TYPE);
}

/** Read the payload on `drop`. Returns null when the drag isn't ours. */
export function readUploadDragPayload(
  dt: DataTransfer | null | undefined,
): UploadDragPayload | null {
  if (!dt) return null;
  const raw = dt.getData(UPLOAD_DND_TYPE) || dt.getData(UPLOAD_DND_JSON);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as Partial<UploadDragPayload>;
    if (!Array.isArray(parsed.ids) || parsed.ids.length === 0) return null;
    return {
      ids: parsed.ids.filter((id): id is string => typeof id === "string"),
      fromFolderId: typeof parsed.fromFolderId === "string" ? parsed.fromFolderId : null,
      label: typeof parsed.label === "string" ? parsed.label : "",
    };
  } catch {
    return null;
  }
}
