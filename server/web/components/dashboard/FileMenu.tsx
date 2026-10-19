"use client";

import { useRouter } from "next/navigation";
import { Fragment, useCallback, useEffect, useMemo, useState, useTransition } from "react";
import { useTranslations } from "next-intl";
import { ContextMenu as ContextMenuPrimitive } from "radix-ui";
import {
  MoreHorizontal,
  Copy,
  ExternalLink,
  Download,
  FileDown,
  Globe,
  Lock,
  Link2,
  Replace,
  Settings2,
  Trash2,
  Check,
  Loader2,
  Plus,
  Folder,
  FolderOpen,
  FolderPlus,
  FolderInput,
  Info,
  Search,
  CornerDownRight,
  Inbox,
} from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
} from "@/components/ui/context-menu";
import { buildLinkFormats, type LinkFormats } from "@/lib/media/link-formats";
import { togglePublish, setVanity, trashUpload } from "@/app/actions/uploads";
import { listFolders, moveUploadsToFolder, createFolder } from "@/app/actions/folders";
import {
  downloadHintKey,
  downloadLabelKey,
  downloadOptionsFor,
  sectionHeadingKey,
  type DownloadOption,
} from "@/lib/media/download-menu";
import { useConvertDownload } from "@/components/dashboard/use-convert-download";
import { bulkTargetSummary } from "@/lib/media/convert-plan";
import { estimateConvertedSize, formatEstimate } from "@/lib/media/convert-size";
import { buildOpenUrl, buildPublicUrl, buildRawUrl, buildDownloadUrl, isImage } from "./media";
import { ConvertDialog, fileToCandidate } from "./ConvertDialog";
import { LinkSettingsDialog } from "./LinkSettingsDialog";
import type { FileItem, FileViewMode } from "./types";

const LINK_FORMAT_KEYS: (keyof LinkFormats)[] = [
  "direct",
  "raw",
  "markdown",
  "bbcode",
  "html",
];

async function writeClipboard(text: string, ok: string, fail: string) {
  try {
    await navigator.clipboard.writeText(text);
    toast.success(ok);
  } catch {
    toast.error(fail);
  }
}

// ── Context-menu primitives ─────────────────────────────────────────────────
// Content, Item and Separator are shared with the rest of the app (see
// components/ui/context-menu). The two submenu pieces below stay local: only
// the file menus nest.

function ContextMenuSubTrigger({
  className,
  children,
  ...props
}: React.ComponentProps<typeof ContextMenuPrimitive.SubTrigger>) {
  return (
    <ContextMenuPrimitive.SubTrigger
      className={cn(
        "focus:bg-accent focus:text-accent-foreground data-[state=open]:bg-accent data-[state=open]:text-accent-foreground flex cursor-default items-center gap-1.5 rounded-md px-1.5 py-1 text-sm outline-hidden select-none [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
        className,
      )}
      {...props}
    >
      {children}
      <CornerDownRight className="ml-auto size-3.5 -rotate-90 opacity-50" />
    </ContextMenuPrimitive.SubTrigger>
  );
}

function ContextMenuSubContent({
  className,
  ...props
}: React.ComponentProps<typeof ContextMenuPrimitive.SubContent>) {
  return (
    <ContextMenuPrimitive.Portal>
      <ContextMenuPrimitive.SubContent
        className={cn(
          "bg-popover text-popover-foreground ring-foreground/10 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 z-50 min-w-36 origin-(--radix-context-menu-content-transform-origin) overflow-hidden rounded-lg p-1 shadow-md ring-1 duration-100",
          className,
        )}
        {...props}
      />
    </ContextMenuPrimitive.Portal>
  );
}

// ── Shared item list ────────────────────────────────────────────────────────
// The dropdown and the context menu must never drift apart, so the items are
// declared once and rendered through whichever primitive set is in play.

interface MenuItemProps {
  children: React.ReactNode;
  onSelect?: () => void;
  variant?: "default" | "destructive";
  disabled?: boolean;
  asChild?: boolean;
  className?: string;
}

interface MenuParts {
  Item: React.ComponentType<MenuItemProps>;
  Separator: React.ComponentType;
  Sub: React.ComponentType<{ children: React.ReactNode }>;
  SubTrigger: React.ComponentType<{ children: React.ReactNode }>;
  SubContent: React.ComponentType<{ children: React.ReactNode; className?: string }>;
}

