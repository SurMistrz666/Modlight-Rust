import type { DuplicateGroup } from "@/lib/data/duplicates";

/**
 * What the duplicate list has selected for removal, as pure functions.
 *
 * Separated from the component because this is the part with consequences: the
 * set it produces is the exact list of ids handed to a delete. It is also the
 * part with an invariant worth stating out loud - a group must come out of the
 * default selection still holding a copy - and an invariant that lives only
 * inside a `useState` initialiser is one nobody can check.
 *
 * The type import is erased at build time, so nothing here drags the
 * server-only data module into the client bundle.
 */

/**
 * The default: remove every copy except the oldest.
 *
 * Oldest and not, say, the one in a folder or the one with the most views,
 * because the oldest is the file the links in circulation point at. A later
 * copy is what happens when somebody re-uploads something they already had; the
 * first upload is the one that has had time to be shared.
 */
export function defaultSelection(groups: readonly DuplicateGroup[]): Set<string> {
  const selected = new Set<string>();
  for (const group of groups) {
    // slice(1) and not "everything but the newest": position 0 is the oldest
    // (bucketByChecksum sorts it there) and it is the one being kept.
    for (const file of group.files.slice(1)) selected.add(file.id);
  }
  return selected;
}

/** Toggle one id, returning a NEW set so React sees the change. */
export function toggleSelection(selected: ReadonlySet<string>, id: string): Set<string> {
  const next = new Set(selected);
  if (!next.delete(id)) next.add(id);
  return next;
}

/** Ids in `selected` that belong to a listed file, in group order. */
export function selectedIds(
  groups: readonly DuplicateGroup[],
  selected: ReadonlySet<string>,
): string[] {
  const ids: string[] = [];
  for (const group of groups) {
    for (const file of group.files) {
      if (selected.has(file.id)) ids.push(file.id);
    }
  }
  return ids;
}

export interface SelectionTotals {
  /** Files marked for removal. */
  files: number;
  /** Bytes those files occupy. */
  bytes: number;
  /** Groups where every listed copy is marked, so nothing would be kept. */
  emptied: number;
}

/**
 * What the current selection would cost.
 *
 * Summed from the files themselves rather than from the group's `reclaimable`,
 * because the user is free to keep three of four copies and the headline number
 * has to follow the checkboxes, not the recommendation.
 *
 * `emptied` exists so the UI can say so. Selecting every copy is allowed - it
 * lands in the trash like anything else - but it is never what "de-duplicate"
 * meant, and it should not happen without the user being told.
 */
export function selectionTotals(
  groups: readonly DuplicateGroup[],
  selected: ReadonlySet<string>,
): SelectionTotals {
  let files = 0;
  let bytes = 0;
  let emptied = 0;

  for (const group of groups) {
    let picked = 0;
    for (const file of group.files) {
      if (!selected.has(file.id)) continue;
      picked++;
      files++;
      bytes += file.size;
    }
    // A group whose list was trimmed (copies > files.length) still has copies
    // off-screen, so clearing the visible ones does not empty it.
    if (picked > 0 && picked === group.files.length && picked === group.copies) emptied++;
  }

  return { files, bytes, emptied };
}

/** Does this group still keep at least one copy under the current selection? */
export function keepsACopy(group: DuplicateGroup, selected: ReadonlySet<string>): boolean {
  if (group.copies > group.files.length) return true;
  return group.files.some((file) => !selected.has(file.id));
}
