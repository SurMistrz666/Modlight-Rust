/**
 * Tagged JSON: plain JSON plus four tags for values JSON cannot carry
 * (docs/CONTRACT.md "Tagged JSON"). Used in both directions: RPC arguments and
 * results, and page payloads.
 *
 *   Date       {"$date": "2026-10-18T12:00:00.000Z"}
 *   undefined  {"$undefined": 1}
 *   bigint     {"$bigint": "123"}
 *   NaN/±Inf   {"$num": "NaN" | "Infinity" | "-Infinity"}
 *
 * `undefined` is kept wherever it appears, object properties included, because
 * upstream code can tell `{ locale: undefined }` from `{}` (`"locale" in input`),
 * and React's server-action encoding keeps the key too.
 */

type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

function isPlainObject(value: unknown): value is Record<string, unknown> {
  if (value === null || typeof value !== "object") return false;
  const proto = Object.getPrototypeOf(value);
  return proto === Object.prototype || proto === null;
}

export function encodeTagged(value: unknown): Json {
  if (value === undefined) return { $undefined: 1 };
  if (value === null) return null;
  switch (typeof value) {
    case "string":
    case "boolean":
      return value;
    case "number":
      if (Number.isNaN(value)) return { $num: "NaN" };
      if (value === Infinity) return { $num: "Infinity" };
      if (value === -Infinity) return { $num: "-Infinity" };
      return value;
    case "bigint":
      return { $bigint: value.toString() };
    case "object":
      break;
    default:
      throw new TypeError(`Cannot send a ${typeof value} to the server`);
  }
  if (value instanceof Date) return { $date: value.toISOString() };
  if (Array.isArray(value)) return value.map((v) => encodeTagged(v));
  if (isPlainObject(value)) {
    const out: Record<string, Json> = {};
    for (const [k, v] of Object.entries(value)) out[k] = encodeTagged(v);
    return out;
  }
  throw new TypeError(`Cannot send ${Object.prototype.toString.call(value)} to the server`);
}

export function decodeTagged<T = unknown>(value: unknown): T {
  return decode(value) as T;
}

function decode(value: unknown): unknown {
  if (value === null || typeof value !== "object") return value;
  if (Array.isArray(value)) return value.map(decode);
  const obj = value as Record<string, unknown>;
  const keys = Object.keys(obj);
  if (keys.length === 1) {
    const [key] = keys;
    const v = obj[key];
    if (key === "$date" && typeof v === "string") return new Date(v);
    if (key === "$undefined") return undefined;
    if (key === "$bigint" && typeof v === "string") return BigInt(v);
    if (key === "$num" && typeof v === "string") return Number(v);
  }
  const out: Record<string, unknown> = {};
  for (const k of keys) out[k] = decode(obj[k]);
  return out;
}
