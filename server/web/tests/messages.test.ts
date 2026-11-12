/**
 * The dictionaries the panel renders with (server/messages/{en,pl}, sent by
 * the server in `layouts.root.messages`): both languages carry the same keys,
 * and Polish counts are inflected. Polish has three forms for whole numbers
 * (1 plik, 2-4 pliki, 5+ plików, with 12-14 and 22-24 rules), so a message that
 * puts a count before a noun needs a plural with `one`, `few` and `many`.
 */
import { describe, expect, it } from "vitest";
import { createTranslator } from "next-intl";
import { fixtureMessages } from "../src/fixtures/source";

type Tree = { [key: string]: string | Tree };

function flatten(tree: Tree, prefix = "", out: Record<string, string> = {}): Record<string, string> {
  for (const [key, value] of Object.entries(tree)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (typeof value === "string") out[path] = value;
    else flatten(value, path, out);
  }
  return out;
}

const en = flatten(fixtureMessages("en") as Tree);
const pl = flatten(fixtureMessages("pl") as Tree);

/** The selectors of every `{arg, plural, ...}` in an ICU message, nested ones included. */
function pluralSelectors(message: string): string[][] {
  const found: string[][] = [];
  const start = /\{\s*\w+\s*,\s*plural\s*,/g;
  for (let m = start.exec(message); m; m = start.exec(message)) {
    const selectors: string[] = [];
    let i = m.index + m[0].length;
    for (;;) {
      while (/\s/.test(message[i] ?? "")) i++;
      const sel = /^(?:offset:\d+|=?[\w-]+)/.exec(message.slice(i));
      if (!sel) break;
      i += sel[0].length;
      if (sel[0].startsWith("offset:")) continue;
      while (/\s/.test(message[i] ?? "")) i++;
      if (message[i] !== "{") break;
      for (let depth = 0; i < message.length; i++) {
        if (message[i] === "{") depth++;
        else if (message[i] === "}" && --depth === 0) {
          i++;
          break;
        }
      }
      selectors.push(sel[0]);
    }
    found.push(selectors);
  }
  return found;
}

describe("the dictionaries", () => {
  it("have the same keys in English and Polish", () => {
    expect(Object.keys(pl).filter((k) => !(k in en))).toEqual([]);
    expect(Object.keys(en).filter((k) => !(k in pl))).toEqual([]);
  });

  it("inflect every Polish count three ways", () => {
    const missing: string[] = [];
    for (const [key, message] of Object.entries(pl)) {
      for (const selectors of pluralSelectors(message)) {
        const absent = ["one", "few", "many", "other"].filter((c) => !selectors.includes(c));
        if (absent.length > 0) missing.push(`${key}: no ${absent.join(", ")}`);
      }
    }
    expect(missing).toEqual([]);
  });
});

describe("Polish counts read right", () => {
  // The dictionaries are untyped here (they come from JSON files at run time).
  const t = createTranslator({ locale: "pl", messages: fixtureMessages("pl") }) as unknown as (
    key: string,
    values?: Record<string, number>,
  ) => string;
  const cases: [string, Record<string, number>, string][] = [
    ["files.menu.moveManyToFolder", { count: 2 }, "Przenieś 2 pliki do folderu…"],
    ["files.menu.moveManyToFolder", { count: 5 }, "Przenieś 5 plików do folderu…"],
    ["files.menu.moveManyToFolder", { count: 22 }, "Przenieś 22 pliki do folderu…"],
    ["files.menu.trashMany", { count: 3 }, "Przenieś 3 pliki do kosza"],
    ["files.menu.trashMany", { count: 12 }, "Przenieś 12 plików do kosza"],
    ["upload.toast.many", { count: 4 }, "Wysłano 4 pliki"],
    ["upload.toast.many", { count: 7 }, "Wysłano 7 plików"],
    ["upload.results.copiedAll", { count: 2 }, "Skopiowano 2 linki"],
    ["upload.results.copiedAll", { count: 25 }, "Skopiowano 25 linków"],
    ["upload.encrypted.copiedAll", { count: 3 }, "Skopiowano 3 linki"],
    ["upload.dock.liveDone", { failed: 0, total: 1 }, "Zakończono wysyłanie 1 pliku."],
    ["upload.dock.liveDone", { failed: 0, total: 3 }, "Zakończono wysyłanie 3 plików."],
    ["upload.dock.liveDone", { failed: 2, total: 9 }, "Wysyłanie zakończone, 2 pliki nie zostały wysłane."],
    ["upload.dock.liveDone", { failed: 5, total: 9 }, "Wysyłanie zakończone, 5 plików nie zostało wysłanych."],
    ["upload.dock.liveDone", { failed: 1, total: 9 }, "Wysyłanie zakończone, 1 plik nie został wysłany."],
    ["upload.duplicates.bodyPartial", { count: 2, total: 5 }, "2 z 5 wybranych plików mają dokładnie taką samą zawartość jak pliki, które już masz."],
    ["upload.duplicates.bodyPartial", { count: 5, total: 9 }, "5 z 9 wybranych plików ma dokładnie taką samą zawartość jak pliki, które już masz."],
  ];
  for (const [key, values, expected] of cases) {
    it(`${key} ${JSON.stringify(values)}`, () => {
      expect(t(key, values)).toBe(expected);
    });
  }

  it("profile.duplicates.unchecksummed picks one form", () => {
    expect(t("profile.duplicates.unchecksummed", { count: 3 })).toMatch(/^3 pliki nie mają zapisanej sumy/);
    expect(t("profile.duplicates.unchecksummed", { count: 6 })).toMatch(/^6 plików nie ma zapisanej sumy/);
  });
});
