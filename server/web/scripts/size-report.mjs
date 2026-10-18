#!/usr/bin/env node
// Bundle size: what this build ships, against what the upstream Next build ships.
//
//   node scripts/size-report.mjs [--ref http://127.0.0.1:8420] [--cookie '<Cookie header>']
//
// For each page, "first load" is every script and stylesheet a browser fetches
// to render it on a fresh visit (no cache): for the SPA, the entry chunk with
// its static imports and the page's own chunk and layout chunk with theirs; for
// upstream, every /_next/static script and stylesheet the server-rendered HTML
// references. Translations are not code on either side: upstream inlines them in
// the RSC payload of the document, the SPA in its inlined page payload
// (layouts.root.messages), so they are left out of both columns. Sizes are raw and
// gzip -9 (what a compressing proxy roughly sends). Authenticated upstream pages
// are measured only when --cookie carries a session (the cookie is never printed).
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { WEB_ROOT } from "./lib/upstream.mjs";

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : fallback;
};
const REF = opt("--ref", "http://127.0.0.1:8420");
const COOKIE = opt("--cookie", process.env.MODLIGHT_REF_COOKIE ?? "");

const DIST = path.join(WEB_ROOT, "dist");
const manifest = JSON.parse(fs.readFileSync(path.join(DIST, ".vite", "manifest.json"), "utf8"));

const gz = (buf) => zlib.gzipSync(buf, { level: 9 }).length;
const kb = (n) => `${(n / 1024).toFixed(1)} KB`;

function closure(keys) {
  const files = new Set();
  const visit = (key) => {
    const entry = manifest[key];
    if (!entry || files.has(entry.file)) return;
    files.add(entry.file);
    for (const css of entry.css ?? []) files.add(css);
    for (const imp of entry.imports ?? []) visit(imp);
  };
  keys.forEach(visit);
  return files;
}

function measureLocal(files) {
  let raw = 0;
  let gzip = 0;
  let js = 0;
  let css = 0;
  for (const f of files) {
    const buf = fs.readFileSync(path.join(DIST, f));
    raw += buf.length;
    gzip += gz(buf);
    if (f.endsWith(".js")) js += buf.length;
    if (f.endsWith(".css")) css += buf.length;
  }
  return { raw, gzip, js, css, count: files.size };
}

const entryKey = Object.keys(manifest).find((k) => manifest[k].isEntry);
const pageKey = (p) => Object.keys(manifest).find((k) => k === p);

const PAGES = [
  { url: "/login", page: "src/pages/(auth)/login/page.tsx", layout: "src/pages/(auth)/layout.tsx" },
  { url: "/welcome", page: "src/pages/welcome/page.tsx" },
  { url: "/download", page: "src/pages/download/page.tsx" },
  { url: "/nothing/here", page: "src/pages/[userCode]/[mediaCode]/page.tsx", note: "404 / media page" },
  { url: "/dashboard?all=1", page: "src/pages/(app)/dashboard/page.tsx", layout: "src/pages/(app)/layout.tsx", auth: true },
  { url: "/dashboard/profile", page: "src/pages/(app)/dashboard/profile/page.tsx", layout: "src/pages/(app)/layout.tsx", auth: true },
];

async function measureUpstream(url) {
  const headers = { "accept-language": "en" };
  if (COOKIE) headers.cookie = COOKIE;
  const res = await fetch(REF + url, { headers, redirect: "manual" });
  if (res.status >= 300 && res.status < 400) return null;
  const html = await res.text();
  const refs = new Set();
  for (const m of html.matchAll(/(?:src|href)="(\/_next\/static\/[^"]+\.(?:js|css))"/g)) refs.add(m[1]);
  let raw = 0;
  let gzip = 0;
  let js = 0;
  let css = 0;
  for (const ref of refs) {
    const buf = Buffer.from(await (await fetch(REF + ref)).arrayBuffer());
    raw += buf.length;
    gzip += gz(buf);
    if (ref.endsWith(".js")) js += buf.length;
    else css += buf.length;
  }
  // The RSC payload inlined in the HTML (props + translations) is part of the
  // first load upstream; the SPA's equivalent is the inlined page payload.
  const flight = [...html.matchAll(/self\.__next_f\.push\((\[[\s\S]*?\])\)<\/script>/g)].reduce((n, m) => n + m[1].length, 0);
  return { raw, gzip, js, css, count: refs.size, flight };
}

const rows = [];
for (const p of PAGES) {
  const keys = [entryKey, pageKey(p.page), p.layout && pageKey(p.layout)].filter(Boolean);
  const mine = measureLocal(closure(keys));
  let theirs = null;
  if (!p.auth || COOKIE) {
    try {
      theirs = await measureUpstream(p.url);
    } catch (err) {
      theirs = { error: String(err.message ?? err) };
    }
  }
  rows.push({ p, mine, theirs });
}

const allJs = fs.readdirSync(path.join(DIST, "assets")).filter((f) => f.endsWith(".js"));
const allCss = fs.readdirSync(path.join(DIST, "assets")).filter((f) => f.endsWith(".css"));
const total = measureLocal(new Set([...allJs, ...allCss].map((f) => `assets/${f}`)));

console.log(`SPA build: ${allJs.length} JS + ${allCss.length} CSS files, ${kb(total.raw)} raw, ${kb(total.gzip)} gzip (all routes)`);
console.log("");
console.log("first load per page (JS + CSS, no cache)            SPA raw / gzip           upstream raw / gzip   upstream inline RSC");
for (const { p, mine, theirs } of rows) {
  const left = `${p.url}${p.note ? ` (${p.note})` : ""}`.padEnd(50);
  const m = `${kb(mine.raw)} / ${kb(mine.gzip)}`.padEnd(24);
  let t = "not measured (needs --cookie)";
  if (theirs?.error) t = `error: ${theirs.error}`;
  else if (theirs === null && (!p.auth || COOKIE)) t = "redirected";
  else if (theirs) t = `${kb(theirs.raw)} / ${kb(theirs.gzip)}`.padEnd(22) + `  ${kb(theirs.flight)}`;
  console.log(`${left} ${m} ${t}`);
}
