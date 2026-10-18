/**
 * The page-data protocol between the SPA and the Rust server
 * (docs/CONTRACT.md "Page data").
 *
 *   GET /_page?path=<pathname>&<the page's own query string>
 *
 * answers with one of the payload shapes below. The very first page of a visit
 * does not ask: the server inlines the same payload into the HTML as
 * <script id="__MODLIGHT_PAGE__" type="application/json">.
 */
import type { RootLayoutData, AppLayoutData, AuthLayoutData } from "../contract/layouts";
import type { RouteId } from "../contract/routes";
import { decodeTagged } from "./tagged-json";
import { MASKED_SERVER_ERROR } from "./errors";

export interface HeadTag {
  tag: "meta" | "link";
  attrs: Record<string, string>;
}

/** What upstream's `metadata` / `generateMetadata` resolve to, rendered as tags. */
export interface HeadMeta {
  title: string;
  tags: HeadTag[];
}

export interface Layouts {
  root: RootLayoutData;
  "(app)"?: AppLayoutData;
  "(auth)"?: AuthLayoutData;
}

export interface PageOk<P = unknown> {
  route: RouteId;
  params: Record<string, string>;
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

export interface PageFailure {
  error: { message: string; digest?: string };
  /** Present when the failure happened below a layout whose data did load. */
  route?: RouteId;
  params?: Record<string, string>;
  layouts: Partial<Layouts> & Pick<Layouts, "root">;
  meta?: HeadMeta;
}

/** The URL is not a page of the SPA (a route handler, a file): load it as a document. */
export interface PageDocument {
  document: true;
}

export type PagePayload = PageOk | PageRedirect | PageNotFound | PageFailure | PageDocument;

export const isOk = (p: PagePayload): p is PageOk => "route" in p && "props" in p && !("error" in p);
export const isRedirect = (p: PagePayload): p is PageRedirect => "redirect" in p;
export const isNotFound = (p: PagePayload): p is PageNotFound => "notFound" in p;
export const isFailure = (p: PagePayload): p is PageFailure => "error" in p;
export const isDocument = (p: PagePayload): p is PageDocument => "document" in p;

/** The query string /_page is asked with: `path` first, then the page's own parameters. */
export function pageDataUrl(url: URL): string {
  const qs = new URLSearchParams();
  qs.append("path", url.pathname);
  for (const [k, v] of url.searchParams) qs.append(k, v);
  return `/_page?${qs.toString()}`;
}

export type PageSource = (url: URL, signal?: AbortSignal) => Promise<PagePayload>;

/** The real source: the Rust server. */
export const fetchPagePayload: PageSource = async (url, signal) => {
  let res: Response;
  try {
    res = await fetch(pageDataUrl(url), {
      method: "GET",
      headers: { Accept: "application/json", "X-Modlight-Page": "1" },
      credentials: "same-origin",
      signal,
    });
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
  return {
    error: { message: MASKED_SERVER_ERROR },
    layouts: { root: undefined as unknown as RootLayoutData },
  };
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
