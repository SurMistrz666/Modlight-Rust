/**
 * The router's own error element (src/routes/root.tsx `RouterError`): when the
 * loader itself fails (a page chunk that cannot be loaded), the SPA reloads the
 * URL once. If it fails again after that reload, the visitor used to get an
 * empty page; now the page says it could not be loaded and offers to try again,
 * in the language of the dictionary the tab holds.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { routes } from "../src/routes/tree";
import { resetLoader, setLoaderRoutes, setPageSource, settle as settlePayload } from "../src/routes/loader";
import { installRouter } from "../src/runtime/navigation";
import { resetMessages } from "../src/runtime/messages";
import { setRootData } from "../src/runtime/root-data";
import { expandFixture, fixtureMessages, FIXTURES } from "../src/fixtures/source";

// RouterError keys the reload by the document's URL (a memory router leaves jsdom's alone).
const KEY = () => `ml-reload:${window.location.pathname}${window.location.search}`;

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  window.sessionStorage.clear();
});

afterEach(() => {
  vi.restoreAllMocks();
  window.sessionStorage.clear();
  resetMessages();
  resetLoader();
});

async function mountFailing() {
  setLoaderRoutes(routes);
  // The loader throws as it does when a page's code cannot be fetched.
  setPageSource(async () => {
    throw new TypeError("Failed to fetch dynamically imported module");
  });
  const router = createMemoryRouter(routes, { initialEntries: ["/dashboard"] });
  installRouter(router, routes);
  const view = render(<RouterProvider router={router} />);
  for (let i = 0; i < 50; i++) await act(() => new Promise((r) => setTimeout(r, 10)));
  return view;
}

describe("a page whose code keeps failing to load", () => {
  it("is reloaded once, and the reload is remembered", async () => {
    const view = await mountFailing();
    expect(window.sessionStorage.getItem(KEY())).toBe("1");
    view.unmount();
  });

  it("then says so and offers to try again instead of an empty page", async () => {
    window.sessionStorage.setItem(KEY(), "1");
    const view = await mountFailing();
    expect(view.container.textContent).toContain("This page could not be loaded");
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
    view.unmount();
  });

  it("speaks the language of the dictionary the tab already holds", async () => {
    // An earlier page of this visit brought the Polish dictionary.
    const payload = expandFixture(FIXTURES["dashboard-empty"]);
    const root = (payload as unknown as { layouts: { root: Record<string, unknown> } }).layouts.root;
    root.locale = "pl";
    root.messages = fixtureMessages("pl");
    await settlePayload(payload, new URL("http://localhost/dashboard"));
    setRootData(root as never);
    window.sessionStorage.setItem(KEY(), "1");
    const view = await mountFailing();
    expect(view.container.textContent).toContain("Nie udało się wczytać tej strony");
    expect(screen.getByRole("button", { name: "Spróbuj ponownie" })).toBeTruthy();
    view.unmount();
  });
});
