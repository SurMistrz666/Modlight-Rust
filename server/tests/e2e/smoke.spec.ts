import { test, expect } from "@playwright/test";

// Smoke tests against a running dev/prod server (E2E_BASE_URL or localhost:8420).

test("landing page loads with branding", async ({ page }) => {
  await page.goto("/");
  await expect(page).toHaveTitle(/Modlight/i);
});

test("login page is reachable", async ({ page }) => {
  await page.goto("/login");
  await expect(page.locator("body")).toContainText(/sign in|log in|email/i);
});

test("dashboard redirects unauthenticated users to login", async ({ page }) => {
  await page.goto("/dashboard");
  await expect(page).toHaveURL(/\/login/);
});

test("ShareX upload endpoint rejects a request without a token", async ({ request }) => {
  const res = await request.post("/upload", {
    multipart: { upload: { name: "x.txt", mimeType: "text/plain", buffer: Buffer.from("hi") } },
  });
  expect(res.status()).toBe(400); // "Token not specified."
  const body = await res.json();
  expect(body.message).toMatch(/token/i);
});
