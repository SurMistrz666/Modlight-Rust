/**
 * The segments between the root and a page: the two route-group layouts, the
 * dashboard segment (error.tsx + loading.tsx), the page itself, and the catch-all.
 */
import { useEffect } from "react";
import { Outlet, useLocation, useNavigation, useRouteLoaderData } from "react-router";
import DashboardError from "@/app/(app)/dashboard/error";
import DashboardLoading from "../pages/(app)/dashboard/loading";
import { isFailure, isOk, type Layouts, type PagePayload } from "../runtime/payload";
import { makeServerError } from "../runtime/errors";
import type { RouteId } from "../contract/routes";
import { SegmentErrorBoundary, Throw } from "./boundary";
import { getAppLayout, getAuthLayout, getPage } from "./modules";

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
    // The layout itself failed: upstream that is the root error boundary's job.
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
 * app/(app)/dashboard/: error.tsx around everything below, loading.tsx while
 * a navigation to another dashboard page is on its way. As in Next 16, a change
 * of search params alone does not count (same segment, the old page stays up),
 * and nor does a deeper change under the same child (admin -> admin/users/x).
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

  return (
    <SegmentErrorBoundary errorComponent={DashboardError} resetKey={payload}>
      {pending ? <DashboardLoading /> : <Outlet />}
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
