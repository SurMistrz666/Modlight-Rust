import { test, expect, type Page } from "@playwright/test";

/**
 * Authenticated-UI E2E suite for Modlight.
 *
 * Each test is fully independent and registers its OWN fresh user through the
 * /register UI. In dev (email verification OFF) a successful sign-up issues a
 * session token immediately, so RegisterForm redirects straight to /dashboard
 * — i.e. registration auto-logs-in. We never share state between tests.
 *
 * Route compiles are slow on first hit, so we bump the per-test timeout.
 */
test.setTimeout(120_000);

const PASSWORD = "Password123!";

/** Build a unique email per test so parallel runs never collide. */
function uniqueEmail(prefix = "e2e"): string {
  const stamp = Date.now().toString(36);
  const rand = Math.random().toString(36).slice(2, 8);
  return `${prefix}-${stamp}-${rand}@local.test`;
}

/**
 * Register a brand-new user via the /register form. In dev this auto-logs-in
 * and lands on /dashboard. Returns the credentials for reuse (e.g. re-login).
 */
async function registerFreshUser(
  page: Page,
): Promise<{ name: string; email: string; password: string }> {
  const email = uniqueEmail();
  const name = "E2E Tester";

  await page.goto("/register");
  await page.waitForLoadState("domcontentloaded");

  // RegisterForm: #name, #email, #password + "Create account" submit button.
  await page.locator("#name").fill(name);
  await page.locator("#email").fill(email);
  await page.locator("#password").fill(PASSWORD);

  // Submit, retrying if auth rate-limiting ("Too many requests") trips on a
  // burst of test registrations from the same IP.
  for (let attempt = 1; attempt <= 4; attempt++) {
    await page.getByRole("button", { name: /create account/i }).click();
    const landed = await page
      .waitForURL(/\/dashboard(\?|$|\/)/, { timeout: 30_000 })
      .then(() => true)
      .catch(() => false);
    if (landed) break;
    const rateLimited = await page.getByText(/too many requests/i).count();
    if (rateLimited && attempt < 4) {
      await page.waitForTimeout(8_000);
      continue;
    }
    if (attempt === 4) {
      await page.waitForURL(/\/dashboard(\?|$|\/)/, { timeout: 30_000 });
    }
  }
  await page.waitForLoadState("networkidle");

  return { name, email, password: PASSWORD };
}

/** Open the top-bar user menu (avatar/name button) and click "Sign out". */
async function signOut(page: Page): Promise<void> {
  // The trigger is a ghost Button in the <header> holding the avatar + the
  // user's name/email. The name span is hidden on small viewports but the
  // accessible name still includes it, so match on it first. Fall back to the
  // last header button (the user-menu trigger sits at the far right) to stay
  // resilient to viewport/layout changes.
  const userMenu = page.getByRole("button", { name: /E2E Tester|local\.test/i });
  if (await userMenu.count()) {
    await userMenu.first().click();
  } else {
    await page.locator("header").getByRole("button").last().click();
  }

  await page.getByRole("menuitem", { name: /sign out/i }).click();
  await page.waitForURL(/\/login(\?|$)/, { timeout: 30_000 });
}

