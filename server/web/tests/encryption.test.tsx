/**
 * Per-account file encryption in the panel (src/encryption/, server/src/vault/,
 * docs/ENCRYPTION.md): the unlock page of a link and of a gallery, the Settings
 * card, the library banner and the admin notes. The server is replaced by an
 * action transport that records every call.
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

/** Answer the `encryption` actions; record only those (the pages call others). */
function answer(answers: Record<string, unknown>): Call[] {
  const calls: Call[] = [];
  setActionTransport(async (module, name, args) => {
    if (module === "encryption") calls.push({ module, name, args });
    return { ok: answers[`${module}.${name}`] ?? null };
  });
  return calls;
}

function type(el: Element | null, value: string) {
  fireEvent.change(el as HTMLInputElement, { target: { value } });
}

async function settle() {
  await act(() => new Promise((r) => setTimeout(r, 40)));
}

afterEach(() => {
  setActionTransport(httpTransport);
});

describe("the unlock page of a link", () => {
  it("names nothing about the file and remembers for 30 days by default", async () => {
    const calls = answer({ "encryption.unlockOwnerFiles": { ok: false, reason: "wrong" } });
    const view = await mount("media-vault");
    const text = view.container.textContent ?? "";
    expect(text).toContain("This file is protected");
    expect(text).toContain("Remember on this browser for 30 days");
    expect(text).not.toContain("vaulted.png");
    const remember = view.container.querySelector("#ml-vault-remember");
    expect(remember?.getAttribute("data-state")).toBe("checked");

    type(view.container.querySelector("#ml-vault-password"), "not the password");
    await act(async () => {
      fireEvent.submit(view.container.querySelector("form")!);
    });
    await settle();
    expect(calls).toEqual([
      {
        module: "encryption",
        name: "unlockOwnerFiles",
        args: [{ mediaId: "aaaaaaaa-0000-4000-8000-000000000009" }, "not the password", true],
      },
    ]);
    expect(view.container.querySelector("[role=alert]")?.textContent).toBe("Wrong password. Try again.");
    // The field is emptied for the next try.
    expect((view.container.querySelector("#ml-vault-password") as HTMLInputElement).value).toBe("");
  });

  it("says how long to wait when the attempts ran out", async () => {
    answer({ "encryption.unlockOwnerFiles": { ok: false, reason: "rate-limited", retryAfter: 300 } });
    const view = await mount("media-vault");
    type(view.container.querySelector("#ml-vault-password"), "x");
    fireEvent.click(view.container.querySelector("#ml-vault-remember")!);
    await act(async () => {
      fireEvent.submit(view.container.querySelector("form")!);
    });
    await settle();
    expect(view.container.querySelector("[role=alert]")?.textContent).toBe(
      "Too many attempts. Try again in 5 minutes.",
    );
  });

  it("asks the gallery's visitors for the owner's password, by tag", async () => {
    const calls = answer({ "encryption.unlockOwnerFiles": { ok: true } });
    const view = await mount("user-gallery-vault");
    expect(view.container.textContent).toContain("These files are protected");
    type(view.container.querySelector("#ml-vault-password"), "correct horse");
    await act(async () => {
      fireEvent.submit(view.container.querySelector("form")!);
    });
    await settle();
    expect(calls[0].args).toEqual([{ tag: "alicja" }, "correct horse", true]);
  });

  it("is in Polish for a Polish visitor", async () => {
    answer({});
    const view = await mount("media-vault", (p) => {
      const root = (p as unknown as { layouts: { root: Record<string, unknown> } }).layouts.root;
      root.locale = "pl";
      root.messages = fixtureMessages("pl");
    });
    const text = view.container.textContent ?? "";
    expect(text).toContain("Ten plik jest chroniony");
    expect(text).toContain("Zapamiętaj w tej przeglądarce na 30 dni");
  });
});

