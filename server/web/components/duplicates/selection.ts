import type { DismissalInput, DuplicateGroup } from "@/lib/data/duplicates";

/**
 * Per-SET operations on the selection, for the screen that lists whole groups.
 *
 * The library-wide half of this algebra already exists in
 * components/profile/duplicate-selection.ts - `defaultSelection`,
 * `toggleSelection`, `selectedIds`, `selectionTotals`, `keepsACopy` - and is
 * imported from there rather than written again. There is exactly one
 * definition of "which ids does the trash button send", because the day there
 * are two is the day one of them stops leaving a copy behind.
 *
 * What is new here is choosing WHICH copy survives, which the profile card had
 * no control for: it offered the default (keep the oldest) and a checkbox per
 * row. On a page built around the question "what do I keep?", picking the
 * keeper has to be one click.
 *
 * The type imports are erased at build time, so nothing here drags the
 * server-only data module into the client bundle.
 */

/**
 * Keep exactly one copy of `group` and mark the rest.
 *
 * Scoped to the group: ids belonging to other sets are carried through
 * untouched, so pressing "keep this one" in one set cannot quietly rewrite a
 * decision made in another.
 *
 * A set whose file list was trimmed (`copies > files.length`) keeps its unlisted
 * copies whatever happens here - they are not on screen, so they are not being
 * proposed for anything.
 */
export function keepOnly(
  selected: ReadonlySet<string>,
  group: DuplicateGroup,
  keepId: string,
): Set<string> {
  const next = new Set(selected);
  for (const file of group.files) {
    if (file.id === keepId) next.delete(file.id);
    else next.add(file.id);
  }
  return next;
}

/** Take a whole set out of the selection: none of its copies will be trashed. */
export function keepEverything(
  selected: ReadonlySet<string>,
  group: DuplicateGroup,
): Set<string> {
  const next = new Set(selected);
  for (const file of group.files) next.delete(file.id);
  return next;
}

/** Which copy of this set is being kept, when exactly one is. */
export function keptId(group: DuplicateGroup, selected: ReadonlySet<string>): string | null {
  const kept = group.files.filter((file) => !selected.has(file.id));
  return kept.length === 1 ? kept[0].id : null;
}

/**
 * The dismissal records for these sets, in the shape the action wants.
 *
 * `copies` and `bytes` are a note about what the set looked like when it was
 * waved off, never an input to a decision - the scan always recounts. They are
 * sent so the row can say "you kept 4 copies of this in March" instead of
 * showing a bare digest.
 */
export function dismissalsFor(groups: readonly DuplicateGroup[]): DismissalInput[] {
  return groups.map((group) => ({
    checksum: group.checksum,
    copies: group.copies,
    bytes: group.bytes,
  }));
}

/** The sets still being counted, and the ones the owner has waved off. */
export function splitByDismissed(groups: readonly DuplicateGroup[]): {
  counted: DuplicateGroup[];
  kept: DuplicateGroup[];
} {
  const counted: DuplicateGroup[] = [];
  const kept: DuplicateGroup[] = [];
  for (const group of groups) (group.dismissed ? kept : counted).push(group);
  return { counted, kept };
}
