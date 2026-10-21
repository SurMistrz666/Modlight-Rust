/**
 * Server actions as RPC (docs/CONTRACT.md "Server actions", PORT.md section 7).
 *
 *   POST /_rpc/<module>/<export>
 *   Content-Type: application/json, X-Modlight-Rpc: 1     body: tagged JSON array of the arguments
 *   (a single FormData argument goes as multipart/form-data with X-Modlight-Rpc: form)
 *
 * The answer is always a JSON envelope:
 *
 *   {"ok": <tagged JSON>}                 resolve with it
 *   {"redirect": "<url>",                 navigate there (push unless redirectType says
 *    "redirectType"?: "push"|"replace"}   "replace") and reject with Next's redirect error
 *   {"notFound": true}                    reject with Next's not-found error
 *   {"error": {"message", "digest"?}}     reject with Error(message) (+ .digest)
 *   ...plus "revalidate": true            the current page's data is fetched again
 *                                         before the promise settles
 *
 * That is what Next 16's client does with a server action's response
 * (router-reducer/reducers/server-action-reducer.js): the revalidated page
 * arrives with the result; a redirect navigates AND rejects the call with a
 * NEXT_REDIRECT error marked `handled`; a thrown error reaches the caller with
 * the production-masked message and a digest.
 */
import { encodeTagged, decodeTagged } from "./tagged-json";
import { makeNotFoundError, makeRedirectError, makeServerError, MASKED_SERVER_ERROR } from "./errors";
import { navigate, refresh } from "./navigation";

interface Envelope {
  ok?: unknown;
  redirect?: string;
  redirectType?: "push" | "replace";
  notFound?: boolean;
  error?: { message?: string; digest?: string };
  revalidate?: boolean;
}

/**
 * `headers`: extra request headers for this one call (withActionHeaders); not
 * upstream, Next's server actions have no such thing.
 */
export type ActionTransport = (
  module: string,
  name: string,
  args: unknown[],
  headers?: Record<string, string>,
) => Promise<Envelope>;

export const httpTransport: ActionTransport = async (module, name, args, headers) => {
  const single = args.length === 1 ? args[0] : undefined;
  const init: RequestInit =
    single instanceof FormData
      ? { method: "POST", body: single, headers: { ...headers, "X-Modlight-Rpc": "form" } }
      : {
          method: "POST",
          body: JSON.stringify(args.map((a) => encodeTagged(a))),
          headers: { ...headers, "Content-Type": "application/json", "X-Modlight-Rpc": "1" },
        };
  init.credentials = "same-origin";

  let res: Response;
  try {
    res = await fetch(`/_rpc/${encodeURIComponent(module)}/${encodeURIComponent(name)}`, init);
  } catch {
    // Next rejects a server action whose request failed with this message.
    throw new TypeError("Failed to fetch");
  }
  const type = res.headers.get("content-type") ?? "";
  if (!type.includes("application/json")) {
    throw new Error("An unexpected response was received from the server.");
  }
  return decodeTagged<Envelope>(await res.json());
};

let transport: ActionTransport = httpTransport;

/** Fixture mode and tests swap the network for a local answer. */
export function setActionTransport(next: ActionTransport): void {
  transport = next;
}

let scopedHeaders: Record<string, string> | undefined;

/**
 * Run `call` (which starts one server action synchronously, as every stub
 * does) with extra request headers on that action's request: how a form hands
 * the optional Cloudflare Turnstile token (`x-captcha-response`) to an action
 * without changing the action's arguments (src/turnstile/). Not upstream.
 */
export function withActionHeaders<T>(headers: Record<string, string> | undefined, call: () => Promise<T>): Promise<T> {
  const previous = scopedHeaders;
  scopedHeaders = headers && Object.keys(headers).length > 0 ? headers : undefined;
  try {
    return call();
  } finally {
    scopedHeaders = previous;
  }
}

export async function callAction<T>(module: string, name: string, args: ArrayLike<unknown>): Promise<T> {
  // Read before the first await: withActionHeaders only holds them while the
  // stub runs synchronously.
  const headers = scopedHeaders;
  const envelope = headers
    ? await transport(module, name, Array.from(args), headers)
    : await transport(module, name, Array.from(args));

  if (typeof envelope.redirect === "string") {
    const type = envelope.redirectType === "replace" ? "replace" : "push";
    navigate(envelope.redirect, { replace: type === "replace" });
    const error = makeRedirectError(envelope.redirect, type);
    error.handled = true;
    throw error;
  }
  if (envelope.revalidate) {
    try {
      await refresh();
    } catch {
      /* the action itself succeeded; a failed refresh shows up on the page */
    }
  }
  if (envelope.notFound) throw makeNotFoundError();
  if (envelope.error) {
    throw makeServerError(envelope.error.message || MASKED_SERVER_ERROR, envelope.error.digest);
  }
  return envelope.ok as T;
}
