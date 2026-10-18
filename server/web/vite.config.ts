/// <reference types="vitest/config" />
import fs from "node:fs";
import path from "node:path";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";

const ROOT = path.dirname(new URL(import.meta.url).pathname);
const OVERRIDES = path.join(ROOT, "src", "overrides");
const SHIMS = path.join(ROOT, "src", "shims");

/** The Rust development server (docs/PORT.md section 3). */
const RUST_DEV = process.env.MODLIGHT_RUST_URL ?? "http://127.0.0.1:8421";

const EXTENSIONS = ["", ".ts", ".tsx", ".js", ".mjs", ".json", ".css", "/index.ts", "/index.tsx"];

function firstFile(base: string): string | null {
  for (const ext of EXTENSIONS) {
    const p = base + ext;
    if (fs.existsSync(p) && fs.statSync(p).isFile()) return p;
  }
  return null;
}

/** next/* and next-intl entry points the browser build provides. */
const SHIMMED: Record<string, string> = {
  "next/link": path.join(SHIMS, "next", "link.tsx"),
  "next/navigation": path.join(SHIMS, "next", "navigation.ts"),
  "next/dynamic": path.join(SHIMS, "next", "dynamic.tsx"),
  "next-intl": path.join(SHIMS, "next-intl.tsx"),
  "client-only": path.join(SHIMS, "empty.ts"),
  "geist/font/sans": path.join(SHIMS, "geist", "font.ts"),
  "geist/font/mono": path.join(SHIMS, "geist", "font.ts"),
};

/**
 * Module resolution that lets upstream files compile unchanged:
 *
 *  - `@/app/actions/<m>` -> src/rpc/actions/<m>.ts (generated RPC stubs);
 *  - `@/<path>` -> src/overrides/<path> when a browser version exists (ports of
 *    upstream Server Components, type mirrors of server-only modules, the public
 *    `config`), otherwise <web root>/<path> (verbatim upstream copies);
 *  - next/link, next/navigation, next/dynamic, next-intl -> src/shims/;
 *  - any other next/* or next-intl/* import, and `server-only`, is a build error:
 *    server code has no business in this bundle.
 */
function modlightResolve(): Plugin {
  return {
    name: "modlight-resolve",
    enforce: "pre",
    resolveId(source, importer) {
      if (SHIMMED[source]) return SHIMMED[source];
      if (source === "server-only" || source === "next" || source.startsWith("next/") || source.startsWith("next-intl/")) {
        this.error(`"${source}" imported by ${importer ?? "?"} has no browser counterpart (server-only code reached the SPA bundle)`);
      }
      if (source.startsWith("@/app/actions/")) {
        const hit = firstFile(path.join(ROOT, "src", "rpc", "actions", source.slice("@/app/actions/".length)));
        if (!hit) this.error(`no RPC stub for ${source}; run scripts/gen-actions.mjs`);
        return hit;
      }
      if (source.startsWith("@/")) {
        const rest = source.slice(2);
        return firstFile(path.join(OVERRIDES, rest)) ?? firstFile(path.join(ROOT, rest)) ?? null;
      }
      return null;
    },
  };
}

/**
 * Same-origin document requests the SPA renders itself; everything else goes to
 * the Rust server in dev. Mirrors the page / route-handler split of
 * src/routes/tree.tsx (ROUTES vs SERVER_PATHS).
 */
function isSpaDocument(url: string, accept: string): boolean {
  if (!accept.includes("text/html")) return false;
  const pathname = url.split("?")[0];
  const s = pathname.split("/").filter(Boolean);
  const SERVER_FIRST = ["_rpc", "_page", "api", "avatar", "bg", "embed", "oembed", "upload", "brand", "fonts", "icon.svg", "apple-icon.png"];
  if (s.length > 0 && SERVER_FIRST.includes(s[0])) return false;
  if (s[0] === "dashboard") return true;
  if (s[0] === "s") return s[s.length - 1] === "unlock" && (s.length === 3 || s.length === 4);
  if (s[0] === "a") return s.length === 3;
  if (s[0] === "u") return s.length === 2;
  if (s[0] === "download") return s.length === 1;
  if (s[0] === "invite") return s.length === 2;
  return s.length <= 2;
}

const VITE_OWN = /^\/(@vite|@id|@fs|@react-refresh|src\/|node_modules\/|components\/|lib\/|hooks\/|app\/|messages\/|types\/|__vitest|index\.html)/;

/** `process.env.NEXT_PUBLIC_*` replacements, exactly what Next would inline. */
function publicEnv(): Record<string, string> {
  const out: Record<string, string> = {
    "process.env.NEXT_PUBLIC_TUS_CHUNK_SIZE": "undefined",
  };
  for (const [key, value] of Object.entries(process.env)) {
    if (key.startsWith("NEXT_PUBLIC_")) out[`process.env.${key}`] = JSON.stringify(value);
  }
  return out;
}

export default defineConfig(({ mode }) => {
  const fixtures = mode === "fixtures";
  return {
    plugins: [modlightResolve(), react()],
    define: {
      "import.meta.env.MODLIGHT_FIXTURES": JSON.stringify(fixtures),
      // Next inlines NEXT_PUBLIC_* at build time and leaves unset ones undefined;
      // components/upload/uppy-client.ts reads NEXT_PUBLIC_TUS_CHUNK_SIZE.
      ...publicEnv(),
    },
    server: {
      host: "127.0.0.1",
      port: 5173,
      strictPort: true,
      proxy: fixtures
        ? undefined
        : {
            // Everything that is not the SPA itself is the Rust server's: /_rpc,
            // /_page, /api, /upload, /oembed, media (/{tag}/{code}/raw...), short
            // links, avatars, wallpapers, embeds, /download/app, icons.
            "/": {
              target: RUST_DEV,
              changeOrigin: false,
              bypass(req) {
                const url = req.url ?? "/";
                if (VITE_OWN.test(url)) return url;
                if (isSpaDocument(url, String(req.headers.accept ?? ""))) return "/index.html";
                return undefined;
              },
            },
          },
    },
    preview: { host: "127.0.0.1", port: 4173, strictPort: true },
    build: {
      outDir: "dist",
      assetsDir: "assets",
      manifest: true,
      sourcemap: true,
      target: "es2022",
    },
    test: {
      environment: "jsdom",
      include: ["tests/**/*.test.{ts,tsx}"],
      setupFiles: ["tests/setup.ts"],
      css: false,
      testTimeout: 30_000,
    },
  };
});
