/**
 * The one router instance, and the Next-style navigation verbs built on it.
 *
 * next/navigation's useRouter() and next/link both end up here, so a link and
 * a programmatic push follow the same rules:
 *
 *  - another origin, a route handler or file of the Rust server (/s/{code},
 *    /{tag}/{code}/raw, /download/app: src/contract/routes.ts SERVER_PATHS), or
 *    a URL no route matches: a document load. That is Next's fallback to a
 *    browser navigation: a route handler does not answer with RSC, and an
 *    unmatched URL answers its RSC request with a 404;
 *  - otherwise a client navigation that fetches /_page. A page that calls
 *    notFound() is rendered in place (its RSC answer is a 200 upstream).
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

/** Is this path one of the SPA's pages (a client navigation)? */
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
