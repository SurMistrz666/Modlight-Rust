/**
 * The panel shell's drawer on a phone (src/overrides/components/app-shell.tsx).
 * The port moved the account card (with its Profile / Admin menu) and the app
 * name into the drawer; following one of those links must close the drawer like
 * every navigation row in it does, not leave it covering the new page.
 */
import { describe, expect, it } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { routes } from "../src/routes/tree";
import { setLoaderRoutes, setPageSource } from "../src/routes/loader";
import { installRouter } from "../src/runtime/navigation";
import { setActionTransport } from "../src/runtime/rpc";
import { expandFixture, FIXTURES } from "../src/fixtures/source";

async function settle(router: ReturnType<typeof createMemoryRouter>) {
  for (let i = 0; i < 200 && (!router.state.initialized || router.state.navigation.state !== "idle"); i++) {
    await act(() => new Promise((r) => setTimeout(r, 10)));
  }
  await act(() => new Promise((r) => setTimeout(r, 30)));
}

async function mount() {
  setActionTransport(async () => ({ ok: null }));
  setLoaderRoutes(routes);
  setPageSource(async (url) => {
    if (url.pathname === "/dashboard/profile") return expandFixture(FIXTURES["dashboard-profile"]);
    if (url.pathname === "/dashboard/admin") return expandFixture(FIXTURES["dashboard-admin"]);
    return expandFixture(FIXTURES["dashboard-empty"]);
  });
  const router = createMemoryRouter(routes, { initialEntries: ["/dashboard?all=1"] });
  installRouter(router, routes);
  const view = render(<RouterProvider router={router} />);
  await settle(router);
  return { router, view };
}

async function openDrawer() {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Menu" }));
  });
  const drawer = await screen.findByRole("dialog");
  expect(drawer.textContent).toContain("Navigation");
  return drawer;
}

describe("the phone drawer", () => {
  for (const [item, path] of [
    ["Profile", "/dashboard/profile"],
    ["Admin", "/dashboard/admin"],
  ] as const) {
    it(`closes when the account menu's ${item} is followed`, async () => {
      const { router, view } = await mount();
      const drawer = await openDrawer();
      // The account card in the drawer opens the account menu.
      const card = [...drawer.querySelectorAll("button")].find((b) => b.getAttribute("aria-label") === "Alicja")!;
      await act(async () => {
        fireEvent.keyDown(card, { key: "Enter" });
      });
      const link = await screen.findByRole("menuitem", { name: item });
      await act(async () => {
        fireEvent.click(link);
      });
      await settle(router);
      expect(router.state.location.pathname).toBe(path);
      expect(screen.queryByRole("dialog")).toBeNull();
      view.unmount();
    });
  }

  it("closes when the app name at its foot is followed", async () => {
    const { router, view } = await mount();
    const drawer = await openDrawer();
    const brand = [...drawer.querySelectorAll("a")].find((a) => a.textContent?.startsWith("Modlight"))!;
    await act(async () => {
      fireEvent.click(brand);
    });
    await settle(router);
    expect(router.state.location.pathname).toBe("/dashboard");
    expect(screen.queryByRole("dialog")).toBeNull();
    view.unmount();
  });
});