const DROPDOWN_PARTS: MenuParts = {
  Item: (p) => <DropdownMenuItem {...p} />,
  Separator: () => <DropdownMenuSeparator />,
  Sub: ({ children }) => <DropdownMenuSub>{children}</DropdownMenuSub>,
  SubTrigger: ({ children }) => <DropdownMenuSubTrigger>{children}</DropdownMenuSubTrigger>,
  SubContent: ({ children, className }) => (
    <DropdownMenuSubContent className={className}>{children}</DropdownMenuSubContent>
  ),
};

const CONTEXT_PARTS: MenuParts = {
  Item: (p) => <ContextMenuItem {...p} />,
  Separator: () => <ContextMenuSeparator />,
  Sub: ({ children }) => <ContextMenuPrimitive.Sub>{children}</ContextMenuPrimitive.Sub>,
  SubTrigger: ({ children }) => <ContextMenuSubTrigger>{children}</ContextMenuSubTrigger>,
  SubContent: ({ children, className }) => (
    <ContextMenuSubContent className={className}>{children}</ContextMenuSubContent>
  ),
};

export interface FileActionProps {
  item: FileItem;
  userCode: string;
  appUrl: string;
  copyRaw?: boolean;
  /**
   * Ids that destructive / move actions apply to. When the user right-clicks a
   * file inside a multi-selection this is the whole selection, so "Move to…"
   * doesn't silently act on one file out of twelve.
   */
  targetIds?: string[];
  /**
   * The rows behind `targetIds`, when the caller has them.
   *
   * Conversion is the one action that has to read the files it is about (their
   * type decides which formats are on offer and how many of a mixed selection a
   * target reaches), so ids alone are not enough. Absent, "Convert" acts on the
   * file under the cursor, which is always correct if narrower.
   */
  targetItems?: FileItem[];
  /** Folder currently being browsed — lets a move optimistically empty the view. */
  currentFolderId?: string | null;
  /** Owner's own library, or an administrator reviewing it. See types.ts. */
  mode?: FileViewMode;
  onOpenDetails?: () => void;
  onTrashed?: (id: string) => void;
  onTrashedMany?: (ids: string[]) => void;
  onMoved?: (ids: string[], folderId: string | null) => void;
}

/**
 * Everything the menus need: derived URLs, the mutations, and the dialog
 * plumbing. Shared by the dropdown trigger and the right-click menu.
 */
