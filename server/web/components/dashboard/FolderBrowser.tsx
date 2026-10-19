"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useRef, useState, useTransition } from "react";
import { useTranslations } from "next-intl";
import { ContextMenu as ContextMenuPrimitive } from "radix-ui";
import {
  ChevronRight,
  FileStack,
  FolderInput,
  FolderOpen,
  FolderPlus,
  Globe,
  Inbox,
  Loader2,
  MoreHorizontal,
  Pencil,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { humanFileSize } from "@/lib/format";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
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
  moveUploadsToFolder,
  renameFolder,
  setFolderVisibility,
} from "@/app/actions/folders";
import { DeleteFolderDialog, type DeleteFolderTarget } from "./DeleteFolderDialog";
import { ShareFolderDialog } from "./FolderTree";
import { FolderCard, type BrowserFolder } from "./FolderCard";

/* -------------------------------------------------------------------------- */
/* Menu primitives (styled to match components/ui/dropdown-menu)               */
/* -------------------------------------------------------------------------- */

function CtxContent({
  className,
  ...props
}: React.ComponentProps<typeof ContextMenuPrimitive.Content>) {
  return (
    <ContextMenuPrimitive.Portal>
      <ContextMenuPrimitive.Content
        data-slot="context-menu-content"
        className={cn(
          "bg-popover text-popover-foreground ring-foreground/10 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95 z-50 min-w-52 origin-(--radix-context-menu-content-transform-origin) overflow-hidden rounded-lg p-1 text-sm shadow-md ring-1 duration-100",
          className,
        )}
        {...props}
      />
    </ContextMenuPrimitive.Portal>
  );
}

