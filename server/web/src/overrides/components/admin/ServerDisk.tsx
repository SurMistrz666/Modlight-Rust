// Client port of upstream components/admin/ServerDisk.tsx (an async Server Component there).
// @upstream components/admin/ServerDisk.tsx
// Deviations: `async` dropped; getTranslations/getLocale -> useTranslations/useLocale;
// the disk insights arrive as a prop (the admin page payload carries
// `diskInsights`, upstream awaited diskInsights() here).
import {
  ArrowDownWideNarrow,
  Cloud,
  HardDrive,
  HelpCircle,
  Shapes,
  Snowflake,
  Trash2,
  TriangleAlert,
  Users,
} from "lucide-react";
import { useLocale, useTranslations } from "next-intl";
import { Card } from "@/components/ui/card";
import { humanFileSize, relativeTime } from "@/lib/format";
import { cn } from "@/lib/utils";
import { diskPressure, type DiskPressure } from "@/lib/upload/disk";
import { TRASH_RETENTION_DAYS } from "@/lib/maintenance/trash";
import { publicUrl } from "@/lib/media/urls";
import type { MediaKind } from "@/lib/media/mime";
import {
  TRASH_DUE_SOON_DAYS,
  type AccountUsage,
  type DiskInsights,
  type HeavyFile,
} from "@/lib/data/admin-disk";
import type { ServerStorageUsage } from "@/lib/data/admin";

/**
 * How full the machine is, and what is filling it.
 *
 * Per-account quota answers a different question entirely. This host serves ten
 * other production sites off the same disk, and a full disk takes every one of
 * them down at the same moment, so the top card exists to be understood at a
 * glance: the colour says whether to act, the numbers say what to act on.
 *
 * Everything below that card answers the question an operator asks next, which
 * a single bar never could: WHICH files, WHOSE files, and which of them nobody
 * has fetched in half a year. All of it is aggregated by the database over the
 * sizes recorded when each file arrived (lib/data/admin-disk.ts carries the
 * cost of every query), so nothing here is sampled or extrapolated.
 *
 * The insights are fetched here rather than passed in: they are wanted wherever
 * this card is wanted, and the page already awaits its own props in parallel.
 */
export function ServerDisk({ usage, insights }: { usage: ServerStorageUsage; insights: DiskInsights }) {
  const t = useTranslations("admin.disk");
  const locale = useLocale();

  return (
    <div className="flex flex-col gap-4">
      {usage.disk ? (
        <FilesystemCard usage={usage} disk={usage.disk} t={t} />
      ) : (
        <DatabaseOnlyCard usage={usage} t={t} />
      )}
      <Insights insights={insights} locale={locale} t={t} />
    </div>
  );
}

// ---------------------------------------------------------------------------
// Translator
//
// next-intl's `t` is passed down rather than re-requested in every child: one
// namespace lookup, and the children stay dumb. This project declares no global
// Messages type, so the callable shape is written out once here.
// ---------------------------------------------------------------------------

type T = (key: string, values?: Record<string, string | number | Date>) => string;

// ---------------------------------------------------------------------------
// The machine
// ---------------------------------------------------------------------------

