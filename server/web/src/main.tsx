/**
 * Boot: read the page the server inlined, load what it needs (translations,
 * the page's code), then start the router on it without a second request.
 * Without an inlined page (the vite dev server) the router asks /_page itself.
 */
import "@/app/globals.css";
import "./fonts/geist.css";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createBrowserRouter, RouterProvider } from "react-router";
import { routes } from "./routes/tree";
import { setLoaderRoutes, settle } from "./routes/loader";
import { installRouter } from "./runtime/navigation";
import { readInlinePayload, type PagePayload } from "./runtime/payload";
import { installGlobalHandlers } from "./runtime/global-handlers";
import { applyShellDefaults } from "./runtime/shell";

async function boot(): Promise<void> {
  if (import.meta.env.MODLIGHT_FIXTURES) {
    const { installFixtures } = await import("./fixtures/install");
    await installFixtures();
  }

  setLoaderRoutes(routes);
  let hydrationData: { loaderData: Record<string, PagePayload> } | undefined;
  const inline = readInlinePayload();
  if (inline) {
    try {
      const page = await settle(inline, new URL(window.location.href));
      hydrationData = { loaderData: { root: page } };
    } catch {
      // A redirect payload inline: let the router's own first load follow it.
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
