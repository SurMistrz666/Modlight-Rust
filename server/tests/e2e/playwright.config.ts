// Upstream's playwright.config.ts (Modlight, tests/e2e), configuration only:
// the specs next to this file are byte-for-byte upstream's tests/e2e/*.spec.ts.
// testDir is this directory; E2E_BASE_URL picks the server (the reference or
// the Rust stack). Run from a scratch project that has @playwright/test
// 1.60.0 and its Chromium (see server/tests/e2e/README.md).
import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: ".",
  fullyParallel: true,
  retries: 0,
  reporter: "list",
  use: {
    baseURL: process.env.E2E_BASE_URL || "http://localhost:8420",
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
});
