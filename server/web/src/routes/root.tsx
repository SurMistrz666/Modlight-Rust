/**
 * The root route: app/layout.tsx (providers), app/error.tsx (root error
 * boundary), app/not-found.tsx, plus what Next does around them in the browser:
 * <html> attributes, <head> metadata, scroll restoration.
 */
import { useEffect, useRef } from "react";
import { Outlet, ScrollRestoration, useLoaderData } from "react-router";
import GlobalRouteError from "@/app/error";
import RootLayout from "../pages/layout";
import NotFound from "../pages/not-found";
import type { RootLayoutData } from "../contract/layouts";
import { isFailure, isNotFound, isOk, type HeadMeta, type PagePayload } from "../runtime/payload";
import { messagesFor } from "../runtime/messages";
import { makeServerError } from "../runtime/errors";
import { SegmentErrorBoundary, Throw } from "./boundary";

export function usePayload(): PagePayload {
  return useLoaderData() as PagePayload;
}

export function rootDataOf(payload: PagePayload): RootLayoutData {
  return (payload as { layouts: { root: RootLayoutData } }).layouts.root;
}

/**
 * React writes <html> attributes only when the rendered value changes, and the
 * first values came with the document. So: nothing on mount, then exactly the
 * attributes whose layout value changed (a language or skin switch followed by
 * router.refresh(), for instance).
 */
function useHtmlAttributes(root: RootLayoutData): void {
  const previous = useRef<RootLayoutData | null>(null);
  useEffect(() => {
    const before = previous.current;
    previous.current = root;
    if (!before) return;
    const html = document.documentElement;
    if (before.locale !== root.locale) html.setAttribute("lang", root.locale);
    if (before.mode !== root.mode) html.setAttribute("data-mode", root.mode);
    if (before.skin !== root.skin) html.setAttribute("data-theme", root.skin);
  }, [root]);
}

/** Next replaces the page's <title> and metadata tags on every navigation. */
function useHead(meta: HeadMeta | undefined): void {
  useEffect(() => {
    if (!meta) return;
    if (document.title !== meta.title) document.title = meta.title;
    for (const node of Array.from(document.head.querySelectorAll("[data-ml-head]"))) node.remove();
    for (const tag of meta.tags) {
      const el = document.createElement(tag.tag);
      for (const [name, value] of Object.entries(tag.attrs)) el.setAttribute(name, value);
      el.setAttribute("data-ml-head", "");
      document.head.appendChild(el);
    }
  }, [meta]);
}

export function RootRoute() {
  const payload = usePayload();
  const root = rootDataOf(payload);
  useHtmlAttributes(root);
  useHead("meta" in payload ? payload.meta : undefined);

  let content: React.ReactNode;
  if (isNotFound(payload)) content = <NotFound />;
  else if (isFailure(payload) && !payload.route) {
    content = <Throw error={makeServerError(payload.error.message, payload.error.digest)} />;
  } else if (isOk(payload) || isFailure(payload)) content = <Outlet />;

  return (
    <RootLayout data={root} messages={messagesFor(root.locale)}>
      <SegmentErrorBoundary root errorComponent={GlobalRouteError} notFound={<NotFound />} resetKey={payload}>
        {content}
      </SegmentErrorBoundary>
      <ScrollRestoration />
    </RootLayout>
  );
}

/** Shown only when the page was not inlined (vite dev server) while /_page loads. */
export function HydrateFallback() {
  return null;
}
