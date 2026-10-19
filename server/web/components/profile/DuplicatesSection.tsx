"use client";

import { useMemo, useState, useTransition } from "react";
import { useRouter } from "next/navigation";
import {
  CopyCheck,
  Eye,
  ExternalLink,
  Folder,
  Loader2,
  RotateCcw,
  ScanSearch,
  ShieldCheck,
  Trash2,
  TriangleAlert,
} from "lucide-react";
import { toast } from "sonner";
import { useFormatter, useTranslations } from "next-intl";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import { Separator } from "@/components/ui/separator";
import { KindIcon, extLabel, kindFromMime, kindTint } from "@/components/dashboard/media";
import { humanFileSize } from "@/lib/format";
import { cn } from "@/lib/utils";
import { scanMyDuplicates, trashDuplicates } from "@/app/actions/duplicates";
import type { DuplicateGroup, DuplicateScan } from "@/lib/data/duplicates";
import {
  defaultSelection,
  keepsACopy,
  selectedIds,
  selectionTotals,
  toggleSelection,
} from "./duplicate-selection";

/**
 * "Find duplicates" — retroactive, by checksum, on files that are already here.
 *
 * The screen is built around one promise: nothing happens until a button that
 * says what it will do is clicked. The scan is read-only, the checkboxes are a
 * proposal, and the delete is a move to the trash. That is deliberate for a
 * feature whose worst case is destroying the only copy of something.
 */
export function DuplicatesSection() {
  const t = useTranslations("profile.duplicates");
  const router = useRouter();

  const [scan, setScan] = useState<DuplicateScan | null>(null);
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [scanning, startScan] = useTransition();
  const [deleting, startDelete] = useTransition();

  const groups = scan?.groups ?? [];
  const totals = useMemo(() => selectionTotals(groups, selected), [groups, selected]);
  const busy = scanning || deleting;

  function run(after?: (found: DuplicateScan) => void) {
    startScan(async () => {
      const result = await scanMyDuplicates();
      if (!result.ok) {
        toast.error(t(`error.${result.error}`));
        return;
      }
      setScan(result.scan);
      setSelected(defaultSelection(result.scan.groups));
      after?.(result.scan);
    });
  }

  function remove() {
    const ids = selectedIds(groups, selected);
    if (ids.length === 0 || busy) return;

    startDelete(async () => {
      const result = await trashDuplicates(ids);
      if (!result.ok) {
        toast.error(t(`error.${result.error}`));
        return;
      }
      toast.success(
        t("trashed", { count: result.trashed, size: humanFileSize(result.bytes) }),
      );
      // The quota card and the sidebar counts both moved; and the list on screen
      // now describes a library that no longer exists, so it is re-read rather
      // than patched. A scan is three queries.
      router.refresh();
      run();
    });
  }

  return (
    <Card className="xl:col-span-2">
      <CardHeader className="border-b">
        <CardTitle className="flex items-center gap-2">
          <CopyCheck className="text-primary size-4" />
          {t("title")}
        </CardTitle>
        <CardDescription>{t("desc")}</CardDescription>
      </CardHeader>

      <CardContent className="flex-1 space-y-4">
        {/* Why the answer can be trusted, stated before the answer arrives. */}
        <div className="border-primary/25 bg-primary/5 flex items-start gap-2.5 rounded-lg border px-3 py-2.5">
          <ShieldCheck className="text-primary mt-0.5 size-4 shrink-0" />
          <div className="space-y-0.5">
            <p className="text-sm font-medium">{t("byChecksumTitle")}</p>
            <p className="text-muted-foreground text-xs leading-relaxed">{t("byChecksumBody")}</p>
          </div>
        </div>

        {!scan && !scanning && <p className="text-muted-foreground text-sm">{t("idle")}</p>}

        {scanning && !scan && (
          <p className="text-muted-foreground flex items-center gap-2 text-sm">
            <Loader2 className="size-4 animate-spin" />
            {t("scanning")}
          </p>
        )}

        {scan && (
          <>
            <ScanSummary scan={scan} />

            {groups.length === 0 ? (
              <p className="text-muted-foreground text-sm">{t("none")}</p>
            ) : (
              <div className="space-y-3">
                {groups.map((group) => (
                  <GroupCard
                    key={group.checksum}
                    group={group}
                    selected={selected}
                    disabled={busy}
                    onToggle={(id) => setSelected((prev) => toggleSelection(prev, id))}
                  />
                ))}
              </div>
            )}

            {scan.truncated && (
              <p className="text-muted-foreground text-xs">
                {t("truncated", { shown: groups.length, total: scan.totalGroups })}
              </p>
            )}

            {totals.emptied > 0 && (
              <p className="text-destructive flex items-start gap-1.5 text-xs leading-relaxed">
                <TriangleAlert className="mt-0.5 size-3.5 shrink-0" />
                {t("emptiedWarning", { count: totals.emptied })}
              </p>
            )}
          </>
        )}
      </CardContent>

      <CardFooter className="flex-wrap items-center justify-between gap-3 border-t">
        {/* The recoverability note stays put even once a selection exists: the
            moment it matters most is the moment before the destructive button
            is pressed, which is exactly when a selection is on screen. */}
        <div className="space-y-0.5 text-xs">
          {scan && groups.length > 0 && (
            <p className="font-medium tabular-nums">
              {t("selection", { count: totals.files, size: humanFileSize(totals.bytes) })}
            </p>
          )}
          <p className="text-muted-foreground">{t("trashNote")}</p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {scan && groups.length > 0 && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              disabled={busy}
              onClick={() => setSelected(defaultSelection(groups))}
            >
              <RotateCcw className="size-4" />
              {t("resetSelection")}
            </Button>
          )}

          <Button type="button" variant="outline" size="sm" disabled={busy} onClick={() => run()}>
            {scanning ? <Loader2 className="size-4 animate-spin" /> : <ScanSearch className="size-4" />}
            {scan ? t("rescan") : t("scan")}
          </Button>

          {scan && groups.length > 0 && (
            <Button
              type="button"
              variant="destructive"
              size="sm"
              disabled={busy || totals.files === 0}
              onClick={remove}
            >
              {deleting ? <Loader2 className="size-4 animate-spin" /> : <Trash2 className="size-4" />}
              {t("trashSelected", { count: totals.files })}
            </Button>
          )}
        </div>
      </CardFooter>
    </Card>
  );
}

