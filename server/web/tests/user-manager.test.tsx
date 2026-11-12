/**
 * Admin -> Users (src/overrides/components/admin/UserManager.tsx): the edit and
 * delete dialogs read the account back from the server when they open. An
 * answer that arrives after the dialog was closed and opened for somebody else
 * belongs to the first account and must not land in the second one's dialog.
 */
import { afterEach, describe, expect, it } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { NextIntlClientProvider } from "next-intl";
import { UserManager } from "@/components/admin/UserManager";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { AdminUserDetail, AdminUserRow } from "@/lib/admin-users";
import { httpTransport, setActionTransport } from "../src/runtime/rpc";
import { fixtureMessages } from "../src/fixtures/source";

function row(id: string, name: string, email: string): AdminUserRow {
  return {
    id,
    name,
    email,
    role: "user",
    banned: false,
    createdAt: "2026-10-01T10:00:00.000Z",
    tag: name.toLowerCase(),
    avatar: null,
    usedQuota: 0,
    maxQuota: -1,
    files: 3,
    allowRootShortLinks: false,
    emailVerified: true,
    invitePending: false,
  };
}

function detail(r: AdminUserRow): AdminUserDetail {
  return { ...r, username: null, twoFactorEnabled: false, publicProfile: false, skipTagInUrl: false, hasPassword: true };
}

const ANNA = row("u-anna", "Anna", "anna@example.com");
const BOB = row("u-bob", "Bob", "bob@example.com");

/** A promise the test settles by hand. */
function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

type Pending = { name: string; userId: string; answer: (ok: unknown) => void };

function serve(): { pending: Pending[]; calls: { name: string; args: unknown[] }[] } {
  const pending: Pending[] = [];
  const calls: { name: string; args: unknown[] }[] = [];
  setActionTransport(async (module, name, args) => {
    calls.push({ name, args });
    if (name === "listAdminUsers") return { ok: [ANNA, BOB] };
    if (module === "admin" && (name === "loadUserDetail" || name === "loadDeletionImpact")) {
      const d = deferred<unknown>();
      pending.push({ name, userId: String(args[0]), answer: d.resolve });
      return { ok: await d.promise };
    }
    return { ok: null };
  });
  return { pending, calls };
}

async function settle() {
  for (let i = 0; i < 5; i++) await act(() => new Promise((r) => setTimeout(r, 10)));
}

async function mount() {
  const view = render(
    <NextIntlClientProvider locale="en" messages={fixtureMessages("en")} timeZone="UTC">
      <TooltipProvider>
        <UserManager currentUserId="u-admin" mailReady />
      </TooltipProvider>
    </NextIntlClientProvider>,
  );
  await settle();
  return view;
}

/** Right-click the account's row and choose a menu entry. */
async function choose(email: string, item: string) {
  const cell = screen.getAllByText(email).find((el) => el.closest("tr"))!;
  fireEvent.contextMenu(cell.closest("tr")!);
  await settle();
  fireEvent.click(screen.getByRole("menuitem", { name: new RegExp(item) }));
  await settle();
}

async function cancelDialog() {
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  await settle();
}

afterEach(() => setActionTransport(httpTransport));

describe("the edit dialog", () => {
  it("shows the account it was opened for, whatever order the answers come in", async () => {
    const { pending, calls } = serve();
    await mount();

    await choose("anna@example.com", "Edit account");
    await cancelDialog();
    await choose("bob@example.com", "Edit account");
    expect(pending.map((p) => p.userId)).toEqual(["u-anna", "u-bob"]);

    // Bob's answer first, then Anna's, which was asked for earlier.
    await act(async () => pending[1].answer(detail(BOB)));
    await settle();
    await act(async () => pending[0].answer(detail(ANNA)));
    await settle();

    const email = document.querySelector<HTMLInputElement>("#eu-email");
    expect(email?.value).toBe("bob@example.com");

    // And what is saved goes to Bob.
    fireEvent.change(document.querySelector("#eu-name")!, { target: { value: "Robert" } });
    await settle();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await settle();
    const update = calls.find((c) => c.name === "updateUserAccount");
    expect(update?.args[0]).toBe("u-bob");
  });
});

describe("the delete dialog", () => {
  it("states what the account it was opened for holds", async () => {
    const { pending } = serve();
    await mount();

    await choose("anna@example.com", "Delete account");
    await cancelDialog();
    await choose("bob@example.com", "Delete account");
    expect(pending.map((p) => p.userId)).toEqual(["u-anna", "u-bob"]);

    await act(async () => pending[1].answer({ files: 3, bytes: 300, folders: 1, tokens: 0 }));
    await settle();
    await act(async () => pending[0].answer({ files: 3000, bytes: 3_000_000, folders: 9, tokens: 4 }));
    await settle();

    const dialog = screen.getByRole("dialog");
    expect(dialog.textContent).toContain("bob@example.com");
    expect(dialog.textContent).not.toContain("3,000");
    expect(dialog.textContent).toContain("300 B");
  });
});
