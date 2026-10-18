/**
 * The page-data protocol between the SPA and the Rust server
 * (docs/CONTRACT.md section 4, server/src/pages/mod.rs).
 *
 *   GET /_page?path=<encodeURIComponent(pathname)>&<the page's own query string>
 *   X-Modlight-Messages: <locale>:<messagesHash>      (once a dictionary is held)
 *
 * answers one of the payload shapes below. The very first page of a visit does
 * not ask: the server inlines the same payload into the HTML as
 * <script id="__MODLIGHT_PAGE__" type="application/json">.
 */
import type { RootLayoutData, AppLayoutData, AuthLayoutData } from "../contract/layouts";
import type { RouteId } from "../contract/routes";
import { decodeTagged } from "./tagged-json";
import { MASKED_SERVER_ERROR } from "./errors";
import { messagesHeader } from "./messages";

/**
 * The resolved head of the page (server/src/pages/meta.rs `ResolvedHead`): what
 * the SPA updates on a client navigation. Open Graph, Twitter, alternates and
 * icons are rendered into the first document only (bots read nothing else).
 */
export interface HeadMeta {
  title: string;
  description?: string | null;
  applicationName?: string | null;
  /** "noindex, nofollow" etc. */
  robots?: string | null;
}

export interface Layouts {
  root: RootLayoutData;
  "(app)"?: AppLayoutData;
  "(auth)"?: AuthLayoutData;
}

export interface PageOk<P = unknown> {
  route: RouteId;
  params: Record<string, string | string[]>;
  layouts: Layouts;
  props: P;
  meta: HeadMeta;
}

export interface PageRedirect {
  redirect: string;
  status: number;
}

export interface PageNotFound {
  notFound: true;
  layouts: Pick<Layouts, "root">;
  meta: HeadMeta;
}

/** A loader that failed (500). `layouts` holds those that loaded before it. */
export interface PageFailure {
  error: { message: string; digest?: string };
  layouts: Partial<Layouts>;
}

/** The URL is a route handler or a static file: load it as a document. */
export interface PageReload {
  reload: true;
}

/** 501: the Rust server has no loader for this page or one of its layouts yet. */
export interface PageNotImplemented {
  notImplemented: true;
  route: RouteId;
  /** The upstream file whose loader is missing (a layout's or the page's). */
  file: string;
  layouts: Partial<Layouts>;
}

export type PagePayload = PageOk | PageRedirect | PageNotFound | PageFailure | PageReload | PageNotImplemented;

export const isOk = (p: PagePayload): p is PageOk => "route" in p && "props" in p;
export const isRedirect = (p: PagePayload): p is PageRedirect => "redirect" in p;
export const isNotFound = (p: PagePayload): p is PageNotFound => "notFound" in p;
export const isFailure = (p: PagePayload): p is PageFailure => "error" in p;
export const isReload = (p: PagePayload): p is PageReload => "reload" in p;
export const isNotImplemented = (p: PagePayload): p is PageNotImplemented => "notImplemented" in p;

/** `/_page?path=<encodeURIComponent(pathname)>&<query>`: the query passes through as it is. */
export function pageDataUrl(url: URL): string {
  const query = url.search.length > 1 ? `&${url.search.slice(1)}` : "";
  return `/_page?path=${encodeURIComponent(url.pathname)}${query}`;
}

export type PageSource = (url: URL, signal?: AbortSignal, opts?: { withoutMessages?: boolean }) => Promise<PagePayload>;

/** The real source: the Rust server. */
export const fetchPagePayload: PageSource = async (url, signal, opts) => {
  const headers: Record<string, string> = { Accept: "application/json" };
  const held = opts?.withoutMessages ? null : messagesHeader();
  if (held) headers["X-Modlight-Messages"] = held;
  let res: Response;
  try {
    res = await fetch(pageDataUrl(url), { method: "GET", headers, credentials: "same-origin", signal });
  } catch (err) {
    if (signal?.aborted) throw err;
    return networkFailure();
  }
  const type = res.headers.get("content-type") ?? "";
  if (!type.includes("application/json")) return networkFailure();
  try {
    return decodeTagged<PagePayload>(await res.json());
  } catch {
    return networkFailure();
  }
};

function networkFailure(): PageFailure {
  // Rendered by the nearest error boundary, like a failed RSC fetch upstream.
  return { error: { message: MASKED_SERVER_ERROR }, layouts: {} };
}

/** The payload the server inlined into the HTML, if any (read once). */
export function readInlinePayload(): PagePayload | null {
  const el = document.getElementById("__MODLIGHT_PAGE__");
  if (!el || !el.textContent) return null;
  try {
    return decodeTagged<PagePayload>(JSON.parse(el.textContent));
  } catch {
    return null;
  }
}
