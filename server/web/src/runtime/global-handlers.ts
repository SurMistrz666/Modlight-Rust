/**
 * Next's app router listens for redirect errors nobody caught (an action or an
 * event handler that called `redirect()` and did not handle the rejection) and
 * turns them into navigations (client/components/app-router.js,
 * handleUnhandledRedirect). Server-action redirects arrive already handled
 * (rpc.ts navigates before rejecting), so those are only silenced here.
 */
import { isRedirectError, redirectTarget } from "./errors";
import { navigate } from "./navigation";

let installed = false;

export function installGlobalHandlers(): void {
  if (installed) return;
  installed = true;
  const handle = (event: ErrorEvent | PromiseRejectionEvent) => {
    const error = "reason" in event ? event.reason : event.error;
    if (!isRedirectError(error)) return;
    event.preventDefault();
    if (error.handled) return;
    const { url, type } = redirectTarget(error);
    navigate(url, { replace: type === "replace" });
  };
  window.addEventListener("error", handle);
  window.addEventListener("unhandledrejection", handle);
}
