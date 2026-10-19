"use client";

import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useRef, useState, useTransition } from "react";
import { useTranslations } from "next-intl";
import { ContextMenu as ContextMenuPrimitive } from "radix-ui";
import {
  Check,
  ChevronRight,
  ChevronsDownUp,
  ChevronsUpDown,
  Copy,
  ExternalLink,
  Folder,
  FolderInput,
  FolderOpen,
  FolderPlus,
  Globe,
  Inbox,
  LayoutGrid,
  Loader2,
  MoreHorizontal,
  Pencil,
  Plus,
  Trash2,
  X,
} from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Label } from "@/components/ui/label";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  createFolder,
  deleteFolder,
  moveUnfiledToFolder,
  moveUploadsToFolder,
  renameFolder,
  setFolderVisibility,
} from "@/app/actions/folders";
import { DeleteFolderDialog, type DeleteFolderTarget } from "./DeleteFolderDialog";
import { hasUploadDrag, readUploadDragPayload } from "./types";

/* -------------------------------------------------------------------------- */
/* Right-click menus                                                           */
/* -------------------------------------------------------------------------- */

/*
 * A row's "…" button and its right-click menu have to be the same menu, so the
 * items are declared once and rendered through whichever primitive set is in
 * play (see `RowMenuParts` below) — the pattern components/dashboard/FileMenu
 * already uses for file tiles.
 *
 * The context-menu primitives live here rather than in components/ui because
 * the app shell needs the identical surface for its own rail rows, and this is
 * the file it already imports the tree from.
 */

function RowContextMenuContent({
  className,
  ...props
}: React.ComponentProps<typeof ContextMenuPrimitive.Content>) {
  return (
    <ContextMenuPrimitive.Portal>
      <ContextMenuPrimitive.Content
        data-slot="context-menu-content"
        className={cn(
          "bg-popover text-popover-foreground ring-foreground/10 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95 z-50 max-h-(--radix-context-menu-content-available-height) min-w-48 origin-(--radix-context-menu-content-transform-origin) overflow-x-hidden overflow-y-auto rounded-lg p-1 text-sm shadow-md ring-1 duration-100",
          className,
        )}
        {...props}
      />
    </ContextMenuPrimitive.Portal>
  );
}

function RowContextMenuItem({
  className,
  variant = "default",
  ...props
}: React.ComponentProps<typeof ContextMenuPrimitive.Item> & {
  variant?: "default" | "destructive";
}) {
  return (
    <ContextMenuPrimitive.Item
      data-slot="context-menu-item"
      data-variant={variant}
      className={cn(
        "focus:bg-accent focus:text-accent-foreground not-data-[variant=destructive]:focus:**:text-accent-foreground data-[variant=destructive]:text-destructive data-[variant=destructive]:focus:bg-destructive/10 data-[variant=destructive]:focus:text-destructive dark:data-[variant=destructive]:focus:bg-destructive/20 data-[variant=destructive]:*:[svg]:text-destructive relative flex cursor-default items-center gap-1.5 rounded-md px-1.5 py-1 text-sm outline-hidden select-none data-disabled:pointer-events-none data-disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
        className,
      )}
      {...props}
    />
  );
}

function RowContextMenuSeparator({
  className,
  ...props
}: React.ComponentProps<typeof ContextMenuPrimitive.Separator>) {
  return (
    <ContextMenuPrimitive.Separator
      className={cn("bg-foreground/10 -mx-1 my-1 h-px", className)}
      {...props}
    />
  );
}

/** The right-click surface for a navigation row, styled like the dropdown. */
export const RowMenu = {
  Root: ContextMenuPrimitive.Root,
  Trigger: ContextMenuPrimitive.Trigger,
  Content: RowContextMenuContent,
  Item: RowContextMenuItem,
  Separator: RowContextMenuSeparator,
};

/**
 * Open a row's context menu from the keyboard (Menu key, or Shift+F10).
 *
 * Radix listens for `contextmenu`, so the cheapest correct answer is to raise
 * that event ourselves at the focused row — the menu then opens, positions and
 * traps focus exactly as it does for the mouse. Text inputs are left alone:
 * inside a rename field the browser's own menu is the one with "Paste" in it.
 */
export function openRowMenuOnKey(e: React.KeyboardEvent<HTMLElement>) {
  if (e.key !== "ContextMenu" && !(e.key === "F10" && e.shiftKey)) return;

  const target = e.target as HTMLElement | null;
  if (target?.closest("input, textarea, [contenteditable='true']")) return;

  e.preventDefault();
  e.stopPropagation();

  const host = e.currentTarget;
  const anchor = target && host.contains(target) ? target : host;
  const rect = anchor.getBoundingClientRect();
  host.dispatchEvent(
    new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      // Left-ish of the row so the menu opens next to what is focused rather
      // than off in the middle of a wide sidebar.
      clientX: Math.round(rect.left + Math.min(rect.width / 2, 96)),
      clientY: Math.round(rect.top + rect.height / 2),
    }),
  );
}

