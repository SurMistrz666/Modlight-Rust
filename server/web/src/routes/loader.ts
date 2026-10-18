/**
 * The single data loader: every navigation and every refresh asks the server
 * for the whole page (layouts + page props) in one request, then makes sure the
 * code and translations that page needs are loaded before React renders it.
 */
import { matchRoutes, redirect, type LoaderFunctionArgs, type RouteObject } from "react-router";
import type { RootLayoutData } from "../contract/layouts";
import {
  fetchPagePayload,
  isFailure,
  isNotImplemented,
  isOk,
  isRedirect,
  isReload,
  type Layouts,
  type PagePayload,
  type PageSource,
} from "../runtime/payload";
import { isSpaPath } from "../runtime/navigation";
import { acceptMessages } from "../runtime/messages";
import { setRootData } from "../runtime/root-data";
import { ensureLayout, ensurePage, isRouteId } from "./modules";

let source: PageSource = fetchPagePayload;
let lastLayouts: Partial<Layouts> = {};
let routeTable: RouteObject[] = [];

export function setPageSource(next: PageSource): void {
  source = next;
}

export function setLoaderRoutes(routes: RouteObject[]): void {
  routeTable = routes;
}

const never = () => new Promise<never>(() => {});

/** The page id react-router matched for a pathname, to start loading its code early. */
function predictRoute(pathname: string): string | null {
  const matches = matchRoutes(routeTable, pathname);
  const leaf = matches?.[matches.length - 1]?.route.id;
  return leaf ?? null;
}

export async function rootLoader({ request }: LoaderFunctionArgs): Promise<PagePayload> {
  const url = new URL(request.url);
  const predicted = predictRoute(url.pathname);
  if (isRouteId(predicted)) void ensurePage(predicted).catch(() => {});
  const payload = await source(url, request.signal);
  return settle(payload, url, request.signal);
}

/**
 * Turn a payload into something renderable: follow redirects, hand route
 * handlers to the browser, take the translations, load the code. Also used
 * for the payload the server inlined into the first document.
 */
export async function settle(payload: PagePayload, url: URL, signal?: AbortSignal): Promise<PagePayload> {
  if (isReload(payload)) {
    window.location.assign(url.href);
    return never();
  }
  if (isRedirect(payload)) {
    const target = new URL(payload.redirect, url);
    if (target.origin === window.location.origin && isSpaPath(target.pathname)) {
      throw redirect(target.pathname + target.search + target.hash, payload.status);
    }
    window.location.replace(target.href);
    return never();
  }

  // Layouts that did not come (a failed or unported loader) keep the last ones
  // this tab rendered, so an error inside the panel still shows the panel.
  const layouts = (payload as { layouts?: Partial<Layouts> }).layouts ?? {};
  if ((isFailure(payload) || isNotImplemented(payload)) && !layouts.root && lastLayouts.root) {
    layouts.root = lastLayouts.root;
  }
  const root = layouts.root as RootLayoutData | undefined;
  if (!root) {
    // Not even the root layout loaded (first request of a visit failed).
    return payload;
  }
  if (!acceptMessages(root)) {
    // The server thinks this tab holds a dictionary it does not: ask again in full.
    const again = await source(url, signal, { withoutMessages: true });
    return settle(again, url, signal);
  }
  setRootData(root);

  const loads: Promise<unknown>[] = [];
  if (layouts["(app)"]) loads.push(ensureLayout("(app)"));
  if (layouts["(auth)"]) loads.push(ensureLayout("(auth)"));
  if (isOk(payload) && isRouteId(payload.route)) loads.push(ensurePage(payload.route));
  await Promise.all(loads);

  lastLayouts = { ...layouts };
  return payload;
}
