#!/usr/bin/env node
// Generates src/rpc/actions/<module>.ts from upstream app/actions/<module>.ts.
//
// Every exported server action becomes a stub with the same name, parameters and
// return type that POSTs to /_rpc/<module>/<export> (see src/runtime/rpc.ts and
// docs/CONTRACT.md "Server actions"). Exported types and interfaces are copied
// verbatim, together with the local types and `import type`s they need, so the
// components that import types from "@/app/actions/<module>" keep compiling.
//
// Parsing only: TypeScript's parser reads upstream as text, nothing is executed.
//
//   node scripts/gen-actions.mjs [--upstream <dir>]
import fs from "node:fs";
import path from "node:path";
import { WEB_ROOT, findUpstream } from "./lib/upstream.mjs";
import { closeOver, importLines, indexModule, referencedNames, statementText, ts } from "./lib/extract.mjs";

const UPSTREAM = findUpstream();
const ACTIONS = path.join(UPSTREAM, "app", "actions");
const OUT = path.join(WEB_ROOT, "src", "rpc", "actions");

/**
 * Return types upstream leaves to inference. Written out from the upstream body
 * named next to each; `contract/db` holds the drizzle row shapes.
 */
const INFERRED_RETURNS = {
  "folders.createFolder": {
    type: "Promise<FolderRecord>",
    note: "createFolderFor() in lib/folders.ts returns FolderRecord",
    imports: { "@/lib/folders": ["FolderRecord"] },
  },
  "folders.renameFolder": { type: "Promise<void>", note: "awaits renameFolderFor(), returns nothing" },
  "folders.moveFolder": { type: "Promise<void>", note: "awaits moveFolderFor(), returns nothing" },
  "folders.setFolderVisibility": { type: "Promise<void>", note: "awaits setFolderVisibilityFor(), returns nothing" },
  "folders.moveUploadsToFolder": {
    type: "Promise<{ moved: number }>",
    note: "returns moveUploadsToFolderFor(), typed Promise<{ moved: number }> in lib/folders.ts",
  },
  "folders.listFolders": {
    type: "Promise<{ id: string; parentId: string | null; name: string; slug: string; isPublic: boolean; count: number }[]>",
    note: "db.select({ id, parentId, name, slug, isPublic, count }) over folders",
  },
  "shortlinks.listShortLinks": {
    type: "Promise<ShortLinksRow[]>",
    note: "db.select().from(shortLinks): whole rows, passwordHash included",
    imports: { "../../contract/db": ["ShortLinksRow"] },
  },
  "tokens.myTokens": {
    type: "Promise<ApiTokensRow[]>",
    note: "listApiTokens() = db.select().from(apiTokens): whole rows, tokenHash included",
    imports: { "../../contract/db": ["ApiTokensRow"] },
  },
};

const isExported = (node) => (ts.getCombinedModifierFlags(node) & ts.ModifierFlags.Export) !== 0;

function rewriteSpecifier(fromRel, spec) {
  if (spec.startsWith("./") || spec.startsWith("../")) {
    const joined = path.posix.join(path.posix.dirname(fromRel), spec);
    return "@/" + joined.replace(/\.(tsx?)$/, "");
  }
  return spec;
}

function paramText(sf, p, i) {
  const name = ts.isIdentifier(p.name) ? p.name.text : `arg${i}`;
  let type = p.type ? p.type.getText(sf) : null;
  if (!type && p.initializer) {
    const init = p.initializer;
    if (ts.isStringLiteral(init) || ts.isNoSubstitutionTemplateLiteral(init)) type = "string";
    else if (ts.isNumericLiteral(init)) type = "number";
    else if (init.kind === ts.SyntaxKind.TrueKeyword || init.kind === ts.SyntaxKind.FalseKeyword) type = "boolean";
  }
  if (!type) throw new Error(`cannot type parameter ${name}`);
  const optional = Boolean(p.questionToken || p.initializer);
  const rest = p.dotDotDotToken ? "..." : "";
  return `${rest}${name}${optional && !rest ? "?" : ""}: ${type}`;
}

const written = [];
const manifest = [];
let total = 0;
fs.mkdirSync(OUT, { recursive: true });