function useFileActions({
  item,
  userCode,
  appUrl,
  copyRaw,
  targetIds,
  targetItems,
  onTrashed,
  onTrashedMany,
}: FileActionProps) {
  const t = useTranslations("files");
  const router = useRouter();
  const [pending, startTransition] = useTransition();
  const [vanityOpen, setVanityOpen] = useState(false);
  const [linkSettingsOpen, setLinkSettingsOpen] = useState(false);
  const [moveOpen, setMoveOpen] = useState(false);
  const [convertTo, setConvertTo] = useState<string | null>(null);

  const ids = targetIds && targetIds.length > 0 ? targetIds : [item.id];
  const many = ids.length > 1;

  const publicUrl = buildPublicUrl(appUrl, userCode, item.code, item.ext);
  const openUrl = buildOpenUrl(userCode, item.code, item.ext);
  const rawUrl = buildRawUrl(appUrl, userCode, item.code, item.ext);
  const downloadHref = buildDownloadUrl(appUrl, userCode, item.code);
  const image = isImage(item.mimeType, item.ext);
  // What the server can actually produce from THIS file. The dashboard is the
  // owner's own view, so the encoder-backed targets are all on offer here.
  const convertTargets = downloadOptionsFor(item.mimeType, item.ext, item.frames, {
    canEncode: true,
    // The row's own picture size. Only one entry reads it - the web-playable
    // copy, which is offered exactly when the recording is too big for a
    // hardware decoder - and it is absent rather than disabled otherwise.
    width: item.width,
    height: item.height,
  });

  // What "Convert" acts on: the whole selection when the caller handed it over,
  // otherwise this one file.
  const convertItems = targetItems && targetItems.length > 1 ? targetItems : [item];

  const formats: LinkFormats = buildLinkFormats({
    url: publicUrl,
    rawUrl,
    filename: item.filename,
    isImage: image,
  });

  const copyFormat = useCallback(
    (key: keyof LinkFormats) => {
      writeClipboard(
        formats[key],
        t("toast.labelCopied", { label: t(`linkFormats.${key}`) }),
        t("toast.couldntCopy"),
      );
    },
    [formats, t],
  );

  function onTogglePublish() {
    startTransition(async () => {
      try {
        const res = await togglePublish(item.id);
        toast.success(res.published ? t("toast.nowPublic") : t("toast.nowPrivate"));
        router.refresh();
      } catch {
        toast.error(t("toast.couldntChangeVisibility"));
      }
    });
  }

  function onTrash() {
    // Optimistic: drop the cards immediately, then reconcile.
    if (many) onTrashedMany?.(ids);
    else onTrashed?.(item.id);

    startTransition(async () => {
      const results = await Promise.allSettled(ids.map((id) => trashUpload(id)));
      const ok = results.filter((r) => r.status === "fulfilled").length;
      const failed = results.length - ok;
      if (ok > 0) toast.success(t("toast.movedToTrash", { count: ok }));
      if (failed > 0) toast.error(t("toast.couldntMoveToTrash", { count: failed }));
      router.refresh();
    });
  }

  return {
    t,
    pending,
    ids,
    many,
    image,
    convertTargets,
    convertItems,
    convertTo,
    setConvertTo,
    publicUrl,
    openUrl,
    downloadHref,
    quickCopyKey: (copyRaw ? "raw" : "direct") as keyof LinkFormats,
    copyFormat,
    onTogglePublish,
    onTrash,
    vanityOpen,
    setVanityOpen,
    linkSettingsOpen,
    setLinkSettingsOpen,
    moveOpen,
    setMoveOpen,
  };
}

type Actions = ReturnType<typeof useFileActions>;

/**
 * Dialogs opened from a menu item must be mounted outside the menu (which
 * unmounts on select). Opening them on the next tick also avoids Radix's
 * close-animation leaving `pointer-events: none` on the body.
 */
function defer(fn: () => void) {
  setTimeout(fn, 0);
}

