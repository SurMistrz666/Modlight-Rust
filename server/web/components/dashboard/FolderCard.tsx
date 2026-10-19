"use client";

import { useRef, useState } from "react";
import { useTranslations } from "next-intl";
import { Folder, FolderOpen, Globe, Layers } from "lucide-react";
import { cn } from "@/lib/utils";
import { humanFileSize } from "@/lib/format";
import { hasUploadDrag, readUploadDragPayload } from "./types";

/** A folder tile as the browser needs it: totals already rolled up. */
export interface BrowserFolder {
  id: string;
  name: string;
  slug: string;
  isPublic: boolean;
  /** Files directly inside this folder. */
  directCount: number;
  /** Files in this folder and everything nested below it. */
  fileCount: number;
  /** Bytes in this folder and everything nested below it. */
  size: number;
  /** Folders nested directly below it. */
  subfolderCount: number;
  /** Folders nested below it at any depth — what a delete would take. */
  descendantCount: number;
}

/**
 * One tile in the folder explorer.
 *
 * Deliberately compact rather than thumbnail-dominant like FileCard: a folder
 * has no preview worth 4:3 of screen, and the point of this view is to see the
 * whole shelf at once. Icon, name, and the two numbers that actually decide
 * where you click — how many files, how much disk.
 */
export function FolderCard({
  folder,
  selected = false,
  onOpen,
  onSelect,
  onDropIds,
  menuSlot,
}: {
  folder: BrowserFolder;
  selected?: boolean;
  onOpen: () => void;
  onSelect: () => void;
  /** Files dropped onto this tile are moved into the folder. */
  onDropIds?: (ids: string[]) => void;
  /** The "…" trigger, supplied by the browser so both menus share one source. */
  menuSlot?: React.ReactNode;
}) {
  const t = useTranslations("dashboard.folderBrowser");
  const [dropping, setDropping] = useState(false);
  const dragDepth = useRef(0);
  const pointer = useRef<{ type: string; at: number }>({ type: "mouse", at: 0 });

  const empty = folder.fileCount === 0 && folder.subfolderCount === 0;

  return (
    <div
      data-selected={selected || undefined}
      onPointerDown={(e) => {
        pointer.current = { type: e.pointerType, at: Date.now() };
      }}
      // A finger has no double-click: on touch a single tap opens the folder,
      // which is what tapping a folder means everywhere else on a phone. With a
      // mouse the tile still selects on click and opens on double-click. A long
      // press is the context-menu gesture, so it must not count as a tap.
      onClick={() => {
        const { type, at } = pointer.current;
        const touch = type === "touch" || type === "pen";
        if (touch && Date.now() - at < 600) onOpen();
        else onSelect();
      }}
      onDoubleClick={onOpen}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          onOpen();
        }
      }}
      onDragEnter={(e) => {
        if (!hasUploadDrag(e.dataTransfer)) return;
        dragDepth.current += 1;
        setDropping(true);
      }}
      onDragLeave={() => {
        dragDepth.current = Math.max(0, dragDepth.current - 1);
        if (dragDepth.current === 0) setDropping(false);
      }}
      onDragOver={(e) => {
        if (!hasUploadDrag(e.dataTransfer)) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
      }}
      onDrop={(e) => {
        dragDepth.current = 0;
        setDropping(false);
        const payload = readUploadDragPayload(e.dataTransfer);
        if (!payload) return;
        e.preventDefault();
        e.stopPropagation();
        if (payload.fromFolderId === folder.id) return;
        onDropIds?.(payload.ids);
      }}
      className={cn(
        "group/folder bg-card ring-foreground/10 focus-visible:ring-ring focus-visible:ring-offset-background relative flex cursor-pointer touch-manipulation items-center gap-3 rounded-xl p-3 ring-1 transition-[box-shadow,transform,background-color] duration-200 outline-none select-none",
        "hover:ring-foreground/20 hover:shadow-foreground/5 hover:-translate-y-0.5 hover:shadow-lg",
        "focus-visible:ring-2 focus-visible:ring-offset-2",
        selected && "ring-primary bg-primary/[0.04] ring-2",
        dropping && "ring-primary bg-primary/10 ring-2",
      )}
    >
      {/* Icon tile — public folders get their own tint plus a globe badge, so
          "who can see this" is readable at a glance across a whole grid. */}
      <div
        className={cn(
          "relative grid size-11 shrink-0 place-items-center rounded-lg transition-transform group-hover/folder:scale-105",
          folder.isPublic
            ? "bg-primary/15 text-primary"
            : empty
              ? "bg-muted text-muted-foreground"
              : "bg-accent text-foreground",
        )}
      >
        {empty ? <FolderOpen className="size-5" /> : <Folder className="size-5" />}
        {folder.isPublic ? (
          <span
            title={t("public")}
            className="bg-primary text-primary-foreground ring-card absolute -right-1 -bottom-1 grid size-4 place-items-center rounded-full ring-2"
          >
            <Globe className="size-2.5" />
          </span>
        ) : null}
      </div>

      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <p className="truncate text-sm font-medium" title={folder.name}>
          {folder.name}
        </p>
        <p className="text-muted-foreground truncate text-xs tabular-nums">
          {folder.fileCount > 0
            ? `${t("fileCount", { count: folder.fileCount })} · ${humanFileSize(folder.size)}`
            : t("emptyFolder")}
        </p>
        {folder.subfolderCount > 0 ? (
          <p className="text-muted-foreground/80 mt-0.5 flex items-center gap-1 text-[0.6875rem]">
            <Layers className="size-3 shrink-0" />
            {t("subfolderCount", { count: folder.subfolderCount })}
          </p>
        ) : null}
      </div>

      {menuSlot ? (
        <div className="-mr-1 shrink-0 self-start" onClick={(e) => e.stopPropagation()}>
          {menuSlot}
        </div>
      ) : null}
    </div>
  );
}
