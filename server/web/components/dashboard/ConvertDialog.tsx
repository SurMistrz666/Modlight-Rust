"use client";

/**
 * "Convert" - the destructive sibling of "Download as…".
 *
 * Download as hands you a copy and leaves the file alone. This one changes what
 * is stored, so the whole component is built around three sentences the reader
 * has to be able to act on BEFORE anything happens:
 *
 *   1. how many of the selected files this actually touches, and why the rest
 *      are being left alone (already in that format is not a failure);
 *   2. whether the original survives, said in terms of files and links rather
 *      than in terms of the word "irreversible";
 *   3. what it costs, in bytes, with the honesty rules of convert-size.ts.
 *
 * Files are converted STRICTLY ONE AT A TIME. The shared limiter admits two
 * transforms across the entire server (lib/media/transform-cost.ts), so firing a
 * selection in parallel would refuse its own later files with "too many
 * transforms" and take every thumbnail on the site down with it. One at a time
 * is also what makes the progress list truthful.
 *
 * No refusal is allowed to disappear. Every file ends in exactly one of
 * converted / skipped / failed, each failure carries a sentence, and the summary
 * counts all three.
 */

import { useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslations } from "next-intl";
import { toast } from "sonner";
import { noteKey } from "@/lib/media/convert-notes";
import {
  AlertTriangle,
  ArrowRight,
  Check,
  Copy,
  Loader2,
  Minus,
  Replace,
  RotateCcw,
  X,
} from "lucide-react";
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
import { humanFileSize } from "@/lib/format";
import { convertUpload } from "@/app/actions/convert";
import {
  planBulkConversion,
  type ConvertCandidate,
  type ConvertMode,
  type ConvertOutcome,
  type ConvertSkipReason,
} from "@/lib/media/convert-plan";
import {
  downloadHintKey,
  downloadLabelKey,
  downloadOptionsFor,
} from "@/lib/media/download-menu";
import { mediaKind } from "@/lib/media/mime";
import { estimateTargetSize, estimateTotalSize, formatEstimate } from "@/lib/media/convert-size";
import { buildPublicUrl } from "./media";
import type { FileItem } from "./types";

/**
 * A "too many transforms in progress" answer is a queue, not an error: two
 * conversions may already be running for somebody else. The route's own
 * Retry-After is 5 s, so this waits the same and gives up after three attempts
 * rather than holding the reader's selection hostage.
 */
const BUSY_RETRIES = 2;
const BUSY_DELAY_MS = 5000;

type RowStatus =
  | "waiting"
  | "running"
  | "retrying"
  | "done"
  | "failed"
  /** The server re-checked the gates and found nothing to do after all. */
  | "skipped"
  | "stopped";

interface RowState {
  status: RowStatus;
  /** Failure reason key, for the sentence under the name. */
  reason?: string;
  before: number;
  after?: number;
  /** Something true about the result nobody asked for ("clipped"). */
  note?: string;
}

export interface ConvertDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The files the reader picked. Mixed types are expected and handled. */
  items: FileItem[];
  /** Target format name, from the same table the menu renders. */
  to: string;
  userCode: string;
  /** The owner's chosen hostname, so the dialog can show the link that survives. */
  appUrl: string;
  /** Called once the run has finished, e.g. to drop the selection. */
  onFinished?: () => void;
}