function FilesystemCard({
  usage,
  disk,
  t,
}: {
  usage: ServerStorageUsage;
  disk: NonNullable<ServerStorageUsage["disk"]>;
  t: T;
}) {
  const { totalBytes, freeBytes, usedBytes } = disk;
  const appBytes = usage.liveBytes + usage.trashedBytes;
  const pressure = diskPressure(freeBytes, totalBytes, usage.minFreeBytes);

  // Whatever occupies the filesystem that is not ours: the other sites, the
  // database, the system. Clamped because thumbnails and cached conversions are
  // real bytes that no `uploads.size` accounts for, and a rounding artefact must
  // never render as a negative-width segment.
  const otherBytes = Math.max(0, usedBytes - appBytes);
  const freePercent = Math.round(share(freeBytes, totalBytes));

  return (
    <Card
      className={cn(
        "gap-0 p-5",
        pressure === "critical" && "border-destructive/50",
        pressure === "low" && "border-amber-500/50",
      )}
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <Header title={t("title")} desc={t("descLocal")} icon={HardDrive} tone={pressure} />
        <span
          className={cn(
            "rounded-full px-2.5 py-1 text-xs font-semibold tabular-nums",
            TONE_CHIP[pressure],
          )}
        >
          {t("freeBadge", { percent: freePercent })}
        </span>
      </div>

      <div className="mt-5 flex items-baseline gap-2">
        <span className="font-heading text-3xl leading-none font-semibold tracking-tight tabular-nums">
          {humanFileSize(usedBytes)}
        </span>
        <span className="text-muted-foreground text-sm tabular-nums">
          {t("ofTotal", { total: humanFileSize(totalBytes) })}
        </span>
      </div>

      <div className="mt-3">
        <Bar
          segments={[
            { key: "live", bytes: usage.liveBytes, className: "bg-primary" },
            { key: "trash", bytes: usage.trashedBytes, className: "bg-amber-500" },
            { key: "other", bytes: otherBytes, className: "bg-muted-foreground/40" },
          ]}
          total={totalBytes}
        />
      </div>

      <Legend
        items={[
          {
            key: "live",
            dot: "bg-primary",
            label: t("live"),
            value: humanFileSize(usage.liveBytes),
            hint: t("files", { count: usage.liveFiles }),
          },
          {
            key: "trash",
            dot: "bg-amber-500",
            label: t("trash"),
            value: humanFileSize(usage.trashedBytes),
            hint: t("trashHint", { days: TRASH_RETENTION_DAYS }),
          },
          {
            key: "other",
            dot: "bg-muted-foreground/40",
            label: t("other"),
            value: humanFileSize(otherBytes),
            hint: t("otherHint"),
          },
          {
            key: "free",
            dot: "bg-muted ring-1 ring-border",
            label: t("free"),
            value: humanFileSize(freeBytes),
            hint: t("freeHint"),
            valueClassName:
              pressure === "critical"
                ? "text-destructive"
                : pressure === "low"
                  ? "text-amber-600 dark:text-amber-400"
                  : undefined,
          },
        ]}
      />

      {pressure === "low" || pressure === "critical" ? (
        <div
          className={cn(
            "mt-4 flex items-start gap-2 rounded-lg px-3 py-2.5 text-sm",
            pressure === "critical"
              ? "bg-destructive/10 text-destructive"
              : "bg-amber-500/10 text-amber-700 dark:text-amber-400",
          )}
        >
          <TriangleAlert className="mt-0.5 size-4 shrink-0" />
          <p>
            {pressure === "critical" ? t("criticalNote") : t("lowNote")}{" "}
            {usage.minFreeBytes > 0
              ? t("reserveNote", { size: humanFileSize(usage.minFreeBytes) })
              : null}
          </p>
        </div>
      ) : null}
    </Card>
  );
}

/**
 * No filesystem to report on: either an object store, or a statfs that failed.
 *
 * Both get the database totals and an honest label rather than the app
 * container's own disk dressed up as the storage.
 */