test.describe("Authenticated app flows", () => {
  test("register lands on dashboard with empty state", async ({ page }) => {
    await registerFreshUser(page);

    await expect(page).toHaveURL(/\/dashboard(\?|$|\/)/);

    // "Your files" heading from app/(app)/dashboard/page.tsx.
    await expect(
      page.getByRole("heading", { name: /your files/i }),
    ).toBeVisible();

    // Fresh account → EmptyState ("No files yet").
    await expect(page.getByRole("heading", { name: /no files yet/i })).toBeVisible();
    await expect(
      page.getByRole("link", { name: /upload your first file/i }),
    ).toBeVisible();
  });

  test("sign out returns to /login", async ({ page }) => {
    await registerFreshUser(page);
    await signOut(page);

    await expect(page).toHaveURL(/\/login(\?|$)/);
    // Login form is present after sign-out.
    await expect(page.getByRole("button", { name: /^sign in$/i })).toBeVisible();
  });

  test("log back in reaches dashboard", async ({ page }) => {
    const creds = await registerFreshUser(page);
    await signOut(page);

    // Now log back in via the /login UI.
    await page.goto("/login");
    await page.waitForLoadState("domcontentloaded");

    await page.locator("#email").fill(creds.email);
    await page.locator("#password").fill(creds.password);
    await page.getByRole("button", { name: /^sign in$/i }).click();

    await page.waitForURL(/\/dashboard(\?|$|\/)/, { timeout: 60_000 });
    await expect(page.getByRole("heading", { name: /your files/i })).toBeVisible();
  });

  test("web upload via Uppy then file appears in library", async ({ page }) => {
    await registerFreshUser(page);

    // Unique filename so the assertion is unambiguous in the library.
    const filename = `e2e-upload-${Date.now()}-${Math.random()
      .toString(36)
      .slice(2, 8)}.txt`;

    await page.goto("/dashboard/upload");
    await page.waitForLoadState("networkidle");

    // Uppy's Dashboard renders a hidden <input type="file"> once it mounts
    // (it is loaded via next/dynamic ssr:false, so wait for it to attach).
    const fileInput = page.locator('input[type="file"]').first();
    await fileInput.waitFor({ state: "attached", timeout: 30_000 });

    await fileInput.setInputFiles({
      name: filename,
      mimeType: "text/plain",
      buffer: Buffer.from("hello from the modlight e2e suite"),
    });

    // autoProceed is false, so we must start the upload. Uppy renders an
    // "Upload N file" / "Upload" button in the Dashboard once a file is added.
    const uploadBtn = page
      .getByRole("button", { name: /^upload(\s|$)/i })
      .first();
    await expect(uploadBtn).toBeVisible({ timeout: 15_000 });
    await uploadBtn.click();

    // Wait for completion: success toast (sonner) and/or the Uploader's own
    // success card with a "View files" link. Either is sufficient; we try the
    // toast first, then fall back to the success card, then network idle.
    const successToast = page.getByText(/upload complete|uploads complete/i);
    const successCard = page.getByText(/uploaded and registered/i);
    const viewFilesLink = page.getByRole("link", { name: /view files/i });

    await Promise.race([
      successToast.first().waitFor({ state: "visible", timeout: 60_000 }),
      successCard.first().waitFor({ state: "visible", timeout: 60_000 }),
      viewFilesLink.first().waitFor({ state: "visible", timeout: 60_000 }),
    ]);

    // Settle any in-flight tus/registration requests before navigating.
    await page.waitForLoadState("networkidle");

    // Back to the library; the freshly uploaded file should be listed.
    await page.goto("/dashboard");
    await page.waitForLoadState("networkidle");

    // FileCard renders the filename in a <p title={filename}> and as text.
    await expect(page.getByText(filename, { exact: false })).toBeVisible({
      timeout: 30_000,
    });
  });

  test("open file preview and the viewer renders", async ({ page }) => {
    await registerFreshUser(page);

    const filename = `e2e-preview-${Date.now()}-${Math.random()
      .toString(36)
      .slice(2, 8)}.txt`;

    // --- upload a file first (same flow as the upload test) ---
    await page.goto("/dashboard/upload");
    await page.waitForLoadState("networkidle");

    const fileInput = page.locator('input[type="file"]').first();
    await fileInput.waitFor({ state: "attached", timeout: 30_000 });
    await fileInput.setInputFiles({
      name: filename,
      mimeType: "text/plain",
      buffer: Buffer.from("preview me — modlight e2e viewer check"),
    });

    const uploadBtn = page
      .getByRole("button", { name: /^upload(\s|$)/i })
      .first();
    await expect(uploadBtn).toBeVisible({ timeout: 15_000 });
    await uploadBtn.click();

    await Promise.race([
      page
        .getByText(/upload complete|uploads complete/i)
        .first()
        .waitFor({ state: "visible", timeout: 60_000 }),
      page
        .getByText(/uploaded and registered/i)
        .first()
        .waitFor({ state: "visible", timeout: 60_000 }),
      page
        .getByRole("link", { name: /view files/i })
        .first()
        .waitFor({ state: "visible", timeout: 60_000 }),
    ]);
    await page.waitForLoadState("networkidle");

    // --- go to the library and open the file's public preview ---
    await page.goto("/dashboard");
    await page.waitForLoadState("networkidle");

    const fileText = page.getByText(filename, { exact: false }).first();
    await expect(fileText).toBeVisible({ timeout: 30_000 });

    // FileCard wraps the preview/meta in a card. The preview thumbnail is an
    // <a target="_blank"> opening the public viewer URL, and the FileMenu's
    // hover "Open" quick-action + dropdown "Open" item are also such links.
    // These anchors open in a new tab, so rather than juggle popups we resolve
    // the public href and navigate to it directly — far more robust than
    // depending on hover state or popup capture.
    const openLink = page.getByRole("link", { name: /^open$/i }).first();

    let href: string | null = null;
    if (await openLink.count()) {
      href = await openLink.getAttribute("href");
    }
    if (!href) {
      // Fallback: the card's preview anchor (first new-tab anchor in the grid).
      href = await page.locator('a[target="_blank"]').first().getAttribute("href");
    }
    expect(href, "expected a public preview href on the file card").toBeTruthy();

    await page.goto(href as string);
    await page.waitForLoadState("networkidle");

    // MediaViewer renders the filename as an <h1>. That heading proves the
    // viewer rendered. For a .txt it is the "text" stage (TextView).
    await expect(
      page.getByRole("heading", { name: new RegExp(escapeRegExp(filename), "i") }),
    ).toBeVisible({ timeout: 30_000 });
  });
});

/** Escape user-derived strings before embedding them in a RegExp. */
function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