describe("the Settings card", () => {
  it("turns encryption on only with the warning acknowledged", async () => {
    const calls = answer({
      "encryption.enableEncryption": { ok: true },
      "encryption.loadEncryptionStatus": {
        state: "encrypting",
        unlocked: true,
        progress: { done: 0, total: 42 },
        error: false,
        since: null,
        grants: [],
        minLength: 8,
      },
    });
    const view = await mount("dashboard-profile");
    const card = view.container.querySelector("[data-encryption-state]")!;
    expect(card.getAttribute("data-encryption-state")).toBe("off");
    const turnOn = [...card.querySelectorAll("button")].find((b) => b.textContent?.includes("Turn on encryption"))!;
    await act(async () => {
      fireEvent.click(turnOn);
    });
    await settle();
    const dialog = document.querySelector("[role=dialog]")!;
    expect(dialog.textContent).toContain("A forgotten password means lost files");
    type(dialog.querySelector("#ml-vault-new"), "correct horse battery");
    type(dialog.querySelector("#ml-vault-confirm"), "correct horse battery");
    const submit = [...dialog.querySelectorAll("button")].find((b) => b.textContent?.includes("Encrypt my files"))!;
    // Not before the box is ticked.
    expect(submit.hasAttribute("disabled")).toBe(true);
    fireEvent.click(dialog.querySelector("button[role=checkbox]")!);
    expect(submit.hasAttribute("disabled")).toBe(false);
    await act(async () => {
      fireEvent.click(submit);
    });
    await settle();
    expect(calls.map((c) => c.name)).toEqual(["enableEncryption", "loadEncryptionStatus"]);
    expect(calls[0].args).toEqual(["correct horse battery", "correct horse battery", true]);
    expect(view.container.querySelector("[data-encryption-state]")?.getAttribute("data-encryption-state")).toBe(
      "encrypting",
    );
    expect(view.container.textContent).toContain("Encrypting your files: 0 of 42");
  });

  it("refuses two different passwords without asking the server", async () => {
    const calls = answer({});
    const view = await mount("dashboard-profile");
    const card = view.container.querySelector("[data-encryption-state]")!;
    await act(async () => {
      fireEvent.click([...card.querySelectorAll("button")].find((b) => b.textContent?.includes("Turn on"))!);
    });
    await settle();
    const dialog = document.querySelector("[role=dialog]")!;
    type(dialog.querySelector("#ml-vault-new"), "correct horse battery");
    type(dialog.querySelector("#ml-vault-confirm"), "correct horse batterY");
    fireEvent.click(dialog.querySelector("button[role=checkbox]")!);
    await act(async () => {
      fireEvent.submit(dialog.querySelector("form")!);
    });
    await settle();
    expect(calls).toEqual([]);
    expect(dialog.querySelector("p[role=alert]")?.textContent).toBe("The two passwords are different.");
  });

  it("shows progress, this browser and the grants, and revokes one", async () => {
    const calls = answer({ "encryption.revokeVaultGrant": { ok: true } });
    const view = await mount("dashboard-profile-encrypted");
    const text = view.container.textContent ?? "";
    expect(text).toContain("Encrypting your files: 17 of 42");
    expect(text).toContain("Unlocked on this browser");
    expect(text).toContain("Chrome 141 · macOS");
    expect(text).toContain("This browser");
    expect(text).toContain("Link visitor");
    const visitor = view.container.querySelector("[data-grant-kind=visitor]")!;
    await act(async () => {
      fireEvent.click([...visitor.querySelectorAll("button")].find((b) => b.textContent === "Revoke")!);
    });
    await settle();
    expect(calls[0]).toEqual({
      module: "encryption",
      name: "revokeVaultGrant",
      args: ["0b9a3c4e-0000-4000-8000-000000000002"],
    });
  });
});

describe("the library", () => {
  it("offers the password while this browser is locked", async () => {
    const calls = answer({ "encryption.unlockMyFiles": { ok: false, reason: "wrong" } });
    const view = await mount("dashboard-vault");
    const banner = view.container.querySelector("[data-vault-banner]")!;
    expect(banner.textContent).toContain("Your files are encrypted");
    type(banner.querySelector("input[type=password]"), "guess");
    await act(async () => {
      fireEvent.submit(banner);
    });
    await settle();
    expect(calls[0]).toEqual({ module: "encryption", name: "unlockMyFiles", args: ["guess", true] });
    expect(banner.querySelector("[role=alert]")?.textContent).toBe("Wrong password.");
  });

  it("says nothing for an account without encryption, or once unlocked", async () => {
    answer({});
    const plain = await mount("dashboard");
    expect(plain.container.querySelector("[data-vault-banner]")).toBeNull();
    plain.unmount();
    const unlocked = await mount("dashboard-vault", (p) => {
      (p as unknown as { props: { vault: { unlocked: boolean } } }).props.vault.unlocked = true;
    });
    expect(unlocked.container.querySelector("[data-vault-banner]")).toBeNull();
  });
});

describe("the admin pages", () => {
  it("tell an administrator that an account's files cannot be opened", async () => {
    answer({});
    const view = await mount("dashboard-admin-user-files", (p) => {
      (p as unknown as { props: Record<string, unknown> }).props.encryption = "on";
    });
    expect(view.container.querySelector("[data-admin-vault-notice]")?.textContent).toContain(
      "Administrators cannot open, preview or download them",
    );
    expect(view.container.textContent).toContain("Encrypted");
  });
});