function DatabaseOnlyCard({ usage, t }: { usage: ServerStorageUsage; t: T }) {
  const objectStore = usage.driver !== "local";
  const appBytes = usage.liveBytes + usage.trashedBytes;
  const appFiles = usage.liveFiles + usage.trashedFiles;

  return (
    <Card className="gap-0 p-5">
      <Header
        title={t("title")}
        desc={
          objectStore ? t("descObject", { driver: usage.driver.toUpperCase() }) : t("descUnmeasured")
        }
        icon={objectStore ? Cloud : HelpCircle}
        tone="muted"
      />
      <div className="mt-5">
        <Bar
          segments={[
            { key: "live", bytes: usage.liveBytes, className: "bg-primary" },
            { key: "trash", bytes: usage.trashedBytes, className: "bg-amber-500" },
          ]}
          total={appBytes}
        />
      </div>
      <Legend
        items={[
          {
            key: "live",
            dot: "bg-primary",
            label: t("live"),
            value: humanFileSize(usage.liveBytes),
            hint: t("files", { count: usage.liveFiles }),
          },
          {
            key: "trash",
            dot: "bg-amber-500",
            label: t("trash"),
            value: humanFileSize(usage.trashedBytes),
            hint: t("trashHint", { days: TRASH_RETENTION_DAYS }),
          },
          {
            key: "app",
            dot: "bg-muted-foreground/40",
            label: t("appTotal"),
            value: humanFileSize(appBytes),
            hint: t("files", { count: appFiles }),
          },
        ]}
      />
    </Card>
  );
}

// ---------------------------------------------------------------------------
// Where the space went
// ---------------------------------------------------------------------------

