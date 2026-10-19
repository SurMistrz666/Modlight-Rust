/**
 * The root route: app/layout.tsx (providers), app/error.tsx (root error
 * boundary), app/not-found.tsx, plus what Next does around them in the browser:
 * <html> attributes, <head> metadata, scroll restoration.
 */
import { useEffect, useRef } from "react";
import { Outlet, ScrollRestoration, useLoaderData, useRouteError } from "react-router";
import GlobalRouteError from "@/app/error";
import RootLayout from "../pages/layout";
import NotFound from "../pages/not-found";
import type { RootLayoutData } from "../contract/layouts";
import { isNotFound, isNotImplemented, type HeadMeta, type PagePayload } from "../runtime/payload";
import { messagesFor } from "../runtime/messages";
import { SegmentErrorBoundary } from "./boundary";
import { NotImplemented } from "./not-implemented";

export function usePayload(): PagePayload {
  return useLoaderData() as PagePayload;
}

export function rootDataOf(payload: PagePayload): RootLayoutData | undefined {
  return (payload as { layouts?: { root?: RootLayoutData } }).layouts?.root;
}

/**
 * React writes <html> attributes only when the rendered value changes, and the
 * first values came with the document. So: nothing on mount, then exactly the
 * attributes whose layout value changed (a language or skin switch followed by
 * router.refresh(), for instance).
 */
function useHtmlAttributes(root: RootLayoutData | undefined): void {
  const previous = useRef<RootLayoutData | null>(null);
  useEffect(() => {
    if (!root) return;
    const before = previous.current;
    previous.current = root;
    if (!before) return;
    const html = document.documentElement;
    if (before.locale !== root.locale) html.setAttribute("lang", root.locale);
    if (before.mode !== root.mode) html.setAttribute("data-mode", root.mode);
    if (before.theme !== root.theme) html.setAttribute("data-theme", root.theme);
  }, [root]);
}

/** The metadata tags a page navigation removes (they describe the first page only). */
const STALE_ON_NAVIGATION = [
  'meta[property^="og:"]',
  'meta[name^="twitter:"]',
  'link[rel="canonical"]',
  'link[rel="alternate"]',
].join(",");

function setNamedMeta(name: string, content: string | null | undefined): void {
  let el = document.head.querySelector<HTMLMetaElement>(`meta[name="${name}"]`);
  if (content === null || content === undefined || content === "") {
    el?.remove();
    return;
  }
  if (!el) {
    el = document.createElement("meta");
    el.setAttribute("name", name);
    document.head.appendChild(el);
  }
  if (el.getAttribute("content") !== content) el.setAttribute("content", content);
}

/**
 * Next swaps the page's <title> and metadata on every navigation. The payload
 * carries the resolved title, description, application name and robots; the
 * Open Graph / Twitter / alternate tags of the first document describe that
 * first page only, so they go when another page shows.
 */
function useHead(meta: HeadMeta | undefined): void {
  const first = useRef(true);
  useEffect(() => {
    if (!meta) return;
    if (document.title !== meta.title) document.title = meta.title;
    setNamedMeta("description", meta.description);
    setNamedMeta("application-name", meta.applicationName);
    setNamedMeta("robots", meta.robots);
    if (first.current) {
      first.current = false;
      return;
    }
    for (const node of Array.from(document.head.querySelectorAll(STALE_ON_NAVIGATION))) node.remove();
  }, [meta]);
}

export function RootRoute() {
  const payload = usePayload();
  const root = rootDataOf(payload);
  useHtmlAttributes(root);
  useHead("meta" in payload ? payload.meta : undefined);
  useEffect(() => {
    // A page rendered: RouterError may reload this URL again next time.
    try {
      window.sessionStorage.removeItem(`ml-reload:${window.location.pathname}${window.location.search}`);
    } catch {
      /* storage unavailable */
    }
  }, [payload]);

  if (!root) return isNotImplemented(payload) ? <NotImplemented payload={payload} /> : <NoRootLayout />;

  return (
    <RootLayout data={root} messages={messagesFor(root)}>
      <SegmentErrorBoundary root errorComponent={GlobalRouteError} notFound={<NotFound />} resetKey={payload}>
        {isNotFound(payload) ? <NotFound /> : <Outlet />}
      </SegmentErrorBoundary>
      <ScrollRestoration />
    </RootLayout>
  );
}

/**
 * Not even the root layout's data arrived (the very first /_page request
 * failed, e.g. the vite dev server without a backend): nothing can be
 * translated or themed, so say so plainly.
 */
function NoRootLayout() {
  return (
    <main className="grid min-h-dvh place-items-center p-6 text-center text-sm">
      <div className="space-y-3">
        <p className="font-semibold">This page could not be loaded.</p>
        <p className="text-muted-foreground">The server did not answer the page data request.</p>
        <button type="button" className="underline underline-offset-4" onClick={() => window.location.reload()}>
          Try again
        </button>
      </div>
    </main>
  );
}

/** Shown only when the page was not inlined (vite dev server) while /_page loads. */
export function HydrateFallback() {
  return null;
}

/**
 * The router's own error element: only reached when the loader itself threw
 * (a page chunk that no longer exists after a deploy, a network failure while
 * loading code). Next falls back to a document navigation in that situation;
 * so does this, once per URL, before showing anything.
 */
export function RouterError() {
  const error = useRouteError();
  useEffect(() => {
    const key = `ml-reload:${window.location.pathname}${window.location.search}`;
    let reloaded = false;
    try {
      reloaded = window.sessionStorage.getItem(key) === "1";
      window.sessionStorage.setItem(key, "1");
    } catch {
      /* storage unavailable: reload anyway, at most once per mount */
    }
    if (!reloaded) window.location.reload();
    else console.error(error);
  }, [error]);
  return null;
}
