/**
 * Fixture pages: payload files under fixtures/pages/*.json, so any route can be
 * rendered with no backend (`vite --mode fixtures`, and the smoke test).
 *
 * A fixture file is { "url", "payload" } where `payload` is what GET /_page
 * answers (tagged JSON), with two conveniences:
 * - `"layouts": "$root"`, `"$app"` or `"$auth"` stands for the layout set in
 *   fixtures/layouts.json;
 * - a root layout without `messages` gets the dictionary of its locale from
 *   server/messages (what the Rust server sends), under `messagesHash` "fixture".
 *
 * Which fixture a URL gets: `?__fixture=<file name>` when given, else the first
 * file (alphabetically) whose payload route is the route the URL matches.
 */
import { matchRoutes, type RouteObject } from "react-router";
import { decodeTagged } from "../runtime/tagged-json";
import type { PagePayload } from "../runtime/payload";
import layoutsFile from "../../fixtures/layouts.json";

export interface FixtureFile {
  url: string;
  payload: Record<string, unknown>;
}

const files = import.meta.glob<FixtureFile>("../../fixtures/pages/*.json", { eager: true, import: "default" });

/** server/messages/<locale>/<namespace>.json: the dictionaries the server would send. */
const messageFiles = import.meta.glob<Record<string, unknown>>("../../../messages/*/*.json", {
  eager: true,
  import: "default",
});

/**
 * The 17 namespaces upstream's i18n/request.ts loads for the client (mail.json
 * is server-only), plus `turnstile` (not upstream: the optional Cloudflare
 * Turnstile widget and card), `authorizePickup` (not upstream: the desktop
 * sign-in pickup) and `encryption` (not upstream: per-account file
 * encryption), as server/src/i18n/request.rs lists them.
 */
const NAMESPACES = [
  "common", "nav", "auth", "dashboard", "profile",
  "paste", "links", "albums", "tokens", "admin",
  "public", "landing", "viewer", "files", "upload",
  "download", "authorize", "turnstile", "authorizePickup", "encryption",
];

export function fixtureMessages(locale: string): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const ns of NAMESPACES) {
    const hit = Object.entries(messageFiles).find(([path]) => path.endsWith(`/messages/${locale}/${ns}.json`));
    if (hit) out[ns] = hit[1];
  }
  return out;
}

export const FIXTURES: Record<string, FixtureFile> = Object.fromEntries(
  Object.entries(files)
    .map(([path, file]) => [path.replace(/^.*\/(.+)\.json$/, "$1"), file] as const)
    .sort(([a], [b]) => a.localeCompare(b)),
);

const LAYOUT_SETS: Record<string, string[]> = {
  $root: ["root"],
  $app: ["root", "(app)"],
  $auth: ["root", "(auth)"],
};

export function expandFixture(file: FixtureFile): PagePayload {
  const payload = structuredClone(file.payload);
  if (typeof payload.layouts === "string") {
    const all = layoutsFile as Record<string, unknown>;
    payload.layouts = Object.fromEntries(
      (LAYOUT_SETS[payload.layouts] ?? ["root"]).map((k) => [k, structuredClone(all[k])]),
    );
  }
  const root = (payload.layouts as { root?: Record<string, unknown> } | undefined)?.root;
  if (root && root.messages === undefined) root.messages = fixtureMessages(String(root.locale));
  return decodeTagged<PagePayload>(payload);
}

export function routeIdFor(routes: RouteObject[], pathname: string): string | null {
  const matches = matchRoutes(routes, pathname);
  return matches?.[matches.length - 1]?.route.id ?? null;
}

export function fixtureFor(routes: RouteObject[], url: URL): FixtureFile | null {
  const named = url.searchParams.get("__fixture");
  if (named && FIXTURES[named]) return FIXTURES[named];
  const id = routeIdFor(routes, url.pathname);
  for (const file of Object.values(FIXTURES)) {
    if ((file.payload as { route?: string }).route === id) return file;
  }
  return null;
}
