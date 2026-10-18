/**
 * The route tree, mirroring upstream app/:
 *
 *   app/layout.tsx (root)                      error.tsx, not-found.tsx
 *   ├── page.tsx, welcome/, download/
 *   ├── (app)/layout.tsx
 *   │   └── dashboard/ (error.tsx, loading.tsx)
 *   │       ├── page.tsx, admin/, admin/users/[id]/files/, duplicates/, folders/,
 *   │       └── links/, paste/, profile/, tokens/, trash/, upload/
 *   ├── (auth)/layout.tsx
 *   │   └── 2fa/, authorize/, forgot/, invite/[token]/, login/, register/, reset/
 *   ├── [userCode]/page.tsx, [userCode]/[mediaCode]/page.tsx
 *   ├── a/[userCode]/[slug]/, s/[code]/unlock/, s/[code]/[name]/unlock/, u/[userCode]/
 *   └── *  (anything else: not a page of the SPA)
 *
 * Route groups are pathless layout routes; dynamic segments are `:params`. Static
 * segments outrank dynamic ones in react-router's ranking exactly as in Next's,
 * so /login is never read as a [userCode].
 */
import type { RouteObject } from "react-router";
import { ROUTES, type RouteId } from "../contract/routes";
import { rootLoader } from "./loader";
import { RootRoute, HydrateFallback, RouterError } from "./root";
import { AppLayoutRoute, AuthLayoutRoute, DashboardSegment, PageRoute, CatchAllRoute } from "./segments";
import { CATCH_ALL, SERVER_PATH } from "../runtime/navigation";

function leaf(id: RouteId, path: string | null): RouteObject {
  const element = <PageRoute id={id} />;
  return path === null ? { id, index: true, element } : { id, path, element };
}

/** The pattern of a route relative to its parent segment. */
function rel(id: RouteId, base: string): string {
  const pattern: string = ROUTES[id].pattern;
  return pattern.slice(base.length).replace(/^\//, "");
}

const dashboardPages: RouteId[] = [
  "/(app)/dashboard/admin",
  "/(app)/dashboard/admin/users/[id]/files",
  "/(app)/dashboard/duplicates",
  "/(app)/dashboard/folders",
  "/(app)/dashboard/links",
  "/(app)/dashboard/paste",
  "/(app)/dashboard/profile",
  "/(app)/dashboard/tokens",
  "/(app)/dashboard/trash",
  "/(app)/dashboard/upload",
];

const authPages: RouteId[] = [
  "/(auth)/2fa",
  "/(auth)/authorize",
  "/(auth)/forgot",
  "/(auth)/invite/[token]",
  "/(auth)/login",
  "/(auth)/register",
  "/(auth)/reset",
];

const topPages: RouteId[] = [
  "/welcome",
  "/download",
  "/[userCode]",
  "/[userCode]/[mediaCode]",
  "/a/[userCode]/[slug]",
  "/s/[code]/unlock",
  "/s/[code]/[name]/unlock",
  "/u/[userCode]",
];

/**
 * URLs the Rust server answers itself, not pages: upstream route handlers (the
 * route.ts files under app/), the raw upload paths of server.ts and public/ files.
 * They are in the table only so matching follows Next's precedence: /s/promo
 * is the short-link handler, not a [userCode]/[mediaCode] page. Navigating to
 * one is a document load (src/runtime/navigation.ts).
 */
export const SERVER_PATHS = [
  "/:userCode/:mediaCode/:action", // app/[userCode]/[mediaCode]/[action]/route.ts
  "/:userCode/:mediaCode/delete/:token", // app/[userCode]/[mediaCode]/delete/[token]/route.ts
  "/a/:userCode/:slug/zip", // app/a/[userCode]/[slug]/zip/route.ts
  "/api/*", // the route.ts files under app/api, plus /api/upload and /api/files (server.ts)
  "/avatar/:userCode", // app/avatar/[userCode]/route.ts
  "/bg/:userCode", // app/bg/[userCode]/route.ts
  "/download/app", // app/download/app/route.ts
  "/embed/:version/:card", // app/embed/[version]/[card]/route.ts
  "/oembed", // app/oembed/route.ts
  "/s/:code", // app/s/[code]/route.ts
  "/s/:code/:name", // app/s/[code]/[name]/route.ts
  "/upload", // server.ts (ShareX)
  "/icon.svg", // app/icon.svg
  "/apple-icon.png", // app/apple-icon.png
  "/brand/*", // public/brand
  "/fonts/*", // public/fonts
];

/** Next matches paths case-sensitively (/Dashboard is a [userCode]); react-router does not by default. */
function caseSensitive(list: RouteObject[]): RouteObject[] {
  return list.map((r) => {
    const children = r.children ? caseSensitive(r.children) : undefined;
    return { ...r, ...(r.path !== undefined ? { caseSensitive: true } : {}), ...(children ? { children } : {}) } as RouteObject;
  });
}

export const routes: RouteObject[] = caseSensitive([
  {
    id: "root",
    path: "/",
    loader: rootLoader,
    // Every navigation asks for the whole page; layouts are part of the answer.
    shouldRevalidate: () => true,
    element: <RootRoute />,
    errorElement: <RouterError />,
    HydrateFallback,
    children: [
      leaf("/", null),
      ...topPages.map((id) => leaf(id, rel(id, ""))),
      {
        id: "(app)",
        element: <AppLayoutRoute />,
        children: [
          {
            id: "dashboard",
            path: "dashboard",
            element: <DashboardSegment />,
            children: [
              leaf("/(app)/dashboard", null),
              ...dashboardPages.map((id) => leaf(id, rel(id, "/dashboard"))),
            ],
          },
        ],
      },
      {
        id: "(auth)",
        element: <AuthLayoutRoute />,
        children: authPages.map((id) => leaf(id, rel(id, ""))),
      },
      ...SERVER_PATHS.map((path) => ({ id: `${SERVER_PATH}${path}`, path: path.slice(1), element: <CatchAllRoute /> })),
      { id: CATCH_ALL, path: "*", element: <CatchAllRoute /> },
    ],
  },
]);