/** The columns the planner reads, out of what the dashboard already has. */
export function fileToCandidate(item: FileItem): ConvertCandidate {
  return {
    id: item.id,
    ext: item.ext,
    mimeType: item.mimeType,
    frames: item.frames,
    // Carried so the web-playable target can be planned for the same files the
    // menu offered it on; a candidate without them simply never sees it.
    width: item.width,
    height: item.height,
    size: item.size,
    // The dashboard's `kind` is the coarse display type; only "paste" matters
    // here, and it means the same thing in both vocabularies.
    kind: item.kind === "paste" ? "paste" : "file",
    // Trashed files are not in this list at all - the trash has its own view.
    deletedAt: null,
    expiresAt: item.expiresAt ? new Date(item.expiresAt) : null,
  };
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

export function ConvertDialog({
  open,
  onOpenChange,
  items,
  to,
  userCode,
  appUrl,
  onFinished,
}: ConvertDialogProps) {
  const t = useTranslations("files");
  const tCommon = useTranslations("common");
  const router = useRouter();

  const [mode, setMode] = useState<ConvertMode>("copy");
  const [phase, setPhase] = useState<"confirm" | "running" | "done">("confirm");
  const [rows, setRows] = useState<Record<string, RowState>>({});
  const stopRef = useRef(false);
  const runningRef = useRef(false);

  // "WEBMP4" is an internal target name, not a format anybody recognises, so it
  // gets a sentence of its own; every other target is its own best label.
  const formatKey = downloadLabelKey(to);
  const format = formatKey ? t(formatKey) : to.toUpperCase();
  const byId = useMemo(() => new Map(items.map((i) => [i.id, i])), [items]);

  // A copy is the reversible choice, so a fresh dialog always starts there:
  // pressing Enter must never be the destructive gesture.
  useEffect(() => {
    if (!open) return;
    // Reopening while a backgrounded run is still going must show that run, not
    // wipe its list and pretend nothing is happening.
    if (runningRef.current) return;
    setMode("copy");
    setPhase("confirm");
    setRows({});
    stopRef.current = false;
  }, [open, to]);

  const plan = useMemo(
    () => planBulkConversion(items.map(fileToCandidate), to, mode),
    [items, to, mode],
  );

  const eligible = useMemo(
    () => plan.convert.map((c) => byId.get(c.id)).filter((i): i is FileItem => !!i),
    [plan, byId],
  );

  const skippedByReason = useMemo(() => {
    const map = new Map<ConvertSkipReason, FileItem[]>();
    for (const s of plan.skipped) {
      const item = byId.get(s.id);
      if (!item) continue;
      const list = map.get(s.reason) ?? [];
      list.push(item);
      map.set(s.reason, list);
    }
    return map;
  }, [plan, byId]);

  const sourceBytes = eligible.reduce((sum, i) => sum + i.size, 0);
  const total = estimateTotalSize(eligible, to);

  /**
   * Does this conversion throw away most of the source?
   *
   * A video turned into an animation keeps a few seconds of it, silently: that
   * is the right trade for "make me a shareable clip", and a catastrophe for
   * "change this file's format". The generic "cannot be undone" is not enough
   * warning, because it describes the ACTION and the surprise is in the RESULT -
   * so when the replace card is the destructive one, it has to say what is lost.
   */
  const replaceClips = useMemo(
    () =>
      (to === "gif" || to === "webp") &&
      eligible.some((i) => mediaKind(i.mimeType, i.ext) === "video"),
    [eligible, to],
  );

  /**
   * What the result IS, when the format name alone does not say it: a clip out
   * of a video, the sound without the picture, one frame out of many. This is
   * what stands in the "after" column when the size cannot honestly be given.
   */
  const resultHint = useMemo(() => {
    const first = items.length === 1 ? items[0] : null;
    if (!first) return null;
    const option = downloadOptionsFor(first.mimeType, first.ext, first.frames, {
      canEncode: true,
      width: first.width,
      height: first.height,
    }).find((o) => o.to === to);
    return option ? downloadHintKey(option) : null;
  }, [items, to]);

  /**
   * The one-file case, and it is deliberately about the SELECTION rather than
   * about how many files survived the plan. Three files of which one is eligible
   * still needs the breakdown ("2 are already PNG"), which is exactly the thing
   * the user asked to be told before clicking.
   */
  const single = items.length === 1 ? (eligible[0] ?? null) : null;
  const singleEstimate = single ? estimateTargetSize(single, to) : null;

  const done = Object.values(rows).filter((r) => r.status === "done").length;
  const failed = Object.values(rows).filter((r) => r.status === "failed").length;
  const finishedCount = done + failed;

  // ---- Running ------------------------------------------------------------

  const run = useCallback(async () => {
    if (runningRef.current || eligible.length === 0) return;
    runningRef.current = true;
    stopRef.current = false;

    const initial: Record<string, RowState> = {};
    for (const item of eligible) initial[item.id] = { status: "waiting", before: item.size };
    setRows(initial);
    setPhase("running");

    const patch = (id: string, next: Partial<RowState>) =>
      setRows((prev) => ({ ...prev, [id]: { ...prev[id], ...next } as RowState }));

    let converted = 0;
    let failures = 0;
    let lateSkips = 0;

    for (const item of eligible) {
      if (stopRef.current) {
        patch(item.id, { status: "stopped" });
        continue;
      }
      patch(item.id, { status: "running" });

      let outcome: ConvertOutcome | null = null;
      for (let attempt = 0; attempt <= BUSY_RETRIES; attempt++) {
        try {
          outcome = await convertUpload(item.id, to, mode);
        } catch {
          // A thrown action is a transport or session failure; the server never
          // throws for a refusal it can name.
          outcome = { status: "failed", uploadId: item.id, reason: "encoder" };
        }
        if (outcome.status === "failed" && outcome.reason === "busy" && attempt < BUSY_RETRIES) {
          patch(item.id, { status: "retrying" });
          await sleep(BUSY_DELAY_MS);
          patch(item.id, { status: "running" });
          continue;
        }
        break;
      }

      if (!outcome) continue;

      if (outcome.status === "converted" || outcome.status === "created") {
        converted++;
        patch(item.id, { status: "done", after: outcome.size, note: outcome.note });
      } else if (outcome.status === "failed") {
        failures++;
        patch(item.id, { status: "failed", reason: outcome.reason });
      } else {
        // The server re-checks every gate the client planned against, and it is
        // the authority. A file it now considers a no-op is neither a success
        // nor a failure, and saying so is the whole point of three outcomes.
        lateSkips++;
        patch(item.id, { status: "skipped" });
      }
    }

    runningRef.current = false;
    setPhase("done");
    router.refresh();
    onFinished?.();

    const skipped = plan.skipped.length + lateSkips;
    if (stopRef.current) {
      toast.info(t("convert.stopped", { done: converted, total: eligible.length }));
    } else if (failures > 0) {
      toast.error(
        t("convert.doneSummaryFailed", { ok: converted, failed: failures, skipped }),
      );
    } else {
      toast.success(t("convert.doneSummary", { ok: converted, skipped }));
    }
  }, [eligible, mode, to, plan.skipped.length, router, onFinished, t]);

  const retryOne = useCallback(
    async (id: string) => {
      const item = byId.get(id);
      if (!item) return;
      setRows((prev) => ({ ...prev, [id]: { ...prev[id], status: "running" } }));
      let outcome: ConvertOutcome;
      try {
        outcome = await convertUpload(id, to, mode);
      } catch {
        outcome = { status: "failed", uploadId: id, reason: "encoder" };
      }
      setRows((prev) => ({
        ...prev,
        [id]:
          outcome.status === "converted" || outcome.status === "created"
            ? { ...prev[id], status: "done", after: outcome.size, note: outcome.note }
            : outcome.status === "failed"
              ? { ...prev[id], status: "failed", reason: outcome.reason }
              : { ...prev[id], status: "skipped" },
      }));
      router.refresh();
    },
    [byId, to, mode, router],
  );

  // ---- Copy --------------------------------------------------------------

  const link = single ? buildPublicUrl(appUrl, userCode, single.code, single.ext) : null;
  const nothingToDo = eligible.length === 0;

  const title =
    phase === "confirm"
      ? single
        ? t("convert.dialogTitle", { format })
        : t("convert.dialogTitleMany", {
            n: eligible.length,
            total: items.length,
            format,
          })
      : t("convert.progress", { done: finishedCount, total: eligible.length });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription className="truncate">
            {items.length === 1
              ? items[0].filename
              : t("convert.describeMany", { count: items.length })}
          </DialogDescription>
        </DialogHeader>

        {phase === "confirm" ? (
          <div className="flex flex-col gap-4">
            {/* Before / after */}
            {single ? (
              <div className="ring-foreground/10 flex items-center gap-3 rounded-lg p-3 ring-1">
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-medium">
                    {(single.ext || "").toUpperCase() || t("convert.unknownFormat")}
                  </p>
                  <p className="text-muted-foreground text-xs tabular-nums">
                    {humanFileSize(single.size)}
                  </p>
                </div>
                <ArrowRight className="text-muted-foreground size-4 shrink-0" aria-hidden />
                <div className="min-w-0 flex-1 text-right">
                  <p className="truncate text-sm font-medium">{format}</p>
                  <p className="text-muted-foreground text-xs tabular-nums">
                    {singleEstimate
                      ? singleEstimate.confidence === "exact"
                        ? formatEstimate(singleEstimate.bytes)
                        : t("convert.approxSize", { size: formatEstimate(singleEstimate.bytes) })
                      : resultHint
                        ? t(resultHint)
                        : t("convert.unknownSize")}
                  </p>
                </div>
              </div>
            ) : (
              <ul className="ring-foreground/10 flex flex-col gap-1.5 rounded-lg p-3 text-sm ring-1">
                <li className="flex items-center justify-between gap-3">
                  <span>{t("convert.willConvert", { n: eligible.length, format })}</span>
                  <span className="text-muted-foreground shrink-0 text-xs tabular-nums">
                    {humanFileSize(sourceBytes)}
                    {total.counted > 0 ? (
                      <>
                        {" "}
                        <ArrowRight className="inline size-3" aria-hidden />{" "}
                        {t("convert.approxSize", { size: formatEstimate(total.bytes) })}
                      </>
                    ) : null}
                  </span>
                </li>
                {total.unknown > 0 && total.counted > 0 ? (
                  <li className="text-muted-foreground text-xs">
                    {t("convert.unknownResultSize", { n: total.unknown })}
                  </li>
                ) : null}
                {[...skippedByReason.entries()].map(([reason, list]) => (
                  <li key={reason} className="flex items-center justify-between gap-3">
                    <span className="text-muted-foreground">
                      {t(`convert.skip.${skipKey(reason)}`, { n: list.length, format })}
                    </span>
                    <span className="text-muted-foreground shrink-0 text-xs">
                      {t("convert.unchanged")}
                    </span>
                  </li>
                ))}
              </ul>
            )}

            {/* Keep or replace */}
            <div role="radiogroup" aria-label={t("convert.modeLabel")} className="flex flex-col gap-2">
              <ModeCard
                selected={mode === "copy"}
                onSelect={() => setMode("copy")}
                icon={<Copy className="size-4" />}
                title={single ? t("convert.keepTitle") : t("convert.keepTitleMany")}
                body={
                  single
                    ? t("convert.keepBody", {
                        format: (single.ext || "").toUpperCase(),
                        size: total.counted > 0 ? formatEstimate(total.bytes) : t("convert.someSpace"),
                      })
                    : t("convert.keepBodyMany", {
                        n: eligible.length,
                        size: total.counted > 0 ? formatEstimate(total.bytes) : t("convert.someSpace"),
                      })
                }
              />
              <ModeCard
                selected={mode === "replace"}
                onSelect={() => setMode("replace")}
                icon={<Replace className="size-4" />}
                title={single ? t("convert.replaceTitle") : t("convert.replaceTitleMany")}
                body={
                  single && link
                    ? t("convert.replaceBody", {
                        url: link.replace(/^https?:\/\//, ""),
                        format,
                      })
                    : t("convert.replaceBodyMany")
                }
                warning={replaceClips ? t("convert.replaceClips") : t("convert.replaceNoUndo")}
              />
            </div>

            {nothingToDo ? (
              <p className="text-muted-foreground text-sm">
                {t("convert.nothingToDo", { format })}
              </p>
            ) : null}
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            <Progress
              value={eligible.length ? (finishedCount / eligible.length) * 100 : 0}
              aria-label={t("convert.progress", { done: finishedCount, total: eligible.length })}
            />
            <ul className="ring-foreground/10 divide-foreground/10 max-h-64 divide-y overflow-y-auto rounded-lg ring-1">
              {eligible.map((item) => (
                <ResultRow
                  key={item.id}
                  name={item.filename}
                  state={rows[item.id]}
                  onRetry={() => retryOne(item.id)}
                />
              ))}
            </ul>
            {phase === "running" ? (
              <p className="text-muted-foreground text-xs">{t("convert.keepTabOpen")}</p>
            ) : null}
          </div>
        )}

        <DialogFooter showCloseButton={phase === "confirm"}>
          {phase === "confirm" ? (
            <Button onClick={run} disabled={nothingToDo}>
              {mode === "copy" ? <Copy className="size-4" /> : <Replace className="size-4" />}
              {mode === "copy"
                ? single
                  ? t("convert.confirmCopy")
                  : t("convert.confirmCopyMany", { n: eligible.length })
                : single
                  ? t("convert.confirmReplace")
                  : t("convert.confirmReplaceMany", { n: eligible.length })}
            </Button>
          ) : phase === "running" ? (
            <>
              <Button variant="ghost" onClick={() => onOpenChange(false)}>
                {t("convert.background")}
              </Button>
              <Button
                variant="outline"
                onClick={() => {
                  stopRef.current = true;
                }}
              >
                <X className="size-4" />
                {t("convert.stop")}
              </Button>
            </>
          ) : (
            <Button onClick={() => onOpenChange(false)}>{tCommon("close")}</Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** Skip reasons are kebab-case in the domain and camelCase in the dictionary. */
function skipKey(reason: ConvertSkipReason): string {
  switch (reason) {
    case "already-in-format":
      return "alreadyInFormat";
    case "unsupported-target":
      return "unsupported";
    default:
      return reason;
  }
}

/** Failure reasons, same idea. Every one of them has a sentence. */
function failureKey(reason: string): string {
  switch (reason) {
    case "not-found":
      return "notFound";
    case "unsupported-target":
      return "unsupported";
    case "too-large":
      return "tooLarge";
    case "no-space":
      return "noSpace";
    default:
      return reason;
  }
}

function ModeCard({
  selected,
  onSelect,
  icon,
  title,
  body,
  warning,
}: {
  selected: boolean;
  onSelect: () => void;
  icon: React.ReactNode;
  title: string;
  body: string;
  /** Shown only while this card is the chosen one, so it reads as a consequence. */
  warning?: string;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      onClick={onSelect}
      className={cn(
        "flex w-full items-start gap-3 rounded-lg p-3 text-left ring-1 transition-colors",
        selected
          ? "bg-primary/5 ring-primary/40"
          : "ring-foreground/10 hover:bg-muted/50",
      )}
    >
      <span className={cn("mt-0.5 shrink-0", selected ? "text-primary" : "text-muted-foreground")}>
        {icon}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-sm font-medium">{title}</span>
        <span className="text-muted-foreground block text-xs leading-relaxed break-words">
          {body}
        </span>
        {selected && warning ? (
          <span className="text-muted-foreground mt-1 block text-xs font-medium">{warning}</span>
        ) : null}
      </span>
      <span
        aria-hidden
        className={cn(
          "mt-1 size-3.5 shrink-0 rounded-full ring-1",
          selected ? "bg-primary ring-primary" : "ring-foreground/25",
        )}
      />
    </button>
  );
}

function ResultRow({
  name,
  state,
  onRetry,
}: {
  name: string;
  state: RowState | undefined;
  onRetry: () => void;
}) {
  const t = useTranslations("files");
  const status = state?.status ?? "waiting";

  return (
    <li className="flex items-center gap-2 px-3 py-2 text-sm">
      <span className="shrink-0">
        {status === "done" ? (
          <Check className="text-primary size-4" />
        ) : status === "failed" ? (
          <AlertTriangle className="text-destructive size-4" />
        ) : status === "running" || status === "retrying" ? (
          <Loader2 className="text-muted-foreground size-4 animate-spin" />
        ) : (
          <Minus className="text-muted-foreground size-4" />
        )}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate">{name}</span>
        {status === "failed" && state?.reason ? (
          <span className="text-destructive block text-xs">
            {t(`convert.reason.${failureKey(state.reason)}`)}
          </span>
        ) : status === "retrying" ? (
          <span className="text-muted-foreground block text-xs">{t("convert.retrying")}</span>
        ) : status === "waiting" ? (
          <span className="text-muted-foreground block text-xs">{t("convert.waiting")}</span>
        ) : status === "stopped" ? (
          <span className="text-muted-foreground block text-xs">{t("convert.notStarted")}</span>
        ) : status === "skipped" ? (
          <span className="text-muted-foreground block text-xs">{t("convert.notNeeded")}</span>
        ) : noteKey(state?.note) ? (
          <span className="text-muted-foreground block text-xs">
            {t(`convert.note.${noteKey(state?.note)}`)}
          </span>
        ) : null}
      </span>
      {status === "done" && state?.after != null ? (
        <span className="text-muted-foreground shrink-0 text-xs tabular-nums">
          {humanFileSize(state.before)} <ArrowRight className="inline size-3" aria-hidden />{" "}
          {humanFileSize(state.after)}
        </span>
      ) : status === "failed" ? (
        <Button variant="ghost" size="icon-sm" onClick={onRetry} aria-label={t("convert.retry")}>
          <RotateCcw className="size-3.5" />
        </Button>
      ) : null}
    </li>
  );
}

