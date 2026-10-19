"use client";

import { useEffect, useMemo, useRef, useState, useTransition } from "react";
import { useRouter } from "next/navigation";
import { CopyCheck, Loader2, RotateCcw, ScanSearch, ShieldCheck, ShieldOff, Trash2, TriangleAlert } from "lucide-react";
import { toast } from "sonner";
import { useTranslations } from "next-intl";
import { Button } from "@/components/ui/button";
import { humanFileSize } from "@/lib/format";
import { cn } from "@/lib/utils";
import {
  defaultSelection,
  selectedIds,
  selectionTotals,
  toggleSelection,
} from "@/components/profile/duplicate-selection";
import {
  countDuplicateSetsAgain,
  keepDuplicateSets,
  trashDuplicates,
} from "@/app/actions/duplicates";
import { DuplicateGroupCard } from "./DuplicateGroupCard";
import { dismissalsFor, keepEverything, keepOnly, splitByDismissed } from "./selection";
import type { DuplicateGroup, DuplicateScan } from "@/lib/data/duplicates";

/**
 * The duplicate cleanup screen.
 *
 * Three promises hold the whole thing up, and every decision below serves one
 * of them:
 *
 *  1. Nothing happens until a button that says what it will do is pressed. The
 *     scan is read-only, the checkboxes are a proposal, and the delete is a
 *     move to the trash where a file sits restorable for thirty days.
 *  2. Every set keeps a copy by default, and the screen says so loudly when the
 *     current selection would not.
 *  3. "I want these copies" is an answer the screen accepts and remembers, so
 *     the reminder can stop asking. That is what separates this from a nag.
 *
 * The scan arrives as a prop from the server component, rather than being
 * fetched on mount: the page exists to show duplicates, so an empty screen with
 * a "scan" button in the middle of it would be a click charged for nothing.
 * Every mutation ends in `router.refresh()`, which re-runs the page and hands a
 * fresh scan down - the list on screen describes a library that no longer
 * exists the moment anything is trashed or kept.
 */