interface RowMenuItemProps {
  children: React.ReactNode;
  onSelect?: () => void;
  variant?: "default" | "destructive";
  disabled?: boolean;
  asChild?: boolean;
  className?: string;
}

/** Lets one item list render as a dropdown or as a right-click menu. */
export interface RowMenuParts {
  Item: React.ComponentType<RowMenuItemProps>;
  Separator: React.ComponentType;
}

export const ROW_DROPDOWN_PARTS: RowMenuParts = {
  Item: (p) => <DropdownMenuItem {...p} />,
  Separator: () => <DropdownMenuSeparator />,
};

export const ROW_CONTEXT_PARTS: RowMenuParts = {
  Item: (p) => <RowContextMenuItem {...p} />,
  Separator: () => <RowContextMenuSeparator />,
};

/**
 * Dialogs opened from a menu item must outlive the menu (which unmounts on
 * select). Opening them on the next tick also avoids Radix's close animation
 * leaving `pointer-events: none` behind on the body.
 */
function defer(fn: () => void) {
  setTimeout(fn, 0);
}

/* -------------------------------------------------------------------------- */
/* URL helpers                                                                 */
/* -------------------------------------------------------------------------- */

/**
 * Params that survive a navigation between library views. Filters and paging
 * deliberately do not: opening a folder while a "> 100 MB, private, page 47"
 * filter is still armed looks like an empty folder.
 */
const STICKY_PARAMS = ["view", "sort", "per"] as const;

/**
 * Build a link into the flat file list at /dashboard.
 *
 * Note the `all` patch key used by callers that mean "the whole list": a bare
 * /dashboard redirects to the folder explorer for accounts that have folders,
 * so an unqualified link would bounce. The dashboard itself ignores the value.
 */

export function buildDashboardHref(
  params: URLSearchParams | ReadonlyURLSearchParamsLike,
  patch: Record<string, string | null | undefined>,
): string {
  const next = new URLSearchParams();
  for (const key of STICKY_PARAMS) {
    const value = params.get(key);
    if (value) next.set(key, value);
  }
  for (const [key, value] of Object.entries(patch)) {
    if (value) next.set(key, value);
  }
  const qs = next.toString();
  return qs ? `/dashboard?${qs}` : "/dashboard";
}

/** Structural stand-in for Next's ReadonlyURLSearchParams. */
interface ReadonlyURLSearchParamsLike {
  get(name: string): string | null;
}

/* -------------------------------------------------------------------------- */
/* Types                                                                       */
/* -------------------------------------------------------------------------- */

export interface FolderTreeNode {
  id: string;
  parentId: string | null;
  name: string;
  slug: string;
  isPublic: boolean;
  /** Files directly inside this folder. */
  count: number;
  /** Files in this folder and everything nested below it. */
  totalCount: number;
  children: FolderTreeNode[];
}

interface FolderTreeProps {
  folders: FolderTreeNode[];
  /** Folder id from the URL, or "none" when the "Unsorted" view is open. */
  activeFolderId?: string | null;
  /** Files that sit in no folder at all. */
  unsortedCount?: number;
  userCode?: string | null;
  onNavigate?: () => void;
  /**
   * Keeps DOM ids unique when the rail and the mobile drawer are both mounted
   * (the rail is only `display: none` below `lg`, not unmounted).
   */
  scope?: string;
}

const OPEN_STORAGE_KEY = "ml_folders_open";
/** Whether the whole "Folders" rail section is expanded. */
const SECTION_STORAGE_KEY = "ml_folders_section";

/* -------------------------------------------------------------------------- */
/* Tree helpers                                                                */
/* -------------------------------------------------------------------------- */

/** Ids of every node that actually has something to expand. */
function expandableIds(nodes: FolderTreeNode[], out: string[] = []): string[] {
  for (const n of nodes) {
    if (n.children.length > 0) {
      out.push(n.id);
      expandableIds(n.children, out);
    }
  }
  return out;
}

/** Folders nested under `node`, at any depth. */
function countDescendants(node: FolderTreeNode): number {
  return node.children.reduce((sum, child) => sum + 1 + countDescendants(child), 0);
}

/* -------------------------------------------------------------------------- */
/* Shared item lists                                                           */
/* -------------------------------------------------------------------------- */

