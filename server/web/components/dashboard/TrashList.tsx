"use client";

import { useRouter } from "next/navigation";
import { useEffect, useRef, useState, useTransition } from "react";
import { useLocale, useTranslations } from "next-intl";
import { Trash2, RotateCcw, AlertTriangle, Clock, Loader2, Sparkles } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { humanFileSize, relativeTime } from "@/lib/format";
import {
  restoreUpload,
  deleteUploadForever,
  emptyTrash,
  trashSummary,
} from "@/app/actions/uploads";
import { Thumbnail } from "./Thumbnail";
import { buildRawUrl, isImage, kindIcon, kindTint } from "./media";
import type { FileItem } from "./types";

/**
 * A trashed file carries one extra fact the library view doesn't need: how long
 * it has left. `daysLeft` is resolved on the server so both renders agree —
 * deriving it from Date.now() inside a client component would hydrate wrong
 * across a midnight boundary.
 */
export interface TrashItem extends FileItem {
  daysLeft: number | null;
}

export function TrashList({
  items,
  userCode,
  appUrl,
  retentionDays,
}: {
  items: TrashItem[];
  userCode: string;
  appUrl: string;
  /** Days an item survives in the trash before the sweeper destroys it. */
  retentionDays: number;
}) {
  const t = useTranslations("dashboard");
  // relativeTime formats through Intl, so it has to be told whose language
  // this is; without it a Polish reader gets English ages.
  const locale = useLocale();
  const tc = useTranslations("common");
  const router = useRouter();
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const [confirm, setConfirm] = useState<TrashItem | null>(null);
  const [confirmEmpty, setConfirmEmpty] = useState(false);
  /** What emptying would destroy: null until the count comes back. */
  const [totals, setTotals] = useState<{ files: number; bytes: number } | null>(null);
  const [countFailed, setCountFailed] = useState(false);
  const [pending, startTransition] = useTransition();
  const cancelEmptyRef = useRef<HTMLButtonElement>(null);
  /**
   * Which count request the dialog is currently showing.
   *
   * Closing and reopening starts a second count, and the first one can land
   * after it. Without this the dialog would quote a number older than the one
   * it just asked for - in the one dialog whose entire job is to state a true
   * number before destroying anything.
   */
  const countRun = useRef(0);

  useEffect(() => setHidden(new Set()), [items]);
  const visible = items.filter((i) => !hidden.has(i.id));

  function restore(item: TrashItem) {
    setHidden((prev) => new Set(prev).add(item.id));
    startTransition(async () => {
      try {
        await restoreUpload(item.id);
        toast.success(t("trash.fileRestored"));
        router.refresh();
      } catch {
        toast.error(t("trash.couldntRestore"));
        router.refresh();
      }
    });
  }

  function deleteForever(item: TrashItem) {
    setHidden((prev) => new Set(prev).add(item.id));
    setConfirm(null);
    startTransition(async () => {
      try {
        await deleteUploadForever(item.id);
        toast.success(t("trash.deletedPermanently"));
        router.refresh();
      } catch {
        toast.error(t("trash.couldntDelete"));
        router.refresh();
      }
    });
  }

  /**
   * Open the confirmation, and only then go and count.
   *
   * The number is read from the database at this moment rather than taken from
   * `items`, which is one page of the trash: an account with 900 trashed files
   * sees 120 of them here, and "120 items" on a dialog that destroys 900 is a
   * lie in the one place it matters most. Until the count arrives the
   * destructive button stays disabled, so there is no window in which a
   * mis-click can act on a number nobody has read.
   */
  function openEmpty() {
    const run = ++countRun.current;
    setTotals(null);
    setCountFailed(false);
    setConfirmEmpty(true);
    // `userCode` is whose trash this list is showing - the signed-in account on
    // /dashboard/trash, the target account in an admin's view of a library. The
    // server decides whether this caller may reach that tag at all.
    trashSummary(userCode)
      .then((res) => {
        if (countRun.current === run) setTotals(res);
      })
      .catch(() => {
        if (countRun.current === run) setCountFailed(true);
      });
  }

  function empty() {
    setConfirmEmpty(false);
    setHidden(new Set(items.map((i) => i.id)));
    startTransition(async () => {
      try {
        const res = await emptyTrash(userCode);
        // Report what actually happened, not what was asked for. Somebody else
        // can empty this trash between the count and the click - an admin, the
        // thirty-day sweep, the owner in another tab - and "0 items deleted"
        // announced as a success reads as a failure nobody explained.
        if (res.deleted > 0) {
          toast.success(
            t("trash.emptiedWithSize", {
              count: res.deleted,
              size: humanFileSize(res.bytes),
            }),
          );
        } else if (res.failed === 0) {
          toast.success(t("trash.alreadyEmpty"));
        }
        // Rows storage refused to release are still there. Saying so is the
        // difference between "the trash is empty" and a list that comes back
        // half full for no stated reason.
        if (res.failed > 0) {
          toast.warning(t("trash.emptyPartialTitle"), {
            description: t("trash.emptyPartialDesc", { count: res.failed }),
          });
        }
        router.refresh();
      } catch {
        toast.error(t("trash.couldntEmpty"));
        router.refresh();
      }
    });
  }

  if (visible.length === 0) {
    return (
      <div className="bg-card/40 ring-foreground/10 flex flex-col items-center justify-center gap-3 rounded-xl px-6 py-20 text-center ring-1">
        <div className="bg-muted text-muted-foreground grid size-14 place-items-center rounded-2xl">
          <Sparkles className="size-7" />
        </div>
        <h3 className="text-base font-medium">{t("trash.emptyTitle")}</h3>
        <p className="text-muted-foreground max-w-sm text-sm">{t("trash.emptyDesc")}</p>
      </div>
    );
  }

  return (
    <>
      {/* The retention promise sits directly above the list it applies to. The
          control that overrides it deliberately does NOT sit here: this row is
          one line above the first "Restore" button, and a permanent delete for
          the whole trash should never be a near miss for restoring one file. */}
      <p className="text-muted-foreground flex items-center gap-1.5 text-xs">
        <Clock className="size-3.5 shrink-0" />
        {t("trash.retentionNotice", { days: retentionDays })}
      </p>

      <div className="bg-card ring-foreground/10 divide-border flex flex-col divide-y overflow-hidden rounded-xl ring-1">
        {visible.map((item) => {
          const image = isImage(item.mimeType, item.ext);
          const rawUrl = buildRawUrl(appUrl, userCode, item.code, item.ext);
          const Icon = kindIcon(item.mimeType);
          const urgent = item.daysLeft != null && item.daysLeft <= 3;
          return (
            <div key={item.id} className="flex items-center gap-3 p-3">
              <div className="size-10 shrink-0 overflow-hidden rounded-md">
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
                      "grid size-10 place-items-center rounded-md",
                      kindTint(item.mimeType),
                    )}
                  >
                    <Icon className="size-4" />
                  </div>
                )}
              </div>

              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-medium" title={item.filename}>
                  {item.filename}
                </p>
                <p className="text-muted-foreground text-xs">
                  {humanFileSize(item.size)} ·{" "}
                  {t("trash.uploadedAgo", { time: relativeTime(item.createdAt, locale) })}
                </p>
              </div>

              {item.daysLeft != null ? (
                <span
                  className={cn(
                    "hidden shrink-0 rounded-full px-2 py-0.5 text-[0.6875rem] font-medium tabular-nums sm:inline-block",
                    urgent
                      ? "bg-destructive/10 text-destructive"
                      : "bg-muted text-muted-foreground",
                  )}
                >
                  {t("trash.daysLeft", { count: item.daysLeft })}
                </span>
              ) : null}

              <div className="flex shrink-0 items-center gap-1.5">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => restore(item)}
                  disabled={pending}
                >
                  <RotateCcw className="size-3.5" />
                  <span className="hidden sm:inline">{t("trash.restore")}</span>
                </Button>
                <Button
                  variant="destructive"
                  size="sm"
                  onClick={() => setConfirm(item)}
                  disabled={pending}
                >
                  <Trash2 className="size-3.5" />
                  <span className="hidden sm:inline">{t("trash.delete")}</span>
                </Button>
              </div>
            </div>
          );
        })}
      </div>

      {/* Emptying the trash lives below everything it destroys, past the last
          row, with its own frame and its own sentence. Reaching it is a
          decision; the per-file buttons above are the ones you reach by
          reflex. Hidden without a tag, because there is then no trash this
          component can name and the count would only ever fail. */}
      {userCode ? (
        <div className="border-destructive/25 bg-destructive/5 flex flex-wrap items-center justify-between gap-3 rounded-xl border border-dashed px-4 py-3">
          <div className="min-w-0">
            <p className="text-sm font-medium">{t("trash.dangerTitle")}</p>
            <p className="text-muted-foreground text-xs">{t("trash.dangerDesc")}</p>
          </div>
          <Button
            variant="outline"
            size="sm"
            onClick={openEmpty}
            disabled={pending}
            className="text-destructive hover:text-destructive shrink-0"
          >
            <Trash2 className="size-3.5" /> {t("trash.emptyTrash")}
          </Button>
        </div>
      ) : null}

      <Dialog open={Boolean(confirm)} onOpenChange={(o) => !o && setConfirm(null)}>
        <DialogContent>
          <DialogHeader>
            <div className="bg-destructive/10 text-destructive mb-1 grid size-10 place-items-center rounded-lg">
              <AlertTriangle className="size-5" />
            </div>
            <DialogTitle>{t("trash.deleteForeverTitle")}</DialogTitle>
            <DialogDescription>
              {t.rich("trash.deleteForeverDesc", {
                filename: confirm?.filename ?? "",
                strong: (chunks) => (
                  <span className="text-foreground font-medium">{chunks}</span>
                ),
              })}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter showCloseButton>
            <Button
              variant="destructive"
              onClick={() => confirm && deleteForever(confirm)}
              disabled={pending}
            >
              {pending ? (
                <Loader2 className="size-4 animate-spin" />
              ) : (
                <Trash2 className="size-4" />
              )}
              {t("trash.deleteForever")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={confirmEmpty} onOpenChange={setConfirmEmpty}>
        <DialogContent
          // Focus lands on "Cancel", never on the button that destroys. Radix
          // would otherwise focus whatever comes first in the dialog, and a
          // stray Enter or Space arriving right after the click that opened
          // this is exactly the accident the confirmation exists to prevent.
          onOpenAutoFocus={(event) => {
            event.preventDefault();
            cancelEmptyRef.current?.focus();
          }}
        >
          <DialogHeader>
            <div className="bg-destructive/10 text-destructive mb-1 grid size-10 place-items-center rounded-lg">
              <AlertTriangle className="size-5" />
            </div>
            <DialogTitle>{t("trash.emptyTrashTitle")}</DialogTitle>
            <DialogDescription>
              {countFailed
                ? t("trash.countFailed")
                : totals === null
                  ? t("trash.counting")
                  : totals.files === 0
                    ? t("trash.alreadyEmpty")
                    : t("trash.emptyTrashDescReal", {
                        count: totals.files,
                        size: humanFileSize(totals.bytes),
                      })}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <DialogClose asChild>
              <Button ref={cancelEmptyRef} variant="outline" disabled={pending}>
                {tc("cancel")}
              </Button>
            </DialogClose>
            <Button
              variant="destructive"
              onClick={empty}
              // Nothing to confirm until there is a real number to confirm.
              disabled={pending || countFailed || totals === null || totals.files === 0}
            >
              {pending || (totals === null && !countFailed) ? (
                <Loader2 className="size-4 animate-spin" />
              ) : (
                <Trash2 className="size-4" />
              )}
              {totals && totals.files > 0
                ? t("trash.emptyTrashCta", { count: totals.files })
                : t("trash.emptyTrash")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