function Insights({ insights, locale, t }: { insights: DiskInsights; locale: string; t: T }) {
  const { trash } = insights;
  // Denominator for the per-account bars: the accounts add up to it by
  // construction (the tail is folded, never dropped), so the widths reconcile.
  const accountBytes = insights.accounts.reduce((sum, a) => sum + a.liveBytes + a.trashedBytes, 0);

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h3 className="font-heading text-sm font-semibold tracking-tight">{t("insightsTitle")}</h3>
        <p className="text-muted-foreground max-w-prose text-xs leading-relaxed">
          {/* On an object store the rows are all there is, and saying so is the
              difference between a total and a guess: derived thumbnails, cached
              conversions and the leftovers of an interrupted multipart upload
              are real objects that no row here accounts for, and only a listing
              of the bucket could count them. On local storage those same bytes
              are at least visible in aggregate, inside the "Other data" segment
              of the card above, which is why that case gets a milder sentence. */}
          {insights.local
            ? t("insightsDesc")
            : t("insightsDescObject", { driver: insights.driver.toUpperCase() })}
        </p>
      </div>

      {/* The trash comes first: it is the only line here that turns back into
          free space on its own, and the only one that can report a broken job. */}
      {trash.files > 0 ? (
        <Card className="gap-0 p-5">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <Header
              title={t("trashTitle")}
              desc={t("trashDesc", { days: TRASH_RETENTION_DAYS })}
              icon={Trash2}
              tone={trash.overdueFiles > 0 ? "low" : "muted"}
            />
            <span className="font-heading text-2xl leading-none font-semibold tracking-tight tabular-nums">
              {humanFileSize(trash.bytes)}
            </span>
          </div>

          <dl className="mt-4 grid grid-cols-2 gap-x-4 gap-y-3 sm:grid-cols-3">
            <Stat
              label={t("trashHeld")}
              value={humanFileSize(trash.bytes)}
              hint={t("files", { count: trash.files })}
            />
            <Stat
              label={t("trashDueSoon", { days: TRASH_DUE_SOON_DAYS })}
              value={humanFileSize(trash.dueSoonBytes)}
              hint={t("files", { count: trash.dueSoonFiles })}
            />
            <Stat
              label={t("trashOldest")}
              value={trash.oldest ? relativeTime(trash.oldest, locale) : "-"}
              hint={t("trashOldestHint")}
            />
          </dl>

          {/* Should always be zero: the sweeper runs every six hours. Anything
              else is the panel reporting that the job is not running, which is
              the only way an operator would ever find out. */}
          {trash.overdueFiles > 0 ? (
            <div className="mt-4 flex items-start gap-2 rounded-lg bg-amber-500/10 px-3 py-2.5 text-sm text-amber-700 dark:text-amber-400">
              <TriangleAlert className="mt-0.5 size-4 shrink-0" />
              <p>
                {t("trashOverdue", {
                  count: trash.overdueFiles,
                  size: humanFileSize(trash.overdueBytes),
                  days: TRASH_RETENTION_DAYS,
                })}
              </p>
            </div>
          ) : null}
        </Card>
      ) : null}

      <div className="grid gap-4 lg:grid-cols-2">
        <Panel title={t("kindsTitle")} desc={t("kindsDesc")} icon={Shapes}>
          {insights.kinds.length === 0 ? (
            <Empty text={t("kindsEmpty")} />
          ) : (
            <>
              <Bar
                segments={insights.kinds.map((k) => ({
                  key: k.kind,
                  bytes: k.bytes,
                  className: KIND_COLOR[k.kind],
                }))}
                total={insights.kindBytes}
              />
              <ul className="mt-4 flex flex-col gap-2.5">
                {insights.kinds.map((k) => (
                  <li key={k.kind} className="flex items-center gap-2.5 text-sm">
                    <span
                      className={cn("size-2 shrink-0 rounded-full", KIND_COLOR[k.kind])}
                      aria-hidden
                    />
                    <span className="min-w-0 flex-1 truncate">{t(`kind.${k.kind}`)}</span>
                    <span className="text-muted-foreground shrink-0 text-xs tabular-nums">
                      {t("files", { count: k.files })}
                    </span>
                    <span className="w-20 shrink-0 text-right font-medium tabular-nums">
                      {humanFileSize(k.bytes)}
                    </span>
                  </li>
                ))}
              </ul>
            </>
          )}
        </Panel>

        <Panel title={t("accountsTitle")} desc={t("accountsDesc")} icon={Users}>
          {insights.accounts.length === 0 ? (
            <Empty text={t("accountsEmpty")} />
          ) : (
            <ul className="flex flex-col gap-3">
              {insights.accounts.map((a) => (
                <li key={accountKey(a)}>
                  <div className="flex items-baseline justify-between gap-3 text-sm">
                    <span className="min-w-0 truncate font-medium">{accountLabel(a, t)}</span>
                    <span className="shrink-0 tabular-nums">
                      {humanFileSize(a.liveBytes + a.trashedBytes)}
                    </span>
                  </div>
                  <div className="mt-1.5">
                    <Bar
                      segments={[
                        { key: "live", bytes: a.liveBytes, className: "bg-primary" },
                        { key: "trash", bytes: a.trashedBytes, className: "bg-amber-500" },
                      ]}
                      total={accountBytes}
                      height="h-1.5"
                    />
                  </div>
                  <p className="text-muted-foreground mt-1 text-xs">
                    {t("files", { count: a.liveFiles })}
                    {a.trashedFiles > 0
                      ? ` · ${t("accountsTrashed", { size: humanFileSize(a.trashedBytes) })}`
                      : ""}
                  </p>
                </li>
              ))}
            </ul>
          )}
        </Panel>

        <Panel title={t("biggestTitle")} desc={t("biggestDesc")} icon={ArrowDownWideNarrow}>
          {insights.biggest.length === 0 ? (
            <Empty text={t("biggestEmpty")} />
          ) : (
            <FileList
              files={insights.biggest}
              meta={(f) =>
                [
                  ownerLabel(f, t),
                  f.createdAt ? t("uploadedAgo", { time: relativeTime(f.createdAt, locale) }) : null,
                  t("viewsCount", { count: f.views }),
                ]
                  .filter(Boolean)
                  .join(" · ")
              }
            />
          )}
        </Panel>

        <Panel
          title={t("coldTitle", { days: insights.coldTotals.days })}
          desc={t("coldDesc")}
          icon={Snowflake}
        >
          {insights.cold.length === 0 ? (
            <Empty text={t("coldEmpty", { days: insights.coldTotals.days })} />
          ) : (
            <>
              <p className="text-sm">
                <span className="font-heading text-xl font-semibold tracking-tight tabular-nums">
                  {humanFileSize(insights.coldTotals.bytes)}
                </span>{" "}
                <span className="text-muted-foreground">
                  {t("coldTotal", { count: insights.coldTotals.files })}
                </span>
              </p>
              <div className="mt-3">
                <FileList
                  files={insights.cold}
                  meta={(f) =>
                    [
                      ownerLabel(f, t),
                      f.lastAccessedAt
                        ? t("lastFetched", { time: relativeTime(f.lastAccessedAt, locale) })
                        : t("neverFetched"),
                    ].join(" · ")
                  }
                />
              </div>
              {/* Access has only been counted since the row was written here.
                  Rows that predate the counter carry no history at all, so "no
                  recorded access" is not proof that nobody ever opened the file. */}
              <p className="text-muted-foreground mt-3 text-xs leading-relaxed">{t("coldNote")}</p>
            </>
          )}
        </Panel>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Pieces
// ---------------------------------------------------------------------------

/**
 * One colour per media kind.
 *
 * Never the only carrier of meaning: every segment has a labelled row under it,
 * so the breakdown still reads on a monochrome skin or to a colour-blind eye.
 */
const KIND_COLOR: Record<MediaKind, string> = {
  image: "bg-primary",
  video: "bg-violet-500",
  audio: "bg-sky-500",
  pdf: "bg-rose-500",
  text: "bg-emerald-500",
  archive: "bg-amber-500",
  other: "bg-muted-foreground/40",
};

function accountKey(a: AccountUsage): string {
  return a.id ?? (a.rest ? "rest" : "orphaned");
}

function accountLabel(a: AccountUsage, t: T): string {
  if (a.rest) return t("accountsRest", { count: a.accounts });
  if (a.orphaned) return t("accountsOrphaned");
  return a.name || a.email || a.tag || t("accountsUnknown");
}

function ownerLabel(f: HeavyFile, t: T): string {
  return f.ownerName || f.ownerTag || t("accountsOrphaned");
}

/**
 * A file list an operator can click through.
 *
 * The link is `publicUrl` against an empty base, i.e. the canonical
 * /{tag}/{code}.{ext} path on the host being administered - the app's one
 * definition of that URL, not a second one spelled out here. An orphan has no
 * tag and therefore no reachable URL at all (that is what being an orphan
 * means), so its row is plain text rather than a link that would 404.
 */
function FileList({ files, meta }: { files: HeavyFile[]; meta: (f: HeavyFile) => string }) {
  return (
    <ul className="flex flex-col gap-2.5">
      {files.map((f) => {
        const href = f.ownerTag ? publicUrl(f.ownerTag, f.code, f.ext, "") : null;
        const name = (
          <span className="truncate text-sm font-medium" title={f.filename}>
            {f.filename}
          </span>
        );
        return (
          <li key={f.id} className="flex items-center gap-3">
            <div className="min-w-0 flex-1">
              {href ? (
                <a
                  href={href}
                  target="_blank"
                  rel="noreferrer"
                  className="hover:text-primary flex min-w-0 items-center transition-colors"
                >
                  {name}
                </a>
              ) : (
                <div className="flex min-w-0 items-center">{name}</div>
              )}
              <p className="text-muted-foreground truncate text-xs">{meta(f)}</p>
            </div>
            <span className="shrink-0 text-sm font-medium tabular-nums">
              {humanFileSize(f.size)}
            </span>
          </li>
        );
      })}
    </ul>
  );
}

function Panel({
  title,
  desc,
  icon,
  children,
}: {
  title: string;
  desc: string;
  icon: React.ComponentType<{ className?: string }>;
  children: React.ReactNode;
}) {
  return (
    <Card className="gap-0 p-5">
      <Header title={title} desc={desc} icon={icon} tone="muted" />
      <div className="mt-4">{children}</div>
    </Card>
  );
}

function Stat({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="min-w-0">
      <dt className="text-muted-foreground text-xs font-medium">{label}</dt>
      <dd className="font-heading mt-1 text-base font-semibold tabular-nums">{value}</dd>
      {hint ? <dd className="text-muted-foreground mt-0.5 text-xs">{hint}</dd> : null}
    </div>
  );
}

function Empty({ text }: { text: string }) {
  return <p className="text-muted-foreground py-4 text-center text-xs">{text}</p>;
}

type Tone = DiskPressure | "muted";

const TONE_CHIP: Record<Tone, string> = {
  ok: "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400",
  low: "bg-amber-500/15 text-amber-700 dark:text-amber-400",
  critical: "bg-destructive/10 text-destructive",
  unknown: "bg-muted text-muted-foreground",
  muted: "bg-muted text-muted-foreground",
};

const TONE_ICON: Record<Tone, string> = {
  ok: "bg-primary/10 text-primary",
  low: "bg-amber-500/15 text-amber-600 dark:text-amber-400",
  critical: "bg-destructive/10 text-destructive",
  unknown: "bg-muted text-muted-foreground",
  muted: "bg-muted text-muted-foreground",
};

function Header({
  title,
  desc,
  icon: Icon,
  tone,
}: {
  title: string;
  desc: string;
  icon: React.ComponentType<{ className?: string }>;
  tone: Tone;
}) {
  return (
    <div className="flex items-start gap-3">
      <span className={cn("grid size-9 shrink-0 place-items-center rounded-lg", TONE_ICON[tone])}>
        <Icon className="size-4.5" />
      </span>
      <div>
        <h3 className="font-heading text-sm font-semibold tracking-tight">{title}</h3>
        <p className="text-muted-foreground mt-0.5 max-w-prose text-xs">{desc}</p>
      </div>
    </div>
  );
}

interface Segment {
  key: string;
  bytes: number;
  className: string;
}

/**
 * Stacked bar over a track that stands for the remainder.
 *
 * A segment with real bytes in it is floored at half a percent: on a 2 TB disk
 * the trash can hold gigabytes and still round to a zero-width sliver, and a
 * segment nobody can see is a segment the legend contradicts.
 */
function Bar({
  segments,
  total,
  height = "h-2.5",
}: {
  segments: Segment[];
  total: number;
  height?: string;
}) {
  return (
    <div className={cn("bg-muted flex w-full overflow-hidden rounded-full", height)}>
      {segments.map((s) => {
        const pct = share(s.bytes, total);
        if (pct <= 0) return null;
        return (
          <div
            key={s.key}
            className={s.className}
            style={{ width: `${Math.max(pct, 0.5)}%` }}
            aria-hidden
          />
        );
      })}
    </div>
  );
}

interface LegendItem {
  key: string;
  dot: string;
  label: string;
  value: string;
  hint: string;
  valueClassName?: string;
}

function Legend({ items }: { items: LegendItem[] }) {
  return (
    <dl className="mt-4 grid grid-cols-2 gap-x-4 gap-y-3 sm:grid-cols-4">
      {items.map((item) => (
        <div key={item.key} className="min-w-0">
          <dt className="text-muted-foreground flex items-center gap-1.5 text-xs font-medium">
            <span className={cn("size-2 shrink-0 rounded-full", item.dot)} aria-hidden />
            <span className="truncate">{item.label}</span>
          </dt>
          <dd
            className={cn(
              "font-heading mt-1 text-base font-semibold tabular-nums",
              item.valueClassName,
            )}
          >
            {item.value}
          </dd>
          <dd className="text-muted-foreground mt-0.5 text-xs">{item.hint}</dd>
        </div>
      ))}
    </dl>
  );
}

/** Percentage of `whole` that `part` occupies, clamped and never NaN. */
function share(part: number, whole: number): number {
  if (!Number.isFinite(part) || !Number.isFinite(whole) || whole <= 0) return 0;
  return Math.max(0, Math.min(100, (part / whole) * 100));
}
