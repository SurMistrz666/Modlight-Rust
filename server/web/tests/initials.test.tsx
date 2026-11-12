/**
 * Initials circles (src/avatar/initials.ts): the first two characters of a name
 * are whole characters as a reader sees them, never half of an emoji, a flag cut
 * out of its pair or a skin tone dropped from its hand. Upstream took
 * `text.slice(0, 2)` (UTF-16 code units).
 */
import { afterEach, describe, expect, it } from "vitest";
import { act, render } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { initials } from "../src/avatar/initials";
import { routes } from "../src/routes/tree";
import { setLoaderRoutes, setPageSource } from "../src/routes/loader";
import { installRouter } from "../src/runtime/navigation";
import { setActionTransport } from "../src/runtime/rpc";
import { expandFixture, FIXTURES } from "../src/fixtures/source";
import type { PagePayload } from "../src/runtime/payload";

const CASES: [string, string][] = [
  ["alicja@example.com", "AL"],
  ["Bob", "BO"],
  ["B😀b", "B😀"],
  ["🇵🇱Ola", "🇵🇱O"],
  ["👍🏽Ann", "👍🏽A"],
  ["👨‍👩‍👧 Family", "👨‍👩‍👧 "],
  ["é", "É"],
  ["", ""],
];

describe("initials", () => {
  for (const [text, expected] of CASES) {
    it(`of ${JSON.stringify(text)}`, () => {
      expect(initials(text)).toBe(expected);
    });
  }

  describe("without Intl.Segmenter", () => {
    const saved = Intl.Segmenter;
    afterEach(() => {
      (Intl as { Segmenter?: unknown }).Segmenter = saved;
    });

    it("still never splits a character", () => {
      (Intl as { Segmenter?: unknown }).Segmenter = undefined;
      expect(initials("B😀b")).toBe("B😀");
      expect(initials("𝒜lice")).toBe("𝒜L");
    });
  });
});

describe("the panel's account card", () => {
  it("draws a name that starts with a flag whole", async () => {
    setActionTransport(async () => ({ ok: null }));
    setLoaderRoutes(routes);
    const payload: PagePayload = expandFixture(FIXTURES["dashboard-empty"]);
    const app = (payload as unknown as { layouts: { "(app)": { user: { name: string } } } }).layouts["(app)"];
    app.user.name = "🇵🇱Ola";
    setPageSource(async () => structuredClone(payload));
    const router = createMemoryRouter(routes, { initialEntries: ["/dashboard?all=1"] });
    installRouter(router, routes);
    const view = render(<RouterProvider router={router} />);
    for (let i = 0; i < 200 && (!router.state.initialized || router.state.navigation.state !== "idle"); i++) {
      await act(() => new Promise((r) => setTimeout(r, 10)));
    }
    const fallbacks = [...view.container.querySelectorAll("[data-slot=avatar-fallback]")].map((el) => el.textContent);
    expect(fallbacks.length).toBeGreaterThan(0);
    for (const text of fallbacks) expect(text).toBe("🇵🇱O");
    view.unmount();
  });
});
