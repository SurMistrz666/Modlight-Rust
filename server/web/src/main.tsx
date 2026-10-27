/**
 * Boot: read the page the server inlined, load what it needs (translations,
 * the page's code), then start the router on it without a second request.
 * Without an inlined page (the vite dev server) the router asks /_page itself.
 */
import "@/app/globals.css";
// The Graphite skin (the default): after globals.css so its rules win ties.
import "./styles/graphite.css";
// next/font output for geist/font/{sans,mono}; the classes are on <html> in the
// document template (src/shims/geist/font.ts names them for any importer).
import "./fonts/geist.css";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createBrowserRouter, RouterProvider } from "react-router";
import { routes } from "./routes/tree";
import { setLoaderRoutes, settle } from "./routes/loader";
import { installRouter, isSpaPath } from "./runtime/navigation";
import { isRedirect, readInlinePayload, type PagePayload } from "./runtime/payload";
import { installGlobalHandlers } from "./runtime/global-handlers";
import { applyShellDefaults } from "./runtime/shell";

async function boot(): Promise<void> {
  if (import.meta.env.MODLIGHT_FIXTURES) {
    const { installFixtures } = await import("./fixtures/install");
    await installFixtures();
  }

  setLoaderRoutes(routes);
  installRouter(null, routes);
  let hydrationData: { loaderData: Record<string, PagePayload> } | undefined;
  const inline = readInlinePayload();
  if (inline && isRedirect(inline)) {
    // Upstream streams some redirects inside a 200 document (a page under
    // /dashboard, behind loading.tsx) and the browser then replaces the URL.
    // Same here: replace, then let the router load the target itself.
    const target = new URL(inline.redirect, window.location.href);
    if (target.origin !== window.location.origin || !isSpaPath(target.pathname)) {
      window.location.replace(target.href);
      return;
    }
    window.history.replaceState(null, "", target.pathname + target.search + target.hash);
  } else if (inline) {
    try {
      const page = await settle(inline, new URL(window.location.href));
      hydrationData = { loaderData: { root: page } };
    } catch {
      // Unusable payload: let the router's own first load ask /_page.
    }
  }

  const router = createBrowserRouter(routes, { hydrationData });
  installRouter(router, routes);
  installGlobalHandlers();
  applyShellDefaults(router);

  createRoot(document.getElementById("root")!).render(
    // upstream next.config.ts: reactStrictMode: true
    <StrictMode>
      <RouterProvider router={router} />
    </StrictMode>,
  );
}

void boot();
