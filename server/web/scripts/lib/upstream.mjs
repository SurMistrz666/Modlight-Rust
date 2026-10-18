// Shared helpers for the scripts that read the upstream checkout.
//
// The upstream tree is read as TEXT only: nothing here imports, executes or
// installs anything from it.
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { fileURLToPath } from "node:url";

export const WEB_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");

/**
 * Where the upstream checkout lives: --upstream <dir>, MODLIGHT_UPSTREAM, or the
 * first `upstream/Modlight` found walking up from server/web (the main checkout
 * has it two levels up, a worktree under _wt/<name>/ four levels up).
 */
export function findUpstream(argv = process.argv) {
  const i = argv.indexOf("--upstream");
  if (i >= 0 && argv[i + 1]) return path.resolve(argv[i + 1]);
  if (process.env.MODLIGHT_UPSTREAM) return path.resolve(process.env.MODLIGHT_UPSTREAM);
  let dir = WEB_ROOT;
  for (let n = 0; n < 8; n++) {
    const candidate = path.join(dir, "upstream", "Modlight");
    if (fs.existsSync(path.join(candidate, "package.json"))) return candidate;
    const parent = path.dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }
  throw new Error("upstream/Modlight not found; pass --upstream <dir> or set MODLIGHT_UPSTREAM");
}

export function sha256(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

export function walkFiles(dir, filter = () => true, out = []) {
  if (!fs.existsSync(dir)) return out;
  for (const name of fs.readdirSync(dir).sort()) {
    const p = path.join(dir, name);
    const st = fs.statSync(p);
    if (st.isDirectory()) walkFiles(p, filter, out);
    else if (filter(p)) out.push(p);
  }
  return out;
}

const EXTS = [".ts", ".tsx", ".js", ".mjs", ".json", ".css", "/index.ts", "/index.tsx"];

/** Resolve an import specifier of `fromFile` (under `root`) to a file, or null for a package. */
export function resolveSpecifier(root, fromFile, spec) {
  let base;
  if (spec.startsWith("@/")) base = path.join(root, spec.slice(2));
  else if (spec.startsWith("./") || spec.startsWith("../")) base = path.resolve(path.dirname(fromFile), spec);
  else return null;
  if (fs.existsSync(base) && fs.statSync(base).isFile()) return base;
  for (const e of EXTS) if (fs.existsSync(base + e)) return base + e;
  return { missing: base };
}

/**
 * Import statements of a TS/TSX module: [{ spec, typeOnly }].
 *
 * A regex reader rather than a parser, which is enough for this codebase's
 * import style (one statement per import, `import type` or `{ type X }`).
 */
export function parseImports(file) {
  const src = fs.readFileSync(file, "utf8");
  const out = [];
  const re = /(?:^|\n)\s*(import|export)\s+(type\s+)?([\s\S]*?)\s+from\s+["']([^"']+)["']/g;
  let m;
  while ((m = re.exec(src))) {
    const [, kw, typeKw, clause, spec] = m;
    if (kw === "export" && !/^\{|^\*/.test(clause.trim())) continue;
    let typeOnly = Boolean(typeKw);
    if (!typeOnly) {
      const braces = clause.match(/\{([\s\S]*)\}/);
      const defaultPart = clause.replace(/\{[\s\S]*\}/, "").replace(/,/g, "").trim();
      if (braces && !defaultPart) {
        const names = braces[1].split(",").map((s) => s.trim()).filter(Boolean);
        typeOnly = names.length > 0 && names.every((n) => n.startsWith("type "));
      }
    }
    out.push({ spec, typeOnly });
  }
  const bare = /(?:^|\n)\s*import\s+["']([^"']+)["']/g;
  while ((m = bare.exec(src))) out.push({ spec: m[1], typeOnly: false });
  const dyn = /import\(\s*["']([^"']+)["']\s*\)/g;
  while ((m = dyn.exec(src))) out.push({ spec: m[1], typeOnly: false });
  return out;
}

/** Markers that make a module server-only for the purposes of the browser bundle. */
export function serverOnlyReason(file) {
  const src = fs.readFileSync(file, "utf8");
  if (/^\s*["']use server["']/m.test(src)) return '"use server" module';
  if (/import\s+["']server-only["']/.test(src)) return 'imports "server-only"';
  const m = src.match(/from\s+["'](node:[^"']+|next\/headers|next\/cache|next\/server|next-intl\/server|drizzle-orm[^"']*|pg|sharp|nodemailer|better-auth|better-auth\/(?:api|crypto|next-js|plugins|adapters[^"']*)|@aws-sdk\/[^"']+|@tus\/[^"']+|mammoth|pdf-lib|archiver|busboy|bcryptjs)["']/);
  if (m) return `imports "${m[1]}"`;
  return null;
}
