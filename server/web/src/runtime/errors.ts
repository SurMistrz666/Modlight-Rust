/**
 * The control-flow errors Next.js throws for `redirect()` and `notFound()`, in
 * Next's own digest format, so code written against Next (and Next's own
 * `isRedirectError` checks in libraries) recognises them.
 *
 *   redirect   digest "NEXT_REDIRECT;<push|replace>;<url>;<status>;"
 *   notFound   digest "NEXT_HTTP_ERROR_FALLBACK;404"
 */

export type RedirectType = "push" | "replace";

export interface RedirectError extends Error {
  digest: string;
  /** Set when the navigation already happened (a server action's redirect). */
  handled?: boolean;
}

export interface NotFoundError extends Error {
  digest: "NEXT_HTTP_ERROR_FALLBACK;404";
}

/** The message React's Flight client gives every server error in production. */
export const MASKED_SERVER_ERROR =
  "An error occurred in the Server Components render. The specific message is omitted in production builds to avoid leaking sensitive details. A digest property is included on this error instance which may provide additional details about the nature of the error.";

export function makeRedirectError(url: string, type: RedirectType = "replace", status = 307): RedirectError {
  const error = new Error("NEXT_REDIRECT") as RedirectError;
  error.digest = `NEXT_REDIRECT;${type};${url};${status};`;
  return error;
}

export function makeNotFoundError(): NotFoundError {
  const error = new Error("NEXT_HTTP_ERROR_FALLBACK;404") as NotFoundError;
  error.digest = "NEXT_HTTP_ERROR_FALLBACK;404";
  return error;
}

function digestOf(error: unknown): string | null {
  if (!error || typeof error !== "object" || !("digest" in error)) return null;
  const d = (error as { digest?: unknown }).digest;
  return typeof d === "string" ? d : null;
}

export function isRedirectError(error: unknown): error is RedirectError {
  const d = digestOf(error);
  if (!d) return false;
  const parts = d.split(";");
  return parts[0] === "NEXT_REDIRECT" && (parts[1] === "push" || parts[1] === "replace") && parts.length >= 4;
}

export function redirectTarget(error: RedirectError): { url: string; type: RedirectType } {
  const parts = error.digest.split(";");
  // The URL itself may contain ';' - everything between the type and the status.
  const url = parts.slice(2, -2).join(";");
  return { url, type: parts[1] as RedirectType };
}

export function isNotFoundError(error: unknown): error is NotFoundError {
  return digestOf(error) === "NEXT_HTTP_ERROR_FALLBACK;404";
}

/** An error reported by the server for a page or an action (masked in production). */
export interface ServerError extends Error {
  digest?: string;
}

export function makeServerError(message: string, digest?: string): ServerError {
  const error = new Error(message) as ServerError;
  if (digest) error.digest = digest;
  return error;
}
