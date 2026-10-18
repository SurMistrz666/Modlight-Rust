/**
 * The one router instance, and the Next-style navigation verbs built on it.
 *
 * next/navigation's useRouter() and next/link both end up here, so a link and
 * a programmatic push follow the same rules:
 *
 *  - another origin, or a path that is not a page of the SPA (a route handler
 *    such as /s/{code}, /{tag}/{code}/raw, /download/app): a document load, like
 *    Next's MPA fallback;
 *  - otherwise a client navigation that fetches /_page.
 */
import { matchRoutes, type RouteObject } from "react-router";
import type { createBrowserRouter } from "react-router";

type DataRouter = ReturnType<typeof createBrowserRouter>;

let router: DataRouter | null = null;
let routeTable: RouteObject[] = [];

export function installRouter(r: DataRouter | null, routes: RouteObject[]): void {
  router = r;
  routeTable = routes;
}

export function getRouter(): DataRouter | null {
  return router;
}

/** Route id of the catch-all, and the id prefix of server-only paths (routes/tree.tsx). */
export const CATCH_ALL = "catch-all";
export const SERVER_PATH = "server:";

export function isSpaPath(pathname: string): boolean {
  const matches = matchRoutes(routeTable, pathname);
  if (!matches || matches.length === 0) return false;
  const id = matches[matches.length - 1].route.id ?? "";
  return id !== CATCH_ALL && !id.startsWith(SERVER_PATH);
}

export interface NavigateOptions {
  scroll?: boolean;
}

function resolve(href: string): URL {
  return new URL(href, window.location.href);
}

function documentLoad(url: URL, replace: boolean): void {
  if (replace) window.location.replace(url.href);
  else window.location.assign(url.href);
}

export function navigate(href: string, { replace = false, scroll = true }: NavigateOptions & { replace?: boolean } = {}): void {
  const url = resolve(href);
  if (url.origin !== window.location.origin || !router || !isSpaPath(url.pathname)) {
    documentLoad(url, replace);
    return;
  }
  void router.navigate(url.pathname + url.search + url.hash, {
    replace,
    preventScrollReset: scroll === false,
  });
}

/** router.refresh(): fetch the current page's data again, keep client state. */
export function refresh(): Promise<void> {
  if (!router) return Promise.resolve();
  return router.revalidate();
}

export function back(): void {
  window.history.back();
}

export function forward(): void {
  window.history.forward();
}

/** No-op: pages are dynamic upstream (no prefetch cache worth emulating). */
export function prefetch(_href: string): void {}
