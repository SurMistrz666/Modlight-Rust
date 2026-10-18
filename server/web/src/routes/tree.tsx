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
import { RootRoute, HydrateFallback } from "./root";
import { AppLayoutRoute, AuthLayoutRoute, DashboardSegment, PageRoute, CatchAllRoute } from "./segments";
import { CATCH_ALL } from "../runtime/navigation";

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

export const routes: RouteObject[] = [
  {
    id: "root",
    path: "/",
    loader: rootLoader,
    // Every navigation asks for the whole page; layouts are part of the answer.
    shouldRevalidate: () => true,
    element: <RootRoute />,
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
      { id: CATCH_ALL, path: "*", element: <CatchAllRoute /> },
    ],
  },
];
