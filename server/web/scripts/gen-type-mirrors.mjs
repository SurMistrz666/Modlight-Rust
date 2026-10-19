#!/usr/bin/env node
// Writes src/overrides/<path> for upstream modules that are server-only but whose
// TYPES (and a few pure helpers) the browser code needs.
//
// Each mirror holds declarations copied verbatim from upstream, closed over the
// local declarations they use, plus `import type` lines for what they import.
// When a mirrored declaration refers to a type that lives in another server-only
// module, that module is mirrored too. Nothing is executed: TypeScript's parser
// reads the upstream text.
//
//   node scripts/gen-type-mirrors.mjs [--upstream <dir>]
import fs from "node:fs";
import path from "node:path";
import { WEB_ROOT, findUpstream, serverOnlyReason } from "./lib/upstream.mjs";
import { closeOver, importLines, indexModule, statementText } from "./lib/extract.mjs";

const UPSTREAM = findUpstream();
const OUT = path.join(WEB_ROOT, "src", "overrides");

/**
 * What to mirror. `types: "*"` takes every exported interface and type alias;
 * `values` lists pure functions/constants the browser ports use. Modules reached
 * through type references get `types: "*"` automatically.
 *
 * `manual` supplies a declaration by hand where the upstream one cannot travel
 * (it is computed from a server value, e.g. `typeof table.$inferSelect`). Every
 * manual entry says which upstream expression it stands for.
 */
const MIRRORS = {
  "lib/data/uploads.ts": { types: "*" },
  "lib/data/duplicates.ts": {
    types: "*",
    values: ["MAX_GROUPS", "MAX_ROWS", "MAX_COPIES_LISTED", "REMINDER_MIN_REDUNDANT", "REMINDER_MIN_BYTES", "MAX_TRASH_IDS"],
  },
  "lib/data/sessions.ts": { types: "*" },
  "lib/data/admin.ts": { types: "*", values: ["ORPHAN_PAGE_SIZE", "STORAGE_SCAN_GRACE_MS", "MAX_STORAGE_ORPHAN_SAMPLE"] },
  "lib/data/admin-disk.ts": { types: "*", values: ["COLD_DAYS", "TRASH_DUE_SOON_DAYS"] },
  "lib/data/folders.ts": { types: "*" },
  "lib/data/admin-files.ts": { types: "*" },
  "lib/data/user-prefs.ts": { types: "*", values: ["mayCreateRootShortLink"] },
  "lib/mail-settings.ts": { types: "*" },
  "lib/registration.ts": { types: "*", values: ["REGISTRATION_KEY_MAX"] },
  "lib/upload/disk.ts": { types: "*", values: ["diskPressure", "LOW_FREE_RATIO", "CRITICAL_FREE_RATIO", "fitsOnDisk"] },
  "lib/maintenance/trash.ts": {
    types: "*",
    values: ["TRASH_RETENTION_DAYS", "TRASH_RETENTION_MS", "TRASH_SWEEP_INTERVAL_MS", "trashDaysLeft"],
  },
  "lib/media/render-ticket.ts": {
    types: "*",
    values: ["sandboxShape", "canRenderInline", "allowsScripts", "RENDER_TICKET_TTL", "MAX_RENDER_BYTES"],
  },
  "lib/releases.ts": { types: "*" },
  // UserRow/TokenAuth are `typeof users.$inferSelect` shapes: server-side only.
  "lib/api-token.ts": { types: "*", exclude: ["UserRow", "TokenAuth"] },
  "lib/avatar-store.ts": { types: "*" },
  "lib/media/avatar.ts": { types: "*" },
  "lib/naming.ts": { types: "*" },
  "lib/password.ts": { types: "*" },
  "lib/shortlinks.ts": { types: "*" },
  "lib/folders.ts": {
    types: "*",
    exclude: ["FolderRecord"],
    manual: {
      header: ['import type { FoldersRow } from "../../contract/db";'],
      body: `/** Upstream: \`export type FolderRecord = typeof folders.$inferSelect;\` (a drizzle row). */
export type FolderRecord = FoldersRow;`,
    },
  },
  "lib/releases-core.ts": { types: "*", values: ["APP_NAME"] },
  "lib/auth.ts": {
    types: [],
    manual: {
      header: [
        'import type { BetterAuthOptions } from "better-auth";',
      ],
      body: `/**
 * The type of upstream's \`auth\` instance, as far as the browser needs it.
 *
 * lib/auth-client.ts passes \`typeof auth\` to \`inferAdditionalFields\`, which only
 * reads \`options.user.additionalFields\`. Those are copied from the
 * \`betterAuth({ user: { additionalFields } })\` call in upstream lib/auth.ts.
 */
declare const authOptions: {
  user: {
    additionalFields: {
      userCode: { type: "string"; required: false; input: false };
      usedQuota: { type: "number"; required: false; defaultValue: 0; input: false };
      maxQuota: { type: "number"; required: false; defaultValue: -1; input: false };
      hideUploads: { type: "boolean"; required: false; defaultValue: false; input: true };
      copyRaw: { type: "boolean"; required: false; defaultValue: false; input: true };
      theme: { type: "string"; required: false; defaultValue: "modern"; input: true };
      locale: { type: "string"; required: false; input: true };
    };
  };
} & BetterAuthOptions;

export declare const auth: { options: typeof authOptions };`,
    },
  },
};

