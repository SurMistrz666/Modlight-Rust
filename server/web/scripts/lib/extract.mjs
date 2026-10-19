// Pulls declarations out of an upstream module VERBATIM, with the local
// declarations and imports they depend on. Syntax only (TypeScript's parser, no
// type checker, nothing executed), so the result is exactly the upstream text.
import fs from "node:fs";
import ts from "typescript";

const isExported = (node) =>
  (ts.getCombinedModifierFlags(node) & ts.ModifierFlags.Export) !== 0;

/** Parse once, index top-level declarations and imports. */
export function indexModule(file) {
  const text = fs.readFileSync(file, "utf8");
  const sf = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const decls = new Map(); // name -> { node, kind, exported, isType }
  const imports = new Map(); // local name -> { module, imported, typeOnly }
  const reexports = []; // { node, module, names: [{name, propertyName, isType}], typeOnly }

  for (const st of sf.statements) {
    if (ts.isImportDeclaration(st)) {
      const module = st.moduleSpecifier.text;
      const clause = st.importClause;
      if (!clause) continue;
      const typeOnlyClause = clause.isTypeOnly;
      if (clause.name) imports.set(clause.name.text, { module, imported: "default", typeOnly: typeOnlyClause });
      const nb = clause.namedBindings;
      if (nb && ts.isNamedImports(nb)) {
        for (const el of nb.elements) {
          imports.set(el.name.text, {
            module,
            imported: (el.propertyName ?? el.name).text,
            typeOnly: typeOnlyClause || el.isTypeOnly,
          });
        }
      } else if (nb && ts.isNamespaceImport(nb)) {
        imports.set(nb.name.text, { module, imported: "*", typeOnly: typeOnlyClause });
      }
      continue;
    }
    if (ts.isExportDeclaration(st) && st.moduleSpecifier) {
      const names = [];
      if (st.exportClause && ts.isNamedExports(st.exportClause)) {
        for (const el of st.exportClause.elements) {
          names.push({ name: el.name.text, propertyName: el.propertyName?.text, isType: st.isTypeOnly || el.isTypeOnly });
        }
      }
      reexports.push({ node: st, module: st.moduleSpecifier.text, names, typeOnly: st.isTypeOnly, star: !st.exportClause });
      continue;
    }
    const add = (name, kind, isType) =>
      decls.set(name, { node: st, kind, exported: isExported(st), isType });
    if (ts.isInterfaceDeclaration(st)) add(st.name.text, "interface", true);
    else if (ts.isTypeAliasDeclaration(st)) add(st.name.text, "type", true);
    else if (ts.isEnumDeclaration(st)) add(st.name.text, "enum", false);
    else if (ts.isClassDeclaration(st) && st.name) add(st.name.text, "class", false);
    else if (ts.isFunctionDeclaration(st) && st.name) add(st.name.text, "function", false);
    else if (ts.isVariableStatement(st)) {
      for (const d of st.declarationList.declarations) {
        if (ts.isIdentifier(d.name)) add(d.name.text, "const", false);
      }
    }
  }
  return { file, text, sf, decls, imports, reexports };
}

/** Identifier names referenced anywhere inside `node` (types and values alike). */
export function referencedNames(node) {
  const out = new Set();
  const visit = (n) => {
    if (ts.isIdentifier(n)) out.add(n.text);
    // `typeof x.y` / `x.y` - only the left-most name refers to a binding.
    if (ts.isPropertyAccessExpression(n)) {
      visit(n.expression);
      return;
    }
    if (ts.isQualifiedName(n)) {
      visit(n.left);
      return;
    }
    // Property names in object types / literals are not references.
    if ((ts.isPropertySignature(n) || ts.isPropertyAssignment(n) || ts.isMethodSignature(n)) && n.name) {
      if (n.questionToken) visit(n.questionToken);
      if (n.type) visit(n.type);
      if (n.initializer) visit(n.initializer);
      if (n.parameters) n.parameters.forEach(visit);
      if (n.typeParameters) n.typeParameters.forEach(visit);
      return;
    }
    ts.forEachChild(n, visit);
  };
  visit(node);
  return out;
}

/** Leading comments + statement text, exactly as in the file. */
export function statementText(mod, node) {
  return node.getFullText(mod.sf).replace(/^\s*\n/, "").replace(/^\n+/, "");
}

/**
 * Close over `wanted` (top-level names) through local declarations. Returns the
 * statements to emit (in source order) and the imported names they use.
 */
export function closeOver(mod, wanted) {
  const stmts = new Set();
  const usedImports = new Map(); // local name -> import info
  const missing = [];
  const queue = [...wanted];
  const seen = new Set();
  while (queue.length) {
    const name = queue.shift();
    if (seen.has(name)) continue;
    seen.add(name);
    const d = mod.decls.get(name);
    if (d) {
      stmts.add(d.node);
      for (const ref of referencedNames(d.node)) {
        if (ref === name) continue;
        if (mod.decls.has(ref) || mod.imports.has(ref)) queue.push(ref);
      }
      continue;
    }
    const imp = mod.imports.get(name);
    if (imp) {
      usedImports.set(name, imp);
      continue;
    }
    missing.push(name);
  }
  const ordered = mod.sf.statements.filter((s) => stmts.has(s));
  return { statements: ordered, usedImports, missing };
}

/** `import type { a, b as c } from "m";` lines for the used imports. */
export function importLines(usedImports, { forceType = true, rewrite = (m) => m } = {}) {
  const byModule = new Map();
  for (const [local, imp] of usedImports) {
    const module = rewrite(imp.module);
    if (!byModule.has(module)) byModule.set(module, []);
    byModule.get(module).push({ local, ...imp });
  }
  const lines = [];
  for (const [module, list] of [...byModule].sort((a, b) => a[0].localeCompare(b[0]))) {
    const typeOnly = forceType || list.every((x) => x.typeOnly);
    const defaults = list.filter((x) => x.imported === "default");
    const named = list
      .filter((x) => x.imported !== "default" && x.imported !== "*")
      .map((x) => (x.imported === x.local ? x.local : `${x.imported} as ${x.local}`));
    const ns = list.find((x) => x.imported === "*");
    const parts = [];
    if (defaults.length) parts.push(defaults[0].local);
    if (named.length) parts.push(`{ ${named.join(", ")} }`);
    if (ns) parts.push(`* as ${ns.local}`);
    lines.push(`import ${typeOnly ? "type " : ""}${parts.join(", ")} from "${module}";`);
  }
  return lines;
}

export { ts };
