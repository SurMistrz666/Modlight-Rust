import { test, expect } from "@playwright/test";

test.setTimeout(120000);

// Public user gallery (new feature). The seeded admin (userCode ToqOse) has public files.
test("public user gallery renders for a known user", async ({ page }) => {
  await page.goto("/u/ToqOse");
  await expect(page.locator("body")).not.toContainText(/this page could not be found/i);
  // Should show at least the gallery shell (heading area).
  await expect(page).toHaveURL(/\/u\/ToqOse/);
});

test("public user gallery 404s for an unknown user", async ({ request }) => {
  const res = await request.get("/u/nobody999");
  expect(res.status()).toBe(404);
});

// Locale detection: Polish browsers get lang="pl", others get "en".
test("html lang follows Accept-Language", async ({ browser }) => {
  const pl = await browser.newContext({ locale: "pl-PL" });
  const p1 = await pl.newPage();
  await p1.goto("/login");
  expect(await p1.locator("html").getAttribute("lang")).toBe("pl");
  await pl.close();

  const de = await browser.newContext({ locale: "de-DE" });
  const p2 = await de.newPage();
  await p2.goto("/login");
  expect(await p2.locator("html").getAttribute("lang")).toBe("en");
  await de.close();
});
