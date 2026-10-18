/**
 * Every upstream page, by the id the payload's `route` field carries: its Next
 * app path (route groups stripped, `[params]` kept), exactly as the Rust
 * server's route table names it (server/src/pages/table.rs `APP_ROUTES`).
 * `pattern` is the URL pattern in react-router syntax, `file` the upstream file.
 */
export const ROUTES = {
  "/": { pattern: "/", layouts: ["root"], file: "app/page.tsx" },
  "/welcome": { pattern: "/welcome", layouts: ["root"], file: "app/welcome/page.tsx" },
  "/download": { pattern: "/download", layouts: ["root"], file: "app/download/page.tsx" },

  "/dashboard": { pattern: "/dashboard", layouts: ["root", "(app)"], file: "app/(app)/dashboard/page.tsx" },
  "/dashboard/admin": {
    pattern: "/dashboard/admin",
    layouts: ["root", "(app)"],
    file: "app/(app)/dashboard/admin/page.tsx",
  },
  "/dashboard/admin/users/[id]/files": {
    pattern: "/dashboard/admin/users/:id/files",
    layouts: ["root", "(app)"],
    file: "app/(app)/dashboard/admin/users/[id]/files/page.tsx",
  },
  "/dashboard/duplicates": {
    pattern: "/dashboard/duplicates",
    layouts: ["root", "(app)"],
    file: "app/(app)/dashboard/duplicates/page.tsx",
  },
  "/dashboard/folders": {
    pattern: "/dashboard/folders",
    layouts: ["root", "(app)"],
    file: "app/(app)/dashboard/folders/page.tsx",
  },
  "/dashboard/links": {
    pattern: "/dashboard/links",
    layouts: ["root", "(app)"],
    file: "app/(app)/dashboard/links/page.tsx",
  },
  "/dashboard/paste": {
    pattern: "/dashboard/paste",
    layouts: ["root", "(app)"],
    file: "app/(app)/dashboard/paste/page.tsx",
  },
  "/dashboard/profile": {
    pattern: "/dashboard/profile",
    layouts: ["root", "(app)"],
    file: "app/(app)/dashboard/profile/page.tsx",
  },
  "/dashboard/tokens": {
    pattern: "/dashboard/tokens",
    layouts: ["root", "(app)"],
    file: "app/(app)/dashboard/tokens/page.tsx",
  },
  "/dashboard/trash": {
    pattern: "/dashboard/trash",
    layouts: ["root", "(app)"],
    file: "app/(app)/dashboard/trash/page.tsx",
  },
  "/dashboard/upload": {
    pattern: "/dashboard/upload",
    layouts: ["root", "(app)"],
    file: "app/(app)/dashboard/upload/page.tsx",
  },

  "/2fa": { pattern: "/2fa", layouts: ["root", "(auth)"], file: "app/(auth)/2fa/page.tsx" },
  "/authorize": { pattern: "/authorize", layouts: ["root", "(auth)"], file: "app/(auth)/authorize/page.tsx" },
  "/forgot": { pattern: "/forgot", layouts: ["root", "(auth)"], file: "app/(auth)/forgot/page.tsx" },
  "/invite/[token]": {
    pattern: "/invite/:token",
    layouts: ["root", "(auth)"],
    file: "app/(auth)/invite/[token]/page.tsx",
  },
  "/login": { pattern: "/login", layouts: ["root", "(auth)"], file: "app/(auth)/login/page.tsx" },
  "/register": { pattern: "/register", layouts: ["root", "(auth)"], file: "app/(auth)/register/page.tsx" },
  "/reset": { pattern: "/reset", layouts: ["root", "(auth)"], file: "app/(auth)/reset/page.tsx" },

  "/[userCode]": { pattern: "/:userCode", layouts: ["root"], file: "app/[userCode]/page.tsx" },
  "/[userCode]/[mediaCode]": {
    pattern: "/:userCode/:mediaCode",
    layouts: ["root"],
    file: "app/[userCode]/[mediaCode]/page.tsx",
  },
  "/a/[userCode]/[slug]": { pattern: "/a/:userCode/:slug", layouts: ["root"], file: "app/a/[userCode]/[slug]/page.tsx" },
  "/s/[code]/unlock": { pattern: "/s/:code/unlock", layouts: ["root"], file: "app/s/[code]/unlock/page.tsx" },
  "/s/[code]/[name]/unlock": {
    pattern: "/s/:code/:name/unlock",
    layouts: ["root"],
    file: "app/s/[code]/[name]/unlock/page.tsx",
  },
  "/u/[userCode]": { pattern: "/u/:userCode", layouts: ["root"], file: "app/u/[userCode]/page.tsx" },
} as const;

export type RouteId = keyof typeof ROUTES;
export type LayoutId = "root" | "(app)" | "(auth)";

/**
 * URLs the Rust server answers itself, not pages, so following a link to one
 * is a document load (src/runtime/navigation.ts) and the vite dev server hands
 * the document request to the Rust server (vite.config.ts):
 *
 *  - the 25 upstream route handlers (the route.ts files under app/), in the
 *    order and spelling of server/src/pages/table.rs (`handler(...)`), with
 *    `[param]` as `:param` and `[...all]` as `*`;
 *  - the resumable upload endpoint of upstream server.ts (`/api/files*`; its
 *    POST-only ShareX paths /upload and /api/upload are pages for a GET);
 *  - files: app/icon.svg, app/apple-icon.png, public/ (brand/, fonts/) and the
 *    SPA's own build (assets/).
 *
 * Matching ranks static segments over dynamic ones, as Next does, so /s/promo
 * is the short-link handler, not a [userCode]/[mediaCode] page. A URL that
 * matches no page and none of these (a public file path the table does not
 * spell out, a typo) navigates client-side; the server answers it with
 * `{"reload": true}` or `{"notFound": true}` (CONTRACT.md section 4.2).
 * tests/route-table.test.ts checks this list against table.rs.
 */
export const SERVER_PATHS = [
  "/:userCode/:mediaCode/:action", // app/[userCode]/[mediaCode]/[action]/route.ts
  "/:userCode/:mediaCode/delete/:token", // app/[userCode]/[mediaCode]/delete/[token]/route.ts
  "/a/:userCode/:slug/zip", // app/a/[userCode]/[slug]/zip/route.ts
  "/api/admin/disk",
  "/api/auth/*", // app/api/auth/[...all]/route.ts (Better Auth)
  "/api/avatar",
  "/api/background",
  "/api/client-settings",
  "/api/duplicates",
  "/api/folders",
  "/api/health",
  "/api/link-info",
  "/api/me",
  "/api/releases/app",
  "/api/shortlinks",
  "/api/theme",
  "/api/uploads",
  "/api/uploads/check",
  "/avatar/:userCode", // app/avatar/[userCode]/route.ts
  "/bg/:userCode", // app/bg/[userCode]/route.ts
  "/download/app", // app/download/app/route.ts
  "/embed/:version/:card", // app/embed/[version]/[card]/route.ts
  "/oembed", // app/oembed/route.ts
  "/s/:code", // app/s/[code]/route.ts
  "/s/:code/:name", // app/s/[code]/[name]/route.ts
  "/api/files/*", // server.ts: tus resumable uploads (any method)
  "/icon.svg", // app/icon.svg
  "/apple-icon.png", // app/apple-icon.png
  "/brand/*", // public/brand
  "/fonts/*", // public/fonts
  "/assets/*", // web/dist/assets (this SPA's build)
];
