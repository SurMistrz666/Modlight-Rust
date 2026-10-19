"use client";

import { useState } from "react";
import { Calculator, HardDriveDownload, Loader2, ScanSearch, CircleCheck } from "lucide-react";
import { toast } from "sonner";
import { useLocale, useTranslations } from "next-intl";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { humanFileSize, relativeTime } from "@/lib/format";
import type { QuotaRecalcResult, StorageScanResult } from "@/lib/data/admin";
import { recalculateEveryQuota, runStorageScan } from "@/app/actions/admin";

function errMessage(error: unknown, fallback: string): string {
  if (error && typeof error === "object" && "message" in error) {
    const m = (error as { message?: unknown }).message;
    if (typeof m === "string" && m) return m;
  }
  return fallback;
}

/** A maintenance job: what it does, the button, and its last result inline. */
function Job({
  icon,
  title,
  description,
  action,
  children,
}: {
  icon: React.ReactNode;
  title: string;
  description: string;
  action: React.ReactNode;
  children?: React.ReactNode;
}) {
  return (
    <Card>
      <CardHeader className="border-b">
        <CardTitle className="flex items-center gap-2 text-sm">
          <span className="text-muted-foreground">{icon}</span>
          {title}
        </CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <p className="text-muted-foreground max-w-xl text-xs leading-relaxed">{description}</p>
          <div className="shrink-0">{action}</div>
        </div>
        {children}
      </CardContent>
    </Card>
  );
}

/**
 * Admin → Maintenance.
 *
 * Two jobs that answer the question "did the database and the disk drift
 * apart?" — the one that always follows a hand-made fix in psql, a half-run
 * import, or an account deleted with its files kept.
 */
