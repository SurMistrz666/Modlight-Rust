"use client";

import { useRouter } from "next/navigation";
import { useMemo, useState, useTransition } from "react";
import { useTranslations } from "next-intl";
import { toast } from "sonner";
import {
  Globe,
  Lock,
  Trash2,
  X,
  Loader2,
  Download,
  FolderInput,
  Replace,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { togglePublish, trashUpload } from "@/app/actions/uploads";
import { bulkTargetSummary } from "@/lib/media/convert-plan";
import { downloadLabelKey } from "@/lib/media/download-menu";
import { MoveToFolderDialog } from "./FileMenu";
import { ConvertDialog, fileToCandidate } from "./ConvertDialog";
import { buildDownloadUrl } from "./media";
import type { FileItem, FileViewMode } from "./types";

/**
 * Floating action bar shown when one or more files are selected. Pinned to the
 * bottom-center, rounded, blurred. Runs bulk mutations across the selected
 * items inside a transition, then refreshes.
 *
 * `togglePublish` only flips state, so for Publish/Unpublish we call it
 * conditionally — only on items whose current `published` state differs from
 * the target — to avoid accidentally toggling already-correct files.
 */
export function BulkBar({
  selected,
  items,
  userCode,
  appUrl,
  currentFolderId,
  mode = "owner",
  onClear,
  onTrashed,
  onMoved,
}: {
  /** IDs currently selected. */
  selected: Set<string>;
  /** All visible items, used to read current published state. */
  items: FileItem[];
  /** Owner code for building per-file download URLs. */
  userCode: string;
  /** Base app URL for building per-file download URLs. */
  appUrl: string;
  /** Folder being browsed, so a move can empty the view optimistically. */
  currentFolderId?: string | null;
  /** Owner's own library, or an administrator reviewing it. See types.ts. */
  mode?: FileViewMode;
  /** Clear the selection. */
  onClear: () => void;
  /** Optimistically drop trashed cards/rows from the parent list. */
  onTrashed: (ids: string[]) => void;
  /** Optimistically drop moved cards when the view is scoped to a folder. */
  onMoved?: (ids: string[], folderId: string | null) => void;
}) {
  const t = useTranslations("files");

  /** A target's printed name: its own letters, unless it has none worth showing. */
  const targetLabel = (to: string): string => {
    const key = downloadLabelKey(to);
    return key ? t(key) : to.toUpperCase();
  };
  const router = useRouter();
  const [pending, startTransition] = useTransition();
  const [moveOpen, setMoveOpen] = useState(false);
  const [convertTo, setConvertTo] = useState<string | null>(null);

  const count = selected.size;

  const selectedItems = useMemo(
    () => items.filter((i) => selected.has(i.id)),
    [items, selected],
  );
  const selectedIds = selectedItems.map((i) => i.id);
  const allPublished = selectedItems.length > 0 && selectedItems.every((i) => i.published);

  /**
   * Which formats this selection can go to, and how many files each reaches.
   *
   * The union rather than the intersection, because the intersection of a mixed
   * selection is usually empty and a menu that offers nothing for eight selected
   * files is a menu that is wrong. The count is the sentence the user asked for
   * ("5 of 8") and it is shown BEFORE the click, not reported afterwards.
   */
  const convertTargets = useMemo(
    () => bulkTargetSummary(selectedItems.map(fileToCandidate), "copy"),
    [selectedItems],
  );

  // All selected files already share a folder → pre-select it in the dialog.
  const sharedFolderId = selectedItems.every(
    (i) => i.folderId === (selectedItems[0]?.folderId ?? null),
  )
    ? (selectedItems[0]?.folderId ?? null)
    : undefined;

  function runPublish(target: boolean) {
    // Only flip files that aren't already in the target state.
    const toFlip = selectedItems.filter((i) => i.published !== target);
    if (toFlip.length === 0) {
      toast.info(target ? t("bulk.alreadyPublic") : t("bulk.alreadyPrivate"));
      return;
    }
    startTransition(async () => {
      const results = await Promise.allSettled(toFlip.map((i) => togglePublish(i.id)));
      const ok = results.filter((r) => r.status === "fulfilled").length;
      const failed = results.length - ok;
      if (ok > 0) {
        toast.success(
          target ? t("bulk.published", { count: ok }) : t("bulk.unpublished", { count: ok }),
        );
      }
      if (failed > 0) {
        toast.error(t("bulk.couldntUpdate", { count: failed }));
      }
      router.refresh();
    });
  }

  function runDownload() {
    if (selectedItems.length === 0) return;
    // Trigger each file's download endpoint via a transient anchor. Sequenced
    // with a small stagger so browsers don't drop the queued navigations.
    selectedItems.forEach((item, idx) => {
      const href = buildDownloadUrl(appUrl, userCode, item.code);
      setTimeout(() => {
        const a = document.createElement("a");
        a.href = href;
        a.download = "";
        a.rel = "noopener";
        document.body.appendChild(a);
        a.click();
        a.remove();
      }, idx * 250);
    });
    toast.success(t("bulk.downloading", { count: selectedItems.length }));
  }

  function runTrash() {
    if (selectedIds.length === 0) return;
    // Optimistically remove from the list right away.
    onTrashed(selectedIds);
    onClear();
    startTransition(async () => {
      const results = await Promise.allSettled(selectedIds.map((id) => trashUpload(id)));
      const ok = results.filter((r) => r.status === "fulfilled").length;
      const failed = results.length - ok;
      if (ok > 0) {
        toast.success(t("toast.movedToTrash", { count: ok }));
      }
      if (failed > 0) {
        toast.error(t("toast.couldntMoveToTrash", { count: failed }));
      }
      router.refresh();
    });
  }

  if (count === 0) return null;

  return (
    <>
      <div
        role="region"
        aria-label={t("bulk.region")}
        className="animate-in fade-in slide-in-from-bottom-3 pointer-events-none fixed inset-x-0 bottom-5 z-50 flex justify-center px-4 duration-200"
      >
        <div className="bg-card/90 ring-foreground/10 shadow-foreground/10 pointer-events-auto flex items-center gap-1 rounded-full p-1.5 shadow-xl ring-1 backdrop-blur-md sm:gap-2 sm:pl-2">
          <span className="text-foreground inline-flex items-center gap-2 pr-1 pl-1.5 text-sm font-medium tabular-nums sm:pl-2">
            <span className="bg-primary text-primary-foreground grid size-6 place-items-center rounded-full text-xs font-semibold">
              {count}
            </span>
            <span className="hidden sm:inline">{t("bulk.selected")}</span>
          </span>

          <span className="bg-foreground/10 hidden h-6 w-px sm:block" aria-hidden />

          <div className="flex items-center gap-0.5 sm:gap-1">
            {/* Folder moves are scoped to the signed-in account, so the button
                is absent rather than inert when reviewing someone else's files. */}
            {mode === "owner" ? (
              <Button size="sm" variant="ghost" onClick={() => setMoveOpen(true)} disabled={pending}>
                <FolderInput className="size-4" />
                <span className="hidden md:inline">{t("bulk.move")}</span>
              </Button>
            ) : null}

            {allPublished ? (
              <Button size="sm" variant="ghost" onClick={() => runPublish(false)} disabled={pending}>
                <Lock className="size-4" />
                <span className="hidden md:inline">{t("bulk.unpublish")}</span>
              </Button>
            ) : (
              <Button size="sm" variant="ghost" onClick={() => runPublish(true)} disabled={pending}>
                <Globe className="size-4" />
                <span className="hidden md:inline">{t("bulk.publish")}</span>
              </Button>
            )}

            {convertTargets.length > 0 ? (
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button size="sm" variant="ghost" disabled={pending}>
                    <Replace className="size-4" />
                    <span className="hidden md:inline">{t("convert.action")}</span>
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="center" side="top" className="w-56">
                  <DropdownMenuLabel>{t("convert.targetLabel")}</DropdownMenuLabel>
                  <DropdownMenuSeparator />
                  {convertTargets.map((target) => (
                    <DropdownMenuItem
                      key={target.to}
                      onSelect={() => setTimeout(() => setConvertTo(target.to), 0)}
                    >
                      <span className="flex w-full items-baseline justify-between gap-3">
                        {/* "WEBMP4" is an internal target name, not a format
                            anybody would recognise - see downloadLabelKey. */}
                        <span>{targetLabel(target.to)}</span>
                        <span className="text-muted-foreground text-[11px] tabular-nums">
                          {t("convert.countOf", {
                            n: target.eligible,
                            total: selectedItems.length,
                          })}
                        </span>
                      </span>
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuContent>
              </DropdownMenu>
            ) : null}

            <Button size="sm" variant="ghost" onClick={runDownload} disabled={pending}>
              <Download className="size-4" />
              <span className="hidden md:inline">{t("bulk.download")}</span>
            </Button>

            <Button size="sm" variant="destructive" onClick={runTrash} disabled={pending}>
              {pending ? (
                <Loader2 className="size-4 animate-spin" />
              ) : (
                <Trash2 className="size-4" />
              )}
              <span className="hidden md:inline">{t("bulk.trash")}</span>
            </Button>
          </div>

          <span className="bg-foreground/10 hidden h-6 w-px sm:block" aria-hidden />

          <Button
            size="icon-sm"
            variant="ghost"
            onClick={onClear}
            disabled={pending}
            aria-label={t("bulk.clearSelection")}
            className="rounded-full"
          >
            <X className="size-4" />
          </Button>
        </div>
      </div>

      {/* Mounted while closed too, so "Run in background" keeps the run alive. */}
      <ConvertDialog
        open={convertTo !== null}
        onOpenChange={(open) => {
          if (!open) setConvertTo(null);
        }}
        items={selectedItems}
        to={convertTo ?? ""}
        userCode={userCode}
        appUrl={appUrl}
      />

      {mode === "owner" ? (
        <MoveToFolderDialog
          open={moveOpen}
          onOpenChange={setMoveOpen}
          ids={selectedIds}
          currentFolderId={sharedFolderId ?? currentFolderId ?? null}
          onMoved={(ids, folderId) => {
            onMoved?.(ids, folderId);
            onClear();
          }}
        />
      ) : null}
    </>
  );
}
