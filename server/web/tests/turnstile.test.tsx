/**
 * The optional Cloudflare Turnstile check in the four auth forms (src/turnstile/,
 * server/src/turnstile.rs): the widget and Cloudflare's script exist only when
 * the root layout carries a site key, and the token reaches the request as
 * `x-captcha-response`, to an RPC call and to a Better Auth client call, once.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { routes } from "../src/routes/tree";
import { setLoaderRoutes, setPageSource } from "../src/routes/loader";
import { installRouter } from "../src/runtime/navigation";
import { httpTransport, setActionTransport } from "../src/runtime/rpc";
import { expandFixture, FIXTURES } from "../src/fixtures/source";
import { TURNSTILE_SCRIPT, type TurnstileApi } from "../src/turnstile/widget";
import type { PagePayload } from "../src/runtime/payload";

const SITE_KEY = "1x00000000000000000000AA";

/**
 * Better Auth's client keeps the `fetch` it saw when it was created (its
 * `customFetchImpl`), so the network is replaced before any module loads and a
 * test swaps the handler behind it.
 */
const net = vi.hoisted(() => {
  const state: { handler: ((input: RequestInfo | URL, init?: RequestInit) => Promise<Response>) | null } = {
    handler: null,
  };
  globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) =>
    state.handler
      ? state.handler(input, init)
      : Promise.resolve(new Response("{}", { status: 404, headers: { "content-type": "application/json" } }))) as typeof fetch;
  return state;
});

/** A stand-in for Cloudflare's api.js: renders nothing, hands out tokens. */
function fakeTurnstile() {
  const rendered: { sitekey: string; theme: string; language: string }[] = [];
  let issue: ((token: string) => void) | null = null;
  let n = 0;
  const api: TurnstileApi = {
    render(_container, options) {
      rendered.push({ sitekey: options.sitekey, theme: options.theme, language: options.language });
      issue = options.callback;
      options.callback(`tok-${++n}`);
      return "w1";
    },
    reset: vi.fn(() => {
      issue?.(`tok-${++n}`);
    }),
    remove: vi.fn(),
  };
  return { api, rendered };
}

function withSiteKey(name: string, siteKey: string | null): PagePayload {
  const payload = expandFixture(FIXTURES[name]) as PagePayload & {
    layouts: { root: { config: Record<string, unknown> } };
  };
  payload.layouts.root.config.turnstileSiteKey = siteKey;
  return payload;
}

async function mount(name: string, siteKey: string | null) {
  const payload = withSiteKey(name, siteKey);
  setLoaderRoutes(routes);
  setPageSource(async () => structuredClone(payload));
  const router = createMemoryRouter(routes, { initialEntries: [FIXTURES[name].url] });
  installRouter(router, routes);
  const view = render(<RouterProvider router={router} />);
  for (let i = 0; i < 200 && (!router.state.initialized || router.state.navigation.state !== "idle"); i++) {
    await act(() => new Promise((r) => setTimeout(r, 10)));
  }
  await act(() => new Promise((r) => setTimeout(r, 30)));
  return view;
}

const scripts = () => document.querySelectorAll(`script[src="${TURNSTILE_SCRIPT}"]`);

function type(container: HTMLElement, selector: string, value: string) {
  const input = container.querySelector(selector) as HTMLInputElement;
  fireEvent.change(input, { target: { value } });
}

async function submit(container: HTMLElement) {
  await act(async () => {
    fireEvent.submit(container.querySelector("form")!);
    await new Promise((r) => setTimeout(r, 30));
  });
}

beforeEach(() => {
  delete window.turnstile;
});

afterEach(() => {
  delete window.turnstile;
  setActionTransport(httpTransport);
  net.handler = null;
});

describe("the widget", () => {
  it("is absent, and Cloudflare's script never requested, without a site key", async () => {
    for (const name of ["auth-login", "auth-register", "auth-forgot", "auth-invite"]) {
      const view = await mount(name, null);
      expect(view.container.querySelector("[data-turnstile-widget]"), name).toBeNull();
      view.unmount();
    }
    expect(scripts()).toHaveLength(0);
  });

  it("renders in all four forms with a site key, theme and language included", async () => {
    const fake = fakeTurnstile();
    window.turnstile = fake.api;
    for (const name of ["auth-login", "auth-register", "auth-forgot", "auth-invite"]) {
      const view = await mount(name, SITE_KEY);
      const widget = view.container.querySelector("form [data-turnstile-widget]");
      expect(widget, name).not.toBeNull();
      view.unmount();
    }
    expect(fake.rendered).toHaveLength(4);
    expect(fake.rendered[0]).toEqual({ sitekey: SITE_KEY, theme: "light", language: "en" });
  });

  it("loads Cloudflare's script itself when it is not there yet", async () => {
    const view = await mount("auth-login", SITE_KEY);
    expect(scripts()).toHaveLength(1);
    view.unmount();
  });
});

