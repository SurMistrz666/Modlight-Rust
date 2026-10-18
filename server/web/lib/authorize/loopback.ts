/**
 * Signing a desktop client in without ever showing it a password.
 *
 * The ShareX fork needs an API token. The obvious way to get one is an
 * email + password form inside the app, and that is wrong here for a concrete
 * reason: Modlight has TOTP, so such a form breaks for exactly the accounts that
 * bothered to protect themselves. It also teaches people to type their password
 * into a native window, which is the habit every credential phish depends on.
 *
 * So the browser does the authenticating, and the app only ever receives the
 * token — the loopback redirect of RFC 8252, which is what `gh auth login`,
 * `aws sso login` and every other desktop client already do:
 *
 *   1. the app binds an ephemeral port on 127.0.0.1 and starts listening;
 *   2. it opens `<host>/authorize?port=NNNNN&name=<client>&state=<random>`;
 *   3. the page requires a real session, says in plain language what is being
 *      handed over, and waits for an explicit click;
 *   4. on approve, the server mints a named token and the PAGE posts it to
 *      `http://127.0.0.1:<port>/`, body `{"state":"…","token":"…"}`,
 *      content-type `text/plain;charset=UTF-8`;
 *   5. the app checks `state` against the value it generated, keeps the token,
 *      answers 204 and stops listening.
 *
 * A refusal is delivered the same way, as `{"state":"…","error":"access_denied"}`,
 * so the app can stop waiting instead of hanging until its own timeout.
 *
 * `text/plain` is not sloppiness: it makes the POST a CORS-*simple* request, so
 * no preflight is sent and the twenty-line listener inside the desktop app needs
 * no CORS headers at all. The page uses `mode: "no-cors"` and never reads the
 * reply, so delivery is all it can learn — which is why the UI keeps a manual
 * fallback for the case where the listener is already gone.
 *
 * This module is the part with no I/O in it: what a request has to look like
 * before any of the above may happen. Kept free of `server-only` and of every
 * framework import so it can be unit-tested and read from both sides.
 */

/** Ports below this are privileged; a loopback listener never binds one. */
export const MIN_PORT = 1024;
export const MAX_PORT = 65535;

/** How much of a client's self-chosen name we are willing to render. */
export const CLIENT_NAME_MAX = 64;

/** `api_tokens.name` is varchar(120). */
export const TOKEN_NAME_MAX = 120;

const STATE_MIN = 8;
const STATE_MAX = 128;

/** URL-unreserved characters only (RFC 3986 §2.3). See `parseState`. */
const STATE_RE = new RegExp(`^[A-Za-z0-9._~-]{${STATE_MIN},${STATE_MAX}}$`);

/** A validated authorization request, ready to be shown to a human. */
export interface LoopbackRequest {
  /** Loopback port, as a number — never as the string that arrived. */
  port: number;
  /** The client's nonce, echoed back untouched. */
  state: string;
  /** Sanitized display name; empty when the client did not send a usable one. */
  clientName: string;
  /** Where the token goes. Built here, from the port, and nowhere else. */
  callback: string;
}

export type AuthorizeRequestIssue = "invalid-port" | "invalid-state";

export type ParsedAuthorizeRequest =
  | { ok: true; request: LoopbackRequest }
  | { ok: false; reason: AuthorizeRequestIssue };

/** The query shape Next hands a page: a value, a repeated value, or nothing. */
export type QueryValue = string | string[] | undefined;

/**
 * One value, or nothing.
 *
 * A repeated parameter (`?port=8080&port=1`) is refused rather than resolved.
 * No client sends one by accident, and picking "the first" is precisely how two
 * parties end up reading the same URL differently — which is the entire trick
 * behind parameter smuggling.
 */
function single(value: QueryValue): string | null {
  if (typeof value === "string") return value;
  return null;
}

/**
 * The loopback port, or null.
 *
 * The regex runs BEFORE `Number()` because `Number()` is far too forgiving for
 * something that becomes part of a URL: it reads `0x1f90`, `1e4`, `8080\n` and
 * `+8080` as numbers, and each of those is a string somebody chose to write
 * that way. Four or five digits with no leading zero is what a port looks like.
 */
export function parseLoopbackPort(raw: QueryValue): number | null {
  const value = single(raw)?.trim();
  if (!value || !/^[1-9][0-9]{3,4}$/.test(value)) return null;

  const port = Number(value);
  return port >= MIN_PORT && port <= MAX_PORT ? port : null;
}