function CtxItem({
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

function CtxSeparator() {
  return <ContextMenuPrimitive.Separator className="bg-foreground/10 -mx-1 my-1 h-px" />;
}

interface ItemProps {
  children: React.ReactNode;
  onSelect?: () => void;
  variant?: "default" | "destructive";
  asChild?: boolean;
}

interface MenuParts {
  Item: React.ComponentType<ItemProps>;
  Separator: React.ComponentType;
}

const DROPDOWN_PARTS: MenuParts = {
  Item: (p) => <DropdownMenuItem {...p} />,
  Separator: () => <DropdownMenuSeparator />,
};

const CONTEXT_PARTS: MenuParts = {
  Item: (p) => <CtxItem {...p} />,
  Separator: () => <CtxSeparator />,
};

/**
 * Dialogs opened from a menu item must be mounted outside the menu (which
 * unmounts on select), and opening them on the next tick avoids Radix leaving
 * `pointer-events: none` on the body while its close animation runs.
 */
function defer(fn: () => void) {
  setTimeout(fn, 0);
}

/* -------------------------------------------------------------------------- */
/* Browser                                                                     */
/* -------------------------------------------------------------------------- */

export interface FolderBrowserProps {
  /** Folders shown at this level. */
  folders: BrowserFolder[];
  /** Folder being browsed, or null at the root of the library. */
  currentFolderId: string | null;
  currentFolderName?: string | null;
  /** Files sitting directly in the browsed folder. */
  filesHere?: { count: number; size: number } | null;
  /** Loose files — only meaningful at the root. */
  unsorted?: { count: number; size: number } | null;
  userCode?: string | null;
}

export function FolderBrowser({
  folders,
  currentFolderId,
  currentFolderName,
  filesHere,
  unsorted,
  userCode,
}: FolderBrowserProps) {
  const t = useTranslations("dashboard.folderBrowser");
  const tf = useTranslations("dashboard.folders");
  const router = useRouter();
  const [pending, startTransition] = useTransition();

  const gridRef = useRef<HTMLDivElement | null>(null);
  const tileRefs = useRef<(HTMLDivElement | null)[]>([]);
  const [cursor, setCursor] = useState(0);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const [naming, setNaming] = useState<
    | { mode: "create"; parentId: string | null; parentName?: string | null }
    | { mode: "rename"; id: string; name: string }
    | null
  >(null);
  const [sharing, setSharing] = useState<BrowserFolder | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<DeleteFolderTarget | null>(null);

  // Clamp on read: a folder deleted mid-session shortens the list without the
  // roving cursor ever pointing past the end.
  const activeIndex = Math.min(cursor, Math.max(0, folders.length - 1));

  const run = useCallback(
    (work: () => Promise<unknown>, success?: string) => {
      startTransition(async () => {
        try {
          await work();
          if (success) toast.success(success);
          router.refresh();
        } catch (err) {
          toast.error(err instanceof Error ? err.message : tf("error"));
        }
      });
    },
    [router, tf],
  );

  /**
   * Entering a folder.
   *
   * A folder with subfolders opens *inside* the browser; a leaf goes straight
   * to its file list. Anything else would make double-clicking a plain folder
   * land on a page showing one tile — which is not what "open the folder"
   * means to anyone.
   */
  const openFolder = useCallback(
    (folder: BrowserFolder) => {
      router.push(
        folder.subfolderCount > 0
          ? `/dashboard/folders?folder=${folder.id}`
          : `/dashboard?folder=${folder.id}`,
      );
    },
    [router],
  );

  const askDelete = useCallback((folder: BrowserFolder) => {
    setConfirmDelete({
      id: folder.id,
      name: folder.name,
      fileCount: folder.fileCount,
      subfolderCount: folder.descendantCount,
    });
  }, []);

  const columns = useCallback(() => {
    const el = gridRef.current;
    if (!el) return 1;
    return Math.max(1, getComputedStyle(el).gridTemplateColumns.split(" ").filter(Boolean).length);
  }, []);

  function focusIndex(i: number) {
    const next = Math.max(0, Math.min(folders.length - 1, i));
    setCursor(next);
    tileRefs.current[next]?.focus();
  }

  function onGridKeyDown(e: React.KeyboardEvent<HTMLDivElement>) {
    const target = e.target as HTMLElement;
    if (!target.hasAttribute("data-folder-tile")) return;

    const folder = folders[activeIndex];
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
        next = folders.length - 1;
        break;
      case "Enter":
        if (folder) {
          e.preventDefault();
          openFolder(folder);
        }
        return;
      case "Escape":
        setSelectedId(null);
        return;
      case "F2":
        if (folder) {
          e.preventDefault();
          setNaming({ mode: "rename", id: folder.id, name: folder.name });
        }
        return;
      case "Delete":
        if (folder) {
          e.preventDefault();
          askDelete(folder);
        }
        return;
      default:
        return;
    }

    e.preventDefault();
    focusIndex(next);
  }

  const hasShortcuts = Boolean(filesHere?.count) || Boolean(unsorted?.count);

  return (
    <>
      {/* Ways out of the folder view and into the actual files. */}
      {hasShortcuts ? (
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {filesHere && filesHere.count > 0 ? (
            <ShortcutCard
              href={`/dashboard?folder=${currentFolderId}`}
              icon={<FileStack className="size-5" />}
              title={t("filesHere")}
              meta={`${t("fileCount", { count: filesHere.count })} · ${humanFileSize(filesHere.size)}`}
            />
          ) : null}
          {unsorted && unsorted.count > 0 ? (
            <ShortcutCard
              href="/dashboard?folder=none"
              icon={<Inbox className="size-5" />}
              title={tf("unsorted")}
              meta={`${t("fileCount", { count: unsorted.count })} · ${humanFileSize(unsorted.size)}`}
            />
          ) : null}
        </div>
      ) : null}

      {folders.length > 0 ? (
        <div className="-mb-1 flex items-center justify-end">
          <Button
            variant="outline"
            size="sm"
            className="max-sm:h-11"
            onClick={() =>
              setNaming({
                mode: "create",
                parentId: currentFolderId,
                parentName: currentFolderName,
              })
            }
          >
            <FolderPlus className="size-4" />
            {currentFolderId ? t("newSubfolder") : t("newFolder")}
          </Button>
        </div>
      ) : null}

      {folders.length === 0 ? (
        <EmptyFolders
          currentFolderName={currentFolderName}
          onCreate={() =>
            setNaming({ mode: "create", parentId: currentFolderId, parentName: currentFolderName })
          }
        />
      ) : (
        /* Right-clicking the background (not a tile) offers "New folder" -
           the same affordance a desktop file manager gives you. */
        <ContextMenuPrimitive.Root>
          <ContextMenuPrimitive.Trigger asChild>
            <div
              ref={gridRef}
              role="listbox"
              aria-label={t("regionLabel")}
              onKeyDown={onGridKeyDown}
              className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4"
            >
              {folders.map((folder, index) => (
                <ContextMenuPrimitive.Root
                  key={folder.id}
                  onOpenChange={(open) => open && setSelectedId(folder.id)}
                >
                  <ContextMenuPrimitive.Trigger asChild>
                    <div
                      ref={(el) => {
                        tileRefs.current[index] = el;
                      }}
                      data-folder-tile
                      role="option"
                      aria-selected={selectedId === folder.id}
                      aria-label={folder.name}
                      tabIndex={index === activeIndex ? 0 : -1}
                      onFocus={() => setCursor(index)}
                      // Keep the tile's own right-click menu from also opening
                      // the grid-background one it is nested inside. Radix's
                      // Slot runs the child handler first, so this tile's own
                      // trigger still fires — only the ancestor is cut off.
                      onContextMenu={(e) => e.stopPropagation()}
                      // Same for the touch/pen long-press path. Mouse
                      // pointerdown is deliberately left alone: Radix dismisses
                      // open layers by listening for it on `document`.
                      onPointerDown={(e) => {
                        if (e.pointerType !== "mouse") e.stopPropagation();
                      }}
                      className="focus-visible:ring-ring focus-visible:ring-offset-background rounded-xl outline-none focus-visible:ring-2 focus-visible:ring-offset-2"
                    >
                      <FolderCard
                        folder={folder}
                        selected={selectedId === folder.id}
                        onSelect={() => {
                          setCursor(index);
                          setSelectedId(folder.id);
                        }}
                        onOpen={() => openFolder(folder)}
                        onDropIds={(ids) =>
                          run(async () => {
                            const res = await moveUploadsToFolder(ids, folder.id);
                            if (res.moved > 0) {
                              toast.success(
                                tf("moved", { count: res.moved, name: folder.name }),
                              );
                            }
                          })
                        }
                        menuSlot={
                          <DropdownMenu>
                            <DropdownMenuTrigger asChild>
                              <Button
                                variant="ghost"
                                size="icon-sm"
                                // The tile's only always-visible action: it has
                                // to be thumb-sized on a phone.
                                className="max-sm:size-10"
                                aria-label={tf("actions", { name: folder.name })}
                              >
                                <MoreHorizontal className="size-4" />
                              </Button>
                            </DropdownMenuTrigger>
                            <DropdownMenuContent align="end" className="w-56">
                              <FolderMenuItems
                                parts={DROPDOWN_PARTS}
                                folder={folder}
                                userCode={userCode}
                                onOpen={() => openFolder(folder)}
                                onNewSubfolder={() =>
                                  setNaming({
                                    mode: "create",
                                    parentId: folder.id,
                                    parentName: folder.name,
                                  })
                                }
                                onRename={() =>
                                  setNaming({
                                    mode: "rename",
                                    id: folder.id,
                                    name: folder.name,
                                  })
                                }
                                onShare={() => setSharing(folder)}
                                onDelete={() => askDelete(folder)}
                              />
                            </DropdownMenuContent>
                          </DropdownMenu>
                        }
                      />
                    </div>
                  </ContextMenuPrimitive.Trigger>
                  <CtxContent>
                    <FolderMenuItems
                      parts={CONTEXT_PARTS}
                      folder={folder}
                      userCode={userCode}
                      onOpen={() => openFolder(folder)}
                      onNewSubfolder={() =>
                        setNaming({
                          mode: "create",
                          parentId: folder.id,
                          parentName: folder.name,
                        })
                      }
                      onRename={() =>
                        setNaming({ mode: "rename", id: folder.id, name: folder.name })
                      }
                      onShare={() => setSharing(folder)}
                      onDelete={() => askDelete(folder)}
                    />
                  </CtxContent>
                </ContextMenuPrimitive.Root>
              ))}
            </div>
          </ContextMenuPrimitive.Trigger>
          <CtxContent>
            <CtxItem
              onSelect={() =>
                defer(() =>
                  setNaming({
                    mode: "create",
                    parentId: currentFolderId,
                    parentName: currentFolderName,
                  }),
                )
              }
            >
              <FolderPlus /> {currentFolderId ? t("newSubfolder") : t("newFolder")}
            </CtxItem>
          </CtxContent>
        </ContextMenuPrimitive.Root>
      )}

      {pending ? (
        <p className="text-muted-foreground flex items-center gap-2 text-xs">
          <Loader2 className="size-3.5 animate-spin" /> {t("working")}
        </p>
      ) : null}

      <FolderNameDialog
        state={naming}
        onOpenChange={(v) => !v && setNaming(null)}
        onSubmit={(name) => {
          const current = naming;
          setNaming(null);
          if (!current) return;
          if (current.mode === "create") {
            run(() => createFolder(name, current.parentId), tf("created"));
          } else if (name !== current.name) {
            run(() => renameFolder(current.id, name), tf("renamed"));
          }
        }}
      />

      <ShareFolderDialog
        folder={sharing}
        userCode={userCode}
        onOpenChange={(v) => !v && setSharing(null)}
        onToggle={(id, isPublic) => {
          setSharing((prev) => (prev && prev.id === id ? { ...prev, isPublic } : prev));
          run(
            () => setFolderVisibility(id, isPublic),
            isPublic ? tf("madePublic") : tf("madePrivate"),
          );
        }}
      />

      <DeleteFolderDialog
        folder={confirmDelete}
        onOpenChange={(v) => !v && setConfirmDelete(null)}
        onConfirm={(id, withContents) => {
          setConfirmDelete(null);
          run(async () => {
            const res = await deleteFolder(id, { withContents });
            if (res.trashed > 0) {
              toast.success(tf("deletedWithContents", { count: res.trashed }));
            } else {
              toast.success(tf("deleted"));
            }
          });
        }}
      />
    </>
  );
}

/* -------------------------------------------------------------------------- */
/* Menu items (shared by the "…" dropdown and the right-click menu)            */
/* -------------------------------------------------------------------------- */

function FolderMenuItems({
  parts,
  folder,
  userCode,
  onOpen,
  onNewSubfolder,
  onRename,
  onShare,
  onDelete,
}: {
  parts: MenuParts;
  folder: BrowserFolder;
  userCode?: string | null;
  onOpen: () => void;
  onNewSubfolder: () => void;
  onRename: () => void;
  onShare: () => void;
  onDelete: () => void;
}) {
  const t = useTranslations("dashboard.folderBrowser");
  const tf = useTranslations("dashboard.folders");
  const { Item, Separator } = parts;

  return (
    <>
      <Item onSelect={() => defer(onOpen)}>
        <FolderOpen /> {t("open")}
      </Item>
      <Item asChild>
        <Link href={`/dashboard?folder=${folder.id}`}>
          <FileStack /> {t("showFiles")}
        </Link>
      </Item>
      {folder.subfolderCount > 0 ? (
        <Item asChild>
          <Link href={`/dashboard/folders?folder=${folder.id}`}>
            <ChevronRight /> {t("browseSubfolders")}
          </Link>
        </Item>
      ) : null}

      <Separator />

      <Item onSelect={() => defer(onNewSubfolder)}>
        <FolderPlus /> {tf("newSubfolder")}
      </Item>
      <Item onSelect={() => defer(onRename)}>
        <Pencil /> {tf("rename")}
      </Item>
      <Item onSelect={() => defer(onShare)}>
        <Globe /> {tf("share")}
      </Item>
      {folder.isPublic && userCode ? (
        <Item asChild>
          <a href={`/a/${userCode}/${folder.slug}`} target="_blank" rel="noreferrer">
            <Globe /> {t("openGallery")}
          </a>
        </Item>
      ) : null}

      <Separator />

      <Item variant="destructive" onSelect={() => defer(onDelete)}>
        <Trash2 /> {tf("delete")}
      </Item>
    </>
  );
}

/* -------------------------------------------------------------------------- */
/* Pieces                                                                      */
/* -------------------------------------------------------------------------- */

function ShortcutCard({
  href,
  icon,
  title,
  meta,
}: {
  href: string;
  icon: React.ReactNode;
  title: string;
  meta: string;
}) {
  return (
    <Link
      href={href}
      className="group/shortcut bg-card ring-foreground/10 hover:ring-foreground/20 hover:shadow-foreground/5 flex items-center gap-3 rounded-xl p-3 ring-1 transition-[box-shadow,transform] duration-200 hover:-translate-y-0.5 hover:shadow-lg"
    >
      <span className="bg-primary/10 text-primary grid size-11 shrink-0 place-items-center rounded-lg transition-transform group-hover/shortcut:scale-105">
        {icon}
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="truncate text-sm font-medium">{title}</span>
        <span className="text-muted-foreground truncate text-xs tabular-nums">{meta}</span>
      </span>
      <ChevronRight className="text-muted-foreground size-4 shrink-0 transition-transform group-hover/shortcut:translate-x-0.5" />
    </Link>
  );
}

function EmptyFolders({
  currentFolderName,
  onCreate,
}: {
  currentFolderName?: string | null;
  onCreate: () => void;
}) {
  const t = useTranslations("dashboard.folderBrowser");

  return (
    <div className="bg-card/40 ring-foreground/10 relative flex flex-col items-center justify-center gap-5 overflow-hidden rounded-xl px-4 py-14 text-center ring-1 sm:px-6 sm:py-20">
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 opacity-[0.04] [background-image:linear-gradient(to_right,currentColor_1px,transparent_1px),linear-gradient(to_bottom,currentColor_1px,transparent_1px)] [background-size:24px_24px]"
      />
      <div className="bg-primary/10 text-primary ring-primary/15 relative grid size-14 place-items-center rounded-2xl ring-1">
        <FolderPlus className="size-7" />
      </div>
      <div className="relative space-y-1">
        <h3 className="text-base font-medium">
          {currentFolderName ? t("empty.insideTitle", { name: currentFolderName }) : t("empty.title")}
        </h3>
        <p className="text-muted-foreground mx-auto max-w-md text-sm">{t("empty.desc")}</p>
      </div>
      <div className="text-muted-foreground/90 bg-muted/60 ring-foreground/10 relative flex items-center gap-2 rounded-full px-3 py-1.5 text-xs ring-1">
        <FolderInput className="size-3.5" />
        {t("empty.dragHint")}
      </div>
      <Button onClick={onCreate} className="relative max-sm:h-11">
        <FolderPlus className="size-4" />
        {currentFolderName ? t("newSubfolder") : t("newFolder")}
      </Button>
    </div>
  );
}

type NamingState =
  | { mode: "create"; parentId: string | null; parentName?: string | null }
  | { mode: "rename"; id: string; name: string }
  | null;

function FolderNameDialog({
  state,
  onOpenChange,
  onSubmit,
}: {
  state: NamingState;
  onOpenChange: (open: boolean) => void;
  onSubmit: (name: string) => void;
}) {
  const t = useTranslations("dashboard.folderBrowser");
  const tf = useTranslations("dashboard.folders");
  const tc = useTranslations("common");
  const [value, setValue] = useState("");

  useEffect(() => {
    setValue(state?.mode === "rename" ? state.name : "");
  }, [state]);

  const clean = value.trim();
  const renaming = state?.mode === "rename";
  const parentName = state?.mode === "create" ? state.parentName : null;

  return (
    <Dialog open={Boolean(state)} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>
            {renaming ? tf("rename") : parentName ? tf("newSubfolder") : tf("newFolder")}
          </DialogTitle>
          <DialogDescription>
            {renaming
              ? t("renameDesc")
              : parentName
                ? t("createInsideDesc", { name: parentName })
                : t("createDesc")}
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-1.5">
          <Label htmlFor="folder-browser-name" className="text-xs">
            {tf("namePlaceholder")}
          </Label>
          <Input
            id="folder-browser-name"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && clean) {
                e.preventDefault();
                onSubmit(clean);
              }
            }}
            placeholder={tf("namePlaceholder")}
            maxLength={120}
            autoFocus
          />
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {tc("cancel")}
          </Button>
          <Button disabled={!clean} onClick={() => onSubmit(clean)}>
            {renaming ? tc("save") : tc("create")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
