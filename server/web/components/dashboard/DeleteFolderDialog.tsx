"use client";

import { useEffect, useState } from "react";
import { useTranslations } from "next-intl";
import { AlertTriangle, FolderMinus, Trash2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

/** Everything the dialog needs to describe the damage, resolved by the caller. */
export interface DeleteFolderTarget {
  id: string;
  name: string;
  /** Files in this folder and every folder nested under it. */
  fileCount: number;
  /** Folders nested under it (excluding itself). */
  subfolderCount: number;
}

/**
 * How long "Delete with contents" stays locked. Long enough that the sentence
 * above it gets read, short enough that someone who meant it isn't punished.
 */
const ARM_MS = 5000;
const TICK_MS = 100;

/** Days a trashed file survives — mirrors TRASH_RETENTION_DAYS. */
const TRASH_RETENTION_DAYS = 30;

/**
 * Confirmation for deleting a folder.
 *
 * Folder deletion used to happen silently on a single click, which is how a
 * library of 8000 screenshots loses its structure by accident. Now the two
 * outcomes are spelled out and chosen explicitly:
 *
 *   - "Delete folder only"  — files fall back to Unsorted. Instant, harmless.
 *   - "Delete with contents" — files go to the **trash** (recoverable, bytes
 *     untouched). Armed on a 5s timer with a filling progress bar.
 */
export function DeleteFolderDialog({
  folder,
  onOpenChange,
  onConfirm,
}: {
  folder: DeleteFolderTarget | null;
  onOpenChange: (open: boolean) => void;
  onConfirm: (id: string, withContents: boolean) => void;
}) {
  const t = useTranslations("dashboard.folders");
  const [elapsed, setElapsed] = useState(0);

  const hasContents = (folder?.fileCount ?? 0) > 0 || (folder?.subfolderCount ?? 0) > 0;
  // An empty folder has nothing to lose, so it doesn't get a waiting period.
  const needsArming = Boolean(folder) && hasContents;

  useEffect(() => {
    setElapsed(0);
    if (!folder || !hasContents) return;
    const start = Date.now();
    const id = window.setInterval(() => {
      const next = Math.min(ARM_MS, Date.now() - start);
      setElapsed(next);
      if (next >= ARM_MS) window.clearInterval(id);
    }, TICK_MS);
    return () => window.clearInterval(id);
  }, [folder, hasContents]);

  const armed = !needsArming || elapsed >= ARM_MS;
  const pct = needsArming ? Math.round((elapsed / ARM_MS) * 100) : 100;
  const secondsLeft = Math.max(1, Math.ceil((ARM_MS - elapsed) / 1000));

  const name = folder?.name ?? "";
  const fileCount = folder?.fileCount ?? 0;
  const subfolderCount = folder?.subfolderCount ?? 0;

  return (
    <Dialog open={Boolean(folder)} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <div className="bg-destructive/10 text-destructive mb-1 grid size-10 place-items-center rounded-lg">
            <AlertTriangle className="size-5" />
          </div>
          <DialogTitle>{t("deleteTitle")}</DialogTitle>
          <DialogDescription>
            {subfolderCount > 0
              ? t.rich("deleteDialog.summary", {
                  name,
                  files: fileCount,
                  folders: subfolderCount,
                  strong: (chunks) => <strong className="text-foreground">{chunks}</strong>,
                })
              : t.rich("deleteDialog.summaryNoSub", {
                  name,
                  files: fileCount,
                  strong: (chunks) => <strong className="text-foreground">{chunks}</strong>,
                })}{" "}
            {t("deleteDialog.warning")}
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-2">
          {/* Safe path — always available, listed first because it's the answer
              nine times out of ten. */}
          <div className="ring-foreground/10 flex flex-col gap-2.5 rounded-lg p-3 ring-1">
            <div className="flex items-start gap-2.5">
              <FolderMinus className="text-muted-foreground mt-0.5 size-4 shrink-0" />
              <div className="min-w-0">
                <p className="text-sm font-medium">{t("deleteDialog.keepTitle")}</p>
                <p className="text-muted-foreground mt-0.5 text-xs leading-relaxed">
                  {t("deleteDialog.keepHint")}
                </p>
              </div>
            </div>
            <Button
              variant="outline"
              size="sm"
              className="w-full sm:w-fit sm:self-end"
              onClick={() => folder && onConfirm(folder.id, false)}
            >
              <FolderMinus className="size-4" /> {t("deleteDialog.keepAction")}
            </Button>
          </div>

          {/* Destructive path — same card shape so the difference is the words,
              not a visual trick, plus the arming timer. */}
          <div className="ring-destructive/25 bg-destructive/5 flex flex-col gap-2.5 rounded-lg p-3 ring-1">
            <div className="flex items-start gap-2.5">
              <Trash2 className="text-destructive mt-0.5 size-4 shrink-0" />
              <div className="min-w-0">
                <p className="text-sm font-medium">{t("deleteDialog.purgeTitle")}</p>
                <p className="text-muted-foreground mt-0.5 text-xs leading-relaxed">
                  {t("deleteDialog.purgeHint", { days: TRASH_RETENTION_DAYS })}
                </p>
              </div>
            </div>

            {needsArming && !armed ? (
              <div className="flex flex-col gap-1.5">
                <Progress
                  value={pct}
                  aria-label={t("deleteDialog.armIn", { seconds: secondsLeft })}
                  className="bg-destructive/15 [&_[data-slot=progress-indicator]]:bg-destructive"
                />
                <p className="text-muted-foreground text-[0.6875rem] tabular-nums">
                  {t("deleteDialog.armIn", { seconds: secondsLeft })}
                </p>
              </div>
            ) : null}

            <Button
              variant="destructive"
              size="sm"
              disabled={!armed}
              aria-disabled={!armed}
              className={cn("w-full sm:w-fit sm:self-end", !armed && "cursor-not-allowed")}
              onClick={() => armed && folder && onConfirm(folder.id, true)}
            >
              <Trash2 className="size-4" /> {t("deleteDialog.purgeAction")}
            </Button>
          </div>
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {t("cancel")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
