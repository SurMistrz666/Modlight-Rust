#!/usr/bin/env node
// Bundle audit: proves that no server code reached dist/.
//
//   node scripts/audit-bundle.mjs        (after `vite build`)
//
// 1. Every module that went into the bundle, read from the source maps, must be
//    browser code: an upstream copy (components/, hooks/, lib/ - which by
//    construction holds only the modules sync-upstream.mjs found browser-safe),
//    an SPA file (src/), or a package on the allowlist below.
// 2. No chunk may import a Node built-in, reference a server-only package, or
//    contain eval / new Function (the production CSP has no 'unsafe-eval'), and
//    fixture mode (src/fixtures) must not be in a production build.
//
// (The only environment values the build inlines are NEXT_PUBLIC_*, see
// vite.config.ts publicEnv(); better-auth's client carries an env accessor that
// names AUTH_SECRET & co. as strings, as it does in upstream's bundle.)
//
// Exits 1 with a list of findings, or prints a summary of what is in the bundle.
import fs from "node:fs";
import path from "node:path";
import { WEB_ROOT } from "./lib/upstream.mjs";

const DIST = path.join(WEB_ROOT, "dist");
const ASSETS = path.join(DIST, "assets");

/** npm packages the browser code legitimately uses (and their own dependencies). */
const ALLOWED_PACKAGES = new Set([
  "react", "react-dom", "scheduler", "react-router", "cookie", "set-cookie-parser",
  "use-intl", "intl-messageformat", "@formatjs/icu-messageformat-parser", "@formatjs/icu-skeleton-parser",
  "@formatjs/fast-memoize", "@formatjs/intl-localematcher", "@formatjs/ecma402-abstract", "@schummar/icu-type-parser",
  "next-themes", "sonner", "lucide-react", "clsx", "tailwind-merge", "class-variance-authority", "cmdk",
  "radix-ui", "@radix-ui/*", "@floating-ui/*", "react-remove-scroll", "react-remove-scroll-bar", "react-style-singleton",
  "use-callback-ref", "use-sidecar", "aria-hidden", "get-nonce", "tslib", "detect-node-es",
  "media-chrome", "ce-la-react",
  "@uppy/*", "tus-js-client", "js-base64", "is-stream", "lodash", "lodash.throttle", "nanoid", "namespace-emitter",
  "preact", "classnames", "memoize-one", "mime-match", "wildcard", "shallow-equal", "p-queue", "eventemitter3",
  "p-timeout", "exifr", "proper-lockfile", "retry", "combine-errors", "custom-error-instance", "url-parse",
  "querystringify", "requires-port", "pretty-bytes", "@transloadit/*", "is-mobile", "lodash-es",
  "better-auth", "@better-auth/*", "@better-fetch/fetch", "nanostores", "@nanostores/*", "better-call", "defu",
  "zod", "jose", "uncrypto", "rou3", "@noble/*",
  "blurhash", "dom-helpers", "use-sync-external-store",
]);

/** Packages that must never be in a browser bundle. */
const SERVER_PACKAGES = [
  "next", "drizzle-orm", "drizzle-kit", "pg", "pg-pool", "pg-protocol", "pg-connection-string", "pgpass",
  "kysely", "sharp", "nodemailer", "@aws-sdk", "@smithy", "@tus/server", "@tus/file-store", "@tus/s3-store",
  "busboy", "archiver", "pdf-lib", "mammoth", "bcryptjs", "dotenv", "server-only",
];

function packageOf(source) {
  const i = source.lastIndexOf("node_modules/");
  if (i < 0) return null;
  const rest = source.slice(i + "node_modules/".length).split("/");
  return rest[0].startsWith("@") ? `${rest[0]}/${rest[1]}` : rest[0];
}

function allowed(pkg) {
  if (ALLOWED_PACKAGES.has(pkg)) return true;
  if (pkg.startsWith("@")) return ALLOWED_PACKAGES.has(`${pkg.split("/")[0]}/*`);
  return false;
}

if (!fs.existsSync(ASSETS)) {
  console.error("dist/assets not found: run `vite build` first");
  process.exit(1);
}

const findings = [];
const packages = new Map();
const local = { upstream: 0, spa: 0 };
const jsFiles = fs.readdirSync(ASSETS).filter((f) => f.endsWith(".js"));

for (const file of jsFiles) {
  const code = fs.readFileSync(path.join(ASSETS, file), "utf8");
  const mapPath = path.join(ASSETS, file + ".map");
  if (fs.existsSync(mapPath)) {
    const map = JSON.parse(fs.readFileSync(mapPath, "utf8"));
    for (const raw of map.sources) {
      const source = path.resolve(ASSETS, raw);
      const pkg = packageOf(source.split(path.sep).join("/"));
      if (pkg) {
        packages.set(pkg, (packages.get(pkg) ?? 0) + 1);
        if (SERVER_PACKAGES.some((s) => pkg === s || pkg.startsWith(`${s}/`))) findings.push(`${file}: server package ${pkg} (${raw})`);
        else if (!allowed(pkg)) findings.push(`${file}: package not on the allowlist: ${pkg} (${raw})`);
        continue;
      }
      const rel = path.relative(WEB_ROOT, source).split(path.sep).join("/");
      if (rel.startsWith("..")) {
        findings.push(`${file}: module from outside server/web: ${raw}`);
      } else if (rel.startsWith("src/")) {
        local.spa++;
      } else if (/^(components|hooks|lib|app|messages|types)\//.test(rel)) {
        local.upstream++;
      } else if (!rel.startsWith("\u0000") && !rel.includes("vite/") && rel !== "index.html") {
        findings.push(`${file}: unexpected module ${rel}`);
      }
    }
  } else if (!(file.startsWith("chunk-") && code.length < 4096)) {
    // rolldown's tiny interop-helper chunk has no sources and no map.
    findings.push(`${file}: no source map (build with sourcemap: true to audit)`);
  }

  const nodeImport = code.match(/(?:from\s*|import\s*\(\s*|require\s*\(\s*)["'](node:[a-z_/]+|fs|path|crypto|child_process|os|stream|net|tls|http|https|zlib|worker_threads)["']/);
  if (nodeImport) findings.push(`${file}: imports Node built-in "${nodeImport[1]}"`);
  if (/\beval\s*\(/.test(code)) findings.push(`${file}: calls eval()`);
  if (code.includes("__fixture") && !process.argv.includes("--allow-fixtures")) {
    findings.push(`${file}: fixture mode code in a production build`);
  }
  if (/\bnew\s+Function\s*\(/.test(code)) findings.push(`${file}: uses new Function()`);
}

if (findings.length) {
  console.error(findings.map((f) => "FAIL " + f).join("\n"));
  process.exit(1);
}

console.log(`bundle audit ok: ${jsFiles.length} chunks, ${local.upstream} upstream modules, ${local.spa} SPA modules`);
console.log("packages: " + [...packages.keys()].sort().join(", "));
