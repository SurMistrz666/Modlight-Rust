/**
 * The typed-confirmation rule for irreversible admin actions.
 *
 * Lives here — next to the dialog — rather than in lib/data/admin.ts because
 * BOTH sides need it: the dialog to enable its button, and the Server Action to
 * refuse a request that never went through the dialog. lib/data/admin.ts pulls
 * in `node:fs` and the database pool, so importing a value from it would drag
 * all of that into the client bundle.
 *
 * The client check is a courtesy; app/actions/admin.ts re-runs this against a
 * count it reads itself, which is the check that actually matters.
 */

/**
 * Does the typed confirmation authorise destroying `count` files?
 *
 * "Delete everything" removes bytes that cannot come back, so the gate is
 * retyping the exact number of files — a value the operator has to have read to
 * know. A word ("DELETE") would need translating and, worse, becomes muscle
 * memory; a number changes every time.
 *
 * Digit grouping is stripped first, so the "8,630" / "8 630" / "8.630" the
 * dialog printed can be retyped as-is. Zero never confirms anything: an empty
 * list plus an empty input must not read as consent.
 */
export function confirmationMatches(input: string, count: number): boolean {
  if (!Number.isFinite(count) || count <= 0) return false;
  const clean = input.replace(/[\s,._']/g, "");
  if (!/^\d+$/.test(clean)) return false;
  return Number(clean) === count;
}
