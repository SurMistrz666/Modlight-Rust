/**
 * The SPA's route table must agree with the Rust server's
 * (server/src/pages/table.rs): the same 28 pages (27 upstream, plus Downloads) with the same files and
 * layouts, the same 25 route handlers, and the same answer for every path the
 * server's own matching tests pin down.
 */
import { describe, expect, it } from "vitest";
import { matchRoutes } from "react-router";
import { ROUTES, SERVER_PATHS, type RouteId } from "../src/contract/routes";
import { routes } from "../src/routes/tree";
import { CATCH_ALL, SERVER_PATH } from "../src/runtime/navigation";

/** server/src/pages/table.rs, read through vite (absent outside the repository). */
const table =
  Object.values(
    import.meta.glob<string>("../../src/pages/table.rs", { query: "?raw", import: "default", eager: true }),
  )[0] ?? null;

const LAYOUT_SETS: Record<string, string[]> = {
  ROOT: ["root"],
  APP: ["root", "(app)"],
  AUTH: ["root", "(auth)"],
};

/** Next app path to react-router pattern: [x] -> :x, [...x] -> *. */
function toPattern(appPath: string): string {
  return appPath.replace(/\[\.\.\.[^\]]+\]/g, "*").replace(/\[([^\]]+)\]/g, ":$1");
}

/** What the SPA does with a path: the page id, "handler", or "none". */
function clientRoute(pathname: string): string {
  const matches = matchRoutes(routes, pathname);
  const id = matches?.[matches.length - 1]?.route.id ?? CATCH_ALL;
  if (id.startsWith(SERVER_PATH)) return "handler";
  if (id === CATCH_ALL) return "none";
  return id;
}

describe.skipIf(table === null)("route table agrees with server/src/pages/table.rs", () => {
  const source = table ?? "";
  const pages = [...source.matchAll(/^\s*page\("([^"]+)", "([^"]+)", (ROOT|APP|AUTH)\),/gm)];
  const handlers = [...source.matchAll(/^\s*handler\("([^"]+)", "([^"]+)"\),/gm)];

  it("has the same pages, files and layouts", () => {
    expect(pages.length).toBe(28);
    expect(pages.map((m) => m[1]).sort()).toEqual(Object.keys(ROUTES).sort());
    for (const [, id, file, layouts] of pages) {
      const route = ROUTES[id as RouteId];
      expect([id, route.file, route.layouts]).toEqual([id, file, LAYOUT_SETS[layouts]]);
      expect([id, route.pattern]).toEqual([id, toPattern(id)]);
    }
  });

  it("knows every route handler", () => {
    expect(handlers.length).toBe(25);
    for (const [, handlerPath] of handlers) {
      expect([handlerPath, SERVER_PATHS.includes(toPattern(handlerPath))]).toEqual([handlerPath, true]);
    }
  });

  it("matches every path the server's tests pin down the same way", () => {
    const kinds = new Map<string, "page" | "handler">([
      ...pages.map((m) => [m[1], "page"] as const),
      ...handlers.map((m) => [m[1], "handler"] as const),
    ]);
    const cases = [
      ...source.matchAll(/resolve\("([^"]*)"\)(?:\.unwrap\(\)(?:\.0)?, \(?"([^"]+)"|, (None))/g),
    ];
    expect(cases.length).toBeGreaterThan(15);
    for (const [, pathname, serverRoute, none] of cases) {
      const expected = none ? "none" : kinds.get(serverRoute) === "handler" ? "handler" : serverRoute;
      expect([pathname, clientRoute(pathname)]).toEqual([pathname, expected]);
    }
  });
});
