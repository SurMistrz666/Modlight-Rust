/// <reference types="node" />
/**
 * Downloads -> "Publishing builds": the publish.sh example administrators copy
 * (src/downloads/DownloadsScreen.tsx). It used to paste the change note (and
 * the other values) straight into the JSON body, so a note with a quote, a
 * backslash or a line break made the start request invalid JSON. The script is
 * taken from the rendered page and run with a stand-in `curl` that records the
 * body it was given, which must parse back to the same values.
 */
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { act, render } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { routes } from "../src/routes/tree";
import { setLoaderRoutes, setPageSource } from "../src/routes/loader";
import { installRouter } from "../src/runtime/navigation";
import { expandFixture, FIXTURES } from "../src/fixtures/source";

let dir = "";
let script = "";

beforeAll(async () => {
  setLoaderRoutes(routes);
  setPageSource(async () => expandFixture(FIXTURES["dashboard-downloads"]));
  const router = createMemoryRouter(routes, { initialEntries: ["/dashboard/downloads"] });
  installRouter(router, routes);
  const view = render(<RouterProvider router={router} />);
  for (let i = 0; i < 200 && (!router.state.initialized || router.state.navigation.state !== "idle"); i++) {
    await act(() => new Promise((r) => setTimeout(r, 10)));
  }
  script = view.container.querySelector('pre[aria-label="The same as a shell script (curl):"]')?.textContent ?? "";
  view.unmount();

  dir = fs.mkdtempSync(path.join(os.tmpdir(), "modlight-publish-"));
  fs.writeFileSync(path.join(dir, "publish.sh"), script);
  // A stand-in curl: keeps the start request's body, answers like the server.
  fs.mkdirSync(path.join(dir, "bin"));
  fs.writeFileSync(
    path.join(dir, "bin", "curl"),
    [
      "#!/bin/sh",
      'while [ $# -gt 0 ]; do',
      '  case "$1" in',
      '    --data) printf "%s" "$2" > "$CAPTURE"; shift ;;',
      '    --data-binary) cat > /dev/null; shift ;;',
      "  esac",
      "  shift",
      "done",
      'if [ -s "$CAPTURE" ] && [ ! -e "$CAPTURE.done" ]; then touch "$CAPTURE.done"; printf \'{"upload":"0123abcd","chunkSize":52428800}\'; fi',
      "",
    ].join("\n"),
    { mode: 0o755 },
  );
  fs.writeFileSync(path.join(dir, "Modlight-1.2.0-45-macos-arm64.zip"), "not really a zip");
});

afterAll(() => {
  if (dir) fs.rmSync(dir, { recursive: true, force: true });
});

function publish(note: string): Record<string, unknown> {
  const capture = path.join(dir, `body-${Math.random().toString(36).slice(2)}.json`);
  execFileSync(
    "/bin/sh",
    [path.join(dir, "publish.sh"), path.join(dir, "Modlight-1.2.0-45-macos-arm64.zip"), "macos", "arm64", "1.2.0", "45", note],
    {
      env: { PATH: `${path.join(dir, "bin")}:/usr/bin:/bin`, RELEASE_PUBLISH_TOKEN: "test-token", CAPTURE: capture },
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  return JSON.parse(fs.readFileSync(capture, "utf8")) as Record<string, unknown>;
}

describe("the publish.sh example", () => {
  it("is on the page", () => {
    expect(script).toContain("#!/bin/sh");
    expect(script).toContain("/api/downloads/upload");
  });

  it("sends a plain note as it is", () => {
    const body = publish("What changed");
    expect(body).toMatchObject({
      platform: "macos",
      arch: "arm64",
      version: "1.2.0",
      build: "45",
      filename: "Modlight-1.2.0-45-macos-arm64.zip",
      note: "What changed",
      size: 16,
    });
    expect(String(body.sha256)).toMatch(/^[0-9a-f]{64}$/);
  });

  it("sends quotes, backslashes and line breaks as valid JSON", () => {
    const note = 'Fixed the "Save" button\nC:\\Users\\path handling\n\nThird line';
    expect(publish(note).note).toBe(note);
  });

  it("sends no note as an empty one", () => {
    const capture = path.join(dir, "empty.json");
    execFileSync("/bin/sh", [path.join(dir, "publish.sh"), path.join(dir, "Modlight-1.2.0-45-macos-arm64.zip"), "macos", "arm64", "1.2.0", "45"], {
      env: { PATH: `${path.join(dir, "bin")}:/usr/bin:/bin`, RELEASE_PUBLISH_TOKEN: "test-token", CAPTURE: capture },
      stdio: ["ignore", "pipe", "pipe"],
    });
    expect((JSON.parse(fs.readFileSync(capture, "utf8")) as { note: string }).note).toBe("");
  });
});