for (const file of fs.readdirSync(ACTIONS).filter((f) => f.endsWith(".ts")).sort()) {
  const module = file.replace(/\.ts$/, "");
  const rel = `app/actions/${file}`;
  const mod = indexModule(path.join(ACTIONS, file));
  const sf = mod.sf;

  const functions = [];
  const wantedTypes = new Set();
  for (const [name, d] of mod.decls) {
    if (d.exported && d.isType) wantedTypes.add(name);
  }

  const extraImports = new Map(); // module -> Set(names)
  for (const st of sf.statements) {
    if (!ts.isFunctionDeclaration(st) || !st.name || !isExported(st)) continue;
    const name = st.name.text;
    const isAsync = st.modifiers?.some((m) => m.kind === ts.SyntaxKind.AsyncKeyword);
    if (!isAsync) throw new Error(`${rel}: ${name} is exported but not async`);
    const params = st.parameters.map((p, i) => paramText(sf, p, i));
    let returnType = st.type ? st.type.getText(sf) : null;
    let note = null;
    if (!returnType) {
      const manual = INFERRED_RETURNS[`${module}.${name}`];
      if (!manual) throw new Error(`${rel}: ${name} has no return type and no INFERRED_RETURNS entry`);
      returnType = manual.type;
      note = manual.note;
      for (const [m, names] of Object.entries(manual.imports ?? {})) {
        if (!extraImports.has(m)) extraImports.set(m, new Set());
        names.forEach((n) => extraImports.get(m).add(n));
      }
    }
    // Types the signature mentions must be emitted too (local or imported).
    for (const p of st.parameters) if (p.type) referencedNames(p.type).forEach((n) => wantedTypes.add(n));
    if (st.type) referencedNames(st.type).forEach((n) => wantedTypes.add(n));
    // Only a doc comment that sits directly on the function belongs to it; a
    // section comment further up (separated by a blank line or other comments)
    // describes the module, not this action.
    const jsDoc = (ts.getJSDocCommentsAndTags(st) ?? [])
      .filter((n) => n.kind === ts.SyntaxKind.JSDoc)
      .filter((n) => /^\s*$/.test(sf.text.slice(n.end, st.getStart(sf))) && !/\n\s*\n/.test(sf.text.slice(n.end, st.getStart(sf))))
      .map((n) => n.getText(sf))
      .join("\n");
    functions.push({ name, params, returnType, note, jsDoc });
    manifest.push({
      module,
      name,
      path: `/_rpc/${module}/${name}`,
      params: st.parameters.map((p, i) => {
        const text = paramText(sf, p, i);
        return { text, optional: Boolean(p.questionToken || p.initializer), rest: Boolean(p.dotDotDotToken) };
      }),
      returns: returnType,
      upstream: `${rel}:${sf.getLineAndCharacterOfPosition(st.getStart(sf)).line + 1}`,
    });
  }

  // Only names that are declared or imported in this module are emitted.
  const wanted = [...wantedTypes].filter((n) => mod.decls.has(n) || mod.imports.has(n));
  const { statements, usedImports, missing } = closeOver(mod, wanted);
  if (missing.length) throw new Error(`${rel}: unresolved ${missing.join(", ")}`);

  // Value declarations reached through a type (`typeof x`) cannot be copied into
  // a browser stub; none exist today, and this keeps it that way.
  for (const st of statements) {
    const decl = [...mod.decls.values()].find((d) => d.node === st);
    if (decl && !decl.isType && !(ts.isFunctionDeclaration(st) && isExported(st))) {
      throw new Error(`${rel}: a stub would need the value declaration "${st.getText(sf).slice(0, 60)}..."`);
    }
  }
  const typeStatements = statements.filter((st) => !ts.isFunctionDeclaration(st));

  const imports = new Map();
  for (const [local, imp] of usedImports) {
    imports.set(local, { ...imp, module: rewriteSpecifier(rel, imp.module) });
  }

  const reexports = mod.reexports
    .filter((re) => re.typeOnly || re.names.every((n) => n.isType))
    .map((re) => statementText(mod, re.node).trim());

  const lines = [
    "// GENERATED by scripts/gen-actions.mjs from upstream " + rel + ". Do not edit.",
    `// @upstream ${rel}`,
    "//",
    "// Each export is a browser stub for the upstream server action of the same name:",
    `// it POSTs its arguments to /_rpc/${module}/<name> and resolves with what the`,
    "// action returned (docs/CONTRACT.md, \"Server actions\").",
    "",
    'import { callAction } from "../../runtime/rpc";',
    ...importLines(
      new Map([
        ...imports,
        ...[...extraImports].flatMap(([m, names]) =>
          [...names].map((n) => [n, { module: m, imported: n, typeOnly: true }]),
        ),
      ]),
      { forceType: true },
    ),
    "",
  ];
  if (reexports.length) lines.push(...reexports, "");
  for (const st of typeStatements) lines.push(statementText(mod, st).trimEnd(), "");
  for (const fn of functions) {
    if (fn.jsDoc) lines.push(fn.jsDoc);
    if (fn.note) lines.push(`// Return type inferred upstream: ${fn.note}.`);
    lines.push(
      `export async function ${fn.name}(${fn.params.join(", ")}): ${fn.returnType} {`,
      `  return callAction("${module}", "${fn.name}", arguments);`,
      "}",
      "",
    );
  }
  fs.writeFileSync(path.join(OUT, `${module}.ts`), lines.join("\n").replace(/\n{3,}/g, "\n\n"));
  written.push(`${module} (${functions.length})`);
  total += functions.length;
}

// A machine-readable list for the server side: every RPC path, its parameters
// and return type as TypeScript text, and the upstream line it stubs.
fs.writeFileSync(path.join(OUT, "..", "actions.manifest.json"), JSON.stringify(manifest, null, 2) + "\n");

console.log(`wrote ${written.length} modules, ${total} actions: ${written.join(", ")}`);
