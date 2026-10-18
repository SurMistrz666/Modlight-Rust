/**
 * The wire formats the Rust server implements: tagged JSON, the RPC request
 * and envelope, the /_page request, and the navigation rules of <Link>.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { decodeTagged, encodeTagged } from "../src/runtime/tagged-json";
import { callAction, httpTransport, setActionTransport } from "../src/runtime/rpc";
import { pageDataUrl } from "../src/runtime/payload";
import { isRedirectError, MASKED_SERVER_ERROR } from "../src/runtime/errors";
import { installRouter, isSpaPath } from "../src/runtime/navigation";
import { routes } from "../src/routes/tree";
import { setLoaderRoutes, setPageSource } from "../src/routes/loader";
import { expandFixture, FIXTURES } from "../src/fixtures/source";
import * as uploads from "../src/rpc/actions/uploads";
import * as account from "../src/rpc/actions/account";

describe("tagged JSON", () => {
  it("round-trips the four tags and keeps undefined object keys", () => {
    const value = {
      when: new Date("2026-10-18T12:00:00.000Z"),
      missing: undefined,
      big: 123n,
      nan: Number.NaN,
      inf: -Infinity,
      list: [1, "a", null, undefined],
    };
    const wire = encodeTagged(value);
    expect(wire).toEqual({
      when: { $date: "2026-10-18T12:00:00.000Z" },
      missing: { $undefined: 1 },
      big: { $bigint: "123" },
      nan: { $num: "NaN" },
      inf: { $num: "-Infinity" },
      list: [1, "a", null, { $undefined: 1 }],
    });
    const back = decodeTagged<typeof value>(JSON.parse(JSON.stringify(wire)));
    expect(back.when).toBeInstanceOf(Date);
    expect("missing" in back).toBe(true);
    expect(back.big).toBe(123n);
    expect(Number.isNaN(back.nan)).toBe(true);
    expect(back.list).toEqual([1, "a", null, undefined]);
  });
});

describe("RPC", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("POSTs the passed arguments as a tagged JSON array", async () => {
    const calls: { url: string; init: RequestInit }[] = [];
    vi.stubGlobal("fetch", async (url: string, init: RequestInit) => {
      calls.push({ url, init });
      return new Response(JSON.stringify({ ok: { code: "new-vanity" } }), {
        headers: { "content-type": "application/json" },
      });
    });
    setActionTransport(httpTransport);
    await expect(uploads.setVanity("id-1", "new-vanity")).resolves.toEqual({ code: "new-vanity" });
    expect(calls).toHaveLength(1);
    expect(calls[0].url).toBe("/_rpc/uploads/setVanity");
    expect(calls[0].init.method).toBe("POST");
    expect((calls[0].init.headers as Record<string, string>)["X-Modlight-Rpc"]).toBe("1");
    expect(JSON.parse(String(calls[0].init.body))).toEqual(["id-1", "new-vanity"]);
  });

  it("sends only the arguments the caller passed (defaults are the server's)", async () => {
    const seen: unknown[][] = [];
    setActionTransport(async (_m, _n, args) => {
      seen.push(args);
      return { ok: { deleted: 0, bytes: 0, failed: 0 } };
    });
    await uploads.emptyTrash();
    await uploads.emptyTrash("bobby");
    expect(seen).toEqual([[], ["bobby"]]);
  });

  it("keeps undefined keys, as React's server-action encoding does", async () => {
    let body: unknown;
    setActionTransport(async (_m, _n, args) => {
      body = encodeTagged(args);
      return { ok: { theme: "discord", locale: null } };
    });
    await account.updatePreferences({ locale: undefined });
    expect(body).toEqual([{ locale: { $undefined: 1 } }]);
  });

  it("rejects with Error(message) and digest on an error envelope", async () => {
    setActionTransport(async () => ({ error: { message: MASKED_SERVER_ERROR, digest: "123" } }));
    const err = await uploads.trashUpload("x").catch((e: unknown) => e);
    expect(err).toBeInstanceOf(Error);
    expect((err as Error).message).toBe(MASKED_SERVER_ERROR);
    expect((err as { digest?: string }).digest).toBe("123");
  });

  it("rejects with Next's redirect error on a redirect envelope", async () => {
    setPageSource(async () => expandFixture(FIXTURES["auth-login"]));
    setLoaderRoutes(routes);
    installRouter(createMemoryRouter(routes, { initialEntries: ["/dashboard"] }), routes);
    setActionTransport(async () => ({ redirect: "/login" }));
    const err = await callAction("uploads", "trashUpload", ["x"]).catch((e: unknown) => e);
    expect(isRedirectError(err)).toBe(true);
    expect((err as { digest: string }).digest).toBe("NEXT_REDIRECT;push;/login;307;");
  });
});

describe("page data URL", () => {
  it("puts path first and keeps the page's own parameters, even one named path", () => {
    const url = new URL("https://x.test/dashboard?folder=a&path=b&page=2");
    expect(pageDataUrl(url)).toBe("/_page?path=%2Fdashboard&folder=a&path=b&page=2");
  });
});

describe("navigation", () => {
  it("knows which paths are SPA pages", () => {
    setLoaderRoutes(routes);
    const router = createMemoryRouter(routes, { initialEntries: ["/welcome"] });
    installRouter(router, routes);
    for (const p of ["/", "/login", "/dashboard", "/dashboard/admin/users/u1/files", "/alicja/x.png", "/x.png", "/a/t/s", "/u/t", "/s/c/unlock", "/s/t/n/unlock", "/invite/abc"]) {
      expect([p, isSpaPath(p)]).toEqual([p, true]);
    }
    for (const p of ["/s/promo", "/alicja/x/raw.png", "/a/t/s/zip", "/download/app", "/alicja/x/delete/tok"]) {
      expect([p, isSpaPath(p)]).toEqual([p, false]);
    }
  });

  it("turns a Link click into a client navigation that fetches the next page", async () => {
    const requested: string[] = [];
    setPageSource(async (url) => {
      requested.push(url.pathname + url.search);
      const name = url.pathname === "/dashboard/folders" ? "dashboard-folders" : "dashboard";
      return expandFixture(FIXTURES[name]);
    });
    setLoaderRoutes(routes);
    const router = createMemoryRouter(routes, { initialEntries: ["/dashboard?all=1"] });
    installRouter(router, routes);
    const view = render(<RouterProvider router={router} />);
    for (let i = 0; i < 200 && (!router.state.initialized || router.state.navigation.state !== "idle"); i++) {
      await act(() => new Promise((r) => setTimeout(r, 10)));
    }
    await act(() => new Promise((r) => setTimeout(r, 30)));
    // The "Folders" entry of the panel's sidebar.
    const link = view.container.querySelector('a[href="/dashboard/folders"]');
    expect(link).not.toBeNull();
    await act(async () => {
      fireEvent.click(link!, { button: 0 });
    });
    for (let i = 0; i < 200 && (router.state.navigation.state !== "idle" || router.state.location.pathname !== "/dashboard/folders"); i++) {
      await act(() => new Promise((r) => setTimeout(r, 10)));
    }
    expect(router.state.location.pathname).toBe("/dashboard/folders");
    expect(requested).toEqual(["/dashboard?all=1", "/dashboard/folders"]);
    view.unmount();
  });
});
