"use client";

import { useMemo } from "react";
import {
  Users,
  ShieldCheck,
  HardDrive,
  Files,
  Ban,
  Eye,
  Ghost,
  type LucideIcon,
} from "lucide-react";
import { useTranslations } from "next-intl";
import { cn } from "@/lib/utils";
import { Card } from "@/components/ui/card";

/**
 * Totals computed on the server and passed in as props.
 *
 * Account counts used to be fetched client-side via `admin.listUsers({limit:1000})`
 * purely to be counted — a full user dump over the wire for six numbers. They
 * are aggregate queries on the page now, so the tab renders with no round trip
 * and no loading skeleton.
 */
export interface GlobalStats {
  totalFiles: number;
  totalStorage: string; // already humanized (e.g. "4.2 GB")
  totalViews: number;
  users: number;
  admins: number;
  banned: number;
  /** Uploads with no owner — see Admin → Orphaned files. */
  orphanFiles: number;
  orphanStorage: string; // already humanized
}

interface Kpi {
  label: string;
  value: string;
  hint?: string;
  icon: LucideIcon;
  accent: string; // tailwind text color for the icon
  ring: string; // subtle tinted background for the icon chip
}

function compactNumber(n: number): string {
  if (n < 1000) return String(n);
  return new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 }).format(n);
}

function StatTile({ kpi }: { kpi: Kpi }) {
  return (
    <Card className="relative gap-0 p-5">
      <div
        aria-hidden
        className="pointer-events-none absolute -top-10 -right-10 size-28 rounded-full bg-current opacity-[0.04] blur-2xl"
        style={{ color: "var(--primary)" }}
      />
      <div className="flex items-start justify-between">
        <span className="text-muted-foreground text-xs font-medium tracking-wide uppercase">
          {kpi.label}
        </span>
        <span className={cn("grid size-9 place-items-center rounded-lg", kpi.ring)}>
          <kpi.icon className={cn("size-4.5", kpi.accent)} />
        </span>
      </div>
      <div className="mt-4 flex items-baseline gap-2">
        <span className="font-heading text-3xl leading-none font-semibold tracking-tight tabular-nums">
          {kpi.value}
        </span>
      </div>
      {kpi.hint ? (
        <span className="text-muted-foreground mt-2 text-xs">{kpi.hint}</span>
      ) : null}
    </Card>
  );
}

export function StatsCards({ stats }: { stats: GlobalStats }) {
  const t = useTranslations("admin.stats");

  const kpis = useMemo<Kpi[]>(
    () => [
      {
        label: t("users"),
        value: compactNumber(stats.users),
        hint: t("adminsHint", { count: stats.admins }),
        icon: Users,
        accent: "text-primary",
        ring: "bg-primary/10",
      },
      {
        label: t("filesStored"),
        value: compactNumber(stats.totalFiles),
        hint: t("filesStoredHint"),
        icon: Files,
        accent: "text-sky-500",
        ring: "bg-sky-500/10",
      },
      {
        label: t("storageUsed"),
        value: stats.totalStorage,
        hint: t("storageUsedHint"),
        icon: HardDrive,
        accent: "text-emerald-500",
        ring: "bg-emerald-500/10",
      },
      {
        label: t("totalViews"),
        value: compactNumber(stats.totalViews),
        hint: t("totalViewsHint"),
        icon: Eye,
        accent: "text-amber-500",
        ring: "bg-amber-500/10",
      },
      {
        label: t("admins"),
        value: compactNumber(stats.admins),
        hint: t("adminsHintElevated"),
        icon: ShieldCheck,
        accent: "text-violet-500",
        ring: "bg-violet-500/10",
      },
      {
        label: t("banned"),
        value: compactNumber(stats.banned),
        hint: stats.banned ? t("bannedHint") : t("bannedHintClear"),
        icon: Ban,
        accent: "text-rose-500",
        ring: "bg-rose-500/10",
      },
      // Only when there are any. An "Orphaned: 0" tile on every healthy install
      // trains the operator to ignore the one number that means something is
      // wrong; a tile that appears is a tile that gets read.
      ...(stats.orphanFiles > 0
        ? [
            {
              label: t("orphaned"),
              value: compactNumber(stats.orphanFiles),
              hint: t("orphanedHint", { size: stats.orphanStorage }),
              icon: Ghost,
              accent: "text-amber-500",
              ring: "bg-amber-500/10",
            },
          ]
        : []),
    ],
    [stats, t],
  );

  return (
    <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3">
      {kpis.map((kpi) => (
        <StatTile key={kpi.label} kpi={kpi} />
      ))}
    </div>
  );
}
