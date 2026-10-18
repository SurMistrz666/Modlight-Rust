/**
 * Every upstream page, by the id the payload's `route` field carries: its path
 * under upstream app/, without `page.tsx`. `pattern` is the URL pattern in
 * react-router syntax.
 */
export const ROUTES = {
  "/": { pattern: "/", layouts: ["root"] },
  "/welcome": { pattern: "/welcome", layouts: ["root"] },
  "/download": { pattern: "/download", layouts: ["root"] },

  "/(app)/dashboard": { pattern: "/dashboard", layouts: ["root", "(app)"] },
  "/(app)/dashboard/admin": { pattern: "/dashboard/admin", layouts: ["root", "(app)"] },
  "/(app)/dashboard/admin/users/[id]/files": {
    pattern: "/dashboard/admin/users/:id/files",
    layouts: ["root", "(app)"],
  },
  "/(app)/dashboard/duplicates": { pattern: "/dashboard/duplicates", layouts: ["root", "(app)"] },
  "/(app)/dashboard/folders": { pattern: "/dashboard/folders", layouts: ["root", "(app)"] },
  "/(app)/dashboard/links": { pattern: "/dashboard/links", layouts: ["root", "(app)"] },
  "/(app)/dashboard/paste": { pattern: "/dashboard/paste", layouts: ["root", "(app)"] },
  "/(app)/dashboard/profile": { pattern: "/dashboard/profile", layouts: ["root", "(app)"] },
  "/(app)/dashboard/tokens": { pattern: "/dashboard/tokens", layouts: ["root", "(app)"] },
  "/(app)/dashboard/trash": { pattern: "/dashboard/trash", layouts: ["root", "(app)"] },
  "/(app)/dashboard/upload": { pattern: "/dashboard/upload", layouts: ["root", "(app)"] },

  "/(auth)/2fa": { pattern: "/2fa", layouts: ["root", "(auth)"] },
  "/(auth)/authorize": { pattern: "/authorize", layouts: ["root", "(auth)"] },
  "/(auth)/forgot": { pattern: "/forgot", layouts: ["root", "(auth)"] },
  "/(auth)/invite/[token]": { pattern: "/invite/:token", layouts: ["root", "(auth)"] },
  "/(auth)/login": { pattern: "/login", layouts: ["root", "(auth)"] },
  "/(auth)/register": { pattern: "/register", layouts: ["root", "(auth)"] },
  "/(auth)/reset": { pattern: "/reset", layouts: ["root", "(auth)"] },

  "/[userCode]": { pattern: "/:userCode", layouts: ["root"] },
  "/[userCode]/[mediaCode]": { pattern: "/:userCode/:mediaCode", layouts: ["root"] },
  "/a/[userCode]/[slug]": { pattern: "/a/:userCode/:slug", layouts: ["root"] },
  "/s/[code]/unlock": { pattern: "/s/:code/unlock", layouts: ["root"] },
  "/s/[code]/[name]/unlock": { pattern: "/s/:code/:name/unlock", layouts: ["root"] },
  "/u/[userCode]": { pattern: "/u/:userCode", layouts: ["root"] },
} as const;

export type RouteId = keyof typeof ROUTES;
export type LayoutId = "root" | "(app)" | "(auth)";