/** What the "Folders" heading offers, from its "…" button or a right-click. */
function SectionMenuItems({
  parts,
  sectionOpen,
  toggleSection,
  onNewFolder,
  onNavigate,
  canExpandAll,
  canCollapseAll,
  setAllOpen,
  unsortedCount,
  onTidy,
}: {
  parts: RowMenuParts;
  sectionOpen: boolean;
  toggleSection: () => void;
  onNewFolder: () => void;
  onNavigate?: () => void;
  canExpandAll: boolean;
  canCollapseAll: boolean;
  setAllOpen: (expand: boolean) => void;
  unsortedCount: number;
  onTidy: () => void;
}) {
  const t = useTranslations("dashboard.folders");
  const { Item, Separator } = parts;

  return (
    <>
      <Item asChild>
        <Link href="/dashboard/folders" onClick={onNavigate}>
          <LayoutGrid /> {t("browseAll")}
        </Link>
      </Item>
      <Item asChild>
        <a href="/dashboard/folders" target="_blank" rel="noopener noreferrer">
          <ExternalLink /> {t("openInNewTab")}
        </a>
      </Item>
      <Separator />
      <Item onSelect={onNewFolder}>
        <FolderPlus /> {t("newFolder")}
      </Item>
      <Item onSelect={toggleSection}>
        {sectionOpen ? <ChevronsDownUp /> : <ChevronsUpDown />}
        {sectionOpen ? t("collapseSection") : t("expandSection")}
      </Item>
      {/* Only meaningful once some folder actually has children — a flat
          library leaves these greyed out, which is why the section toggle
          above exists as the always-useful control. */}
      <Item onSelect={() => setAllOpen(true)} disabled={!canExpandAll}>
        <ChevronsUpDown /> {t("expandAll")}
      </Item>
      <Item onSelect={() => setAllOpen(false)} disabled={!canCollapseAll}>
        <ChevronsDownUp /> {t("collapseAll")}
      </Item>
      <Separator />
      <Item onSelect={() => defer(onTidy)} disabled={unsortedCount === 0}>
        <FolderInput /> {t("tidy.action")}
      </Item>
    </>
  );
}

/** What one folder offers, from its "…" button or a right-click on the row. */
function FolderMenuItems({
  parts,
  node,
  href,
  onNavigate,
  isOpen,
  toggleOpen,
  setCreatingIn,
  setRenaming,
  onShare,
  onDelete,
}: {
  parts: RowMenuParts;
  node: FolderTreeNode;
  href: string;
  onNavigate?: () => void;
  isOpen: boolean;
  toggleOpen: (id: string) => void;
  setCreatingIn: (id: string | null | undefined) => void;
  setRenaming: (id: string | null) => void;
  onShare: (node: FolderTreeNode) => void;
  onDelete: (node: FolderTreeNode) => void;
}) {
  const t = useTranslations("dashboard.folders");
  const { Item, Separator } = parts;

  return (
    <>
      {/* The two the browser's own menu would have offered on this link. */}
      <Item asChild>
        <Link href={href} onClick={onNavigate}>
          <FolderOpen /> {t("open")}
        </Link>
      </Item>
      <Item asChild>
        <a href={href} target="_blank" rel="noopener noreferrer">
          <ExternalLink /> {t("openInNewTab")}
        </a>
      </Item>
      <Separator />
      <Item
        onSelect={() => {
          if (!isOpen) toggleOpen(node.id);
          setCreatingIn(node.id);
        }}
      >
        <FolderPlus /> {t("newSubfolder")}
      </Item>
      <Item onSelect={() => setRenaming(node.id)}>
        <Pencil /> {t("rename")}
      </Item>
      <Item onSelect={() => defer(() => onShare(node))}>
        <Globe /> {t("share")}
      </Item>
      <Separator />
      <Item variant="destructive" onSelect={() => defer(() => onDelete(node))}>
        <Trash2 /> {t("delete")}
      </Item>
    </>
  );
}

/* -------------------------------------------------------------------------- */
/* Tree                                                                        */
/* -------------------------------------------------------------------------- */

