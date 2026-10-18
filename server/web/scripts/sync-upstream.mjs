#!/usr/bin/env node
// Copies the upstream browser code into server/web, verbatim, and keeps a
// manifest (upstream.lock.json) that proves it stayed verbatim.
//
//   node scripts/sync-upstream.mjs            copy + rewrite the manifest
//   node scripts/sync-upstream.mjs --check    verify copies and ports, change nothing
//   node scripts/sync-upstream.mjs --accept   like the default, and also accept the
//                                             current upstream hash of every ported file
//
// What is copied (see CONTRACT.md "SPA layout"):
//   - components/**, hooks/**, types/** (except files that have a client port
//     under src/overrides/ with the same path: upstream server components),
//   - the client files under app/ (error boundaries, the short-link unlock form)
//     and app/globals.css,
//   - every lib/ module those files, or the SPA's own src/ files, import -
//     followed transitively. Imports of @/app/actions/* stop at the generated RPC
//     stubs; imports that hit a file under src/overrides/ stop there.
//   - messages/{en,pl}/<namespace>.json for the namespaces the root layout ships.
//
// A module that is server-only (node:*, drizzle, "server-only", next/headers...)
// is never copied. Reaching one without an override is an error: either the
// importer must not need it, or src/overrides/ must provide a browser version
// (types only, or the pure parts) that names its upstream source with a
// `@upstream <path>` tag.
//
// Ported files (pages, layouts, server components, type mirrors) carry
// `@upstream <path>` tags; the manifest records the upstream hash each port was
// written against, and --check fails when upstream has moved since.
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import {
  WEB_ROOT,
  findUpstream,
  parseImports,
  resolveSpecifier,
  serverOnlyReason,
  sha256,
  walkFiles,
} from "./lib/upstream.mjs";

const UPSTREAM = findUpstream();
const CHECK = process.argv.includes("--check");
const ACCEPT = process.argv.includes("--accept");
const MANIFEST = path.join(WEB_ROOT, "upstream.lock.json");
const OVERRIDES = path.join(WEB_ROOT, "src", "overrides");
const SRC = path.join(WEB_ROOT, "src");

/** The namespaces i18n/request.ts loads for the root layout (mail.json is server-only). */
const NAMESPACES = [
  "common", "nav", "auth", "dashboard", "profile",
  "paste", "links", "albums", "tokens", "admin",
  "public", "landing", "viewer", "files", "upload",
  "download", "authorize",
];

/** Client files that live under app/ upstream. */
const APP_CLIENT_FILES = [
  "app/globals.css",
  "app/error.tsx",
  "app/(app)/dashboard/error.tsx",
  "app/s/[code]/unlock/unlock-form.tsx",
];

const errors = [];
const rel = (abs, root = UPSTREAM) => path.relative(root, abs).split(path.sep).join("/");

function overrideFor(upstreamRel) {
  const noExt = upstreamRel.replace(/\.(tsx?|mjs|js)$/, "");
  for (const e of [".ts", ".tsx", ".d.ts", "/index.ts", "/index.tsx"]) {
    const p = path.join(OVERRIDES, noExt + e);
    if (fs.existsSync(p)) return p;
  }
  return null;
}

// ---------------------------------------------------------------------------
// 1. The copy set
// ---------------------------------------------------------------------------
const copy = new Set(); // upstream-relative paths
const queue = [];

function enqueueUpstream(upRel, from) {
  if (copy.has(upRel)) return;
  const ov = overrideFor(upRel);
  if (ov) {
    queue.push({ kind: "local", abs: ov });
    return;
  }
  const abs = path.join(UPSTREAM, upRel);
  if (!fs.existsSync(abs)) {
    errors.push(`missing upstream file ${upRel} (imported by ${from})`);
    return;
  }
  if (/\.(tsx?|mjs|js)$/.test(upRel)) {
    const why = serverOnlyReason(abs);
    if (why) {
      errors.push(`${upRel} is server-only (${why}) but ${from} imports it; add src/overrides/${upRel}`);
      return;
    }
  }
  copy.add(upRel);
  queue.push({ kind: "upstream", abs, rel: upRel });
}

for (const dir of ["components", "hooks", "types"]) {
  for (const abs of walkFiles(path.join(UPSTREAM, dir), (p) => /\.(tsx?|css)$/.test(p))) {
    const r = rel(abs);
    if (overrideFor(r)) continue; // the port is used instead (it is walked via src/)
    enqueueUpstream(r, "(entry)");
  }
}
for (const r of APP_CLIENT_FILES) enqueueUpstream(r, "(entry)");
for (const abs of walkFiles(SRC, (p) => /\.(tsx?)$/.test(p))) queue.push({ kind: "local", abs });

