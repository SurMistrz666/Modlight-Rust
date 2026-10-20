import { test, expect, type Page } from "@playwright/test";

// E2E for the ShareX API contract, token management UI, and admin settings.
// Runs against a live dev/prod server (E2E_BASE_URL or http://localhost:8420).
//
// These flows touch the network (register -> token -> upload -> delete) and the
// real DB, so give each test generous headroom.
test.setTimeout(120_000);

// Seeded admin from the dev environment.
const ADMIN_EMAIL = "admin@local.test";
const ADMIN_PASSWORD = "Password123!";

/** A throwaway email that is unique per invocation (timestamp + random suffix). */
function uniqueEmail(prefix = "e2e"): string {
  const suffix = Math.random().toString(36).slice(2, 8);
  return `${prefix}-${Date.now()}-${suffix}@example.com`;
}

/** Register a brand-new account via the /register UI; better-auth auto-logs in. */
async function registerFreshUser(page: Page): Promise<{ email: string; password: string }> {
  const email = uniqueEmail();
  const password = "Password123!";

  await page.goto("/register");

  await page.locator("#name").fill("E2E Tester");
  await page.locator("#email").fill(email);
  await page.locator("#password").fill(password);

  await page.getByRole("button", { name: /create account/i }).click();

  // On success the form redirects to the dashboard (session issued, no email
  // verification in dev). Wait for the navigation rather than a fixed timeout.
  await page.waitForURL(/\/dashboard(\/|$|\?)/, { timeout: 30_000 });

  return { email, password };
}

/** Sign in via the /login UI and land back inside the app. */
async function login(page: Page, email: string, password: string): Promise<void> {
  await page.goto("/login");
  await page.locator("#email").fill(email);
  await page.locator("#password").fill(password);
  await page.getByRole("button", { name: /^sign in$/i }).click();
  await page.waitForURL(/\/dashboard(\/|$|\?)/, { timeout: 30_000 });
}

test.describe("ShareX API + token management", () => {
  test("full ShareX contract: create token, upload, fetch raw, delete", async ({
    page,
    request,
  }) => {
    // 1) Fresh account (auto-login on register).
    await registerFreshUser(page);

    // 2) Token management UI.
    await page.goto("/dashboard/tokens");

    // Open the create-token dialog. Either the header button or the empty-state
    // CTA may be present depending on whether the user already has tokens.
    const openCreate = page
      .getByRole("button", { name: /create.*token|create your first token/i })
      .first();
    await expect(openCreate).toBeVisible();
    await openCreate.click();

    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();

    // Name the token (the field is pre-filled with a suggestion; tweak it so the
    // test is self-describing).
    const nameInput = dialog.locator("#token-name");
    await expect(nameInput).toBeVisible();
    await nameInput.fill("e2e-playwright");

    // Submit.
    await dialog.getByRole("button", { name: /generate token/i }).click();

    // 3) Read the one-time plaintext token from the copyable field. After
    // creation the dialog swaps to the success view whose only <code> element
    // (under the "Your token" label) holds the full secret. The list preview
    // uses a leading "····" mask, so guard against accidentally grabbing that.
    const tokenCode = dialog.locator("code").first();
    await expect(tokenCode).toBeVisible();
    await expect
      .poll(async () => (await tokenCode.innerText()).trim(), { timeout: 15_000 })
      .toMatch(/^[A-Za-z0-9_-]{20,}$/);

    const token = (await tokenCode.innerText()).trim();
    expect(token).not.toContain("·");
    expect(token.length).toBeGreaterThanOrEqual(20);

    // 4) ShareX single-POST upload using the API key header. Use the request
    // fixture so we exercise the raw server contract independent of the browser.
    const fileBody = "hello e2e";
    const uploadRes = await request.post("/upload", {
      headers: { "x-api-key": token },
      multipart: {
        upload: {
          name: "e2e.txt",
          mimeType: "text/plain",
          buffer: Buffer.from(fileBody),
        },
      },
    });

    expect(uploadRes.status()).toBe(201);
    const uploadJson = await uploadRes.json();
    expect(uploadJson.message).toBe("OK");
    expect(typeof uploadJson.url).toBe("string");
    expect(typeof uploadJson.raw_url).toBe("string");
    expect(typeof uploadJson.deletionUrl).toBe("string");
    expect(uploadJson.url).toBeTruthy();
    expect(uploadJson.raw_url).toBeTruthy();
    expect(uploadJson.deletionUrl).toBeTruthy();

    const rawUrl: string = uploadJson.raw_url;
    const deletionUrl: string = uploadJson.deletionUrl;

    // 5) Raw URL serves the exact bytes back.
    const rawRes = await request.get(rawUrl);
    expect(rawRes.status()).toBe(200);
    expect(await rawRes.text()).toBe(fileBody);

    // 6) Deletion flow: GET renders the confirm page, POST performs the delete.
    const confirmRes = await request.get(deletionUrl);
    expect(confirmRes.status()).toBe(200);
    expect(await confirmRes.text()).toMatch(/delete/i);

    const deleteRes = await request.post(deletionUrl);
    expect(deleteRes.status()).toBe(200);

    // 7) Raw URL is gone after deletion.
    const goneRes = await request.get(rawUrl);
    expect(goneRes.status()).toBe(404);
  });

  test("upload without a token is rejected with 400", async ({ request }) => {
    const res = await request.post("/upload", {
      multipart: {
        upload: {
          name: "no-token.txt",
          mimeType: "text/plain",
          buffer: Buffer.from("nope"),
        },
      },
    });

    expect(res.status()).toBe(400);
    const body = await res.json();
    expect(body.message).toMatch(/token/i);
  });
});