/** The three numbers the page exists to produce. */
function ScanSummary({ scan }: { scan: DuplicateScan }) {
  const t = useTranslations("profile.duplicates");

  return (
    <div className="space-y-2">
      <div className="grid gap-3 sm:grid-cols-3">
        <Figure label={t("stats.groups")} value={scan.totalGroups.toLocaleString()} />
        <Figure label={t("stats.files")} value={scan.totalFiles.toLocaleString()} />
        <Figure
          label={t("stats.reclaimable")}
          value={humanFileSize(scan.reclaimable)}
          accent={scan.reclaimable > 0}
        />
      </div>

      {scan.unchecksummed > 0 && (
        <p className="text-muted-foreground text-xs leading-relaxed">
          {t("unchecksummed", { count: scan.unchecksummed })}
        </p>
      )}
    </div>
  );
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

function GroupCard({
  group,
  selected,
  disabled,
  onToggle,
}: {
  group: DuplicateGroup;
  selected: ReadonlySet<string>;
  disabled: boolean;
  onToggle: (id: string) => void;
}) {
  const t = useTranslations("profile.duplicates");
  const format = useFormatter();
  const kind = kindFromMime(group.files[0]?.mimeType ?? "", group.files[0]?.ext);
  const safe = keepsACopy(group, selected);

  return (
    <div className="ring-foreground/10 overflow-hidden rounded-lg ring-1">
      <div className="bg-muted/40 flex flex-wrap items-center gap-x-3 gap-y-1 px-3 py-2">
        <span className={cn("flex size-7 items-center justify-center rounded-md", kindTint(kind))}>
          <KindIcon kind={kind} mime={group.files[0]?.mimeType} className="size-4" />
        </span>
        <span className="text-sm font-medium">{t("copies", { count: group.copies })}</span>
        <span className="text-muted-foreground text-xs tabular-nums">
          {t("each", { size: humanFileSize(group.size) })}
        </span>
        <span className="ml-auto text-xs font-medium tabular-nums">
          {t("saves", { size: humanFileSize(group.reclaimable) })}
        </span>
      </div>

      <Separator />

      <ul className="divide-foreground/5 divide-y">
        {group.files.map((file, index) => {
          const checked = selected.has(file.id);
          const oldest = index === 0;
          return (
            <li
              key={file.id}
              className={cn(
                "flex items-center gap-3 px-3 py-2 text-sm transition-colors",
                checked && "bg-destructive/5",
              )}
            >
              <Checkbox
                id={`dup-${file.id}`}
                checked={checked}
                disabled={disabled}
                onCheckedChange={() => onToggle(file.id)}
                aria-label={t("removeAria", { filename: file.filename })}
              />

              <label
                htmlFor={`dup-${file.id}`}
                className="min-w-0 flex-1 cursor-pointer space-y-0.5"
              >
                <span className="flex flex-wrap items-center gap-2">
                  <span
                    className={cn("truncate font-medium", checked && "line-through opacity-70")}
                  >
                    {file.filename}
                  </span>
                  <Badge variant="outline" className="font-mono text-[10px]">
                    {extLabel(file)}
                  </Badge>
                  {oldest && (
                    <Badge variant="secondary" className="text-[10px]">
                      {t("oldest")}
                    </Badge>
                  )}
                </span>

                <span className="text-muted-foreground flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs">
                  <span className="tabular-nums">
                    {format.dateTime(new Date(file.createdAt), { dateStyle: "medium" })}
                  </span>
                  <span className="tabular-nums">{humanFileSize(file.size)}</span>
                  {file.folderName && (
                    <span className="inline-flex items-center gap-1">
                      <Folder className="size-3" />
                      {file.folderName}
                    </span>
                  )}
                  <span className="inline-flex items-center gap-1 tabular-nums">
                    <Eye className="size-3" />
                    {file.views.toLocaleString()}
                  </span>
                </span>
              </label>

              <Button asChild variant="ghost" size="icon" className="size-8 shrink-0">
                <a
                  href={file.url}
                  target="_blank"
                  rel="noopener noreferrer"
                  aria-label={t("openAria", { filename: file.filename })}
                >
                  <ExternalLink className="size-4" />
                </a>
              </Button>
            </li>
          );
        })}
      </ul>

      {!safe && (
        <p className="text-destructive bg-destructive/5 flex items-start gap-1.5 px-3 py-2 text-xs">
          <TriangleAlert className="mt-0.5 size-3.5 shrink-0" />
          {t("noCopyKept")}
        </p>
      )}
    </div>
  );
}