const seenLocal = new Set();
while (queue.length) {
  const item = queue.shift();
  if (item.kind === "local") {
    if (seenLocal.has(item.abs)) continue;
    seenLocal.add(item.abs);
  }
  if (!/\.(tsx?|mjs|js)$/.test(item.abs)) continue;
  const fromLabel = item.kind === "upstream" ? item.rel : rel(item.abs, WEB_ROOT);
  for (const imp of parseImports(item.abs)) {
    const spec = imp.spec;
    if (spec.startsWith("@/app/actions/")) continue; // generated RPC stubs
    if (spec.startsWith("@/")) {
      const target = spec.slice(2);
      const ov = overrideFor(target);
      if (ov) {
        queue.push({ kind: "local", abs: ov });
        continue;
      }
      const resolved = resolveSpecifier(UPSTREAM, path.join(UPSTREAM, "x.ts"), spec);
      if (resolved === null) continue;
      if (resolved.missing) {
        // Not an upstream module: an SPA-only path (src/*) resolved by the alias.
        if (item.kind === "local") continue;
        errors.push(`unresolved ${spec} in ${fromLabel}`);
        continue;
      }
      enqueueUpstream(rel(resolved), fromLabel);
    } else if (spec.startsWith("./") || spec.startsWith("../")) {
      if (item.kind === "local") continue; // SPA-internal relative import
      const resolved = resolveSpecifier(UPSTREAM, item.abs, spec);
      if (resolved === null || resolved.missing) {
        errors.push(`unresolved ${spec} in ${fromLabel}`);
        continue;
      }
      enqueueUpstream(rel(resolved), fromLabel);
    }
  }
}

for (const lang of ["en", "pl"]) {
  for (const ns of NAMESPACES) copy.add(`messages/${lang}/${ns}.json`);
}

// ---------------------------------------------------------------------------
// 2. Ports: every `@upstream <path>` tag under src/
// ---------------------------------------------------------------------------
const ports = {}; // upstream path -> [local files]
for (const abs of walkFiles(SRC, (p) => /\.(tsx?|css)$/.test(p))) {
  const text = fs.readFileSync(abs, "utf8");
  for (const m of text.matchAll(/@upstream\s+([^\s*]+)/g)) {
    const up = m[1];
    if (!fs.existsSync(path.join(UPSTREAM, up))) {
      errors.push(`${rel(abs, WEB_ROOT)} names @upstream ${up}, which does not exist`);
      continue;
    }
    (ports[up] ??= []).push(rel(abs, WEB_ROOT));
  }
}

// Nothing is written while the set itself is wrong.
if (errors.length) {
  console.error(errors.map((e) => "error: " + e).join("\n"));
  process.exit(1);
}

// ---------------------------------------------------------------------------
// 3. Copy / check
// ---------------------------------------------------------------------------
let previous = { files: {}, ports: {} };
if (fs.existsSync(MANIFEST)) previous = JSON.parse(fs.readFileSync(MANIFEST, "utf8"));

const files = {};
for (const r of [...copy].sort()) {
  const src = path.join(UPSTREAM, r);
  const dst = path.join(WEB_ROOT, r);
  const hash = sha256(src);
  files[r] = hash;
  if (CHECK) {
    if (!fs.existsSync(dst)) errors.push(`not copied yet: ${r}`);
    else if (sha256(dst) !== hash) errors.push(`differs from upstream: ${r}`);
  } else {
    fs.mkdirSync(path.dirname(dst), { recursive: true });
    fs.copyFileSync(src, dst);
  }
}

// Files from an earlier sync that are no longer part of the set.
for (const r of Object.keys(previous.files ?? {})) {
  if (files[r]) continue;
  const dst = path.join(WEB_ROOT, r);
  if (CHECK) errors.push(`stale copy (no longer imported): ${r}`);
  else if (fs.existsSync(dst)) fs.rmSync(dst);
}

const portHashes = {};
for (const up of Object.keys(ports).sort()) {
  const now = sha256(path.join(UPSTREAM, up));
  const recorded = previous.ports?.[up]?.sha256;
  if (ACCEPT || !recorded) {
    portHashes[up] = { sha256: now, ports: ports[up] };
  } else {
    portHashes[up] = { sha256: recorded, ports: ports[up] };
    if (recorded !== now) {
      const msg = `upstream ${up} changed since its port was written (${ports[up].join(", ")})`;
      if (CHECK) errors.push(msg);
      else console.warn("warning: " + msg);
    }
  }
}

let commit = null;
try {
  commit = execFileSync("git", ["-C", UPSTREAM, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
} catch {
  /* not a git checkout */
}

if (errors.length) {
  console.error(errors.map((e) => "error: " + e).join("\n"));
  process.exit(1);
}

if (!CHECK) {
  const manifest = { upstreamCommit: commit, files, ports: portHashes };
  fs.writeFileSync(MANIFEST, JSON.stringify(manifest, null, 2) + "\n");
  console.log(`copied ${Object.keys(files).length} files, ${Object.keys(portHashes).length} ported upstream files tracked`);
} else {
  console.log(`ok: ${Object.keys(files).length} copies verbatim, ${Object.keys(portHashes).length} ports current`);
}
