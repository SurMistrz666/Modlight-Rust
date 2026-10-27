// Not upstream: Downloads, the desktop app's builds for macOS and Windows
// (server/src/downloads.rs, server/src/routes/api/downloads.rs; server/docs/
// SKELETON.md section 9, item 45). The page's props are what GET
// /api/downloads answers; deleting is DELETE /api/downloads/{id} with the
// session cookie (administrators only).

export type Platform = "macos" | "windows";
export type Arch = "arm64" | "x64";

export const PLATFORMS: readonly Platform[] = ["macos", "windows"];

/** One published build, as the server describes it. */
export interface Build {
  id: string;
  platform: Platform;
  arch: Arch;
  version: string;
  /** May be empty. */
  build: string;
  /** ISO 8601: when the build machine built it. */
  built: string;
  /** ISO 8601: when the server accepted it. */
  published: string;
  size: number;
  /** Lower-case hex. */
  sha256: string;
  filename: string;
  /** An administrator's name, or `publish-token` for a build machine. */
  uploader: string;
  /** May be empty; may span several lines. */
  note: string;
  /** `/downloads/{id}/{filename}` on this server. */
  url: string;
}

export interface PlatformBuilds {
  platform: Platform;
  /** Newest first. */
  builds: Build[];
}

/** GET /api/downloads, and the props of /dashboard/downloads. */
export interface DownloadsList {
  platforms: PlatformBuilds[];
  /** Builds kept per platform and architecture. */
  keep: number;
  admin: boolean;
  /** Whether RELEASE_PUBLISH_TOKEN is set; null for anyone but an administrator. */
  publishToken: boolean | null;
}

/** `uploader` of a build a build machine published. */
export const TOKEN_UPLOADER = "publish-token";

/** The chunk size the server answers a start with (and the scripts assume). */
export const CHUNK_SIZE = 52_428_800;

/**
 * The newest build of each architecture (newest first), and everything else.
 * On top of a platform's card go the first; the rest fold away below.
 */
export function splitNewest(builds: readonly Build[]): { newest: Build[]; older: Build[] } {
  const seen = new Set<string>();
  const newest: Build[] = [];
  const older: Build[] = [];
  for (const b of builds) {
    if (seen.has(b.arch)) {
      older.push(b);
    } else {
      seen.add(b.arch);
      newest.push(b);
    }
  }
  return { newest, older };
}

/** Delete a build (administrators). Throws with the server's message on failure. */
export async function deleteBuild(id: string): Promise<void> {
  const res = await fetch(`/api/downloads/${encodeURIComponent(id)}`, {
    method: "DELETE",
    credentials: "same-origin",
    headers: { Accept: "application/json" },
  });
  if (res.ok) return;
  let message = `HTTP ${res.status}`;
  try {
    const body = (await res.json()) as { error?: unknown };
    if (typeof body.error === "string" && body.error) message = body.error;
  } catch {
    // Not JSON: the status says enough.
  }
  throw new Error(message);
}
