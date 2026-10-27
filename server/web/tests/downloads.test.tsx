/**
 * Not upstream: the Downloads page (src/downloads/, server/src/downloads.rs)
 * and the shell that leads to it. A card per platform with the newest build
 * on top and the older ones folded; administrators also delete builds and read
 * how to publish (with this page's own origin, never a fixed domain); the
 * panel's chrome, the theme toggle included, lives in the left rail.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { routes } from "../src/routes/tree";
import { setLoaderRoutes, setPageSource } from "../src/routes/loader";
import { installRouter } from "../src/runtime/navigation";
import { expandFixture, FIXTURES } from "../src/fixtures/source";
import { splitNewest, type Build, type DownloadsList } from "../src/downloads/api";

const fetchMock = vi.fn<(input: RequestInfo | URL, init?: RequestInit) => Promise<Response>>();

beforeEach(() => {
  fetchMock.mockReset();
  fetchMock.mockResolvedValue(
    new Response(JSON.stringify({ deleted: true }), { status: 200, headers: { "content-type": "application/json" } }),
  );
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

async function settle() {
  await act(() => new Promise((r) => setTimeout(r, 30)));
}

async function mount(name: string) {
  const file = FIXTURES[name];
  setLoaderRoutes(routes);
  setPageSource(async () => expandFixture(file));
  const router = createMemoryRouter(routes, { initialEntries: [file.url] });
  installRouter(router, routes);
  const view = render(<RouterProvider router={router} />);
  for (let i = 0; i < 200 && (!router.state.initialized || router.state.navigation.state !== "idle"); i++) {
    await act(() => new Promise((r) => setTimeout(r, 10)));
  }
  await settle();
  return view;
}

function props(name: string): DownloadsList {
  return (FIXTURES[name].payload as { props: DownloadsList }).props;
}

function card(container: HTMLElement, platform: string): HTMLElement {
  const el = container.querySelector<HTMLElement>(`[data-platform="${platform}"]`);
  if (!el) throw new Error(`no ${platform} card`);
  return el;
}

function button(root: ParentNode, name: string): HTMLButtonElement {
  const hit = [...root.querySelectorAll<HTMLButtonElement>("button")].find(
    (b) => b.getAttribute("aria-label") === name || b.textContent?.trim() === name,
  );
  if (!hit) throw new Error(`no button "${name}"`);
  return hit;
}

describe("splitNewest", () => {
  it("puts the newest build of each architecture on top", () => {
    const b = (id: string, arch: "arm64" | "x64") => ({ id, arch }) as Build;
    const { newest, older } = splitNewest([b("1", "arm64"), b("2", "x64"), b("3", "arm64"), b("4", "x64")]);
    expect(newest.map((x) => x.id)).toEqual(["1", "2"]);
    expect(older.map((x) => x.id)).toEqual(["3", "4"]);
  });
});

describe("Downloads, as an administrator", () => {
  it("shows the newest build on top and folds the older ones", async () => {
    const { container } = await mount("dashboard-downloads");
    const [newest, intel, older] = props("dashboard-downloads").platforms[0].builds;
    const mac = card(container, "macos");

    expect(mac.textContent).toContain("Newest");
    expect(mac.textContent).toContain("1.2.0");
    expect(mac.textContent).toContain("build 45");
    expect(mac.textContent).toContain("Uploads resume after sleep.");
    const link = mac.querySelector<HTMLAnchorElement>(`a[data-download="${newest.id}"]`)!;
    expect(link.getAttribute("href")).toBe(newest.url);
    expect(link.getAttribute("download")).toBe(newest.filename);
    expect(link.textContent).toContain("Download for macOS");
    expect(mac.querySelector("[data-sha256]")?.textContent).toBe(newest.sha256);
    // The other architecture's newest build is offered beside it, not folded away.
    expect(mac.textContent).toContain("Also for Intel (x64)");
    expect(mac.querySelector(`a[href="${intel.url}"]`)).not.toBeNull();
    expect(mac.textContent).toContain(`Sign in to this server: ${window.location.origin}`);

    const toggle = button(mac, "Older builds (1)");
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    const list = document.getElementById(toggle.getAttribute("aria-controls")!)!;
    expect(list.className).toContain("hidden");
    fireEvent.click(toggle);
    await settle();
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(list.className).not.toContain("hidden");
    expect(list.querySelector(`a[href="${older.url}"]`)).not.toBeNull();
    expect(list.textContent).toContain("1.1.4");

    const windows = card(container, "windows");
    expect(windows.textContent).toContain("No builds for this platform yet");
    expect(windows.textContent).toContain("as soon as a build machine publishes it");
  });

  it("explains publishing with this page's origin and says the token is missing", async () => {
    const { container } = await mount("dashboard-downloads");
    const guide = container.querySelector<HTMLElement>("[data-publish-guide]")!;
    expect(guide).not.toBeNull();
    expect(guide.textContent).toContain("Publishing builds");
    expect(guide.textContent).toContain("No publish token");
    expect(guide.textContent).toContain("RELEASE_PUBLISH_TOKEN is not set");
    expect(guide.textContent).toContain(`POST ${window.location.origin}/api/downloads/upload`);
    expect(guide.textContent).toContain(`PUT ${window.location.origin}/api/downloads/upload/<upload>/0`);
    expect(guide.textContent).toContain("/finish");
    expect(guide.textContent).toContain("Modlight-<version>-<build>-windows-x64.zip");
    expect(guide.textContent).not.toContain("example.com");
  });

  it("deletes a build only after asking, with DELETE /api/downloads/{id}", async () => {
    const { container } = await mount("dashboard-downloads");
    const [newest] = props("dashboard-downloads").platforms[0].builds;
    const mac = card(container, "macos");
    fireEvent.click(button(mac, `Delete ${newest.filename}`));
    await settle();
    expect(fetchMock).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("Delete this build?");
    expect(document.body.textContent).toContain(newest.filename);
    fireEvent.click(button(document.body, "Delete build"));
    for (let i = 0; i < 20 && fetchMock.mock.calls.length === 0; i++) await settle();
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0];
    expect(String(url)).toBe(`/api/downloads/${newest.id}`);
    expect(init?.method).toBe("DELETE");
  });
});

describe("Downloads, as a regular account", () => {
  it("downloads, but neither deletes nor sees how to publish", async () => {
    const { container } = await mount("dashboard-downloads-user");
    const [build] = props("dashboard-downloads-user").platforms[1].builds;
    const windows = card(container, "windows");
    expect(windows.querySelector(`a[href="${build.url}"]`)).not.toBeNull();
    expect(windows.textContent).toContain("Download for Windows");
    expect(windows.textContent).toContain("64-bit (x64)");
    expect(windows.textContent).not.toMatch(/build \d/);
    expect(windows.textContent).toContain("build machine");
    expect(card(container, "macos").textContent).toContain("When a build for macOS is published, it will be here.");
    expect(container.querySelector("[data-publish-guide]")).toBeNull();
    expect([...container.querySelectorAll("button")].some((b) => b.textContent?.includes("Delete"))).toBe(false);
  });
});

describe("the shell", () => {
  it("keeps the theme toggle and the account menu in the rail, and lists Downloads", async () => {
    const { container } = await mount("dashboard-downloads-user");
    const rail = container.querySelector<HTMLElement>("aside[data-rail]")!;
    expect(rail).not.toBeNull();
    expect(rail.querySelector('[data-rail-theme] button[aria-label="Change theme"]')).not.toBeNull();
    const header = container.querySelector("header")!;
    expect(header.className).toContain("lg:hidden");
    expect(header.querySelector('button[aria-label="Change theme"]')).toBeNull();
    // The bar only opens the drawer and names the page.
    expect([...header.querySelectorAll("button")].map((b) => b.getAttribute("aria-label"))).toEqual(["Menu"]);
    expect(header.textContent).toContain("Downloads");
    const row = rail.querySelector<HTMLAnchorElement>('a[href="/dashboard/downloads"]')!;
    expect(row.textContent).toContain("Downloads");
    expect(row.getAttribute("aria-current")).toBe("page");
  });
});
