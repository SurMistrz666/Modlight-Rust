// Reference half of docx_parity.py. Runs INSIDE the reference container
// (node 24, which strips TypeScript types itself), next to copies of the
// upstream lib/media/{docx,docx-support,zip-guard,sharp-runtime}.ts and
// lib/runtime/limits.ts whose imports docx_parity.py rewrote to the
// container's own packages: so this is upstream's docxToHtml, unchanged.
//
// Usage: node docx_reference.mjs <corpus dir> [name...]
// Prints {name: {ok, html, images, imagesDropped} | {ok: false, reason}}.
import { readdirSync, readFileSync } from "node:fs";
import { docxToHtml } from "./upstream/docx.ts";

const dir = process.argv[2];
const names = process.argv.length > 3 ? process.argv.slice(3) : readdirSync(dir).sort();
const out = {};
for (const name of names) {
  const buf = readFileSync(`${dir}/${name}`);
  try {
    out[name] = await docxToHtml(buf);
  } catch (e) {
    // docxToHtml never throws for a bad document; record it if it does.
    out[name] = { threw: String(e && e.message) };
  }
}
process.stdout.write(JSON.stringify(out), () => process.exit(0));