/**
 * The client's nonce, or null.
 *
 * Its job is to let the app tell its own request apart from one somebody
 * injected, so a one-character "state" is not one: a minimum length is part of
 * the contract, not politeness. The charset is the URL-unreserved set because
 * this value is echoed into a JSON body handed to a local listener and rendered
 * on the page — no quotes, no angle brackets, no newlines, nothing to escape
 * anywhere downstream.
 */
export function parseState(raw: QueryValue): string | null {
  const value = single(raw)?.trim();
  if (!value || !STATE_RE.test(value)) return null;
  return value;
}

/**
 * The name shown in the prompt, cleaned up.
 *
 * Whoever wrote the link chose this string, and it is displayed inside a
 * permission dialog, which makes it the most valuable text on the page to an
 * attacker. React escapes markup, so the danger left is the characters that
 * change what the surrounding text MEANS: `\p{C}` covers the C0/C1 controls, the
 * zero-width joiners, and — the one that matters — the bidirectional overrides,
 * which can render "moc.live" as "evil.com" in a browser without a single
 * suspicious byte. Whitespace is collapsed for the same reason: a name padded
 * with fifty newlines can push the rest of the prompt off the screen.
 *
 * Truncation is last, so the cap counts characters a person can actually see.
 */
export function sanitizeClientName(raw: QueryValue): string {
  const value = single(raw);
  if (!value) return "";

  return value
    .replace(/\p{C}/gu, " ")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, CLIENT_NAME_MAX)
    .trim();
}

/**
 * The one place a callback URL is ever built.
 *
 * Note the input: a NUMBER that has already been through `parseLoopbackPort`,
 * never the string from the query. That is the whole open-redirect defence.
 * Interpolating the raw parameter would accept `8080@attacker.example` and
 * `8080/../..@evil` as a host, and this URL is where an API token gets posted —
 * an open redirect here is a credential handed to whoever wrote the link.
 *
 * IPv4 loopback only. `localhost` resolves through the host's name resolution
 * (and can be pointed elsewhere), and `[::1]` is a second syntax to get wrong
 * for a listener that can always bind 127.0.0.1 instead.
 */
export function loopbackCallback(port: number): string {
  if (!Number.isInteger(port) || port < MIN_PORT || port > MAX_PORT) {
    throw new Error("loopbackCallback: refusing to build a callback for a non-loopback port");
  }
  return `http://127.0.0.1:${port}/`;
}

/** Validate a whole `/authorize` query. Both the page and the action use this. */
export function parseAuthorizeRequest(params: {
  port?: QueryValue;
  state?: QueryValue;
  name?: QueryValue;
}): ParsedAuthorizeRequest {
  const port = parseLoopbackPort(params.port);
  if (port === null) return { ok: false, reason: "invalid-port" };

  const state = parseState(params.state);
  if (state === null) return { ok: false, reason: "invalid-state" };

  return {
    ok: true,
    request: {
      port,
      state,
      clientName: sanitizeClientName(params.name),
      callback: loopbackCallback(port),
    },
  };
}

/**
 * The link that reproduces this request, for `?next=` on the way to /login.
 *
 * Rebuilt from the parsed values rather than passed through as the raw query
 * string: what we hand to the login page then comes from values this module
 * has already vouched for, and there is no second path by which an unchecked
 * parameter reaches the prompt.
 */
export function authorizePath(request: LoopbackRequest): string {
  const query = new URLSearchParams({ port: String(request.port), state: request.state });
  if (request.clientName) query.set("name", request.clientName);
  return `/authorize?${query.toString()}`;
}

/**
 * What the token is called in the Integrations list.
 *
 * Prefixed, so a year later it is obvious the token was issued by a desktop app
 * asking for it rather than typed in by hand — and so a client that names itself
 * "ShareX" cannot sit in the list looking like the config the user made
 * themselves. Clipped to the column width because the name is client-supplied
 * and a value longer than varchar(120) is a failed INSERT, i.e. a sign-in that
 * mysteriously does not work.
 */
export function tokenNameFor(clientName: string): string {
  const name = sanitizeClientName(clientName);
  if (!name) return "Desktop client";
  return `Desktop: ${name}`.slice(0, TOKEN_NAME_MAX);
}
