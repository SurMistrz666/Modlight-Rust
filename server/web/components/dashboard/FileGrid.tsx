"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslations } from "next-intl";
import {
  ArrowBigUp,
  CheckSquare,
  Command,
  MouseLeft,
  MouseRight,
  Square,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { FileCard } from "./FileCard";
import { BulkBar } from "./BulkBar";
import { FileContextMenu } from "./FileMenu";
import { DetailsDrawer } from "./DetailsDrawer";
import { buildOpenUrl } from "./media";
import { setUploadDragPayload, type FileItem, type FileViewMode } from "./types";

/**
 * Shared selection + optimistic-mutation state for the grid/list views.
 *
 * Lifted into a hook so both FileGrid and FileList behave identically. The
 * gestures follow every desktop file manager, which matters at 6800 files:
 *   • plain click      → open details (or collapse a multi-selection)
 *   • plain double     → open the file itself in a new tab
 *   • ⌘/Ctrl + click   → toggle one
 *   • Shift + click    → extend from the anchor
 *   • ⌘/Ctrl + A       → select everything on the page
 *   • Escape           → clear
 * Right-click is deliberately NOT a selection gesture any more: it opens the
 * app's own context menu instead.
 */
export function useFileSelection(
  items: FileItem[],
  opts: { currentFolderId?: string | null } = {},
) {
  const { currentFolderId } = opts;
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const anchorRef = useRef<string | null>(null);

  useEffect(() => {
    setHidden(new Set());
    setSelected(new Set());
    anchorRef.current = null;
  }, [items]);

  const visible = useMemo(() => items.filter((i) => !hidden.has(i.id)), [items, hidden]);

  const hide = useCallback((ids: string[]) => {
    setHidden((prev) => {
      const next = new Set(prev);
      ids.forEach((id) => next.add(id));
      return next;
    });
    setSelected((prev) => {
      if (!ids.some((id) => prev.has(id))) return prev;
      const next = new Set(prev);
      ids.forEach((id) => next.delete(id));
      return next;
    });
  }, []);

  const onTrashed = useCallback((id: string) => hide([id]), [hide]);
  const onBulkTrashed = useCallback((ids: string[]) => hide(ids), [hide]);

  /**
   * A move only removes cards that actually left the view: the file must have
   * been in the folder being browsed and have landed somewhere else. Checking
   * the file's own folder (not just the view's) keeps "all files" honest — a
   * card there stays put and simply shows a new folder chip after the refresh.
   */
  const onMoved = useCallback(
    (ids: string[], folderId: string | null) => {
      if (currentFolderId === undefined || folderId === currentFolderId) return;
      const byId = new Map(items.map((i) => [i.id, i]));
      const leaving = ids.filter((id) => (byId.get(id)?.folderId ?? null) === currentFolderId);
      if (leaving.length > 0) hide(leaving);
    },
    [currentFolderId, hide, items],
  );

  const toggle = useCallback((id: string) => {
    anchorRef.current = id;
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const selectOnly = useCallback((id: string) => {
    anchorRef.current = id;
    setSelected(new Set([id]));
  }, []);

  const selectRange = useCallback(
    (id: string) => {
      const ids = visible.map((i) => i.id);
      const to = ids.indexOf(id);
      if (to < 0) return;
      const from = anchorRef.current ? ids.indexOf(anchorRef.current) : -1;
      if (from < 0) {
        anchorRef.current = id;
        setSelected(new Set([id]));
        return;
      }
      const [a, b] = from <= to ? [from, to] : [to, from];
      setSelected(new Set(ids.slice(a, b + 1)));
    },
    [visible],
  );

  const clearSelection = useCallback(() => {
    anchorRef.current = null;
    setSelected(new Set());
  }, []);

  const allSelected = visible.length > 0 && visible.every((i) => selected.has(i.id));
  const someSelected = selected.size > 0 && !allSelected;

  const toggleSelectAll = useCallback(() => {
    setSelected((prev) => {
      const all = visible.length > 0 && visible.every((i) => prev.has(i.id));
      return all ? new Set() : new Set(visible.map((i) => i.id));
    });
  }, [visible]);

  /** Ids a menu/drag should act on when started from `id`. */
  const targetsFor = useCallback(
    (id: string) => (selected.has(id) && selected.size > 1 ? Array.from(selected) : [id]),
    [selected],
  );

  return {
    visible,
    selected,
    allSelected,
    someSelected,
    anchorRef,
    toggle,
    selectOnly,
    selectRange,
    toggleSelectAll,
    clearSelection,
    targetsFor,
    onTrashed,
    onBulkTrashed,
    onMoved,
  };
}

/**
 * How long a plain click waits before it acts.
 *
 * One click opens the details drawer and two clicks open the file itself, so
 * the first of a pair must not fire. Browsers give no "this was really a single
 * click" signal, only a `detail` counter and a following `dblclick`, hence the
 * grace window. 220 ms sits just above the default double-click speed on
 * Windows and macOS while still feeling immediate.
 */
const DOUBLE_CLICK_GRACE_MS = 220;

/**
 * Defers a plain click so a following double click can cancel it. Shared by the
 * grid and the list so both react identically.
 */
export function useSingleClickGuard() {
  const timer = useRef<number | null>(null);

  const cancel = useCallback(() => {
    if (timer.current != null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
  }, []);

  const schedule = useCallback(
    (run: () => void) => {
      cancel();
      timer.current = window.setTimeout(() => {
        timer.current = null;
        run();
      }, DOUBLE_CLICK_GRACE_MS);
    },
    [cancel],
  );

  // A pending click must not land after the view is gone (trash, navigation).
  useEffect(() => cancel, [cancel]);

  return { schedule, cancel };
}

/**
 * Opens a file's public page in a new, focused tab.
 *
 * `opener` is cleared by hand instead of passing the `noopener` feature string:
 * with `noopener` several browsers return `null`, which would cost us the
 * `focus()` call and the tab would open in the background.
 */
export function openFileInNewTab(url: string) {
  const win = window.open(url, "_blank");
  if (!win) return;
  win.opener = null;
  win.focus();
}

/**
 * True when the gesture started on a control that activates itself (the quick
 * actions, the checkbox, the menu button). Those swallow their own `click`, but
 * `dblclick` still bubbles, and opening a tab on top of "copy link" is exactly
 * the kind of surprise the tile must not produce.
 */
export function fromInteractive(e: React.MouseEvent) {
  const el = e.target as HTMLElement | null;
  return !!el?.closest("a,button,input,label,[role='menuitem'],[role='button']");
}

/** Renders a "12 files" pill as the drag image for multi-file drags. */
function setMultiDragImage(e: React.DragEvent, label: string) {
  const ghost = document.createElement("div");
  ghost.textContent = label;
  ghost.style.cssText =
    "position:fixed;top:-1000px;left:-1000px;padding:6px 12px;border-radius:999px;" +
    "font:500 13px/1.2 system-ui,sans-serif;white-space:nowrap;" +
    "background:var(--primary,#111);color:var(--primary-foreground,#fff);" +
    "box-shadow:0 6px 20px rgba(0,0,0,.25)";
  document.body.appendChild(ghost);
  e.dataTransfer.setDragImage(ghost, 12, 12);
  // The browser snapshots the node synchronously; drop it on the next tick.
  setTimeout(() => ghost.remove(), 0);
}

/**
 * Legend for the mouse gestures the grid and the list both understand.
 *
 * The modifiers are drawn as icons — ⇧ for Shift, ⌘ on a Mac, a mouse with the
 * right button lit for the context menu — with the spelled-out name on
 * `aria-label` and `title`, so a screen reader still announces "Shift" and a
 * pointer user still gets a tooltip. `<kbd>` carries no implicit ARIA role,
 * which is why `role="img"` is spelled out: without it the label is dropped.
 */
export function SelectionHints({ className }: { className?: string }) {
  const t = useTranslations("files.selection");
  const isMac = useIsMac();

  const click = (
    <KeyCap label={t("keys.click")}>
      <MouseLeft className="size-3" aria-hidden />
    </KeyCap>
  );

  const hints: { key: string; keys: React.ReactNode; text: string }[] = [
    { key: "details", keys: click, text: t("hints.details") },
    {
      key: "newTab",
      keys: (
        <KeyCap label={t("keys.doubleClick")}>
          <MouseLeft className="size-3" aria-hidden />
          <span aria-hidden className="text-[9px] leading-none font-semibold">
            ×2
          </span>
        </KeyCap>
      ),
      text: t("hints.newTab"),
    },
    {
      key: "select",
      keys: (
        <>
          <KeyCap label={isMac ? t("keys.cmd") : t("keys.ctrl")}>
            {isMac ? <Command className="size-3" aria-hidden /> : t("keys.ctrl")}
          </KeyCap>
          {click}
        </>
      ),
      text: t("hints.select"),
    },
    {
      key: "range",
      keys: (
        <>
          <KeyCap label={t("keys.shift")}>
            <ArrowBigUp className="size-3" aria-hidden />
          </KeyCap>
          {click}
        </>
      ),
      text: t("hints.range"),
    },
    {
      key: "menu",
      keys: (
        <KeyCap label={t("keys.rightClick")}>
          <MouseRight className="size-3" aria-hidden />
        </KeyCap>
      ),
      text: t("hints.menu"),
    },
  ];

  return (
    <ul
      aria-label={t("hintsLabel")}
      className={cn(
        "text-muted-foreground/80 hidden flex-wrap items-center gap-x-3 gap-y-1.5 text-xs sm:flex",
        className,
      )}
    >
      {hints.map((hint) => (
        <li key={hint.key} className="inline-flex items-center gap-1.5">
          <span className="inline-flex items-center gap-0.5">{hint.keys}</span>
          {hint.text}
        </li>
      ))}
    </ul>
  );
}

function KeyCap({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <kbd
      role="img"
      aria-label={label}
      title={label}
      className="bg-muted text-foreground ring-foreground/10 inline-flex h-5 min-w-5 items-center justify-center gap-0.5 rounded px-1 font-sans text-[10px] leading-none font-medium ring-1"
    >
      {children}
    </kbd>
  );
}

/**
 * True on Apple platforms, where the browser reports the ⌘ key as `metaKey`.
 * Resolved after mount on purpose: the server cannot know which machine this
 * is, and rendering the Mac variant during SSR would hydrate-mismatch.
 */
function useIsMac() {
  const [isMac, setIsMac] = useState(false);

  useEffect(() => {
    const nav = navigator as Navigator & { userAgentData?: { platform?: string } };
    const probe = `${nav.userAgentData?.platform ?? ""} ${nav.platform ?? ""} ${nav.userAgent}`;
    setIsMac(/\b(mac|iphone|ipad|ipod)/i.test(probe));
  }, []);

  return isMac;
}

export function FileGrid({
  items,
  userCode,
  appUrl,
  copyRaw,
  currentFolderId,
  mode = "owner",
}: {
  items: FileItem[];
  userCode: string;
  appUrl: string;
  copyRaw?: boolean;
  /** Folder being browsed — omit for "all files". */
  currentFolderId?: string | null;
  /** Owner's own library, or an administrator reviewing it. See types.ts. */
  mode?: FileViewMode;
}) {
  const t = useTranslations("files");
  const {
    visible,
    selected,
    allSelected,
    someSelected,
    toggle,
    selectOnly,
    selectRange,
    toggleSelectAll,
    clearSelection,
    targetsFor,
    onTrashed,
    onBulkTrashed,
    onMoved,
  } = useFileSelection(items, { currentFolderId });

  const { schedule, cancel } = useSingleClickGuard();
  const gridRef = useRef<HTMLDivElement | null>(null);
  const tileRefs = useRef<(HTMLDivElement | null)[]>([]);
  const [cursor, setActiveIndex] = useState(0);
  const [detailsIndex, setDetailsIndex] = useState<number | null>(null);
  const [draggingIds, setDraggingIds] = useState<string[] | null>(null);

  const drawerOpen = detailsIndex != null;

  // Clamp on read rather than in an effect, so a shorter page (trash, move)
  // can't leave the roving-tabindex cursor pointing past the end for a render.
  const activeIndex = Math.min(cursor, Math.max(0, visible.length - 1));

  const columns = useCallback(() => {
    const el = gridRef.current;
    if (!el) return 1;
    const template = getComputedStyle(el).gridTemplateColumns;
    return Math.max(1, template.split(" ").filter(Boolean).length);
  }, []);

  const focusIndex = useCallback(
    (i: number) => {
      const next = Math.max(0, Math.min(visible.length - 1, i));
      setActiveIndex(next);
      tileRefs.current[next]?.focus();
      return next;
    },
    [visible.length],
  );

  // Escape clears the selection; ⌘/Ctrl+A selects the page.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const el = e.target as HTMLElement | null;
      const typing =
        !!el && (el.isContentEditable || /^(input|textarea|select)$/i.test(el.tagName));
      if (typing) return;

      if (e.key === "Escape" && !drawerOpen && selected.size > 0) {
        clearSelection();
        return;
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "a" && !drawerOpen) {
        if (visible.length === 0) return;
        e.preventDefault();
        toggleSelectAll();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [drawerOpen, selected.size, visible.length, clearSelection, toggleSelectAll]);

  /**
   * One click opens the details drawer, two open the file. The modifier
   * gestures stay instant — only the plain click waits out the double-click
   * window, because it is the only one a second click can supersede.
   */
  function onTileClick(e: React.MouseEvent, item: FileItem, index: number) {
    setActiveIndex(index);
    if (e.metaKey || e.ctrlKey) {
      cancel();
      toggle(item.id);
      return;
    }
    if (e.shiftKey) {
      cancel();
      // Stop the browser from selecting the text between tiles.
      window.getSelection()?.removeAllRanges();
      selectRange(item.id);
      return;
    }
    // The second click of a double click fires `click` too; let `dblclick` win.
    if (e.detail > 1) return;
    schedule(() => {
      if (selected.size > 0) {
        if (selected.size === 1 && selected.has(item.id)) setDetailsIndex(index);
        else selectOnly(item.id);
        return;
      }
      setDetailsIndex(index);
    });
  }

  function onTileDoubleClick(e: React.MouseEvent, item: FileItem) {
    cancel();
    if (e.metaKey || e.ctrlKey || e.shiftKey || fromInteractive(e)) return;
    // A leisurely double click (Windows allows up to half a second between the
    // two) can outlast the grace window, so the drawer may already be open.
    // Take it back rather than leaving both the drawer and the new tab.
    setDetailsIndex(null);
    openFileInNewTab(buildOpenUrl(userCode, item.code, item.ext));
  }

  function onGridKeyDown(e: React.KeyboardEvent<HTMLDivElement>) {
    const target = e.target as HTMLElement;
    if (!target.hasAttribute("data-file-tile")) return;

    const cols = columns();
    let next: number | null = null;
    switch (e.key) {
      case "ArrowRight":
        next = activeIndex + 1;
        break;
      case "ArrowLeft":
        next = activeIndex - 1;
        break;
      case "ArrowDown":
        next = activeIndex + cols;
        break;
      case "ArrowUp":
        next = activeIndex - cols;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = visible.length - 1;
        break;
      case "Enter":
        e.preventDefault();
        setDetailsIndex(activeIndex);
        return;
      case " ":
        e.preventDefault();
        if (visible[activeIndex]) toggle(visible[activeIndex].id);
        return;
      default:
        return;
    }

    e.preventDefault();
    const moved = focusIndex(next);
    if (e.shiftKey && visible[moved]) selectRange(visible[moved].id);
  }

  return (
    <>
      {/* Selection strip */}
      <div className="-mt-1 flex min-h-6 flex-wrap items-center justify-between gap-x-3 gap-y-2">
        <SelectionHints />
        <div className="ml-auto flex items-center gap-3">
          {selected.size > 0 ? (
            <button
              type="button"
              onClick={clearSelection}
              className="text-muted-foreground hover:text-foreground text-xs font-medium transition-colors"
            >
              {t("selection.clear")}
            </button>
          ) : null}
          <button
            type="button"
            onClick={toggleSelectAll}
            aria-pressed={allSelected}
            className={cn(
              "inline-flex items-center gap-1.5 text-xs font-medium transition-colors",
              allSelected || someSelected
                ? "text-foreground"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            {allSelected ? (
              <CheckSquare className="size-3.5" />
            ) : (
              <Square className="size-3.5" />
            )}
            {allSelected ? t("selection.deselectAll") : t("selection.selectAll")}
          </button>
        </div>
      </div>

      <div
        ref={gridRef}
        role="listbox"
        aria-multiselectable
        aria-label={t("selection.regionLabel")}
        onKeyDown={onGridKeyDown}
        className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5"
      >
        {visible.map((item, index) => {
          const isSelected = selected.has(item.id);
          const targets = targetsFor(item.id);
          // Conversion needs the rows, not just their ids: a mixed selection has
          // to be counted per target before anything is clicked.
          const targetItems = targets.length > 1 ? visible.filter((i) => selected.has(i.id)) : [item];
          const isDragging = draggingIds?.includes(item.id) ?? false;

          return (
            <FileContextMenu
              key={item.id}
              item={item}
              userCode={userCode}
              appUrl={appUrl}
              copyRaw={copyRaw}
              targetIds={targets}
              targetItems={targetItems}
              currentFolderId={currentFolderId}
              mode={mode}
              onOpenDetails={() => setDetailsIndex(index)}
              onTrashed={onTrashed}
              onTrashedMany={onBulkTrashed}
              onMoved={onMoved}
              onOpenChange={(open) => {
                // Right-clicking outside the selection narrows to that file, so
                // the menu never acts on something the user can't see.
                if (open && !isSelected && selected.size > 0) selectOnly(item.id);
              }}
            >
              <div
                ref={(el) => {
                  tileRefs.current[index] = el;
                }}
                data-file-tile
                role="option"
                aria-selected={isSelected}
                aria-label={item.filename}
                tabIndex={index === activeIndex ? 0 : -1}
                draggable
                onClick={(e) => onTileClick(e, item, index)}
                onDoubleClick={(e) => onTileDoubleClick(e, item)}
                onFocus={() => setActiveIndex(index)}
                onDragStart={(e) => {
                  const ids = isSelected && selected.size > 1 ? targets : [item.id];
                  const label =
                    ids.length > 1
                      ? t("selection.countSelected", { count: ids.length })
                      : item.filename;
                  setUploadDragPayload(e.dataTransfer, {
                    ids,
                    fromFolderId: item.folderId,
                    label,
                  });
                  if (ids.length > 1) setMultiDragImage(e, label);
                  setDraggingIds(ids);
                }}
                onDragEnd={() => setDraggingIds(null)}
                className={cn(
                  "group/tile focus-visible:ring-ring focus-visible:ring-offset-background relative cursor-pointer rounded-[calc(var(--radius)+0.25rem)] transition-[box-shadow,opacity] outline-none focus-visible:ring-2 focus-visible:ring-offset-2",
                  isSelected &&
                    "ring-primary ring-offset-background ring-2 ring-offset-2",
                  isDragging && "opacity-40",
                )}
              >
                <FileCard
                  item={item}
                  userCode={userCode}
                  appUrl={appUrl}
                  copyRaw={copyRaw}
                  selected={isSelected}
                  selecting={selected.size > 0}
                  onToggleSelect={() => toggle(item.id)}
                  onOpenDetails={() => setDetailsIndex(index)}
                  targetIds={targets}
                  targetItems={targetItems}
                  currentFolderId={currentFolderId}
                  mode={mode}
                  onTrashed={onTrashed}
                  onTrashedMany={onBulkTrashed}
                  onMoved={onMoved}
                />
              </div>
            </FileContextMenu>
          );
        })}
      </div>

      <DetailsDrawer
        items={visible}
        index={detailsIndex}
        onIndexChange={setDetailsIndex}
        onClose={() => setDetailsIndex(null)}
        userCode={userCode}
        appUrl={appUrl}
        copyRaw={copyRaw}
        currentFolderId={currentFolderId}
        mode={mode}
        onTrashed={onTrashed}
        onMoved={onMoved}
      />

      <BulkBar
        selected={selected}
        items={visible}
        userCode={userCode}
        appUrl={appUrl}
        currentFolderId={currentFolderId}
        mode={mode}
        onClear={clearSelection}
        onTrashed={onBulkTrashed}
        onMoved={onMoved}
      />
    </>
  );
}
