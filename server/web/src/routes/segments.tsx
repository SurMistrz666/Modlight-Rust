/**
 * The segments between the root and a page: the two route-group layouts, the
 * dashboard segment (error.tsx + loading.tsx), the page itself, and the catch-all.
 */
import { useEffect, useState } from "react";
import { Outlet, useLocation, useNavigation, useRouteLoaderData } from "react-router";
import DashboardError from "@/app/(app)/dashboard/error";
import DashboardLoading from "../pages/(app)/dashboard/loading";
import { isFailure, isNotImplemented, isOk, type Layouts, type PagePayload } from "../runtime/payload";
import { makeServerError } from "../runtime/errors";
import type { RouteId } from "../contract/routes";
import { SegmentErrorBoundary, Throw } from "./boundary";
import { getAppLayout, getAuthLayout, getPage } from "./modules";
import { NotImplemented } from "./not-implemented";

function useRootPayload(): PagePayload {
  return useRouteLoaderData("root") as PagePayload;
}

function layoutsOf(payload: PagePayload): Partial<Layouts> {
  return "layouts" in payload ? (payload.layouts as Partial<Layouts>) : {};
}

function failureError(payload: PagePayload) {
  return isFailure(payload) ? makeServerError(payload.error.message, payload.error.digest) : null;
}

/** app/(app)/layout.tsx */
export function AppLayoutRoute() {
  const payload = useRootPayload();
  const data = layoutsOf(payload)["(app)"];
  const Layout = getAppLayout();
  if (!data || !Layout) {
    // The layout itself did not load: a gap in the server (501), or a failure,
    // which upstream is the root error boundary's job.
    if (isNotImplemented(payload)) return <NotImplemented payload={payload} />;
    const error = failureError(payload);
    return error ? <Throw error={error} /> : null;
  }
  return (
    <Layout data={data}>
      <Outlet />
    </Layout>
  );
}

/** app/(auth)/layout.tsx */
export function AuthLayoutRoute() {
  const payload = useRootPayload();
  const data = layoutsOf(payload)["(auth)"];
  const Layout = getAuthLayout();
  if (!data || !Layout) {
    if (isNotImplemented(payload)) return <NotImplemented payload={payload} />;
    const error = failureError(payload);
    return error ? <Throw error={error} /> : null;
  }
  return (
    <Layout data={data}>
      <Outlet />
    </Layout>
  );
}

/**
 * The child segment of `dashboard` a pathname selects: Next keeps one loading
 * boundary per child segment, so only a change here shows loading.tsx.
 */
function dashboardChild(pathname: string): string | null {
  const parts = pathname.split("/").filter(Boolean);
  if (parts[0] !== "dashboard") return null;
  return parts[1] ?? "__PAGE__";
}

/**
 * How long a navigation may be on its way before loading.tsx replaces the page
 * that is still on screen.
 *
 * Deliberate difference from Next, which swaps to loading.tsx at once: there
 * the page data is a server render that takes long enough for the skeleton to
 * read as "loading". Here /_page answers in tens of milliseconds, so an
 * immediate swap only flashes the skeleton between two pages, which people see
 * as the screen flickering. The old page stays up until the new one is ready,
 * and the skeleton still appears when the server is genuinely slow.
 */
const LOADING_DELAY_MS = 300;

/**
 * app/(app)/dashboard/: error.tsx around everything below, loading.tsx while
 * a navigation to another dashboard page is on its way (after LOADING_DELAY_MS).
 * As in Next 16, a change of search params alone does not count (same segment,
 * the old page stays up), and nor does a deeper change under the same child
 * (admin -> admin/users/x).
 */
export function DashboardSegment() {
  const payload = useRootPayload();
  const location = useLocation();
  const navigation = useNavigation();
  const target = navigation.state === "loading" ? navigation.location : null;
  const pending =
    target !== null &&
    dashboardChild(target.pathname) !== null &&
    dashboardChild(target.pathname) !== dashboardChild(location.pathname);
  const targetKey = pending ? target.key : null;

  const [slowKey, setSlowKey] = useState<string | null>(null);
  useEffect(() => {
    if (targetKey === null) return;
    const timer = setTimeout(() => setSlowKey(targetKey), LOADING_DELAY_MS);
    return () => clearTimeout(timer);
  }, [targetKey]);
  const showLoading = targetKey !== null && slowKey === targetKey;

  return (
    <SegmentErrorBoundary errorComponent={DashboardError} resetKey={payload}>
      {showLoading ? <DashboardLoading /> : <Outlet />}
    </SegmentErrorBoundary>
  );
}

/**
 * A page. Renders the component of the route the SERVER resolved (payload
 * `route`), keyed by pathname so a new value of a dynamic segment is a new page,
 * as Next's segment state keys make it.
 */
export function PageRoute({ id }: { id: RouteId | null }) {
  const payload = useRootPayload();
  const { pathname } = useLocation();

  useEffect(() => {
    if (isOk(payload) && id !== null && payload.route !== id) {
      console.error(`[modlight] the server resolved ${pathname} to ${payload.route}, the router to ${id}`);
    }
  }, [payload, id, pathname]);

  const error = failureError(payload);
  if (error) return <Throw error={error} />;
  if (isNotImplemented(payload)) return <NotImplemented payload={payload} />;
  if (!isOk(payload)) return null;
  const Page = getPage(payload.route);
  if (!Page) return null;
  return <Page key={pathname} props={payload.props} params={payload.params} />;
}

/**
 * A URL no page matches. On a client navigation that is a route handler (the
 * navigation helpers already sent those to the browser); on a first load the
 * server answered it with the not-found page payload, which the root renders.
 */
export function CatchAllRoute() {
  return <PageRoute id={null} />;
}
