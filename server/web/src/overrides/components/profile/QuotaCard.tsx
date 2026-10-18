// Client port of upstream components/profile/QuotaCard.tsx (an async Server Component there).
// @upstream components/profile/QuotaCard.tsx
// Deviations: `async` dropped, getTranslations -> useTranslations, and the two
// toLocaleString() calls pinned to "en-US": upstream runs them on the server,
// whose default locale is en-US, so that is what every visitor saw.
import { HardDrive, Files, Eye, Infinity as InfinityIcon } from "lucide-react";
import { useTranslations } from "next-intl";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import { Badge } from "@/components/ui/badge";
import { humanFileSize } from "@/lib/format";
import { cn } from "@/lib/utils";

interface QuotaCardProps {
  /** Bytes accounted against quota (authoritative usage counter). */
  usedQuota: number;
  /** Quota ceiling in bytes; -1 means unlimited. */
  maxQuota: number;
  /** Sum of stored object sizes (live uploads). */
  totalSize: number;
  fileCount: number;
  totalViews: number;
}

function Stat({
  icon: Icon,
  label,
  value,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  value: string;
}) {
  return (
    <div className="bg-muted/40 ring-foreground/5 flex flex-col justify-center gap-1 rounded-lg px-3 py-2.5 ring-1">
      <div className="text-muted-foreground flex items-center gap-1.5 text-xs font-medium">
        <Icon className="size-3.5" />
        {label}
      </div>
      <div className="font-heading text-lg font-semibold tabular-nums tracking-tight">{value}</div>
    </div>
  );
}

export function QuotaCard({ usedQuota, maxQuota, totalSize, fileCount, totalViews }: QuotaCardProps) {
  const t = useTranslations("profile.storage");
  const unlimited = maxQuota < 0;
  // Prefer the tracked quota counter; fall back to live object total.
  const used = Math.max(usedQuota, 0) || totalSize;
  const pct = unlimited || maxQuota === 0 ? 0 : Math.min(100, Math.round((used / maxQuota) * 100));
  const near = pct >= 80 && pct < 95;
  const full = pct >= 95;

  return (
    <Card>
      <CardHeader className="border-b">
        <CardTitle className="flex items-center gap-2">
          <HardDrive className="text-primary size-4" />
          {t("title")}
        </CardTitle>
        <CardDescription>{t("desc")}</CardDescription>
      </CardHeader>

      {/* This card usually shares a grid row with a taller one. Rather than
          leave the slack as dead space under the meter, the stat tiles grow
          into it. */}
      <CardContent className="flex flex-1 flex-col gap-5">
        <div className="space-y-2.5">
          <div className="flex items-end justify-between gap-3">
            <div className="space-y-0.5">
              <div className="text-muted-foreground text-xs font-medium uppercase tracking-wide">
                {t("used")}
              </div>
              <div className="font-heading text-2xl font-semibold tabular-nums tracking-tight">
                {humanFileSize(used)}
              </div>
            </div>
            <div className="text-right">
              {unlimited ? (
                <Badge variant="secondary" className="gap-1">
                  <InfinityIcon className="size-3" />
                  {t("unlimited")}
                </Badge>
              ) : (
                <>
                  <div className="text-muted-foreground text-sm tabular-nums">
                    {t("of", { amount: humanFileSize(maxQuota) })}
                  </div>
                  <div
                    className={cn(
                      "text-sm font-semibold tabular-nums",
                      full ? "text-destructive" : near ? "text-amber-500" : "text-foreground",
                    )}
                  >
                    {pct}%
                  </div>
                </>
              )}
            </div>
          </div>

          {!unlimited && (
            <Progress
              value={pct}
              className={cn(
                "h-2",
                full
                  ? "[&_[data-slot=progress-indicator]]:bg-destructive"
                  : near
                    ? "[&_[data-slot=progress-indicator]]:bg-amber-500"
                    : null,
              )}
            />
          )}
        </div>

        <div className="grid flex-1 grid-cols-2 gap-3 sm:grid-cols-3">
          <Stat icon={Files} label={t("files")} value={fileCount.toLocaleString("en-US")} />
          <Stat icon={Eye} label={t("totalViews")} value={totalViews.toLocaleString("en-US")} />
          <Stat
            icon={HardDrive}
            label={t("stored")}
            value={humanFileSize(totalSize)}
          />
        </div>
      </CardContent>
    </Card>
  );
}