function MenuItems({
  parts,
  actions,
  item,
  userCode,
  mode = "owner",
  onOpenDetails,
}: {
  parts: MenuParts;
  actions: Actions;
  item: FileItem;
  userCode: string;
  mode?: FileViewMode;
  onOpenDetails?: () => void;
}) {
  const { Item, Separator, Sub, SubTrigger, SubContent } = parts;
  const { t, pending, many, ids, convertTargets, convertItems } = actions;
  const { start: startDownload } = useConvertDownload();

  /**
   * For a mixed selection the menu has to say how far each target reaches BEFORE
   * it is clicked ("PNG, 5 of 8"). Computed here rather than in the hook because
   * this component only ever renders while a menu is OPEN - the hook runs for
   * every tile on the page, and counting a selection of thirty against nine
   * targets a hundred and twenty times over is work nobody sees.
   */
  const convertSummary = useMemo(
    () =>
      convertItems.length > 1
        ? bulkTargetSummary(convertItems.map(fileToCandidate), "copy")
        : null,
    [convertItems],
  );

  /**
   * What to print for a target. A format name is its own best label, so this is
   * the plain uppercase name for everything except the one target whose value
   * ("webmp4") is an internal name rather than a format.
   */
  const label = (to: string): string => {
    const key = downloadLabelKey(to);
    return key ? t(key) : to.toUpperCase();
  };

  /** The right-hand column of a format row: how big the result will be. */
  const sizeLabel = (option: DownloadOption): string | null => {
    const estimate = estimateConvertedSize(item, option);
    if (!estimate) return null;
    const size = formatEstimate(estimate.bytes);
    return estimate.confidence === "exact" ? size : t("convert.approxSize", { size });
  };

  return (
    <>
      {onOpenDetails ? (
        <>
          <Item onSelect={() => defer(onOpenDetails)}>
            <Info /> {t("menu.details")}
          </Item>
          <Separator />
        </>
      ) : null}

      <Item onSelect={() => actions.copyFormat(actions.quickCopyKey)}>
        <Copy /> {t("menu.copyLink")}
      </Item>
      <Sub>
        <SubTrigger>
          <Link2 /> {t("menu.copyAs")}
        </SubTrigger>
        <SubContent className="w-44">
          {LINK_FORMAT_KEYS.map((k) => (
            <Item key={k} onSelect={() => actions.copyFormat(k)}>
              {t(`linkFormats.${k}`)}
            </Item>
          ))}
        </SubContent>
      </Sub>

      <Separator />

      <Item asChild>
        {/* Relative on purpose: the owner must land on the host they are signed
            in on, or the page hides every encoder-backed format. See buildOpenUrl. */}
        <a href={actions.openUrl} target="_blank" rel="noopener noreferrer">
          <ExternalLink /> {t("menu.open")}
        </a>
      </Item>
      <Item asChild>
        <a href={actions.downloadHref}>
          <Download /> {t("menu.download")}
        </a>
      </Item>
      {convertTargets.length > 0 ? (
        <Sub>
          <SubTrigger>
            <FileDown /> {t("menu.downloadAs")}
          </SubTrigger>
          <SubContent className="w-56">
            {convertTargets.map((option, i) => {
              // Say what the entry really does, or a 63-frame GIF comes back as
              // its first frame with nothing but "PNG" to warn the reader.
              const hint = downloadHintKey(option);
              // Anything that throws the motion away sits under its own
              // heading, so it can never be picked by accident.
              const heading = sectionHeadingKey(option, convertTargets[i - 1]);
              return (
                <Fragment key={option.to}>
                  {heading ? (
                    <div className="text-muted-foreground mt-1 px-1.5 py-1 text-[11px] font-medium tracking-wide uppercase">
                      {t(heading)}
                    </div>
                  ) : null}
                  {/* Not an <a download>. The server can refuse this conversion
                      for reasons only it knows - no audio track, codecs the
                      container will not take, the queue full - and a refusal
                      that arrives as the body of a started download is a
                      "Failed" with no explanation. `start` asks first. */}
                  <Item
                    onSelect={() =>
                      void startDownload(
                        `/${userCode}/${item.code}/convert?to=${option.to}`,
                        label(option.to),
                      )
                    }
                    className="flex-col items-start gap-0"
                  >
                    <span className="flex w-full items-baseline justify-between gap-3">
                      <span>{label(option.to)}</span>
                      {sizeLabel(option) ? (
                        <span className="text-muted-foreground text-[11px] tabular-nums">
                          {sizeLabel(option)}
                        </span>
                      ) : null}
                    </span>
                    {hint ? (
                      <span className="text-muted-foreground text-[11px]">{t(hint)}</span>
                    ) : null}
                  </Item>
                </Fragment>
              );
            })}
          </SubContent>
        </Sub>
      ) : null}

      {/* Convert: same targets, but it changes the stored file rather than
          handing back a copy. Kept as its own entry, never folded into
          "Download as…", because one of the two destroys the original. */}
      {convertSummary ? (
        convertSummary.length > 0 ? (
          <Sub>
            <SubTrigger>
              <Replace /> {t("convert.action")}
            </SubTrigger>
            <SubContent className="w-56">
              {convertSummary.map((summary) => (
                <Item
                  key={summary.to}
                  onSelect={() => defer(() => actions.setConvertTo(summary.to))}
                >
                  <span className="flex w-full items-baseline justify-between gap-3">
                    <span>{label(summary.to)}</span>
                    <span className="text-muted-foreground text-[11px] tabular-nums">
                      {t("convert.countOf", {
                        n: summary.eligible,
                        total: convertItems.length,
                      })}
                    </span>
                  </span>
                </Item>
              ))}
            </SubContent>
          </Sub>
        ) : null
      ) : convertTargets.length > 0 ? (
        <Sub>
          <SubTrigger>
            <Replace /> {t("convert.action")}
          </SubTrigger>
          <SubContent className="w-56">
            {convertTargets.map((option, i) => {
              const hint = downloadHintKey(option);
              const heading = sectionHeadingKey(option, convertTargets[i - 1]);
              return (
                <Fragment key={option.to}>
                  {heading ? (
                    <div className="text-muted-foreground mt-1 px-1.5 py-1 text-[11px] font-medium tracking-wide uppercase">
                      {t(heading)}
                    </div>
                  ) : null}
                  <Item onSelect={() => defer(() => actions.setConvertTo(option.to))}>
                    <span className="flex w-full flex-col items-start gap-0">
                      <span className="flex w-full items-baseline justify-between gap-3">
                        <span>{label(option.to)}</span>
                        {sizeLabel(option) ? (
                          <span className="text-muted-foreground text-[11px] tabular-nums">
                            {sizeLabel(option)}
                          </span>
                        ) : null}
                      </span>
                      {hint ? (
                        <span className="text-muted-foreground text-[11px]">{t(hint)}</span>
                      ) : null}
                    </span>
                  </Item>
                </Fragment>
              );
            })}
          </SubContent>
        </Sub>
      ) : null}

      <Separator />

      {/* Folders belong to the owner's session, so this entry only exists in the
          owner's own view. See FileViewMode in types.ts. */}
      {mode === "owner" ? (
        <Item onSelect={() => defer(() => actions.setMoveOpen(true))}>
          <FolderInput />{" "}
          {many ? t("menu.moveManyToFolder", { count: ids.length }) : t("menu.moveToFolder")}
        </Item>
      ) : null}
      <Item onSelect={actions.onTogglePublish} disabled={pending}>
        {item.published ? <Lock /> : <Globe />}
        {item.published ? t("menu.makePrivate") : t("menu.makePublic")}
      </Item>
      <Item onSelect={() => defer(() => actions.setVanityOpen(true))}>
        <Link2 /> {t("menu.customUrl")}
      </Item>
      <Item onSelect={() => defer(() => actions.setLinkSettingsOpen(true))}>
        <Settings2 /> {t("menu.linkSettings")}
      </Item>

      <Separator />

      <Item variant="destructive" onSelect={actions.onTrash}>
        <Trash2 />{" "}
        {many ? t("menu.trashMany", { count: ids.length }) : t("menu.moveToTrash")}
      </Item>
    </>
  );
}