test.describe("Admin settings", () => {
  test("admin can toggle a setting and persist it", async ({ page }) => {
    await login(page, ADMIN_EMAIL, ADMIN_PASSWORD);

    await page.goto("/dashboard/admin");

    // The Settings tab is the default. Make sure it's active/visible before we
    // poke a switch (be resilient if a tab needs an explicit click).
    const settingsTab = page.getByRole("tab", { name: /settings/i });
    if (await settingsTab.count()) {
      await settingsTab.click().catch(() => {});
    }

    // `register_enabled` is the only global setting left — the older
    // toggles (automatic tagging, image embeds, quota switch, custom head) were
    // removed because nothing read them. Storage limits are per account now.
    const toggle = page.locator("#set-register_enabled");
    await expect(toggle).toBeVisible();

    const before = await toggle.getAttribute("aria-checked");
    await toggle.click();
    // The switch reflects state via aria-checked; confirm it actually flipped.
    await expect
      .poll(async () => toggle.getAttribute("aria-checked"), { timeout: 5_000 })
      .not.toBe(before);
    const after = await toggle.getAttribute("aria-checked");

    // Save. The button is disabled until the form is dirty, so it should now be
    // enabled.
    const saveButton = page.getByRole("button", { name: /save changes/i });
    await expect(saveButton).toBeEnabled();
    await saveButton.click();
    await expect(page.getByText(/settings saved/i).first()).toBeVisible({ timeout: 8_000 });

    // It has to survive a reload — that is the whole point of the settings table.
    await page.reload();
    await toggle.waitFor({ state: "visible" });
    await expect
      .poll(async () => toggle.getAttribute("aria-checked"), { timeout: 8_000 })
      .toBe(after);

    // Put registration back the way we found it: other specs sign new accounts
    // up, and leaving the gate closed would fail them for the wrong reason.
    await toggle.click();
    await page.getByRole("button", { name: /save changes/i }).click();
    await expect
      .poll(async () => toggle.getAttribute("aria-checked"), { timeout: 8_000 })
      .toBe(before);
  });
});