export function DuplicatesScreen({
  scan: initial,
  userCode,
}: {
  scan: DuplicateScan;
  userCode: string;
}) {
  const t = useTranslations("profile.duplicates");
  const router = useRouter();

  const [scan, setScan] = useState(initial);
  const [selected, setSelected] = useState<ReadonlySet<string>>(() =>
    proposalFor(initial.groups),
  );
  const [pending, start] = useTransition();
  const [refreshing, startRefresh] = useTransition();

  /**
   * Re-seed when the server hands down a new scan.
   *
   * Keyed on `scannedAt` rather than on the object identity: a re-render for
   * any other reason would otherwise throw away a selection the user is in the
   * middle of making.
   */
  const seededAt = useRef(initial.scannedAt);
  useEffect(() => {
    if (seededAt.current === initial.scannedAt) return;
    seededAt.current = initial.scannedAt;
    setScan(initial);
    setSelected(proposalFor(initial.groups));
  }, [initial]);

  const { counted, kept } = useMemo(() => splitByDismissed(scan.groups), [scan.groups]);
  const totals = useMemo(() => selectionTotals(scan.groups, selected), [scan.groups, selected]);
  const busy = pending || refreshing;

  /** Bytes and sets the owner has taken out of the count on purpose. */
  const excludedBytes = Math.max(0, scan.reclaimable - scan.active.reclaimable);

  function refresh() {
    startRefresh(() => router.refresh());
  }

  function remove() {
    const ids = selectedIds(scan.groups, selected);
    if (ids.length === 0 || busy) return;

    start(async () => {
      const result = await trashDuplicates(ids);
      if (!result.ok) {
        toast.error(t(`error.${result.error}`));
        return;
      }
      toast.success(t("trashed", { count: result.trashed, size: humanFileSize(result.bytes) }));
      router.refresh();
    });
  }

  function keep(groups: readonly DuplicateGroup[]) {
    if (groups.length === 0 || busy) return;

    start(async () => {
      const result = await keepDuplicateSets(dismissalsFor(groups));
      if (!result.ok) {
        toast.error(t(`error.${result.error}`));
        return;
      }

      // Copies the owner has just said are meant to exist must not stay ticked
      // for the trash button. Done locally as well as by the refresh, because
      // the refresh is a round trip and the checkboxes are on screen now.
      setSelected((prev) => {
        let next = prev;
        for (const group of groups) next = keepEverything(next, group);
        return next;
      });

      toast.success(t("keptToast", { count: result.kept }), {
        // Undo, because "stop reminding me about these" is a decision worth
        // being able to take back in the same second it was made.
        action: {
          label: t("undo"),
          onClick: () => countAgain(result.checksums),
        },
      });
      router.refresh();
    });
  }

  function countAgain(checksums: string[]) {
    if (checksums.length === 0) return;

    start(async () => {
      const result = await countDuplicateSetsAgain(checksums);
      if (!result.ok) {
        toast.error(t(`error.${result.error}`));
        return;
      }
      toast.success(t("countAgainToast", { count: result.restored }));
      router.refresh();
    });
  }

  return (
    <div className="space-y-5">
      {/* Why the answer can be trusted, stated before the answer. */}
      <div className="border-primary/25 bg-primary/5 flex items-start gap-2.5 rounded-xl border px-3.5 py-3">
        <ShieldCheck className="text-primary mt-0.5 size-4 shrink-0" />
        <div className="space-y-0.5">
          <p className="text-sm font-medium">{t("byChecksumTitle")}</p>
          <p className="text-muted-foreground text-xs leading-relaxed">{t("byChecksumBody")}</p>
        </div>
      </div>

      <div className="grid gap-3 sm:grid-cols-3">
        <Figure label={t("stats.groups")} value={scan.totalGroups.toLocaleString()} />
        <Figure label={t("stats.files")} value={scan.totalFiles.toLocaleString()} />
        <Figure
          label={t("stats.reclaimable")}
          value={humanFileSize(scan.reclaimable)}
          accent={scan.reclaimable > 0}
        />
      </div>

      <div className="text-muted-foreground space-y-1 text-xs leading-relaxed">
        {scan.unchecksummed > 0 && <p>{t("unchecksummed", { count: scan.unchecksummed })}</p>}
        {scan.dismissedGroups > 0 && (
          <p>
            {t("keptExcluded", {
              count: scan.dismissedGroups,
              size: humanFileSize(excludedBytes),
            })}
          </p>
        )}
        {scan.truncated && (
          <p>{t("truncated", { shown: scan.groups.length, total: scan.totalGroups })}</p>
        )}
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Button type="button" variant="outline" size="sm" disabled={busy} onClick={refresh}>
          {refreshing ? (
            <Loader2 className="size-4 animate-spin" />
          ) : (
            <ScanSearch className="size-4" />
          )}
          {t("rescan")}
        </Button>

        {counted.length > 0 && (
          <>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              disabled={busy}
              onClick={() => setSelected(proposalFor(scan.groups))}
            >
              <RotateCcw className="size-4" />
              {t("resetSelection")}
            </Button>

            <Button
              type="button"
              variant="ghost"
              size="sm"
              disabled={busy}
              onClick={() => keep(counted)}
            >
              <ShieldOff className="size-4" />
              {t("keepAll")}
            </Button>
          </>
        )}
      </div>

      {scan.groups.length === 0 ? (
        <div className="ring-foreground/10 flex flex-col items-center gap-2 rounded-xl px-6 py-14 text-center ring-1">
          <CopyCheck className="text-muted-foreground size-6" />
          <p className="text-sm font-medium">{t("none")}</p>
        </div>
      ) : (
        <div className="space-y-3">
          {counted.map((group) => (
            <DuplicateGroupCard
              key={group.checksum}
              group={group}
              selected={selected}
              userCode={userCode}
              disabled={busy}
              onToggle={(id) => setSelected((prev) => toggleSelection(prev, id))}
              onKeepOnly={(id) => setSelected((prev) => keepOnly(prev, group, id))}
              onKeepSet={() => keep([group])}
              onCountAgain={() => countAgain([group.checksum])}
            />
          ))}
        </div>
      )}

      {kept.length > 0 && (
        <section className="space-y-3 pt-2">
          <div className="space-y-0.5">
            <h2 className="font-heading text-lg font-semibold tracking-tight">{t("keptHeading")}</h2>
            <p className="text-muted-foreground text-xs leading-relaxed">{t("keptNote")}</p>
          </div>

          {kept.map((group) => (
            <DuplicateGroupCard
              key={group.checksum}
              group={group}
              selected={selected}
              userCode={userCode}
              disabled={busy}
              onToggle={(id) => setSelected((prev) => toggleSelection(prev, id))}
              onKeepOnly={(id) => setSelected((prev) => keepOnly(prev, group, id))}
              onKeepSet={() => keep([group])}
              onCountAgain={() => countAgain([group.checksum])}
            />
          ))}
        </section>
      )}

      {/* The bar that does the destroying. Sticky, because a hundred sets is a
          long scroll and the count of what is ticked has to stay readable next
          to the button that acts on it. */}
      {scan.groups.length > 0 && (
        <div className="bg-card/95 ring-foreground/10 sticky bottom-4 flex flex-wrap items-center justify-between gap-3 rounded-xl px-4 py-3 shadow-lg ring-1 backdrop-blur">
          <div className="space-y-0.5 text-xs">
            <p className="font-medium tabular-nums">
              {t("selection", { count: totals.files, size: humanFileSize(totals.bytes) })}
            </p>
            {totals.emptied > 0 ? (
              <p className="text-destructive flex items-start gap-1.5">
                <TriangleAlert className="mt-0.5 size-3.5 shrink-0" />
                {t("emptiedWarning", { count: totals.emptied })}
              </p>
            ) : (
              <p className="text-muted-foreground">{t("trashNote")}</p>
            )}
          </div>

          <Button
            type="button"
            variant="destructive"
            size="sm"
            disabled={busy || totals.files === 0}
            onClick={remove}
          >
            {pending ? <Loader2 className="size-4 animate-spin" /> : <Trash2 className="size-4" />}
            {t("trashSelected", { count: totals.files })}
          </Button>
        </div>
      )}
    </div>
  );
}

/**
 * What the screen proposes: keep the oldest copy of every set STILL BEING
 * COUNTED, and touch nothing in a set the owner has already said to keep.
 *
 * `defaultSelection` on its own would tick the later copies of a dismissed set
 * too, which would put "these are meant to exist" and "12 copies selected for
 * the trash" on the same screen at the same time.
 */
function proposalFor(groups: readonly DuplicateGroup[]): Set<string> {
  return defaultSelection(splitByDismissed(groups).counted);
}

function Figure({ label, value, accent }: { label: string; value: string; accent?: boolean }) {
  return (
    <div className="bg-muted/40 ring-foreground/5 flex flex-col justify-center gap-1 rounded-lg px-3 py-2.5 ring-1">
      <div className="text-muted-foreground text-xs font-medium">{label}</div>
      <div
        className={cn(
          "font-heading text-lg font-semibold tabular-nums tracking-tight",
          accent && "text-primary",
        )}
      >
        {value}
      </div>
    </div>
  );
}