export function Maintenance() {
  const t = useTranslations("admin.maintenance");
  // Ages in this panel are rendered by Intl, which has to be handed the
  // request locale: next-intl picked it, the host machine did not.
  const locale = useLocale();

  const [recalcBusy, setRecalcBusy] = useState(false);
  const [recalc, setRecalc] = useState<QuotaRecalcResult | null>(null);

  const [scanBusy, setScanBusy] = useState(false);
  const [scan, setScan] = useState<StorageScanResult | null>(null);

  async function doRecalculate() {
    setRecalcBusy(true);
    try {
      const result = await recalculateEveryQuota();
      setRecalc(result);
      toast.success(t("recalcDoneTitle"), {
        description:
          result.corrected === 0
            ? t("recalcClean", { accounts: result.accounts })
            : t("recalcCorrected", { corrected: result.corrected, accounts: result.accounts }),
      });
    } catch (error) {
      toast.error(t("recalcError"), { description: errMessage(error, t("tryAgain")) });
    } finally {
      setRecalcBusy(false);
    }
  }

  async function doScan() {
    setScanBusy(true);
    try {
      const result = await runStorageScan();
      setScan(result);
      if (!result.supported) {
        toast.warning(t("scanUnsupportedTitle"), { description: t("scanUnsupported") });
      } else if (result.orphans === 0) {
        toast.success(t("scanDoneTitle"), { description: t("scanClean", { scanned: result.scanned }) });
      } else {
        toast.warning(t("scanDoneTitle"), {
          description: t("scanFound", {
            count: result.orphans,
            size: humanFileSize(result.orphanBytes),
          }),
        });
      }
    } catch (error) {
      toast.error(t("scanError"), { description: errMessage(error, t("tryAgain")) });
    } finally {
      setScanBusy(false);
    }
  }

  return (
    <div className="flex flex-col gap-5">
      <Job
        icon={<Calculator className="size-4" />}
        title={t("recalcTitle")}
        description={t("recalcDesc")}
        action={
          <Button onClick={() => void doRecalculate()} disabled={recalcBusy}>
            {recalcBusy ? <Loader2 className="size-4 animate-spin" /> : <Calculator className="size-4" />}
            {recalcBusy ? t("recalcRunning") : t("recalcCta")}
          </Button>
        }
      >
        {recalc ? (
          recalc.corrected === 0 ? (
            <p className="flex items-center gap-2 text-xs text-emerald-500">
              <CircleCheck className="size-4" />
              {t("recalcClean", { accounts: recalc.accounts })}
            </p>
          ) : (
            <div className="ring-foreground/10 flex flex-col gap-2 rounded-lg p-3 ring-1">
              <p className="text-xs font-medium">
                {t("recalcCorrected", { corrected: recalc.corrected, accounts: recalc.accounts })}
                <span className="text-muted-foreground font-normal">
                  {" "}
                  {t("recalcDrift", { size: humanFileSize(Math.abs(recalc.drift)) })}
                </span>
              </p>
              <ul className="flex flex-col gap-1">
                {recalc.changes.map((c) => (
                  <li key={c.id} className="flex items-center justify-between gap-3 text-xs">
                    <span className="text-muted-foreground truncate">{c.email}</span>
                    <span className="shrink-0 tabular-nums">
                      <span className="text-muted-foreground line-through">
                        {humanFileSize(c.before)}
                      </span>{" "}
                      → <span className="font-medium">{humanFileSize(c.after)}</span>
                    </span>
                  </li>
                ))}
              </ul>
            </div>
          )
        ) : null}
      </Job>

      <Job
        icon={<ScanSearch className="size-4" />}
        title={t("scanTitle")}
        description={t("scanDesc")}
        action={
          <Button variant="outline" onClick={() => void doScan()} disabled={scanBusy}>
            {scanBusy ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <HardDriveDownload className="size-4" />
            )}
            {scanBusy ? t("scanRunning") : t("scanCta")}
          </Button>
        }
      >
        {scan ? (
          !scan.supported ? (
            <p className="text-muted-foreground text-xs leading-relaxed">
              {t("scanUnsupported")}
            </p>
          ) : (
            <div className="ring-foreground/10 flex flex-col gap-2 rounded-lg p-3 ring-1">
              <div className="flex flex-wrap items-center gap-2">
                <Badge variant="outline" className="text-muted-foreground">
                  {t("scanScanned", { count: scan.scanned })}
                </Badge>
                <Badge
                  variant={scan.orphans > 0 ? "destructive" : "outline"}
                  className={scan.orphans > 0 ? undefined : "text-muted-foreground"}
                >
                  {t("scanOrphans", { count: scan.orphans, size: humanFileSize(scan.orphanBytes) })}
                </Badge>
                {scan.skippedRecent > 0 ? (
                  <Badge variant="outline" className="text-muted-foreground">
                    {t("scanSkipped", { count: scan.skippedRecent })}
                  </Badge>
                ) : null}
              </div>

              {scan.truncated ? (
                <p className="text-xs text-amber-500">{t("scanTruncated")}</p>
              ) : null}

              {scan.orphans === 0 ? (
                <p className="flex items-center gap-2 text-xs text-emerald-500">
                  <CircleCheck className="size-4" /> {t("scanClean", { scanned: scan.scanned })}
                </p>
              ) : (
                <>
                  <p className="text-muted-foreground text-xs leading-relaxed">
                    {t("scanReportOnly")}
                  </p>
                  <ul className="flex max-h-64 flex-col gap-1 overflow-y-auto">
                    {scan.sample.map((o) => (
                      <li key={o.key} className="flex items-center justify-between gap-3 text-xs">
                        <code className="text-muted-foreground truncate font-mono">{o.key}</code>
                        <span className="shrink-0 tabular-nums">
                          {humanFileSize(o.size)}
                          {o.modifiedAt ? (
                            <span className="text-muted-foreground">
                              {" · "}
                              {relativeTime(o.modifiedAt, locale)}
                            </span>
                          ) : null}
                        </span>
                      </li>
                    ))}
                  </ul>
                  {scan.orphans > scan.sample.length ? (
                    <p className="text-muted-foreground text-xs">
                      {t("scanSampleCapped", {
                        shown: scan.sample.length,
                        total: scan.orphans,
                      })}
                    </p>
                  ) : null}
                </>
              )}
            </div>
          )
        ) : null}
      </Job>
    </div>
  );
}
