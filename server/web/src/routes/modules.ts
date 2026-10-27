/**
 * Code splitting by route, like Next's per-segment chunks: each page port and
 * the two group layouts are separate chunks, loaded by the router loader
 * together with (and before rendering) the page data that needs them.
 */
import type { ComponentType, ReactNode } from "react";
import type { RouteId } from "../contract/routes";
import type { AppLayoutData, AuthLayoutData } from "../contract/layouts";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type PageComponent = ComponentType<{ props: any; params: Record<string, string | string[]> }>;

const pageLoaders: Record<RouteId, () => Promise<{ default: PageComponent }>> = {
  "/": () => import("../pages/page"),
  "/welcome": () => import("../pages/welcome/page"),
  "/download": () => import("../pages/download/page"),
  "/dashboard": () => import("../pages/(app)/dashboard/page"),
  "/dashboard/admin": () => import("../pages/(app)/dashboard/admin/page"),
  "/dashboard/admin/users/[id]/files": () => import("../pages/(app)/dashboard/admin/users/[id]/files/page"),
  "/dashboard/downloads": () => import("../pages/(app)/dashboard/downloads/page"),
  "/dashboard/duplicates": () => import("../pages/(app)/dashboard/duplicates/page"),
  "/dashboard/folders": () => import("../pages/(app)/dashboard/folders/page"),
  "/dashboard/links": () => import("../pages/(app)/dashboard/links/page"),
  "/dashboard/paste": () => import("../pages/(app)/dashboard/paste/page"),
  "/dashboard/profile": () => import("../pages/(app)/dashboard/profile/page"),
  "/dashboard/tokens": () => import("../pages/(app)/dashboard/tokens/page"),
  "/dashboard/trash": () => import("../pages/(app)/dashboard/trash/page"),
  "/dashboard/upload": () => import("../pages/(app)/dashboard/upload/page"),
  "/2fa": () => import("../pages/(auth)/2fa/page"),
  "/authorize": () => import("../pages/(auth)/authorize/page"),
  "/forgot": () => import("../pages/(auth)/forgot/page"),
  "/invite/[token]": () => import("../pages/(auth)/invite/[token]/page"),
  "/login": () => import("../pages/(auth)/login/page"),
  "/register": () => import("../pages/(auth)/register/page"),
  "/reset": () => import("../pages/(auth)/reset/page"),
  "/[userCode]": () => import("../pages/[userCode]/page"),
  "/[userCode]/[mediaCode]": () => import("../pages/[userCode]/[mediaCode]/page"),
  "/a/[userCode]/[slug]": () => import("../pages/a/[userCode]/[slug]/page"),
  "/s/[code]/unlock": () => import("../pages/s/[code]/unlock/page"),
  "/s/[code]/[name]/unlock": () => import("../pages/s/[code]/[name]/unlock/page"),
  "/u/[userCode]": () => import("../pages/u/[userCode]/page"),
};

export type AppLayoutComponent = ComponentType<{ data: AppLayoutData; children: ReactNode }>;
export type AuthLayoutComponent = ComponentType<{ data: AuthLayoutData; children: ReactNode }>;

const layoutLoaders = {
  "(app)": () => import("../pages/(app)/layout"),
  "(auth)": () => import("../pages/(auth)/layout"),
};

const pages = new Map<RouteId, PageComponent>();
const layouts = new Map<keyof typeof layoutLoaders, ComponentType<never>>();

export function isRouteId(id: unknown): id is RouteId {
  return typeof id === "string" && id in pageLoaders;
}

export async function ensurePage(id: RouteId): Promise<PageComponent> {
  const hit = pages.get(id);
  if (hit) return hit;
  const mod = await pageLoaders[id]();
  pages.set(id, mod.default);
  return mod.default;
}

export function getPage(id: RouteId): PageComponent | undefined {
  return pages.get(id);
}

export async function ensureLayout(id: keyof typeof layoutLoaders): Promise<void> {
  if (layouts.has(id)) return;
  const mod = await layoutLoaders[id]();
  layouts.set(id, mod.default as ComponentType<never>);
}

export function getAppLayout(): AppLayoutComponent | undefined {
  return layouts.get("(app)") as AppLayoutComponent | undefined;
}

export function getAuthLayout(): AuthLayoutComponent | undefined {
  return layouts.get("(auth)") as AuthLayoutComponent | undefined;
}

/** Preload everything (tests and fixture smoke runs). */
export async function ensureAll(): Promise<void> {
  await Promise.all([
    ...(Object.keys(pageLoaders) as RouteId[]).map(ensurePage),
    ensureLayout("(app)"),
    ensureLayout("(auth)"),
  ]);
}
