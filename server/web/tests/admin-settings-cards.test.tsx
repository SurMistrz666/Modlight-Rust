/**
 * Admin -> Settings and Admin -> Mail: the Cloudflare Turnstile card
 * (src/turnstile/TurnstileSettings.tsx) and the SMTP card it copies
 * (src/overrides/components/admin/MailSettings.tsx). Their saves run inside
 * React transitions; a request that failed outright used to throw out of the
 * transition into the dashboard's error boundary, so the whole admin page
 * turned into "Something went wrong" and the secret typed into the card was
 * gone. The failure now shows in the card, and what was typed stays.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { routes } from "../src/routes/tree";
import { setLoaderRoutes, setPageSource } from "../src/routes/loader";
import { installRouter } from "../src/runtime/navigation";
import { httpTransport, setActionTransport } from "../src/runtime/rpc";
import { expandFixture, FIXTURES } from "../src/fixtures/source";

const calls: string[] = [];

afterEach(() => {
  calls.length = 0;
  setActionTransport(httpTransport);
  vi.restoreAllMocks();
});

/** Every action fails the way a dropped connection does. */
function failEverything() {
  setActionTransport(async (_module, name) => {
    calls.push(name);
    throw new TypeError("Failed to fetch");
  });
}

async function settle(router?: ReturnType<typeof createMemoryRouter>) {
  for (let i = 0; i < 200 && router && (!router.state.initialized || router.state.navigation.state !== "idle"); i++) {
    await act(() => new Promise((r) => setTimeout(r, 10)));
  }
  for (let i = 0; i < 5; i++) await act(() => new Promise((r) => setTimeout(r, 10)));
}

async function mountAdmin(tab: string) {
  // The error boundary logs what it caught; that is the failure under test.
  vi.spyOn(console, "error").mockImplementation(() => {});
  setLoaderRoutes(routes);
  setPageSource(async () => expandFixture(FIXTURES["dashboard-admin"]));
  const router = createMemoryRouter(routes, { initialEntries: ["/dashboard/admin"] });
  installRouter(router, routes);
  const view = render(<RouterProvider router={router} />);
  await settle(router);
  await act(async () => {
    fireEvent.mouseDown(screen.getByRole("tab", { name: tab }));
  });
  await settle();
  return view;
}

function type(selector: string, value: string) {
  fireEvent.change(document.querySelector(selector)!, { target: { value } });
}

describe("a failed save", () => {
  it("on the Turnstile card stays in the card and keeps the typed keys", async () => {
    const view = await mountAdmin("Settings");
    type("#turnstile-site-key", "0x4AAAAAAAtyped");
    type("#turnstile-secret", "0x4AAAAAAAsecret");
    failEverything();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Save Turnstile settings" }));
    });
    await settle();
    expect(calls).toEqual(["saveTurnstileSettings"]);
    // Still the admin page, not the error page.
    expect(screen.getByRole("tab", { name: "Settings" })).toBeTruthy();
    const card = document.querySelector("#turnstile-secret")!.closest("[data-slot=card]")!;
    expect(card.querySelector("[role=alert]")?.textContent).toContain("Couldn't save the Turnstile settings");
    expect(document.querySelector<HTMLInputElement>("#turnstile-site-key")?.value).toBe("0x4AAAAAAAtyped");
    expect(document.querySelector<HTMLInputElement>("#turnstile-secret")?.value).toBe("0x4AAAAAAAsecret");
    // And the button works again.
    expect(screen.getByRole("button", { name: "Save Turnstile settings" }).hasAttribute("disabled")).toBe(false);
    view.unmount();
  });

  it("on the mail card stays in the card and keeps the typed password", async () => {
    const view = await mountAdmin("Mail");
    type("#smtp-host", "smtp.typed.example");
    type("#smtp-password", "typed secret");
    failEverything();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Save mail settings" }));
    });
    await settle();
    expect(calls).toEqual(["saveSmtpSettings"]);
    expect(screen.getByRole("tab", { name: "Mail" })).toBeTruthy();
    const card = document.querySelector("#smtp-password")!.closest("[data-slot=card]")!;
    expect(card.querySelector("[role=alert]")?.textContent).toContain("Couldn't save the mail settings");
    expect(document.querySelector<HTMLInputElement>("#smtp-host")?.value).toBe("smtp.typed.example");
    expect(document.querySelector<HTMLInputElement>("#smtp-password")?.value).toBe("typed secret");
    view.unmount();
  });

  it("of the address-confirmation switch says so beside it", async () => {
    const view = await mountAdmin("Mail");
    failEverything();
    await act(async () => {
      fireEvent.click(document.querySelector("#set-email_verification")!);
    });
    await settle();
    expect(calls).toEqual(["setVerificationRequired"]);
    expect(screen.getByRole("tab", { name: "Mail" })).toBeTruthy();
    const card = document.querySelector("#set-email_verification")!.closest("[data-slot=card]")!;
    expect(card.querySelector("[role=alert]")?.textContent).toBeTruthy();
    // The switch shows what is stored, which did not change.
    expect(document.querySelector("#set-email_verification")?.getAttribute("data-state")).toBe("unchecked");
    view.unmount();
  });
});