export function FolderTree({
  folders,
  activeFolderId,
  unsortedCount = 0,
  userCode,
  onNavigate,
  scope = "rail",
}: FolderTreeProps) {
  const t = useTranslations("dashboard.folders");
  const sectionId = `folder-tree-section-${scope}`;
  const router = useRouter();
  const params = useSearchParams();
  const [pending, startTransition] = useTransition();

  const [open, setOpen] = useState<Set<string>>(new Set());
  // Whole-section collapse. Starts expanded so the server and the first client
  // paint agree; the stored preference is applied in an effect below.
  const [sectionOpen, setSectionOpen] = useState(true);
  const [creatingIn, setCreatingIn] = useState<string | null | undefined>(undefined);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<DeleteFolderTarget | null>(null);
  const [sharing, setSharing] = useState<FolderTreeNode | null>(null);
  const [tidyOpen, setTidyOpen] = useState(false);

  // Parent lookup drives "expand every ancestor of the folder I just opened".
  const parentOf = useMemo(() => {
    const map = new Map<string, string | null>();
    const walk = (nodes: FolderTreeNode[]) => {
      for (const n of nodes) {
        map.set(n.id, n.parentId);
        walk(n.children);
      }
    };
    walk(folders);
    return map;
  }, [folders]);

  // Restore the user's expanded branches, then force the active path open.
  useEffect(() => {
    let restored: string[] = [];
    try {
      const raw = window.localStorage.getItem(OPEN_STORAGE_KEY);
      if (raw) restored = JSON.parse(raw) as string[];
    } catch {
      restored = [];
    }
    setOpen((prev) => {
      const next = new Set([...prev, ...restored.filter((id) => typeof id === "string")]);
      let cursor = activeFolderId && activeFolderId !== "none" ? parentOf.get(activeFolderId) : null;
      const guard = new Set<string>();
      while (cursor && !guard.has(cursor)) {
        guard.add(cursor);
        next.add(cursor);
        cursor = parentOf.get(cursor) ?? null;
      }
      return next;
    });
  }, [activeFolderId, parentOf]);

  // Restore the section's own collapsed state (independent of which branches
  // are expanded inside it).
  useEffect(() => {
    try {
      const raw = window.localStorage.getItem(SECTION_STORAGE_KEY);
      if (raw === "collapsed") setSectionOpen(false);
    } catch {
      // Private mode — the section just stays expanded.
    }
  }, []);

  const toggleSection = useCallback(() => {
    setSectionOpen((prev) => {
      const next = !prev;
      try {
        window.localStorage.setItem(SECTION_STORAGE_KEY, next ? "expanded" : "collapsed");
      } catch {
        // Ignore — collapsing still works for this session.
      }
      return next;
    });
  }, []);

  const persist = useCallback((next: Set<string>) => {
    try {
      window.localStorage.setItem(OPEN_STORAGE_KEY, JSON.stringify([...next]));
    } catch {
      // Private mode / storage full — expansion just won't persist.
    }
  }, []);

  const toggleOpen = useCallback(
    (id: string) => {
      setOpen((prev) => {
        const next = new Set(prev);
        if (next.has(id)) next.delete(id);
        else next.add(id);
        persist(next);
        return next;
      });
    },
    [persist],
  );

  const branches = useMemo(() => expandableIds(folders), [folders]);

  const setAllOpen = useCallback(
    (expand: boolean) => {
      const next = expand ? new Set(branches) : new Set<string>();
      setOpen(next);
      persist(next);
    },
    [branches, persist],
  );

  const run = useCallback(
    (work: () => Promise<unknown>, success?: string) => {
      startTransition(async () => {
        try {
          await work();
          if (success) toast.success(success);
          router.refresh();
        } catch (err) {
          toast.error(err instanceof Error ? err.message : t("error"));
        }
      });
    },
    [router, t],
  );

  const handleDrop = useCallback(
    (ids: string[], folderId: string | null, folderName: string) => {
      if (ids.length === 0) return;
      run(async () => {
        const res = await moveUploadsToFolder(ids, folderId);
        if (res.moved > 0) {
          toast.success(t("moved", { count: res.moved, name: folderName }));
        }
      });
    },
    [run, t],
  );

  const total = folders.reduce((sum, f) => sum + f.totalCount, 0);
  const allExpanded = branches.length > 0 && branches.every((id) => open.has(id));

  const folderCount = folders.reduce((sum, f) => sum + 1 + countDescendants(f), 0);

  const newFolderAtRoot = useCallback(() => {
    setSectionOpen(true);
    try {
      window.localStorage.setItem(SECTION_STORAGE_KEY, "expanded");
    } catch {
      // Ignore — the section still opens for this session.
    }
    setCreatingIn(null);
  }, []);

  /** One description of the section menu, rendered by both surfaces. */
  const sectionMenu = {
    sectionOpen,
    toggleSection,
    onNewFolder: newFolderAtRoot,
    onNavigate,
    canExpandAll: branches.length > 0 && !allExpanded,
    canCollapseAll: open.size > 0,
    setAllOpen,
    unsortedCount,
    onTidy: () => setTidyOpen(true),
  };

  return (
    <div className="flex min-h-0 flex-col gap-1">
      {/* The section header is the collapse control. Clicking the word
          "Folders" folds the whole rail away — the gesture every file manager
          and IDE already trained people to expect. Right-clicking it opens the
          same menu as the "…" button next to it. */}
      <RowMenu.Root>
        <RowMenu.Trigger asChild>
          <div className="flex items-center gap-1 pr-2 pl-1" onKeyDown={openRowMenuOnKey}>
            <button
              type="button"
              onClick={toggleSection}
              aria-expanded={sectionOpen}
              aria-controls={sectionId}
              title={sectionOpen ? t("collapseSection") : t("expandSection")}
              className="text-muted-foreground/70 hover:text-foreground focus-visible:ring-ring group/section flex min-w-0 flex-1 items-center gap-1 rounded px-2 py-1 text-left transition-colors outline-none focus-visible:ring-2 max-lg:min-h-10"
            >
              <ChevronRight
                aria-hidden
                className={cn(
                  "size-3.5 shrink-0 transition-transform duration-200",
                  sectionOpen && "rotate-90",
                )}
              />
              <span className="truncate text-[0.6875rem] font-semibold tracking-[0.08em] uppercase">
                {t("title")}
              </span>
              {!sectionOpen && folderCount > 0 ? (
                <span className="text-muted-foreground/60 shrink-0 text-[0.6875rem] font-medium tabular-nums">
                  {folderCount.toLocaleString()}
                </span>
              ) : null}
            </button>

            <div className="flex shrink-0 items-center gap-1">
              {pending ? <Loader2 className="text-muted-foreground size-3 animate-spin" /> : null}
              <button
                type="button"
                onClick={newFolderAtRoot}
                aria-label={t("newFolder")}
                title={t("newFolder")}
                className="text-muted-foreground hover:bg-accent hover:text-foreground grid size-5 place-items-center rounded transition-colors max-lg:size-9"
              >
                <Plus className="size-3.5" />
              </button>

              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <button
                    type="button"
                    aria-label={t("sectionActions")}
                    title={t("sectionActions")}
                    className="text-muted-foreground hover:bg-accent hover:text-foreground data-[state=open]:bg-accent data-[state=open]:text-foreground grid size-5 place-items-center rounded transition-colors max-lg:size-9"
                  >
                    <MoreHorizontal className="size-3.5" />
                  </button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end" className="w-60">
                  <SectionMenuItems parts={ROW_DROPDOWN_PARTS} {...sectionMenu} />
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
          </div>
        </RowMenu.Trigger>
        <RowMenu.Content className="w-60">
          <SectionMenuItems parts={ROW_CONTEXT_PARTS} {...sectionMenu} />
        </RowMenu.Content>
      </RowMenu.Root>

      {/* Capped so a library with 40 folders can't push the rest of the nav
          (Create / Account / storage meter) off the screen. */}
      <div
        id={sectionId}
        // `hidden` (the utility, not the attribute) so Tailwind's display class
        // wins the cascade against the `flex` on the same element.
        className={cn(
          // Capped on the desktop rail so a 40-folder tree can't push Create /
          // Account away; in the mobile drawer the drawer itself scrolls, and a
          // scroll area inside a scroll area is miserable on a touchscreen.
          "flex max-h-[min(45vh,22rem)] flex-col gap-0.5 overflow-x-hidden overflow-y-auto overscroll-contain px-2 max-lg:max-h-none",
          !sectionOpen && "hidden",
        )}
      >
        {folders.map((node) => (
          <FolderRow
            key={node.id}
            node={node}
            open={open}
            toggleOpen={toggleOpen}
            activeFolderId={activeFolderId}
            params={params}
            onNavigate={onNavigate}
            renaming={renaming}
            setRenaming={setRenaming}
            creatingIn={creatingIn}
            setCreatingIn={setCreatingIn}
            onDropIds={handleDrop}
            onRename={(id, name) => run(() => renameFolder(id, name), t("renamed"))}
            onCreate={(name, parentId) => run(() => createFolder(name, parentId), t("created"))}
            onDelete={(target) =>
              setConfirmDelete({
                id: target.id,
                name: target.name,
                fileCount: target.totalCount,
                subfolderCount: countDescendants(target),
              })
            }
            onShare={setSharing}
          />
        ))}

        {creatingIn === null ? (
          <NameInput
            placeholder={t("namePlaceholder")}
            onCancel={() => setCreatingIn(undefined)}
            onSubmit={(name) => {
              setCreatingIn(undefined);
              run(() => createFolder(name, null), t("created"));
            }}
          />
        ) : null}

        {folders.length === 0 && creatingIn === undefined ? (
          <button
            type="button"
            onClick={() => setCreatingIn(null)}
            className="text-muted-foreground hover:border-foreground/25 hover:text-foreground border-foreground/15 mx-1 mt-1 flex items-center gap-2 rounded-lg border border-dashed px-3 py-2.5 text-left text-xs transition-colors"
          >
            <FolderPlus className="size-4 shrink-0" />
            <span className="leading-snug">{t("emptyHint")}</span>
          </button>
        ) : null}

        {/* Loose files live at the bottom: they're the fallback, not a folder. */}
        {unsortedCount > 0 || total > 0 ? (
          <UnsortedRow
            count={unsortedCount}
            active={activeFolderId === "none"}
            params={params}
            onNavigate={onNavigate}
            onDropIds={(ids) => handleDrop(ids, null, t("unsorted"))}
            onTidy={() => setTidyOpen(true)}
          />
        ) : null}
      </div>

      <DeleteFolderDialog
        folder={confirmDelete}
        onOpenChange={(v) => !v && setConfirmDelete(null)}
        onConfirm={(id, withContents) => {
          setConfirmDelete(null);
          run(async () => {
            const res = await deleteFolder(id, { withContents });
            if (res.trashed > 0) {
              toast.success(t("deletedWithContents", { count: res.trashed }));
            } else {
              toast.success(t("deleted"));
            }
          });
        }}
      />

      <TidyUnfiledDialog
        open={tidyOpen}
        onOpenChange={setTidyOpen}
        count={unsortedCount}
        onConfirm={(name) => {
          setTidyOpen(false);
          run(async () => {
            const res = await moveUnfiledToFolder(name);
            toast.success(
              res.moved > 0
                ? t("tidy.done", { count: res.moved, name: res.folderName })
                : t("tidy.nothing"),
            );
          });
        }}
      />

      <ShareFolderDialog
        folder={sharing}
        userCode={userCode}
        onOpenChange={(v) => !v && setSharing(null)}
        onToggle={(id, isPublic) =>
          run(
            () => setFolderVisibility(id, isPublic),
            isPublic ? t("madePublic") : t("madePrivate"),
          )
        }
      />
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Rows                                                                        */
/* -------------------------------------------------------------------------- */

interface RowProps {
  node: FolderTreeNode;
  open: Set<string>;
  toggleOpen: (id: string) => void;
  activeFolderId?: string | null;
  params: ReadonlyURLSearchParamsLike;
  onNavigate?: () => void;
  renaming: string | null;
  setRenaming: (id: string | null) => void;
  creatingIn: string | null | undefined;
  setCreatingIn: (id: string | null | undefined) => void;
  onDropIds: (ids: string[], folderId: string | null, folderName: string) => void;
  onRename: (id: string, name: string) => void;
  onCreate: (name: string, parentId: string | null) => void;
  onDelete: (node: FolderTreeNode) => void;
  onShare: (node: FolderTreeNode) => void;
}

function FolderRow(props: RowProps) {
  const {
    node,
    open,
    toggleOpen,
    activeFolderId,
    params,
    onNavigate,
    renaming,
    setRenaming,
    creatingIn,
    setCreatingIn,
    onDropIds,
    onRename,
    onCreate,
    onDelete,
    onShare,
  } = props;

  const t = useTranslations("dashboard.folders");
  const [dropping, setDropping] = useState(false);
  const dragDepth = useRef(0);

  const isOpen = open.has(node.id);
  const isActive = activeFolderId === node.id;
  const hasChildren = node.children.length > 0;
  const isRenaming = renaming === node.id;
  const href = buildDashboardHref(params, { folder: node.id });

  /** One description of this folder's actions, rendered by both surfaces. */
  const folderMenu = {
    node,
    href,
    onNavigate,
    isOpen,
    toggleOpen,
    setCreatingIn,
    setRenaming,
    onShare,
    onDelete,
  };

  function onDragEnter(e: React.DragEvent) {
    if (!hasUploadDrag(e.dataTransfer)) return;
    dragDepth.current += 1;
    setDropping(true);
  }

  function onDragLeave() {
    dragDepth.current = Math.max(0, dragDepth.current - 1);
    if (dragDepth.current === 0) setDropping(false);
  }

  function onDragOver(e: React.DragEvent) {
    if (!hasUploadDrag(e.dataTransfer)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
  }

  function onDrop(e: React.DragEvent) {
    dragDepth.current = 0;
    setDropping(false);
    const payload = readUploadDragPayload(e.dataTransfer);
    if (!payload) return;
    e.preventDefault();
    e.stopPropagation();
    // Dropping onto the folder the drag started from is a no-op, not a move.
    if (payload.fromFolderId === node.id) return;
    onDropIds(payload.ids, node.id, node.name);
  }

  return (
    <div>
      {/* Right-click anywhere on the row opens the same menu as its "…"
          button. Disabled while renaming, so the browser's own menu (the one
          with "Paste" in it) stays available inside the text field. */}
      <RowMenu.Root>
        <RowMenu.Trigger asChild disabled={isRenaming}>
          <div
            onDragEnter={onDragEnter}
            onDragLeave={onDragLeave}
            onDragOver={onDragOver}
            onDrop={onDrop}
            onKeyDown={openRowMenuOnKey}
            data-active={isActive || undefined}
            className={cn(
              "group/row relative flex items-center rounded-lg pr-1 transition-colors",
              isActive ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent/60",
              dropping && "ring-primary bg-primary/10 text-foreground ring-2",
            )}
          >
            {/* active accent rail — same language as the main nav */}
            <span
              aria-hidden
              className={cn(
                "bg-primary absolute top-1/2 left-0 h-4 w-0.5 -translate-y-1/2 rounded-full transition-opacity",
                isActive ? "opacity-100" : "opacity-0",
              )}
            />

            {hasChildren ? (
              <button
                type="button"
                onClick={() => toggleOpen(node.id)}
                aria-expanded={isOpen}
                aria-label={isOpen ? t("collapse") : t("expand")}
                className="hover:text-foreground grid size-5 shrink-0 place-items-center rounded transition-colors max-lg:size-9"
              >
                <ChevronRight
                  className={cn("size-3.5 transition-transform", isOpen && "rotate-90")}
                />
              </button>
            ) : (
              <span aria-hidden className="size-5 shrink-0" />
            )}

            {isRenaming ? (
              <NameInput
                inline
                defaultValue={node.name}
                placeholder={t("namePlaceholder")}
                onCancel={() => setRenaming(null)}
                onSubmit={(name) => {
                  setRenaming(null);
                  if (name !== node.name) onRename(node.id, name);
                }}
              />
            ) : (
              <>
                <Link
                  href={href}
                  onClick={onNavigate}
                  aria-current={isActive ? "page" : undefined}
                  className={cn(
                    "flex min-w-0 flex-1 items-center gap-2 py-1.5 text-sm font-medium max-lg:min-h-11",
                    !isActive && "group-hover/row:text-foreground",
                  )}
                >
                  {isActive || (isOpen && hasChildren) ? (
                    <FolderOpen className={cn("size-4 shrink-0", isActive && "text-primary")} />
                  ) : (
                    <Folder className="size-4 shrink-0" />
                  )}
                  <span className="truncate">{node.name}</span>
                  {node.isPublic ? (
                    <Globe
                      className="text-primary/70 size-3 shrink-0"
                      aria-label={t("publicBadge")}
                    />
                  ) : null}
                </Link>

                <span className="text-muted-foreground/70 shrink-0 px-1 text-[0.6875rem] font-medium tabular-nums opacity-100 transition-opacity lg:group-hover/row:opacity-0">
                  {node.totalCount > 0 ? node.totalCount.toLocaleString() : ""}
                </span>

                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <button
                      type="button"
                      aria-label={t("actions", { name: node.name })}
                      // Touch has no hover: below `lg` the trigger sits in the row
                      // permanently instead of appearing on pointer-over.
                      className="text-muted-foreground hover:text-foreground data-[state=open]:text-foreground grid size-6 shrink-0 place-items-center rounded transition-opacity max-lg:size-9 lg:absolute lg:right-1 lg:opacity-0 lg:group-hover/row:opacity-100 lg:focus-visible:opacity-100 lg:data-[state=open]:opacity-100"
                    >
                      <MoreHorizontal className="size-3.5" />
                    </button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="start" className="w-56">
                    <FolderMenuItems parts={ROW_DROPDOWN_PARTS} {...folderMenu} />
                  </DropdownMenuContent>
                </DropdownMenu>
              </>
            )}
          </div>
        </RowMenu.Trigger>
        <RowMenu.Content className="w-56">
          <FolderMenuItems parts={ROW_CONTEXT_PARTS} {...folderMenu} />
        </RowMenu.Content>
      </RowMenu.Root>

      {(hasChildren && isOpen) || creatingIn === node.id ? (
        <div className="border-foreground/10 relative ml-[0.6875rem] flex flex-col gap-0.5 border-l pl-[0.3125rem]">
          {isOpen
            ? node.children.map((child) => (
                <FolderRow key={child.id} {...props} node={child} />
              ))
            : null}
          {creatingIn === node.id ? (
            <NameInput
              placeholder={t("namePlaceholder")}
              onCancel={() => setCreatingIn(undefined)}
              onSubmit={(name) => {
                setCreatingIn(undefined);
                onCreate(name, node.id);
              }}
            />
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function UnsortedRow({
  count,
  active,
  params,
  onNavigate,
  onDropIds,
  onTidy,
}: {
  count: number;
  active: boolean;
  params: ReadonlyURLSearchParamsLike;
  onNavigate?: () => void;
  onDropIds: (ids: string[]) => void;
  /** Sweep every loose file into one folder — the pile's only bulk action. */
  onTidy: () => void;
}) {
  const t = useTranslations("dashboard.folders");
  const [dropping, setDropping] = useState(false);
  const dragDepth = useRef(0);

  const href = buildDashboardHref(params, { folder: "none" });

  return (
    <RowMenu.Root>
      <RowMenu.Trigger asChild>
        <Link
          href={href}
          onClick={onNavigate}
          onKeyDown={openRowMenuOnKey}
          aria-current={active ? "page" : undefined}
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
            if (payload.fromFolderId === null) return;
            onDropIds(payload.ids);
          }}
          className={cn(
            "relative mt-0.5 flex items-center gap-2 rounded-lg py-1.5 pr-2 pl-5 text-sm font-medium transition-colors max-lg:min-h-11",
            active ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
            dropping && "ring-primary bg-primary/10 text-foreground ring-2",
          )}
        >
          <span
            aria-hidden
            className={cn(
              "bg-primary absolute top-1/2 left-0 h-4 w-0.5 -translate-y-1/2 rounded-full transition-opacity",
              active ? "opacity-100" : "opacity-0",
            )}
          />
          <Inbox className={cn("size-4 shrink-0", active && "text-primary")} />
          <span className="flex-1 truncate">{t("unsorted")}</span>
          {count > 0 ? (
            <span className="text-muted-foreground/70 text-[0.6875rem] font-medium tabular-nums">
              {count.toLocaleString()}
            </span>
          ) : null}
        </Link>
      </RowMenu.Trigger>
      <RowMenu.Content className="w-56">
        <RowMenu.Item asChild>
          <Link href={href} onClick={onNavigate}>
            <Inbox /> {t("open")}
          </Link>
        </RowMenu.Item>
        <RowMenu.Item asChild>
          <a href={href} target="_blank" rel="noopener noreferrer">
            <ExternalLink /> {t("openInNewTab")}
          </a>
        </RowMenu.Item>
        <RowMenu.Separator />
        <RowMenu.Item disabled={count === 0} onSelect={() => defer(onTidy)}>
          <FolderInput /> {t("tidy.action")}
        </RowMenu.Item>
      </RowMenu.Content>
    </RowMenu.Root>
  );
}

/* -------------------------------------------------------------------------- */
/* Inline name editor                                                          */
/* -------------------------------------------------------------------------- */

function NameInput({
  defaultValue = "",
  placeholder,
  onSubmit,
  onCancel,
  inline = false,
}: {
  defaultValue?: string;
  placeholder: string;
  onSubmit: (name: string) => void;
  onCancel: () => void;
  inline?: boolean;
}) {
  const [value, setValue] = useState(defaultValue);
  const ref = useRef<HTMLInputElement>(null);

  useEffect(() => {
    ref.current?.focus();
    ref.current?.select();
  }, []);

  function commit() {
    const clean = value.trim();
    if (!clean) {
      onCancel();
      return;
    }
    onSubmit(clean);
  }

  return (
    <div className={cn("flex items-center gap-1 py-0.5", inline ? "min-w-0 flex-1 pr-1" : "px-1")}>
      <Input
        ref={ref}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            commit();
          } else if (e.key === "Escape") {
            e.preventDefault();
            onCancel();
          }
        }}
        // Blur-to-commit keeps the interaction cheap: click away = save.
        onBlur={(e) => {
          if (e.relatedTarget instanceof HTMLElement && e.relatedTarget.dataset.keepFocus) return;
          commit();
        }}
        placeholder={placeholder}
        maxLength={120}
        className="h-7 flex-1 px-2 text-sm"
      />
      <button
        type="button"
        data-keep-focus="1"
        onMouseDown={(e) => e.preventDefault()}
        onClick={commit}
        className="text-muted-foreground hover:text-foreground grid size-6 shrink-0 place-items-center rounded"
      >
        <Check className="size-3.5" />
      </button>
      <button
        type="button"
        data-keep-focus="1"
        onMouseDown={(e) => e.preventDefault()}
        onClick={onCancel}
        className="text-muted-foreground hover:text-foreground grid size-6 shrink-0 place-items-center rounded"
      >
        <X className="size-3.5" />
      </button>
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Dialogs                                                                     */
/* -------------------------------------------------------------------------- */

/**
 * "Tidy up" for loose files.
 *
 * An imported library arrives as thousands of files in no folder at all.
 * Sorting
 * those by hand is not a thing anyone will do, so this sweeps the whole pile
 * into one folder in a single UPDATE.
 */
function TidyUnfiledDialog({
  open,
  onOpenChange,
  count,
  onConfirm,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  count: number;
  onConfirm: (folderName: string) => void;
}) {
  const t = useTranslations("dashboard.folders");
  const fallback = t("tidy.defaultName");
  const [name, setName] = useState(fallback);

  useEffect(() => {
    if (open) setName(fallback);
  }, [open, fallback]);

  const clean = name.trim();

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("tidy.title")}</DialogTitle>
          <DialogDescription>
            {t.rich("tidy.description", {
              count,
              strong: (chunks) => <strong className="text-foreground">{chunks}</strong>,
            })}
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-1.5">
          <Label htmlFor="tidy-folder-name" className="text-xs">
            {t("tidy.nameLabel")}
          </Label>
          <Input
            id="tidy-folder-name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && clean) {
                e.preventDefault();
                onConfirm(clean);
              }
            }}
            maxLength={120}
            autoFocus
          />
          <p className="text-muted-foreground text-xs">{t("tidy.nameHint")}</p>
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {t("cancel")}
          </Button>
          <Button disabled={!clean || count === 0} onClick={() => onConfirm(clean)}>
            <FolderInput className="size-4" /> {t("tidy.confirm", { count })}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** The bits of a folder the share dialog needs — tree node or browser tile. */
export interface ShareableFolder {
  id: string;
  name: string;
  slug: string;
  isPublic: boolean;
}

export function ShareFolderDialog({
  folder,
  userCode,
  onOpenChange,
  onToggle,
}: {
  folder: ShareableFolder | null;
  userCode?: string | null;
  onOpenChange: (open: boolean) => void;
  onToggle: (id: string, isPublic: boolean) => void;
}) {
  const t = useTranslations("dashboard.folders");
  const [copied, setCopied] = useState(false);

  const url =
    folder && userCode
      ? `${typeof window === "undefined" ? "" : window.location.origin}/a/${userCode}/${folder.slug}`
      : "";

  useEffect(() => {
    setCopied(false);
  }, [folder]);

  async function copy() {
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
      toast.success(t("linkCopied"));
    } catch {
      toast.error(t("copyFailed"));
    }
  }

  return (
    <Dialog open={Boolean(folder)} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("shareTitle")}</DialogTitle>
          <DialogDescription>{t("shareDesc")}</DialogDescription>
        </DialogHeader>

        <div className="ring-foreground/10 flex items-center justify-between gap-4 rounded-lg px-3 py-2.5 ring-1">
          <div className="min-w-0">
            <Label htmlFor="folder-public" className="text-sm font-medium">
              {t("publicToggle")}
            </Label>
            <p className="text-muted-foreground mt-0.5 text-xs">
              {folder?.isPublic ? t("publicOn") : t("publicOff")}
            </p>
          </div>
          <Switch
            id="folder-public"
            checked={folder?.isPublic ?? false}
            onCheckedChange={(v) => folder && onToggle(folder.id, v)}
          />
        </div>

        {folder?.isPublic ? (
          <div className="flex items-center gap-2">
            <Input readOnly value={url} className="h-9 font-mono text-xs" />
            <Button variant="outline" size="sm" onClick={copy} className="h-9 shrink-0">
              {copied ? <Check className="size-4" /> : <Copy className="size-4" />}
              {t("copyLink")}
            </Button>
          </div>
        ) : null}

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {t("done")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
