/**
 * The six sign-in forms (src/overrides/components/auth/) when their request
 * gets no usable answer: no connection, a proxy's error page during a restart,
 * a server error. Upstream awaited the call without a catch, so the rejection
 * went nowhere and the form stayed on "Signing in…" with every field disabled
 * until the page was reloaded. Now it says so and can be sent again.
 */
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { routes } from "../src/routes/tree";
import { setLoaderRoutes, setPageSource } from "../src/routes/loader";
import { installRouter } from "../src/runtime/navigation";
import { httpTransport, setActionTransport } from "../src/runtime/rpc";
import { expandFixture, FIXTURES } from "../src/fixtures/source";

const GENERIC = "Something went wrong. Please try again.";

let calls: string[] = [];

/** What Better Auth's client gets from fetch. It keeps the fetch it found when
 * it was created, so the stub stays put and only its answer changes. */
let fetchAnswer: (input: RequestInfo | URL) => Promise<Response>;

beforeAll(() => {
  vi.stubGlobal("fetch", (input: RequestInfo | URL) => {
    calls.push(String(input));
    return fetchAnswer(input);
  });
});

afterAll(() => vi.unstubAllGlobals());

beforeEach(() => {
  calls = [];
  // Every RPC call fails the way a dropped connection does.
  setActionTransport(async (_module, name) => {
    calls.push(name);
    throw new TypeError("Failed to fetch");
  });
  // And so does every Better Auth call.
  fetchAnswer = async () => {
    throw new TypeError("Failed to fetch");
  };
});

afterEach(() => setActionTransport(httpTransport));

async function mount(fixture: string) {
  const file = FIXTURES[fixture];
  setLoaderRoutes(routes);
  setPageSource(async () => expandFixture(file));
  const router = createMemoryRouter(routes, { initialEntries: [file.url] });
  installRouter(router, routes);
  const view = render(<RouterProvider router={router} />);
  for (let i = 0; i < 200 && (!router.state.initialized || router.state.navigation.state !== "idle"); i++) {
    await act(() => new Promise((r) => setTimeout(r, 10)));
  }
  return view;
}

function fill(view: ReturnType<typeof render>, values: Record<string, string>) {
  for (const [id, value] of Object.entries(values)) {
    fireEvent.change(view.container.querySelector(`#${id}`)!, { target: { value } });
  }
}

async function submit(view: ReturnType<typeof render>) {
  await act(async () => {
    fireEvent.submit(view.container.querySelector("form")!);
  });
  for (let i = 0; i < 10; i++) await act(() => new Promise((r) => setTimeout(r, 10)));
}

function expectReadyAgain(view: ReturnType<typeof render>) {
  expect(view.container.textContent).toContain(GENERIC);
  const button = view.container.querySelector<HTMLButtonElement>("button[type=submit]");
  expect(button?.disabled).toBe(false);
}

describe("a request that gets no answer", () => {
  it("sign-in says so and can be sent again", async () => {
    const view = await mount("auth-login");
    fill(view, { identifier: "alicja", password: "correct horse" });
    await submit(view);
    expect(calls).toEqual(["signInWithIdentifier"]);
    expectReadyAgain(view);
    view.unmount();
  });

  it("accepting an invitation says so and can be sent again", async () => {
    const view = await mount("auth-invite");
    fill(view, { password: "correct horse", confirm: "correct horse" });
    await submit(view);
    expect(calls).toEqual(["acceptInvitation"]);
    expectReadyAgain(view);
    view.unmount();
  });

  it("registration says so and can be sent again", async () => {
    const view = await mount("auth-register");
    fill(view, { name: "Alicja", email: "alicja@example.com", password: "correct horse", registrationKey: "k" });
    await submit(view);
    expect(calls.some((c) => c.includes("/sign-up/email"))).toBe(true);
    expectReadyAgain(view);
    view.unmount();
  });

  it("a password reset request says so and can be sent again", async () => {
    const view = await mount("auth-forgot");
    fill(view, { email: "alicja@example.com" });
    await submit(view);
    expect(calls.some((c) => c.includes("/request-password-reset"))).toBe(true);
    expectReadyAgain(view);
    // Not the neutral "check your inbox": nothing was sent.
    expect(view.container.textContent).not.toContain("If an account exists");
    view.unmount();
  });

  // Round 2: the two remaining forms, ported for this (they were upstream copies).
  it("a two-factor check says so and can be sent again", async () => {
    const view = await mount("auth-2fa");
    fill(view, { code: "123456" });
    await submit(view);
    expect(calls.some((c) => c.includes("/two-factor/verify-totp"))).toBe(true);
    expectReadyAgain(view);
    view.unmount();
  });

  it("a new password says so and can be sent again", async () => {
    const view = await mount("auth-reset");
    fill(view, { password: "correct horse", confirm: "correct horse" });
    await submit(view);
    expect(calls.some((c) => c.includes("/reset-password"))).toBe(true);
    expectReadyAgain(view);
    view.unmount();
  });
});

describe("a password reset request the rate limit refused", () => {
  it("is not confirmed as sent", async () => {
    fetchAnswer = async () =>
      new Response(JSON.stringify({ message: "Too many requests. Please try again later." }), {
        status: 429,
        headers: { "content-type": "application/json", "x-retry-after": "60" },
      });
    const view = await mount("auth-forgot");
    fill(view, { email: "alicja@example.com" });
    await submit(view);
    expect(view.container.textContent).toContain("Too many attempts. Try again in a few minutes.");
    expect(view.container.textContent).not.toContain("If an account exists");
    expect(view.container.querySelector<HTMLButtonElement>("button[type=submit]")?.disabled).toBe(false);
    view.unmount();
  });
});