const queue = Object.keys(MIRRORS);
const plan = new Map(Object.entries(MIRRORS).map(([k, v]) => [k, { ...v }]));
const written = [];
const problems = [];

function upstreamRel(fromRel, spec) {
  if (spec.startsWith("@/")) return resolveExt(spec.slice(2));
  if (spec.startsWith("./") || spec.startsWith("../")) {
    return resolveExt(path.posix.join(path.posix.dirname(fromRel), spec));
  }
  return null; // package
}

function resolveExt(p) {
  for (const e of ["", ".ts", ".tsx", "/index.ts"]) {
    const abs = path.join(UPSTREAM, p + e);
    if (fs.existsSync(abs) && fs.statSync(abs).isFile()) return p + e;
  }
  return p;
}

const toAlias = (rel) => "@/" + rel.replace(/\.(tsx?|mjs|js)$/, "").replace(/\/index$/, "");

while (queue.length) {
  const rel = queue.shift();
  const cfg = plan.get(rel);
  if (cfg.done) continue;
  cfg.done = true;
  const abs = path.join(UPSTREAM, rel);
  const mod = indexModule(abs);

  const wanted = new Set(cfg.values ?? []);
  if (cfg.types === "*") {
    for (const [name, d] of mod.decls) if (d.exported && d.isType) wanted.add(name);
  } else if (Array.isArray(cfg.types)) cfg.types.forEach((n) => wanted.add(n));
  for (const n of cfg.exclude ?? []) wanted.delete(n);

  const { statements, usedImports, missing } = closeOver(mod, [...wanted]);
  if (missing.length) problems.push(`${rel}: unknown names ${missing.join(", ")}`);

  // Which imports are needed at runtime (used by an emitted value declaration)?
  const valueImports = new Set();
  for (const st of statements) {
    const isTypeDecl = [...mod.decls.values()].some((d) => d.node === st && d.isType);
    if (isTypeDecl) continue;
    for (const [local] of usedImports) {
      if (new RegExp(`\\b${local}\\b`).test(st.getText(mod.sf))) valueImports.add(local);
    }
  }

  const typeUses = new Map();
  const valueUses = new Map();
  for (const [local, imp] of usedImports) {
    const target = upstreamRel(rel, imp.module);
    let module = imp.module;
    if (target) {
      const targetAbs = path.join(UPSTREAM, target);
      const serverOnly = fs.existsSync(targetAbs) ? serverOnlyReason(targetAbs) : "missing";
      if (serverOnly) {
        if (valueImports.has(local)) {
          problems.push(`${rel}: value ${local} comes from server-only ${target}`);
        } else if (!plan.has(target)) {
          plan.set(target, { types: "*" });
          queue.push(target);
        }
      }
      module = toAlias(target);
    } else if (valueImports.has(local) && !/^(react|clsx)$/.test(imp.module)) {
      problems.push(`${rel}: value ${local} comes from package ${imp.module}`);
    }
    (valueImports.has(local) ? valueUses : typeUses).set(local, { ...imp, module });
  }

  // Re-exports from modules the browser can have (verbatim) or that are mirrored.
  const reexportLines = [];
  for (const re of mod.reexports) {
    const target = upstreamRel(rel, re.module);
    if (!target) continue;
    const serverOnly = serverOnlyReason(path.join(UPSTREAM, target));
    if (serverOnly && !plan.has(target)) {
      if (!re.names.every((n) => n.isType)) continue; // value re-export of server code: drop
      plan.set(target, { types: "*" });
      queue.push(target);
    }
    if (serverOnly && !re.names.every((n) => n.isType)) continue;
    reexportLines.push(
      statementText(mod, re.node)
        .replace(new RegExp(`(["'])${re.module.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\1`), `"${toAlias(target)}"`)
        .trim(),
    );
  }

  const body = statements.map((st) => statementText(mod, st).trimEnd()).join("\n\n");
  const out = [
    "// GENERATED by scripts/gen-type-mirrors.mjs. Do not edit; rerun the script.",
    `// @upstream ${rel}`,
    "//",
    "// The upstream module is server-only. The browser gets its exported types and",
    "// the pure declarations listed in the generator, copied verbatim.",
    "",
    ...(cfg.manual?.header ?? []),
    ...importLines(typeUses, { forceType: true }),
    ...importLines(valueUses, { forceType: false }),
    "",
    ...reexportLines,
    reexportLines.length ? "" : null,
    body,
    cfg.manual?.body ?? null,
    "",
  ]
    .filter((l) => l !== null)
    .join("\n")
    .replace(/\n{3,}/g, "\n\n");

  const dst = path.join(OUT, rel);
  fs.mkdirSync(path.dirname(dst), { recursive: true });
  fs.writeFileSync(dst, out);
  written.push(rel);
}

if (problems.length) {
  console.error(problems.map((p) => "problem: " + p).join("\n"));
  process.exitCode = 1;
}
console.log(`wrote ${written.length} mirrors:\n  ` + written.join("\n  "));
