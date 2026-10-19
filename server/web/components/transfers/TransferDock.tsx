"use client";

import { useCallback, useState } from "react";
import { useTranslations } from "next-intl";
import {
  AlertTriangle,
  Check,
  ChevronDown,
  ChevronUp,
  Copy,
  ExternalLink,
  Pause,
  Play,
  RotateCw,
  UploadCloud,
  X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { humanFileSize } from "@/lib/format";
import { Progress } from "@/components/ui/progress";
import { copyText } from "@/components/transfers/clipboard";
import {
  useTransfers,
  type TransferItem,
  type TransferStats,
} from "@/components/transfers/TransfersProvider";

/**
 * The floating transfer window.
 *
 * It lives in the app shell rather than on the upload page on purpose: a
 * transfer has to survive navigation. Drop a 4 GB video into a folder, carry on
 * browsing, and the queue keeps running with its progress in the corner.
 */
export function TransferDock() {
  const { items, stats, minimized, setMinimized, dockHidden } = useTransfers();

  if (dockHidden || items.length === 0) return null;

  return (
    <div
      className={cn(
        "animate-in fade-in slide-in-from-bottom-2 fixed right-4 bottom-4 z-50 duration-200",
        // The bulk-selection bar owns the bottom centre on a phone; sit above it.
        "w-[22rem] max-w-[calc(100vw-2rem)] max-sm:right-4 max-sm:left-4 max-sm:w-auto max-sm:bottom-24",
      )}
    >
      <div className="bg-card ring-foreground/10 overflow-hidden rounded-xl shadow-lg ring-1">
        <DockHeader stats={stats} minimized={minimized} onToggle={() => setMinimized(!minimized)} />
        {minimized ? null : <DockBody items={items} />}
      </div>
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Header                                                                      */
/* -------------------------------------------------------------------------- */

function DockHeader({
  stats,
  minimized,
  onToggle,
}: {
  stats: TransferStats;
  minimized: boolean;
  onToggle: () => void;
}) {
  const t = useTranslations("upload");
  const { pauseAll, resumeAll, cancelAll, clearFinished } = useTransfers();

  const finished = !stats.running;
  const hasFailures = stats.failed > 0;

  const Icon = hasFailures && finished ? AlertTriangle : finished ? Check : UploadCloud;

  const headline = stats.running
    ? stats.paused
      ? t("dock.paused")
      : t("dock.uploading")
    : hasFailures
      ? t("dock.finishedWithErrors", { count: stats.failed })
      : t("dock.finished");

  const eta = stats.running ? etaParts(stats.eta) : null;
  const detail = [
    t("dock.progress", { done: stats.done, total: stats.total }),
    stats.running && stats.speed > 0 ? t("dock.speed", { speed: humanFileSize(stats.speed) }) : null,
    eta ? t(eta.key, { count: eta.count }) : null,
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <div>
      <div className="flex items-center gap-2 px-3 py-2.5">
        <span
          className={cn(
            "grid size-8 shrink-0 place-items-center rounded-lg",
            hasFailures && finished
              ? "bg-destructive/10 text-destructive"
              : "bg-primary/10 text-primary",
          )}
        >
          <Icon className={cn("size-4", stats.running && !stats.paused && "animate-pulse")} />
        </span>

        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium">{headline}</p>
          <p className="text-muted-foreground truncate text-xs tabular-nums">{detail}</p>
          {/*
            The average gets its own line rather than being crammed into the one
            above, where it would push the remaining time out of a 22rem dock.
            It is here because it cannot be distorted by a gap in sampling: when
            the live number and this one disagree, this is the honest one.
          */}
          {stats.running && stats.average > 0 ? (
            <p className="text-muted-foreground/80 truncate text-[0.6875rem] tabular-nums">
              {t("dock.average", { speed: humanFileSize(stats.average) })}
            </p>
          ) : null}
        </div>

        {stats.running ? (
          <IconButton
            label={stats.paused ? t("dock.resumeAll") : t("dock.pauseAll")}
            onClick={stats.paused ? resumeAll : pauseAll}
          >
            {stats.paused ? <Play className="size-4" /> : <Pause className="size-4" />}
          </IconButton>
        ) : null}

        <IconButton
          label={minimized ? t("dock.expand") : t("dock.collapse")}
          onClick={onToggle}
          expanded={!minimized}
        >
          {minimized ? <ChevronUp className="size-4" /> : <ChevronDown className="size-4" />}
        </IconButton>

        <IconButton
          label={stats.running ? t("dock.cancelAll") : t("dock.close")}
          onClick={stats.running ? cancelAll : clearFinished}
        >
          <X className="size-4" />
        </IconButton>
      </div>

      <Progress
        value={Math.round(stats.percent)}
        className="h-1 rounded-none"
        aria-label={t("dock.overallAria")}
      />

      {/*
        One short, stable sentence for assistive tech. It only changes when the
        counts do, so a screen reader is not read a new number every frame.
      */}
      <p className="sr-only" aria-live="polite" aria-atomic="true">
        {stats.running
          ? t("dock.live", { done: stats.done, total: stats.total })
          : t("dock.liveDone", { total: stats.total, failed: stats.failed })}
      </p>
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Body                                                                        */
/* -------------------------------------------------------------------------- */

function DockBody({ items }: { items: TransferItem[] }) {
  const t = useTranslations("upload");
  const { clearFinished } = useTransfers();
  const settled = items.filter((i) => i.status === "done" || i.status === "error").length;

  return (
    <>
      <ul className="divide-foreground/5 max-h-72 divide-y overflow-y-auto">
        {items.map((item) => (
          <TransferRow key={item.id} item={item} />
        ))}
      </ul>

      {settled > 0 ? (
        <div className="border-foreground/10 flex justify-end border-t px-3 py-1.5">
          <button
            type="button"
            onClick={clearFinished}
            className="text-muted-foreground hover:text-foreground focus-visible:ring-ring rounded px-1.5 py-1 text-xs font-medium transition-colors outline-none focus-visible:ring-2"
          >
            {t("dock.clearFinished")}
          </button>
        </div>
      ) : null}
    </>
  );
}

function TransferRow({ item }: { item: TransferItem }) {
  const t = useTranslations("upload");
  const { toggleFile, cancelFile, retryFile } = useTransfers();
  const [copied, setCopied] = useState(false);

  const copy = useCallback(async () => {
    if (!item.url) return;
    if (await copyText(item.url)) {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    }
  }, [item.url]);

  const done = item.status === "done";
  const failed = item.status === "error";

  const meta = failed
    ? (item.error ?? t("dock.status.error"))
    : done
      ? humanFileSize(item.size)
      : item.size > 0
        ? `${humanFileSize(item.uploaded)} / ${humanFileSize(item.size)}`
        : t(`dock.status.${item.status}`);

  return (
    <li className="flex items-center gap-2 px-3 py-2">
      <div className="min-w-0 flex-1">
        <p className="truncate text-xs font-medium" title={item.name}>
          {item.name}
        </p>

        {done ? (
          <p className="text-muted-foreground truncate text-[0.6875rem] tabular-nums">{meta}</p>
        ) : (
          <>
            <Progress
              value={Math.round(item.percent)}
              className={cn("mt-1.5 h-1", failed && "opacity-50")}
              aria-label={t("dock.fileAria", { name: item.name })}
            />
            <p
              className={cn(
                "mt-1 truncate text-[0.6875rem] tabular-nums",
                failed ? "text-destructive" : "text-muted-foreground",
              )}
            >
              {meta}
            </p>
          </>
        )}
      </div>

      <div className="flex shrink-0 items-center gap-0.5">
        {done && item.url ? (
          <>
            <IconButton label={copied ? t("dock.copied") : t("dock.copy")} onClick={copy} small>
              {copied ? <Check className="text-primary size-3.5" /> : <Copy className="size-3.5" />}
            </IconButton>
            <a
              href={item.url}
              target="_blank"
              rel="noreferrer"
              aria-label={t("dock.open")}
              title={t("dock.open")}
              className="text-muted-foreground hover:bg-accent hover:text-foreground focus-visible:ring-ring grid size-7 place-items-center rounded-md transition-colors outline-none focus-visible:ring-2"
            >
              <ExternalLink className="size-3.5" />
            </a>
          </>
        ) : null}

        {failed ? (
          <IconButton label={t("dock.retry")} onClick={() => retryFile(item.id)} small>
            <RotateCw className="size-3.5" />
          </IconButton>
        ) : null}

        {!done && !failed ? (
          <IconButton
            label={item.status === "paused" ? t("dock.resume") : t("dock.pause")}
            onClick={() => toggleFile(item.id)}
            small
          >
            {item.status === "paused" ? <Play className="size-3.5" /> : <Pause className="size-3.5" />}
          </IconButton>
        ) : null}

        <IconButton
          label={done || failed ? t("dock.remove") : t("dock.cancel")}
          onClick={() => cancelFile(item.id)}
          small
        >
          <X className="size-3.5" />
        </IconButton>
      </div>
    </li>
  );
}

/* -------------------------------------------------------------------------- */
/* Bits                                                                        */
/* -------------------------------------------------------------------------- */

function IconButton({
  label,
  onClick,
  children,
  small,
  expanded,
}: {
  label: string;
  onClick: () => void;
  children: React.ReactNode;
  small?: boolean;
  expanded?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      title={label}
      aria-expanded={expanded}
      className={cn(
        "text-muted-foreground hover:bg-accent hover:text-foreground focus-visible:ring-ring grid shrink-0 place-items-center rounded-md transition-colors outline-none focus-visible:ring-2",
        small ? "size-7" : "size-8",
      )}
    >
      {children}
    </button>
  );
}

/**
 * Remaining time, rounded to whatever unit reads best. Returns the message key
 * rather than the text so the caller keeps `useTranslations`' own typing.
 */
function etaParts(seconds: number | null): { key: string; count: number } | null {
  if (seconds === null || !Number.isFinite(seconds) || seconds <= 0) return null;
  if (seconds < 60) return { key: "dock.etaSeconds", count: Math.max(1, Math.round(seconds)) };
  if (seconds < 3600) return { key: "dock.etaMinutes", count: Math.round(seconds / 60) };
  return { key: "dock.etaHours", count: Math.round(seconds / 3600) };
}
