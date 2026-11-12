/**
 * `?next=` on /login and /2fa (src/pages/_lib/safe-next.ts): after signing in
 * the form goes there, so it must be a path on this site. Upstream let through
 * values the URL parser reads as another host ("/\evil.example", a tab or a
 * line break between the slashes), which made /login an open redirect.
 */
import { afterEach, describe, expect, it } from "vitest";
import { act, fireEvent, render } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { routes } from "../src/routes/tree";
import { setLoaderRoutes, setPageSource } from "../src/routes/loader";
import { installRouter } from "../src/runtime/navigation";
import { httpTransport, setActionTransport } from "../src/runtime/rpc";
import { expandFixture, FIXTURES } from "../src/fixtures/source";
import { safeNext } from "../src/pages/_lib/safe-next";

afterEach(() => setActionTransport(httpTransport));

describe("safeNext", () => {
  it("keeps paths on this site, query and hash included", () => {
    for (const ok of ["/dashboard", "/dashboard/profile", "/dashboard?folder=a&page=2", "/a/b#c", "/%5Cnot-a-slash"]) {
      expect(safeNext(ok)).toBe(ok);
    }
    expect(safeNext(["/dashboard/trash", "/x"])).toBe("/dashboard/trash");
  });

  it("refuses anything that leads to another host", () => {
    for (const bad of [
      undefined,
      "",
      "dashboard",
      "https://evil.example/",
      "//evil.example",
      "/\\evil.example",
      "/\\/evil.example",
      "/\t/evil.example",
      "/\n/evil.example",
      "/\r/evil.example",
      "/a\u0000b",
      "/a\u0085b",
    ]) {
      expect(safeNext(bad), JSON.stringify(bad)).toBe("/dashboard");
    }
  });
});

describe("signing in with a crafted next", () => {
  it("lands on the dashboard, not on another site", async () => {
    setActionTransport(async (module, name) => {
      if (module === "login-identifier" && name === "signInWithIdentifier") return { ok: { status: "ok" } };
      return { ok: null };
    });
    setLoaderRoutes(routes);
    setPageSource(async (url) =>
      expandFixture(url.pathname === "/login" ? FIXTURES["auth-login"] : FIXTURES["dashboard-empty"]),
    );
    const router = createMemoryRouter(routes, { initialEntries: ["/login?next=%2F%5Cevil.example%2Fphish"] });
    installRouter(router, routes);
    const view = render(<RouterProvider router={router} />);
    for (let i = 0; i < 200 && (!router.state.initialized || router.state.navigation.state !== "idle"); i++) {
      await act(() => new Promise((r) => setTimeout(r, 10)));
    }

    fireEvent.change(view.container.querySelector("#identifier")!, { target: { value: "alicja" } });
    fireEvent.change(view.container.querySelector("#password")!, { target: { value: "correct horse" } });
    await act(async () => {
      fireEvent.submit(view.container.querySelector("form")!);
    });
    for (let i = 0; i < 200 && router.state.location.pathname === "/login"; i++) {
      await act(() => new Promise((r) => setTimeout(r, 10)));
    }
    expect(router.state.location.pathname).toBe("/dashboard");
    view.unmount();
  });
});
