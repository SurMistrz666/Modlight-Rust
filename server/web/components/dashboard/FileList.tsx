"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { useLocale, useTranslations } from "next-intl";
import { Eye, Globe, Lock, Check, Folder, Inbox } from "lucide-react";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { humanFileSize, relativeTime } from "@/lib/format";
import { Thumbnail } from "./Thumbnail";
import { FileMenu, FileContextMenu } from "./FileMenu";
import { BulkBar } from "./BulkBar";
import { DetailsDrawer } from "./DetailsDrawer";
import {
  fromInteractive,
  openFileInNewTab,
  SelectionHints,
  useFileSelection,
  useSingleClickGuard,
} from "./FileGrid";
import { buildOpenUrl, buildRawUrl, extLabel, isImage, KindIcon, kindTint } from "./media";
import { setUploadDragPayload, type FileItem, type FileViewMode } from "./types";

export function FileList({
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
  // relativeTime formats through Intl, so it has to be told whose language
  // this is; without it a Polish reader gets English ages.
  const locale = useLocale();
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
  const rowRefs = useRef<(HTMLTableRowElement | null)[]>([]);
  const [cursor, setActiveIndex] = useState(0);
  const [detailsIndex, setDetailsIndex] = useState<number | null>(null);
  const [draggingIds, setDraggingIds] = useState<string[] | null>(null);

  const drawerOpen = detailsIndex != null;

  // Clamped on read: a shorter page must never leave the cursor out of range.
  const activeIndex = Math.min(cursor, Math.max(0, visible.length - 1));

  const focusIndex = useCallback(
    (i: number) => {
      const next = Math.max(0, Math.min(visible.length - 1, i));
      setActiveIndex(next);
      rowRefs.current[next]?.focus();
      return next;
    },
    [visible.length],
  );

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

  /** Same contract as the grid: one click → details, two → the file itself. */
  function onRowClick(e: React.MouseEvent, item: FileItem, index: number) {
    setActiveIndex(index);
    if (e.metaKey || e.ctrlKey) {
      cancel();
      toggle(item.id);
      return;
    }
    if (e.shiftKey) {
      cancel();
      window.getSelection()?.removeAllRanges();
      selectRange(item.id);
      return;
    }
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

  function onRowDoubleClick(e: React.MouseEvent, item: FileItem) {
    cancel();
    if (e.metaKey || e.ctrlKey || e.shiftKey || fromInteractive(e)) return;
    // A slow double click can outrun the grace window and leave the drawer
    // already open; take it back so only the new tab remains.
    setDetailsIndex(null);
    openFileInNewTab(buildOpenUrl(userCode, item.code, item.ext));
  }

  function onRowKeyDown(e: React.KeyboardEvent<HTMLTableRowElement>, index: number) {
    if (e.target !== e.currentTarget) return;
    let next: number | null = null;
    switch (e.key) {
      case "ArrowDown":
        next = index + 1;
        break;
      case "ArrowUp":
        next = index - 1;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = visible.length - 1;
        break;
      case "Enter":
        e.preventDefault();
        setDetailsIndex(index);
        return;
      case " ":
        e.preventDefault();
        toggle(visible[index].id);
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
      {/* Same gesture legend the grid shows, so the two views read alike. */}
      <SelectionHints className="-mt-1" />

      <div className="bg-card ring-foreground/10 overflow-hidden rounded-xl ring-1">
        <Table role="grid" aria-label={t("selection.regionLabel")}>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="w-10 pl-4">
                <Checkbox
                  checked={allSelected ? true : someSelected ? "indeterminate" : false}
                  onCheckedChange={toggleSelectAll}
                  aria-label={t("selection.selectAllFiles")}
                />
              </TableHead>
              <TableHead>{t("list.name")}</TableHead>
              <TableHead className="hidden md:table-cell">{t("list.folder")}</TableHead>
              <TableHead className="hidden sm:table-cell">{t("list.size")}</TableHead>
              <TableHead className="hidden lg:table-cell">{t("list.uploaded")}</TableHead>
              <TableHead className="hidden xl:table-cell">{t("list.views")}</TableHead>
              <TableHead>{t("list.visibility")}</TableHead>
              <TableHead className="w-10 pr-3 text-right">
                <span className="sr-only">{t("list.actions")}</span>
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {visible.map((item, index) => {
              const image = isImage(item.mimeType, item.ext);
              const rawUrl = buildRawUrl(appUrl, userCode, item.code, item.ext);
              const isSelected = selected.has(item.id);
              const targets = targetsFor(item.id);
              // Conversion reads the rows, not just their ids: a mixed selection
              // has to be counted per target before anything is clicked.
              const targetItems =
                targets.length > 1 ? visible.filter((i) => selected.has(i.id)) : [item];
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
                    if (open && !isSelected && selected.size > 0) selectOnly(item.id);
                  }}
                >
                  <TableRow
                    ref={(el) => {
                      rowRefs.current[index] = el;
                    }}
                    data-state={isSelected ? "selected" : undefined}
                    aria-selected={isSelected}
                    tabIndex={index === activeIndex ? 0 : -1}
                    draggable
                    onClick={(e) => onRowClick(e, item, index)}
                    onDoubleClick={(e) => onRowDoubleClick(e, item)}
                    onFocus={() => setActiveIndex(index)}
                    onKeyDown={(e) => onRowKeyDown(e, index)}
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
                      setDraggingIds(ids);
                    }}
                    onDragEnd={() => setDraggingIds(null)}
                    className={cn(
                      "group/row focus-visible:ring-ring cursor-pointer transition-colors outline-none focus-visible:ring-2 focus-visible:ring-inset",
                      isSelected && "bg-primary/5",
                      isDragging && "opacity-40",
                    )}
                  >
                    <TableCell className="pl-4" onClick={(e) => e.stopPropagation()}>
                      <Checkbox
                        checked={isSelected}
                        onCheckedChange={() => toggle(item.id)}
                        tabIndex={-1}
                        aria-label={t("selection.selectFile", { filename: item.filename })}
                      />
                    </TableCell>

                    <TableCell>
                      <div className="flex items-center gap-3">
                        <div
                          className={cn(
                            "relative size-10 shrink-0 overflow-hidden rounded-lg ring-1 transition-shadow",
                            isSelected ? "ring-primary/40" : "ring-foreground/10",
                          )}
                        >
                          {image ? (
                            <Thumbnail
                              src={`${rawUrl}?w=120&h=120`}
                              alt={item.filename}
                              blurhash={item.blurhash}
                              className="size-10"
                            />
                          ) : (
                            <div
                              className={cn(
                                "grid size-10 place-items-center",
                                kindTint(item.kind),
                              )}
                            >
                              <KindIcon kind={item.kind} mime={item.mimeType} className="size-4" />
                            </div>
                          )}
                          {isSelected ? (
                            <span className="bg-primary/70 text-primary-foreground absolute inset-0 grid place-items-center backdrop-blur-[1px]">
                              <Check className="size-4" />
                            </span>
                          ) : null}
                        </div>
                        <div className="min-w-0">
                          <span className="block max-w-[18rem] truncate font-medium">
                            {item.filename}
                          </span>
                          <span className="text-muted-foreground block font-mono text-xs">
                            {item.code}
                            <span className="ml-1.5 font-sans tracking-wide uppercase">
                              {extLabel(item)}
                            </span>
                          </span>
                        </div>
                      </div>
                    </TableCell>

                    <TableCell className="text-muted-foreground hidden md:table-cell">
                      <span className="flex max-w-[10rem] items-center gap-1.5">
                        {item.folderName ? (
                          <>
                            <Folder className="size-3.5 shrink-0" />
                            <span className="truncate">{item.folderName}</span>
                          </>
                        ) : (
                          <>
                            <Inbox className="size-3.5 shrink-0 opacity-60" />
                            <span className="opacity-60">{t("details.noFolder")}</span>
                          </>
                        )}
                      </span>
                    </TableCell>

                    <TableCell className="text-muted-foreground hidden tabular-nums sm:table-cell">
                      {humanFileSize(item.size)}
                    </TableCell>
                    <TableCell className="text-muted-foreground hidden lg:table-cell">
                      {relativeTime(item.createdAt, locale)}
                    </TableCell>
                    <TableCell className="text-muted-foreground hidden xl:table-cell">
                      <span className="inline-flex items-center gap-1 tabular-nums">
                        <Eye className="size-3.5" /> {item.views.toLocaleString()}
                      </span>
                    </TableCell>
                    <TableCell>
                      {item.published ? (
                        <Badge variant="secondary" className="gap-1">
                          <Globe className="size-3" /> {t("card.public")}
                        </Badge>
                      ) : (
                        <Badge variant="outline" className="text-muted-foreground gap-1">
                          <Lock className="size-3" /> {t("card.private")}
                        </Badge>
                      )}
                    </TableCell>
                    <TableCell
                      className="pr-3 text-right"
                      onClick={(e) => e.stopPropagation()}
                    >
                      <FileMenu
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
                      />
                    </TableCell>
                  </TableRow>
                </FileContextMenu>
              );
            })}
          </TableBody>
        </Table>
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
