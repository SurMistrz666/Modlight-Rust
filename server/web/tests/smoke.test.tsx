/**
 * Smoke test: every fixture page (fixtures/pages/*.json, at least one per
 * upstream route) is mounted through the real route tree, loader and layouts,
 * and must render without reaching an error boundary or logging an error.
 */
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { act, render } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { routes } from "../src/routes/tree";
import { setLoaderRoutes, setPageSource } from "../src/routes/loader";
import { installRouter } from "../src/runtime/navigation";
import { setActionTransport } from "../src/runtime/rpc";
import { decodeTagged } from "../src/runtime/tagged-json";
import { FIXTURES, expandFixture } from "../src/fixtures/source";
import { ROUTES } from "../src/contract/routes";
import { isOk } from "../src/runtime/payload";
import actions from "../fixtures/actions.json";

const errors: unknown[][] = [];

/**
 * Console errors that upstream itself produces, by fixture. Each one is an
 * upstream bug reproduced faithfully (docs/CONTRACT.md "Open questions").
 */
const UPSTREAM_ERRORS: Record<string, RegExp[]> = {
  // components/tokens/OneClickShareX.tsx passes `fileName: () => <span/>` to
  // t.rich for "Downloads {fileName} ...", where {fileName} is a plain argument,
  // not a tag: React gets a function as a child (dev warning, empty in prod).
  "dashboard-tokens": [/^Functions are not valid as a React child/],
};

beforeAll(() => {
  vi.spyOn(console, "error").mockImplementation((...args: unknown[]) => {
    errors.push(args);
  });
  setLoaderRoutes(routes);
  setActionTransport(async (module, name) => {
    const answer = (actions as Record<string, unknown>)[`${module}.${name}`];
    return answer === undefined ? { ok: null } : decodeTagged(answer);
  });
});

afterAll(() => {
  vi.restoreAllMocks();
});

async function mount(name: string) {
  const file = FIXTURES[name];
  setPageSource(async () => expandFixture(file));
  const router = createMemoryRouter(routes, { initialEntries: [file.url] });
  installRouter(router, routes);
  const view = render(<RouterProvider router={router} />);
  // The loader resolves the payload, loads translations and the page chunk.
  for (let i = 0; i < 200 && (!router.state.initialized || router.state.navigation.state !== "idle"); i++) {
    await act(() => new Promise((r) => setTimeout(r, 10)));
  }
  await act(() => new Promise((r) => setTimeout(r, 30)));
  return { router, view, file };
}

describe("every fixture page renders", () => {
  for (const name of Object.keys(FIXTURES)) {
    it(name, async () => {
      errors.length = 0;
      const { router, view, file } = await mount(name);
      const payload = expandFixture(file);
      const text = view.container.textContent ?? "";

      expect(router.state.initialized).toBe(true);
      expect(router.state.errors).toBeNull();
      expect(text).not.toContain("Something went wrong");
      if ("notFound" in payload) expect(text).toContain("This link leads nowhere");
      else expect(text).not.toContain("This link leads nowhere");
      if (isOk(payload) && payload.route !== "/") expect(view.container.innerHTML.length).toBeGreaterThan(200);
      if ("meta" in payload && payload.meta) expect(document.title).toBe(payload.meta.title);
      const known = UPSTREAM_ERRORS[name] ?? [];
      const unexpected = errors.map((e) => String(e[0])).filter((m) => !known.some((re) => re.test(m)));
      expect(unexpected.map((m) => m.slice(0, 300))).toEqual([]);
      view.unmount();
    });
  }
});

describe("fixtures cover every upstream page", () => {
  it("has at least one fixture per route", () => {
    const covered = new Set(Object.values(FIXTURES).map((f) => (f.payload as { route?: string }).route));
    const missing = Object.keys(ROUTES).filter((id) => !covered.has(id));
    expect(missing).toEqual([]);
  });
});