/** Dialogs shared by both menu surfaces. */
function FileDialogs({
  actions,
  props,
}: {
  actions: Actions;
  props: FileActionProps;
}) {
  return (
    <>
      <VanityDialog
        open={actions.vanityOpen}
        onOpenChange={actions.setVanityOpen}
        item={props.item}
        appUrl={props.appUrl}
        userCode={props.userCode}
      />
      <LinkSettingsDialog
        open={actions.linkSettingsOpen}
        onOpenChange={actions.setLinkSettingsOpen}
        item={props.item}
      />
      {/* Mounted even while closed: "Run in background" hides the dialog and the
          run keeps going, which it could not if closing unmounted the loop. */}
      <ConvertDialog
        open={actions.convertTo !== null}
        onOpenChange={(open) => {
          if (!open) actions.setConvertTo(null);
        }}
        items={actions.convertItems}
        to={actions.convertTo ?? ""}
        userCode={props.userCode}
        appUrl={props.appUrl}
      />

      {props.mode === "admin" ? null : (
        <MoveToFolderDialog
          open={actions.moveOpen}
          onOpenChange={actions.setMoveOpen}
          ids={actions.ids}
          // For one file the truthful "current" folder is the file's own, even in
          // an unscoped view; for a selection fall back to the folder on screen.
          currentFolderId={actions.many ? (props.currentFolderId ?? null) : props.item.folderId}
          label={actions.many ? undefined : props.item.filename}
          onMoved={props.onMoved}
        />
      )}
    </>
  );
}

// ── Public surfaces ─────────────────────────────────────────────────────────

export function FileMenu({
  align = "end",
  trigger,
  ...props
}: FileActionProps & {
  align?: "start" | "end";
  trigger?: React.ReactNode;
}) {
  const actions = useFileActions(props);
  const { t } = actions;

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          {trigger ?? (
            <Button variant="ghost" size="icon-sm" aria-label={t("card.fileActions")}>
              <MoreHorizontal className="size-4" />
            </Button>
          )}
        </DropdownMenuTrigger>
        <DropdownMenuContent align={align} className="w-56">
          <MenuItems
            parts={DROPDOWN_PARTS}
            actions={actions}
            item={props.item}
            userCode={props.userCode}
            mode={props.mode}
            onOpenDetails={props.onOpenDetails}
          />
        </DropdownMenuContent>
      </DropdownMenu>

      <FileDialogs actions={actions} props={props} />
    </>
  );
}

