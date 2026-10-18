/**
 * Fixture pages: payload files under fixtures/pages/*.json, so any route can be
 * rendered with no backend (`vite --mode fixtures`, and the smoke test).
 *
 * A fixture file is { "url", "payload" } where `payload` is exactly what
 * GET /_page answers (tagged JSON). One convenience: `"layouts": "$root"`,
 * `"$app"` or `"$auth"` stands for the layout set in fixtures/layouts.json.
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
  const layouts = payload.layouts;
  if (typeof layouts === "string") {
    const all = layoutsFile as Record<string, unknown>;
    payload.layouts = Object.fromEntries((LAYOUT_SETS[layouts] ?? ["root"]).map((k) => [k, all[k]]));
  }
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
