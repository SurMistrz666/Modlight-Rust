"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Ghost,
  Loader2,
  RefreshCw,
  Trash2,
  TriangleAlert,
  Archive,
} from "lucide-react";
import { toast } from "sonner";
import { useLocale, useTranslations } from "next-intl";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
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
import type { OrphanRow, OrphanSummary } from "@/lib/data/admin";
import { confirmationMatches } from "@/components/admin/confirm";
import { deleteOrphan, loadOrphans, purgeAllOrphans } from "@/app/actions/admin";

const EMPTY: OrphanSummary = { files: 0, bytes: 0, trashed: 0, oldest: null, newest: null };

function errMessage(error: unknown, fallback: string): string {
  if (error && typeof error === "object" && "message" in error) {
    const m = (error as { message?: unknown }).message;
    if (typeof m === "string" && m) return m;
  }
  return fallback;
}

function SummaryTile({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <Card className="gap-0 p-4">
      <span className="text-muted-foreground text-xs font-medium tracking-wide uppercase">
        {label}
      </span>
      <span className="font-heading mt-2 text-2xl leading-none font-semibold tracking-tight tabular-nums">
        {value}
      </span>
      {hint ? <span className="text-muted-foreground mt-1.5 text-xs">{hint}</span> : null}
    </Card>
  );
}

/**
 * Admin → Orphaned files.
 *
 * A file becomes an orphan when its account is deleted with "keep the files".
 * It is unreachable on the web from that moment (the public URL starts with the
 * owner's tag, and there is no owner), so this screen is the only place it
 * exists — which is exactly why it needs a listing and not just a counter.
 */