/**
 * Right-click surface for a tile or row.
 *
 * Replaces the old behaviour where right-click silently toggled selection —
 * that stole "Copy image" from a library of 8000 screenshots and broke
 * long-press on touch. Selection now lives on ⌘/Ctrl- and Shift-click.
 */
export function FileContextMenu({
  children,
  onOpenChange,
  ...props
}: FileActionProps & {
  children: React.ReactNode;
  onOpenChange?: (open: boolean) => void;
}) {
  const actions = useFileActions(props);

  return (
    <>
      <ContextMenuPrimitive.Root onOpenChange={onOpenChange}>
        <ContextMenuPrimitive.Trigger asChild>{children}</ContextMenuPrimitive.Trigger>
        <ContextMenuContent>
          {actions.many ? (
            <>
              <div className="text-muted-foreground px-1.5 py-1 text-xs font-medium">
                {actions.t("selection.countSelected", { count: actions.ids.length })}
              </div>
              <ContextMenuSeparator />
            </>
          ) : null}
          <MenuItems
            parts={CONTEXT_PARTS}
            actions={actions}
            item={props.item}
            userCode={props.userCode}
            mode={props.mode}
            onOpenDetails={props.onOpenDetails}
          />
        </ContextMenuContent>
      </ContextMenuPrimitive.Root>

      <FileDialogs actions={actions} props={props} />
    </>
  );
}

// ── Move to folder ──────────────────────────────────────────────────────────

interface FolderOption {
  id: string;
  parentId: string | null;
  name: string;
  isPublic: boolean;
  count: number;
}

interface FolderTreeRow extends FolderOption {
  depth: number;
}

/**
 * Flatten the folder list into render order (parents before children).
 * Orphans — a parent that was deleted mid-session — are surfaced at the root
 * instead of disappearing, because a folder you can't see is a folder whose
 * files look lost.
 */
function flattenFolders(rows: FolderOption[]): FolderTreeRow[] {
  const byParent = new Map<string | null, FolderOption[]>();
  const ids = new Set(rows.map((r) => r.id));

  for (const r of rows) {
    const key = r.parentId && ids.has(r.parentId) ? r.parentId : null;
    const list = byParent.get(key) ?? [];
    list.push(r);
    byParent.set(key, list);
  }
  for (const list of byParent.values()) {
    list.sort((a, b) => a.name.localeCompare(b.name));
  }

  const out: FolderTreeRow[] = [];
  const walk = (parent: string | null, depth: number, seen: Set<string>) => {
    for (const row of byParent.get(parent) ?? []) {
      if (seen.has(row.id)) continue; // cycle guard
      seen.add(row.id);
      out.push({ ...row, depth });
      walk(row.id, depth + 1, seen);
    }
  };
  walk(null, 0, new Set());
  return out;
}

const ROOT = "__root__";

