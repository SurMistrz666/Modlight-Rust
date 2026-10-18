import { hasUploadDrag } from "@/components/dashboard/types";

/**
 * Reading a drag that came from OUTSIDE the browser.
 *
 * Two drags look almost identical to the DOM: a tile being moved between
 * folders (our own payload, see components/dashboard/types.ts) and a file
 * arriving from the desktop. They must never be confused — lighting up the
 * "drop to upload" overlay while somebody re-files a screenshot would be both
 * wrong and alarming — so every check here starts by excluding our own type.
 */

/** Hard cap on a single dropped folder, so a stray ~/Downloads cannot hang the tab. */
const MAX_ENTRIES = 500;

/** How deep into a dropped folder tree we walk. */
const MAX_DEPTH = 8;

/**
 * True when this drag carries files from the operating system.
 *
 * `dataTransfer.types` is the only thing readable during `dragover` (the data
 * itself is withheld until `drop` for security), which is exactly why the
 * internal payload is written under a distinctive MIME type.
 */
export function isExternalFileDrag(dt: DataTransfer | null | undefined): boolean {
  if (!dt) return false;
  if (hasUploadDrag(dt)) return false;
  return Array.from(dt.types).includes("Files");
}

/**
 * Elements that handle their own file drops.
 *
 * The Uppy Dashboard is the obvious one: on /dashboard/upload it already is a
 * drop target, and it stops propagation, but a browser that delivers the event
 * elsewhere would otherwise get two handlers. `data-no-global-drop` lets any
 * future widget opt out the same way.
 */
export function isExemptTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  return Boolean(target.closest(".uppy-Dashboard, [data-no-global-drop]"));
}

type EntryLike = {
  isFile: boolean;
  isDirectory: boolean;
  file?: (cb: (f: File) => void, err: (e: unknown) => void) => void;
  createReader?: () => {
    readEntries: (cb: (entries: EntryLike[]) => void, err: (e: unknown) => void) => void;
  };
};

function entryFile(entry: EntryLike): Promise<File | null> {
  return new Promise((resolve) => {
    if (!entry.file) return resolve(null);
    entry.file(
      (f) => resolve(f),
      () => resolve(null),
    );
  });
}

/**
 * `readEntries` returns at most ~100 entries per call and signals the end with
 * an empty batch, so a large folder has to be drained in a loop.
 */
function readBatch(
  reader: ReturnType<NonNullable<EntryLike["createReader"]>>,
): Promise<EntryLike[]> {
  return new Promise((resolve) => {
    reader.readEntries(
      (entries) => resolve(entries),
      () => resolve([]),
    );
  });
}

async function walkEntry(entry: EntryLike, out: File[], depth: number): Promise<void> {
  if (out.length >= MAX_ENTRIES) return;

  if (entry.isFile) {
    const file = await entryFile(entry);
    if (file) out.push(file);
    return;
  }

  if (!entry.isDirectory || depth >= MAX_DEPTH || !entry.createReader) return;
  const reader = entry.createReader();
  for (;;) {
    const batch = await readBatch(reader);
    if (batch.length === 0) break;
    for (const child of batch) {
      if (out.length >= MAX_ENTRIES) return;
      await walkEntry(child, out, depth + 1);
    }
  }
}

/**
 * Every file in a drop, expanding dropped folders.
 *
 * `dataTransfer.items` is snapshot-only: it is emptied as soon as the event
 * handler returns, so the entries are captured synchronously up front and the
 * async walk happens afterwards. Without that, awaiting anything first would
 * leave us reading an empty list.
 *
 * Falls back to `dataTransfer.files` where the entry API is unavailable, which
 * loses folder expansion but never loses a plain multi-file drop.
 */
export async function filesFromDrop(dt: DataTransfer | null): Promise<File[]> {
  if (!dt) return [];

  const entries: EntryLike[] = [];
  const plain: File[] = [];
  if (dt.items && dt.items.length > 0) {
    for (const item of Array.from(dt.items)) {
      if (item.kind !== "file") continue;
      const getEntry = (item as DataTransferItem & { webkitGetAsEntry?: () => EntryLike | null })
        .webkitGetAsEntry;
      const entry = typeof getEntry === "function" ? getEntry.call(item) : null;
      if (entry) entries.push(entry);
      else {
        const file = item.getAsFile();
        if (file) plain.push(file);
      }
    }
  }

  if (entries.length === 0) {
    return plain.length > 0 ? plain : Array.from(dt.files ?? []);
  }

  const out: File[] = [...plain];
  for (const entry of entries) {
    if (out.length >= MAX_ENTRIES) break;
    await walkEntry(entry, out, 0);
  }
  return out;
}
