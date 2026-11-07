/**
 * Not upstream: the Settings card "Metadata in uploads" (src/strip-metadata/,
 * server/src/actions/strip_metadata.rs, docs/SKELETON.md section 9 item 47).
 * The server is replaced by an action transport that records every call.
 */
import { afterEach, describe, expect, it } from "vitest";
import { act, fireEvent, render } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { routes } from "../src/routes/tree";
import { setLoaderRoutes, setPageSource } from "../src/routes/loader";
import { installRouter } from "../src/runtime/navigation";
import { httpTransport, setActionTransport } from "../src/runtime/rpc";
import { expandFixture, fixtureMessages, FIXTURES } from "../src/fixtures/source";
import type { PagePayload } from "../src/runtime/payload";

type Call = { module: string; name: string; args: unknown[] };

async function mount(name: string, edit?: (payload: PagePayload) => void) {
  const payload = expandFixture(FIXTURES[name]);
  edit?.(payload);
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

/** Answer `strip-metadata` with `answer` (a value, or an Error to throw); record those calls. */
function serve(answer: (args: unknown[]) => unknown): Call[] {
  const calls: Call[] = [];
  setActionTransport(async (module, name, args) => {
    if (module !== "strip-metadata") return { ok: null };
    calls.push({ module, name, args });
    const result = answer(args);
    if (result instanceof Error) return { error: { message: result.message } };
    return { ok: result };
  });
  return calls;
}

async function settle() {
  await act(() => new Promise((r) => setTimeout(r, 40)));
}

const card = (view: ReturnType<typeof render>) => view.container.querySelector("[data-strip-metadata]")!;
const toggle = (view: ReturnType<typeof render>) => card(view).querySelector("button[role=switch]")!;

afterEach(() => {
  setActionTransport(httpTransport);
});

describe("the metadata card in Settings", () => {
  it("is off by default and says what it does, for which files, and that old files stay", async () => {
    serve(() => null);
    const view = await mount("dashboard-profile");
    expect(card(view).getAttribute("data-strip-metadata")).toBe("off");
    expect(toggle(view).getAttribute("aria-checked")).toBe("false");
    const text = card(view).textContent ?? "";
    expect(text).toContain("Remove metadata from files on upload");
    expect(text).toContain("Location (GPS)");
    expect(text).toContain("Files you have already uploaded stay as they are.");
    const formats = [...card(view).querySelectorAll("[data-strip-formats] > *")].map((b) => b.textContent);
    expect(formats).toEqual(expect.arrayContaining(["JPEG", "HEIC", "MP4", "PDF", "DOCX", "ODT"]));
    expect(card(view).querySelector("[data-strip-kept]")).toBeNull();
  });

  it("turns on through the server and shows what the server stored", async () => {
    const calls = serve(([enabled]) => ({ enabled, kept: [], keptTotal: 0, formats: ["JPEG"] }));
    const view = await mount("dashboard-profile");
    await act(async () => {
      fireEvent.click(toggle(view));
    });
    await settle();
    expect(calls).toEqual([{ module: "strip-metadata", name: "setStripMetadata", args: [true] }]);
    expect(card(view).getAttribute("data-strip-metadata")).toBe("on");
    expect(toggle(view).getAttribute("aria-checked")).toBe("true");
    expect(document.body.textContent).toContain("Metadata will be removed from your next uploads.");
  });

  it("puts the switch back when the server does not take it", async () => {
    serve(() => new Error("boom"));
    const view = await mount("dashboard-profile");
    await act(async () => {
      fireEvent.click(toggle(view));
    });
    await settle();
    expect(card(view).getAttribute("data-strip-metadata")).toBe("off");
    expect(toggle(view).getAttribute("aria-checked")).toBe("false");
    expect(toggle(view).hasAttribute("disabled")).toBe(false);
    expect(document.body.textContent).toContain("The setting could not be saved. Try again.");
  });

  it("lists the files that kept their metadata, with the reason and a link", async () => {
    serve(() => null);
    const view = await mount("dashboard-profile-encrypted");
    expect(card(view).getAttribute("data-strip-metadata")).toBe("on");
    const kept = card(view).querySelector("[data-strip-kept]")!;
    expect(kept.textContent).toContain("23 files kept their metadata");
    expect(kept.textContent).toContain("The 2 most recent are listed.");
    const rows = [...kept.querySelectorAll("li")];
    expect(rows.map((r) => r.getAttribute("data-kept-reason"))).toEqual(["too-large", "signed"]);
    expect(rows[0].textContent).toContain("too large to clean");
    expect(rows[1].textContent).toContain("digitally signed");
    expect(rows[0].querySelector("a")?.getAttribute("href")).toBe("/alicja/kx7wq2.mp4");
  });

  it("speaks Polish", async () => {
    serve(() => null);
    const view = await mount("dashboard-profile-encrypted", (payload) => {
      const root = (payload as unknown as { layouts: { root: Record<string, unknown> } }).layouts.root;
      root.locale = "pl";
      root.messages = fixtureMessages("pl");
    });
    const text = card(view).textContent ?? "";
    expect(text).toContain("Usuwaj metadane z plików przy wysyłaniu");
    expect(text).toContain("Pliki już wysłane zostają bez zmian.");
    expect(text).toContain("23 pliki zachowały metadane");
    expect(text).toContain("za duży do oczyszczenia");
  });
});