export function MoveToFolderDialog({
  open,
  onOpenChange,
  ids,
  currentFolderId,
  label,
  onMoved,
}: {
  open: boolean;
  onOpenChange: (v: boolean) => void;
  /** Uploads to move. */
  ids: string[];
  /** Folder they're in now — pre-selected and marked, so the target is obvious. */
  currentFolderId?: string | null;
  /** Single-file name shown in the description. */
  label?: string;
  onMoved?: (ids: string[], folderId: string | null) => void;
}) {
  const t = useTranslations("files");
  const tCommon = useTranslations("common");
  const router = useRouter();
  const [pending, startTransition] = useTransition();
  const [rows, setRows] = useState<FolderOption[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [query, setQuery] = useState("");
  const [target, setTarget] = useState<string>(currentFolderId ?? ROOT);
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setCreating(false);
    setNewName("");
    setTarget(currentFolderId ?? ROOT);
    setLoading(true);
    listFolders()
      .then((data) =>
        setRows(
          data.map((f) => ({
            id: f.id,
            parentId: f.parentId,
            name: f.name,
            isPublic: f.isPublic,
            count: Number(f.count),
          })),
        ),
      )
      .catch(() => toast.error(t("move.couldntLoad")))
      .finally(() => setLoading(false));
  }, [open, currentFolderId, t]);

  const tree = useMemo(() => flattenFolders(rows ?? []), [rows]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return tree;
    return tree.filter((r) => r.name.toLowerCase().includes(q));
  }, [tree, query]);

  function onCreate() {
    const name = newName.trim();
    if (!name) return;
    startTransition(async () => {
      try {
        // New folders nest under the highlighted one, so building
        // "screenshots / 2026" takes two steps instead of a trip to settings.
        const parentId = target === ROOT ? null : target;
        const folder = await createFolder(name, parentId);
        setRows((prev) => [
          ...(prev ?? []),
          {
            id: folder.id,
            parentId: folder.parentId,
            name: folder.name,
            isPublic: folder.isPublic,
            count: 0,
          },
        ]);
        setTarget(folder.id);
        setNewName("");
        setCreating(false);
        toast.success(t("move.folderCreated", { name: folder.name }));
      } catch (e) {
        toast.error(e instanceof Error ? e.message : t("move.couldntCreate"));
      }
    });
  }

  function onConfirm() {
    const folderId = target === ROOT ? null : target;
    onOpenChange(false);
    onMoved?.(ids, folderId);
    startTransition(async () => {
      try {
        const res = await moveUploadsToFolder(ids, folderId);
        const name = folderId ? (rows?.find((r) => r.id === folderId)?.name ?? "") : null;
        toast.success(
          name
            ? t("toast.movedToFolder", { count: res.moved, name })
            : t("toast.movedOutOfFolders", { count: res.moved }),
        );
        router.refresh();
      } catch (e) {
        toast.error(e instanceof Error ? e.message : t("move.couldntMove"));
        router.refresh();
      }
    });
  }

  const unchanged = (currentFolderId ?? ROOT) === target;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("move.title")}</DialogTitle>
          <DialogDescription className="truncate">
            {label ?? t("move.describeMany", { count: ids.length })}
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-2">
          <div className="relative">
            <Search className="text-muted-foreground pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2" />
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t("move.searchPlaceholder")}
              className="pl-8"
              autoFocus
            />
          </div>

          <div className="ring-foreground/10 max-h-64 overflow-y-auto rounded-lg ring-1">
            <FolderRow
              icon={<Inbox className="size-4" />}
              name={t("move.noFolder")}
              hint={t("move.noFolderHint")}
              depth={0}
              selected={target === ROOT}
              current={(currentFolderId ?? null) === null}
              onSelect={() => setTarget(ROOT)}
              onConfirm={onConfirm}
              currentLabel={t("move.currentBadge")}
            />

            {loading ? (
              <div className="text-muted-foreground flex items-center gap-2 px-3 py-4 text-sm">
                <Loader2 className="size-4 animate-spin" /> {tCommon("loading")}
              </div>
            ) : filtered.length === 0 ? (
              <div className="text-muted-foreground px-3 py-4 text-sm">
                {query ? t("move.noMatches") : t("move.noFolders")}
              </div>
            ) : (
              filtered.map((f) => (
                <FolderRow
                  key={f.id}
                  icon={
                    target === f.id ? (
                      <FolderOpen className="size-4" />
                    ) : (
                      <Folder className="size-4" />
                    )
                  }
                  name={f.name}
                  hint={t("move.fileCount", { count: f.count })}
                  depth={query ? 0 : f.depth}
                  selected={target === f.id}
                  current={currentFolderId === f.id}
                  onSelect={() => setTarget(f.id)}
                  onConfirm={onConfirm}
                  currentLabel={t("move.currentBadge")}
                />
              ))
            )}
          </div>

          {creating ? (
            <div className="flex items-end gap-2">
              <div className="flex flex-1 flex-col gap-1.5">
                <Label htmlFor="move-new-folder" className="text-xs">
                  {target === ROOT
                    ? t("move.newFolderAtRoot")
                    : t("move.newFolderInside", {
                        name: rows?.find((r) => r.id === target)?.name ?? "",
                      })}
                </Label>
                <Input
                  id="move-new-folder"
                  value={newName}
                  onChange={(e) => setNewName(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      onCreate();
                    }
                  }}
                  placeholder={t("move.newFolderPlaceholder")}
                  autoFocus
                />
              </div>
              <Button onClick={onCreate} disabled={pending || !newName.trim()}>
                {pending ? (
                  <Loader2 className="size-4 animate-spin" />
                ) : (
                  <Plus className="size-4" />
                )}
                {tCommon("create")}
              </Button>
            </div>
          ) : (
            <Button
              variant="ghost"
              size="sm"
              className="text-muted-foreground w-fit"
              onClick={() => setCreating(true)}
            >
              <FolderPlus className="size-4" /> {t("move.newFolder")}
            </Button>
          )}
        </div>

        <DialogFooter showCloseButton>
          <Button onClick={onConfirm} disabled={pending || unchanged}>
            {pending ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <FolderInput className="size-4" />
            )}
            {t("move.confirm", { count: ids.length })}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function FolderRow({
  icon,
  name,
  hint,
  depth,
  selected,
  current,
  currentLabel,
  onSelect,
  onConfirm,
}: {
  icon: React.ReactNode;
  name: string;
  hint?: string;
  depth: number;
  selected: boolean;
  current?: boolean;
  currentLabel: string;
  onSelect: () => void;
  onConfirm: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      onDoubleClick={onConfirm}
      aria-current={selected ? "true" : undefined}
      className={cn(
        "border-foreground/5 hover:bg-muted/60 flex w-full items-center gap-2 border-b px-3 py-2 text-left text-sm transition-colors last:border-b-0",
        selected && "bg-primary/10 text-foreground hover:bg-primary/10",
      )}
      style={{ paddingLeft: `${0.75 + depth * 1.1}rem` }}
    >
      <span className={cn("shrink-0", selected ? "text-primary" : "text-muted-foreground")}>
        {icon}
      </span>
      <span className="min-w-0 flex-1 truncate">{name}</span>
      {current ? (
        <span className="text-muted-foreground bg-muted rounded px-1.5 py-0.5 text-[10px] font-medium tracking-wide uppercase">
          {currentLabel}
        </span>
      ) : null}
      {hint ? (
        <span className="text-muted-foreground shrink-0 text-xs tabular-nums">{hint}</span>
      ) : null}
      {selected ? <Check className="text-primary size-4 shrink-0" /> : null}
    </button>
  );
}