describe("the token", () => {
  it("goes to the sign-in action as x-captcha-response, and the widget starts over", async () => {
    const fake = fakeTurnstile();
    window.turnstile = fake.api;
    const calls: { name: string; headers?: Record<string, string> }[] = [];
    setActionTransport(async (_module, name, _args, headers) => {
      calls.push({ name, headers });
      return { ok: { status: "captcha" } };
    });
    const view = await mount("auth-login", SITE_KEY);
    type(view.container, "#identifier", "alice");
    type(view.container, "#password", "secret password");
    await submit(view.container);
    expect(calls).toEqual([{ name: "signInWithIdentifier", headers: { "x-captcha-response": "tok-1" } }]);
    expect(fake.api.reset).toHaveBeenCalledWith("w1");
    // The refusal is shown in words, not as "wrong password".
    expect(view.container.textContent).toContain("The verification did not go through");
    // The next attempt carries the fresh token.
    await submit(view.container);
    expect(calls[1].headers).toEqual({ "x-captcha-response": "tok-2" });
  });

  it("is not sent, and the form asks for the check, while the widget has none", async () => {
    const api: TurnstileApi = { render: () => "w1", reset: vi.fn(), remove: vi.fn() };
    window.turnstile = api;
    const calls: unknown[] = [];
    setActionTransport(async (...args) => {
      calls.push(args);
      return { ok: { status: "ok" } };
    });
    const view = await mount("auth-login", SITE_KEY);
    type(view.container, "#identifier", "alice");
    type(view.container, "#password", "secret password");
    await submit(view.container);
    expect(calls).toHaveLength(0);
    expect(view.container.textContent).toContain("Complete the verification below first.");
  });

  it("goes to the invitation action too", async () => {
    window.turnstile = fakeTurnstile().api;
    const calls: { name: string; headers?: Record<string, string> }[] = [];
    setActionTransport(async (_module, name, _args, headers) => {
      calls.push({ name, headers });
      return { ok: { ok: false, reason: "captcha-unavailable" } };
    });
    const view = await mount("auth-invite", SITE_KEY);
    type(view.container, "#password", "long enough password");
    type(view.container, "#confirm", "long enough password");
    await submit(view.container);
    expect(calls.filter((c) => c.name === "acceptInvitation")).toEqual([
      { name: "acceptInvitation", headers: { "x-captcha-response": "tok-1" } },
    ]);
    expect(view.container.textContent).toContain("The verification service could not be reached");
  });

  it("goes to Better Auth's sign-up and reset requests as a header", async () => {
    window.turnstile = fakeTurnstile().api;
    const seen: { url: string; token: string | null }[] = [];
    net.handler = async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const headers = new Headers(init?.headers ?? (input instanceof Request ? input.headers : undefined));
      seen.push({ url, token: headers.get("x-captcha-response") });
      return new Response(JSON.stringify({ message: "Captcha verification failed", code: "VERIFICATION_FAILED" }), {
        status: 403,
        headers: { "content-type": "application/json" },
      });
    };

    const register = await mount("auth-register", SITE_KEY);
    type(register.container, "#name", "Alice");
    type(register.container, "#email", "alice@example.com");
    type(register.container, "#password", "long enough password");
    await submit(register.container);
    expect(seen.find((s) => s.url.endsWith("/api/auth/sign-up/email"))?.token).toBe("tok-1");
    expect(register.container.textContent).toContain("The verification did not go through");
    register.unmount();

    const forgot = await mount("auth-forgot", SITE_KEY);
    type(forgot.container, "#email", "alice@example.com");
    await submit(forgot.container);
    expect(seen.find((s) => s.url.endsWith("/api/auth/request-password-reset"))?.token).toMatch(/^tok-/);
    // A refused check is no "check your inbox".
    expect(forgot.container.textContent).toContain("The verification did not go through");
  });

  it("is never added without a site key", async () => {
    const calls: { headers?: Record<string, string> }[] = [];
    setActionTransport(async (_module, _name, _args, headers) => {
      calls.push({ headers });
      return { ok: { status: "invalid" } };
    });
    const view = await mount("auth-login", null);
    type(view.container, "#identifier", "alice");
    type(view.container, "#password", "secret password");
    await submit(view.container);
    expect(calls).toEqual([{ headers: undefined }]);
  });
});
