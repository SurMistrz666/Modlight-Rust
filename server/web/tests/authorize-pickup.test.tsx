/**
 * Not upstream: the desktop sign-in pickup on /authorize (SKELETON.md 9). With
 * a `challenge` in the request, approve and deny carry it to the server, the
 * page says the app picks the key up by itself, the loopback POST is still
 * tried, and the key stays available to copy. Without one, upstream's flow.
 */
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { act, fireEvent, render } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { routes } from "../src/routes/tree";
import { setLoaderRoutes, setPageSource } from "../src/routes/loader";
import { installRouter } from "../src/runtime/navigation";
import { setActionTransport } from "../src/runtime/rpc";
import { expandFixture, FIXTURES } from "../src/fixtures/source";

const CHALLENGE = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

const calls: { module: string; name: string; args: unknown[] }[] = [];
const posts: { url: string; body: string }[] = [];

beforeAll(() => {
  setLoaderRoutes(routes);
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    posts.push({ url: String(input), body: String(init?.body ?? "") });
    // What Safari does: the loopback request never goes out.
    throw new TypeError("Load failed");
  }) as typeof fetch;
});

afterEach(() => {
  calls.length = 0;
  posts.length = 0;
});

async function mount(fixture: string) {
  setActionTransport(async (module, name, args) => {
    calls.push({ module, name, args });
    if (name === "approveDesktopClient") {
      return { ok: { ok: true, token: "tok-123456", callback: "http://127.0.0.1:53682/", state: "abcdefghijklmnop" } };
    }
    return { ok: { ok: true } };
  });
  const file = FIXTURES[fixture];
  setPageSource(async () => expandFixture(file));
  const router = createMemoryRouter(routes, { initialEntries: [file.url] });
  installRouter(router, routes);
  const view = render(<RouterProvider router={router} />);
  for (let i = 0; i < 200 && (!router.state.initialized || router.state.navigation.state !== "idle"); i++) {
    await act(() => new Promise((r) => setTimeout(r, 10)));
  }
  await act(() => new Promise((r) => setTimeout(r, 30)));
  return view;
}

async function settle() {
  for (let i = 0; i < 20; i++) await act(() => new Promise((r) => setTimeout(r, 10)));
}

describe("authorize with a pickup challenge", () => {
  it("approves with the challenge and says the app collects the key itself", async () => {
    const view = await mount("auth-authorize-pickup");
    expect(view.container.textContent).toContain("collects the key from this server itself");
    fireEvent.click(view.getByText("Approve and connect"));
    await settle();
    expect(calls).toHaveLength(1);
    expect(calls[0].name).toBe("approveDesktopClient");
    expect(calls[0].args[0]).toMatchObject({ port: "53682", state: "abcdefghijklmnop", challenge: CHALLENGE });
    expect(view.container.textContent).toContain("is picking up its key");
    // The loopback was still tried, and the key is there as a last resort.
    expect(posts.some((p) => p.url === "http://127.0.0.1:53682/" && p.body.includes("tok-123456"))).toBe(true);
    expect(view.container.textContent).toContain("tok-123456");
    expect(view.container.textContent).not.toContain("did not answer");
    view.unmount();
  });

  it("records a refusal for the pickup", async () => {
    const view = await mount("auth-authorize-pickup");
    fireEvent.click(view.getByText("Not now"));
    await settle();
    expect(calls.map((c) => c.name)).toEqual(["denyDesktopClient"]);
    expect(calls[0].args[0]).toMatchObject({ state: "abcdefghijklmnop", challenge: CHALLENGE });
    expect(view.container.textContent).toContain("Nothing was issued");
    view.unmount();
  });
});

describe("authorize without a challenge (older apps)", () => {
  it("keeps upstream's flow: no challenge, manual key when the loopback fails", async () => {
    const view = await mount("auth-authorize");
    fireEvent.click(view.getByText("Approve and connect"));
    await settle();
    expect(calls[0].args[0]).not.toHaveProperty("challenge");
    expect(view.container.textContent).toContain("did not answer");
    expect(view.getByText("Send it to the app again")).toBeTruthy();
    view.unmount();
  });

  it("denies without calling the server", async () => {
    const view = await mount("auth-authorize");
    fireEvent.click(view.getByText("Not now"));
    await settle();
    expect(calls).toHaveLength(0);
    view.unmount();
  });
});