// ── Custom URL ──────────────────────────────────────────────────────────────

function VanityDialog({
  open,
  onOpenChange,
  item,
  appUrl,
  userCode,
}: {
  open: boolean;
  onOpenChange: (v: boolean) => void;
  item: FileItem;
  appUrl: string;
  userCode: string;
}) {
  const t = useTranslations("files");
  const tCommon = useTranslations("common");
  const router = useRouter();
  const [value, setValue] = useState(item.code);
  const [pending, startTransition] = useTransition();

  useEffect(() => {
    if (open) setValue(item.code);
  }, [open, item.code]);

  function save() {
    const v = value.trim();
    if (!v) {
      toast.error(t("vanity.enterCustomUrl"));
      return;
    }
    startTransition(async () => {
      try {
        const res = await setVanity(item.id, v);
        toast.success(t("vanity.updated"));
        onOpenChange(false);
        setValue(res.code);
        router.refresh();
      } catch (e) {
        toast.error(e instanceof Error ? e.message : t("vanity.couldntUpdate"));
      }
    });
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("vanity.title")}</DialogTitle>
          <DialogDescription>{t("vanity.description")}</DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-2">
          <Label htmlFor={`vanity-${item.id}`}>{t("vanity.slugLabel")}</Label>
          <div className="flex items-center gap-2">
            <span className="text-muted-foreground truncate text-xs">
              {appUrl.replace(/^https?:\/\//, "")}/{userCode}/
            </span>
            <Input
              id={`vanity-${item.id}`}
              value={value}
              onChange={(e) => setValue(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") save();
              }}
              autoFocus
              className="flex-1"
            />
          </div>
          <p className="text-muted-foreground text-xs">{t("vanity.hint")}</p>
        </div>
        <DialogFooter showCloseButton>
          <Button onClick={save} disabled={pending}>
            {pending ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <Check className="size-4" />
            )}
            {tCommon("save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