export function OrphanFiles() {
  const t = useTranslations("admin.orphans");
  const tc = useTranslations("common");
  // relativeTime formats through Intl, which needs to be told whose language
  // this is; without it a Polish admin reads English ages in a Polish table.
  const locale = useLocale();

  const [summary, setSummary] = useState<OrphanSummary>(EMPTY);
  const [items, setItems] = useState<OrphanRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [busyId, setBusyId] = useState<string | null>(null);

  const [purgeOpen, setPurgeOpen] = useState(false);
  const [confirmation, setConfirmation] = useState("");
  const [purging, setPurging] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const data = await loadOrphans();
      setSummary(data.summary);
      setItems(data.items);
    } catch (error) {
      toast.error(t("loadError"), { description: errMessage(error, t("tryAgain")) });
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    void load();
  }, [load]);

  const confirmed = useMemo(
    () => confirmationMatches(confirmation, summary.files),
    [confirmation, summary.files],
  );

  async function removeOne(row: OrphanRow) {
    setBusyId(row.id);
    try {
      const res = await deleteOrphan(row.id);
      if (!res.ok) {
        toast.error(t("deleteError"), { description: t("deleteGone") });
      } else {
        toast.success(t("deletedToast", { name: row.filename }));
      }
      await load();
    } catch (error) {
      toast.error(t("deleteError"), { description: errMessage(error, t("tryAgain")) });
    } finally {
      setBusyId(null);
    }
  }

  async function purgeEverything() {
    setPurging(true);
    try {
      const res = await purgeAllOrphans(confirmation);
      if (!res.ok) {
        toast.error(t("purgeError"), {
          description: res.reason === "empty" ? t("purgeEmpty") : t("purgeMismatch"),
        });
      } else {
        toast.success(t("purgedTitle"), {
          description: t("purgedDesc", { count: res.deleted, size: humanFileSize(res.bytes) }),
        });
        if (res.failed > 0) {
          toast.warning(t("purgeFailedTitle"), {
            description: t("purgeFailedDesc", { count: res.failed }),
          });
        }
      }
      setPurgeOpen(false);
      setConfirmation("");
      await load();
    } catch (error) {
      toast.error(t("purgeError"), { description: errMessage(error, t("tryAgain")) });
    } finally {
      setPurging(false);
    }
  }

  const capped = summary.files > items.length;

  return (
    <div className="flex flex-col gap-5">
      {/* What this section is — an orphan is an unusual state, so say it plainly. */}
      <div className="border-foreground/10 bg-card flex items-start gap-3 rounded-xl border p-4">
        <span className="bg-amber-500/10 text-amber-500 grid size-9 shrink-0 place-items-center rounded-lg">
          <Ghost className="size-4.5" />
        </span>
        <div className="min-w-0">
          <p className="text-sm font-medium">{t("title")}</p>
          <p className="text-muted-foreground mt-0.5 text-xs leading-relaxed">{t("intro")}</p>
        </div>
      </div>

      {loading && items.length === 0 ? (
        <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
          {Array.from({ length: 4 }).map((_, i) => (
            <Skeleton key={i} className="h-24 rounded-xl" />
          ))}
        </div>
      ) : (
        <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
          <SummaryTile label={t("statFiles")} value={summary.files.toLocaleString()} />
          <SummaryTile label={t("statSize")} value={humanFileSize(summary.bytes)} />
          <SummaryTile
            label={t("statTrashed")}
            value={summary.trashed.toLocaleString()}
            hint={t("statTrashedHint")}
          />
          <SummaryTile
            label={t("statOldest")}
            value={summary.oldest ? relativeTime(summary.oldest, locale) : "-"}
            hint={summary.oldest ? t("statOldestHint") : undefined}
          />
        </div>
      )}

      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <p className="text-muted-foreground text-xs">
          {capped ? t("showingCapped", { shown: items.length, total: summary.files }) : null}
        </p>
        <div className="flex items-center gap-2">
          <Button variant="outline" onClick={() => void load()} disabled={loading}>
            <RefreshCw className={cn("size-4", loading && "animate-spin")} /> {t("refresh")}
          </Button>
          <Button
            variant="destructive"
            disabled={loading || summary.files === 0}
            onClick={() => {
              setConfirmation("");
              setPurgeOpen(true);
            }}
          >
            <Trash2 className="size-4" /> {t("purgeAll")}
          </Button>
        </div>
      </div>

      <div className="ring-foreground/10 overflow-x-auto rounded-xl ring-1">
        <Table>
          <TableHeader>
            <TableRow className="bg-muted/40 hover:bg-muted/40">
              <TableHead className="pl-4">{t("colFile")}</TableHead>
              <TableHead className="hidden md:table-cell">{t("colCode")}</TableHead>
              <TableHead className="text-right">{t("colSize")}</TableHead>
              <TableHead className="hidden lg:table-cell">{t("colUploaded")}</TableHead>
              <TableHead className="pr-4 text-right">{t("colActions")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading ? (
              Array.from({ length: 4 }).map((_, i) => (
                <TableRow key={i} className="hover:bg-transparent">
                  <TableCell className="pl-4"><Skeleton className="h-3 w-48" /></TableCell>
                  <TableCell className="hidden md:table-cell">
                    <Skeleton className="h-5 w-20 rounded-md" />
                  </TableCell>
                  <TableCell><Skeleton className="ml-auto h-3 w-14" /></TableCell>
                  <TableCell className="hidden lg:table-cell"><Skeleton className="h-3 w-16" /></TableCell>
                  <TableCell className="pr-4 text-right">
                    <Skeleton className="ml-auto size-7 rounded-md" />
                  </TableCell>
                </TableRow>
              ))
            ) : items.length === 0 ? (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={5} className="py-14 text-center whitespace-normal">
                  <div className="text-muted-foreground mx-auto flex max-w-sm flex-col items-center gap-2">
                    <span className="bg-muted grid size-11 place-items-center rounded-full">
                      <Ghost className="size-5" />
                    </span>
                    <p className="text-foreground text-sm font-medium">{t("empty")}</p>
                    <p className="text-xs leading-relaxed">{t("emptyHint")}</p>
                  </div>
                </TableCell>
              </TableRow>
            ) : (
              items.map((row) => (
                <TableRow key={row.id}>
                  <TableCell className="pl-4">
                    <div className="flex min-w-0 items-center gap-2">
                      <span className="truncate text-sm font-medium" title={row.filename}>
                        {row.filename}
                      </span>
                      {row.deletedAt ? (
                        <Badge variant="outline" className="text-muted-foreground h-5 gap-1 px-1.5">
                          <Archive className="size-3" /> {t("inTrash")}
                        </Badge>
                      ) : null}
                    </div>
                    <span className="text-muted-foreground block truncate text-xs">
                      {row.mimeType}
                    </span>
                  </TableCell>
                  <TableCell className="hidden md:table-cell">
                    <code className="bg-muted text-foreground/80 rounded-md px-1.5 py-0.5 font-mono text-xs">
                      {row.code}
                    </code>
                  </TableCell>
                  <TableCell className="text-right text-sm tabular-nums">
                    {humanFileSize(row.size)}
                  </TableCell>
                  <TableCell className="text-muted-foreground hidden text-xs lg:table-cell">
                    {row.createdAt ? relativeTime(row.createdAt, locale) : "-"}
                  </TableCell>
                  <TableCell className="pr-4 text-right">
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      disabled={busyId === row.id}
                      onClick={() => void removeOne(row)}
                    >
                      {busyId === row.id ? (
                        <Loader2 className="size-4 animate-spin" />
                      ) : (
                        <Trash2 className="text-destructive size-4" />
                      )}
                      <span className="sr-only">{t("deleteOne", { name: row.filename })}</span>
                    </Button>
                  </TableCell>
                </TableRow>
              ))
            )}
          </TableBody>
        </Table>
      </div>

      {/* Purge-everything dialog. The gate is retyping the exact file count —
          a number the admin has to have read to know. */}
      <Dialog open={purgeOpen} onOpenChange={(o) => !purging && setPurgeOpen(o)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("purgeTitle")}</DialogTitle>
            <DialogDescription>
              {t("purgeDesc", {
                count: summary.files,
                size: humanFileSize(summary.bytes),
              })}
            </DialogDescription>
          </DialogHeader>

          <div className="flex flex-col gap-3 py-1">
            <div className="border-destructive/30 bg-destructive/5 flex items-start gap-2.5 rounded-lg border p-3">
              <TriangleAlert className="text-destructive mt-0.5 size-4 shrink-0" />
              <p className="text-muted-foreground text-xs leading-relaxed">{t("purgeWarning")}</p>
            </div>
            <div className="flex flex-col gap-2">
              <Label htmlFor="orphan-confirm">
                {t("purgeConfirmLabel", { count: summary.files })}
              </Label>
              <Input
                id="orphan-confirm"
                value={confirmation}
                inputMode="numeric"
                autoComplete="off"
                spellCheck={false}
                className="font-mono"
                placeholder={String(summary.files)}
                onChange={(e) => setConfirmation(e.target.value)}
              />
            </div>
          </div>

          <DialogFooter>
            <DialogClose asChild>
              <Button variant="outline" disabled={purging}>
                {tc("cancel")}
              </Button>
            </DialogClose>
            <Button
              variant="destructive"
              disabled={!confirmed || purging}
              onClick={() => void purgeEverything()}
            >
              {purging ? <Loader2 className="size-4 animate-spin" /> : <Trash2 className="size-4" />}
              {purging ? t("purging") : t("purgeCta", { count: summary.files })}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
