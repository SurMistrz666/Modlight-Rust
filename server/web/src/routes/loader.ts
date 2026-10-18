/**
 * The single data loader: every navigation and every refresh asks the server
 * for the whole page (layouts + page props) in one request, then makes sure the
 * code and translations that page needs are loaded before React renders it.
 */
import { matchRoutes, redirect, type LoaderFunctionArgs, type RouteObject } from "react-router";
import type { RootLayoutData } from "../contract/layouts";
import {
  fetchPagePayload,
  isDocument,
  isFailure,
  isOk,
  isRedirect,
  type Layouts,
  type PagePayload,
  type PageSource,
} from "../runtime/payload";
import { isSpaPath } from "../runtime/navigation";
import { loadMessages } from "../runtime/messages";
import { setRootData } from "../runtime/root-data";
import { ensureLayout, ensurePage, isRouteId } from "./modules";

let source: PageSource = fetchPagePayload;
let lastLayouts: Layouts | null = null;
let routeTable: RouteObject[] = [];

export function setPageSource(next: PageSource): void {
  source = next;
}

export function setLoaderRoutes(routes: RouteObject[]): void {
  routeTable = routes;
}

/** Used only if the very first request of a visit fails before any layout loaded. */
function fallbackRoot(): RootLayoutData {
  return {
    locale: "en",
    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    mode: "system",
    skin: "discord",
    config: {
      appName: "Modlight",
      embedBrand: "Modlight",
      appUrl: window.location.origin,
      appDomains: [window.location.origin],
      shortUrl: window.location.origin,
    },
  };
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
  return settle(payload, url);
}

/**
 * Turn a payload into something renderable: follow redirects, hand non-pages to
 * the browser, load translations and code. Also used for the inlined payload of
 * the first page.
 */
export async function settle(payload: PagePayload, url: URL): Promise<PagePayload> {
  if (isDocument(payload)) {
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
  if (isFailure(payload)) {
    payload = {
      ...payload,
      layouts: { ...(lastLayouts ?? { root: fallbackRoot() }), ...stripEmpty(payload.layouts) } as Layouts,
    };
  }
  const layouts = (payload as { layouts: Partial<Layouts> }).layouts;
  const root = layouts.root ?? lastLayouts?.root ?? fallbackRoot();
  await loadMessages(root.locale);
  setRootData(root);

  const loads: Promise<unknown>[] = [];
  if (layouts["(app)"]) loads.push(ensureLayout("(app)"));
  if (layouts["(auth)"]) loads.push(ensureLayout("(auth)"));
  if (isOk(payload) && isRouteId(payload.route)) loads.push(ensurePage(payload.route));
  await Promise.all(loads);

  if (isOk(payload) || isFailure(payload)) lastLayouts = { ...(layouts as Layouts), root };
  return payload;
}

function stripEmpty(layouts: Partial<Layouts> | undefined): Partial<Layouts> {
  const out: Partial<Layouts> = {};
  if (!layouts) return out;
  for (const [k, v] of Object.entries(layouts)) {
    if (v) (out as Record<string, unknown>)[k] = v;
  }
  return out;
}
