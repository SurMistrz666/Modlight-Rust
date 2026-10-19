"use client";

import * as React from "react";
import { Code2, ExternalLink, FileText, Play, ShieldAlert } from "lucide-react";
import { useTranslations } from "next-intl";
import { cn } from "@/lib/utils";
import { TextView, type TextUnavailable } from "@/components/viewer/TextView";
import type { SourceMode, SourceReading } from "@/lib/media/source-preview";

interface SourceViewProps {
  /** Raw text content, already read server-side. */
  content?: string | null;
  size: number;
  unavailable?: TextUnavailable;
  filename: string;
  /** The raw response for this file. Always offered, whatever else is. */
  rawUrl: string;
  /**
   * Every reading this file has, computed server-side by `sourceReadings`.
   *
   * Passed in rather than derived here so the URLs stay one function's
   * responsibility, and so this file imports no part of the MIME machinery: a
   * client component that called `showsSource` would drag lib/media/sniff.ts,
   * with its byte-signature tables, into the bundle of a public page. The type
   * import above is erased at compile time and costs nothing.
   */
  readings: SourceReading[];
  /** SVG runs with a stricter sandbox than HTML: no scripts at all. */
  shape?: "html" | "svg" | null;
  /** Display name of the language ("Python"), or null when there is nothing to say. */
  language?: string | null;
}

const ICONS: Record<SourceMode, React.ComponentType<{ className?: string }>> = {
  source: Code2,
  page: Play,
  raw: FileText,
};

/**
 * Source first, always.
 *
 * An uploaded page is someone else's code, so it is shown as code until a human
 * decides otherwise - for everyone, including the file's own owner. Opening on
 * the rendered reading would mean every visit to a shared link runs an upload's
 * script, and the one thing a sandboxed document can still do is look
 * convincing; making that take a deliberate click is the whole difference
 * between hosting a page and publishing one. The choice lives in this
 * component's state and nowhere else: not in the URL (a link carrying
 * ?render=1 would be a one-click phishing page under our domain), not in a
 * cookie, not in the database. Switching back unmounts the frame, which is what
 * kills whatever it was running.
 */
export function SourceView({
  content,
  size,
  unavailable,
  filename,
  rawUrl,
  readings,
  shape,
  language,
}: SourceViewProps) {
  const t = useTranslations("viewer.render");
  const [mode, setMode] = React.useState<SourceMode>("source");

  const renderPath = readings.find((r) => r.mode === "page")?.href ?? null;
  const showPage = mode === "page" && renderPath !== null;

  return (
    <div
      className={cn(
        "mx-auto flex h-full w-full min-h-0 flex-col gap-2",
        // Code wants a measure: past roughly 100 characters a line stops being
        // readable, so the source stays at 5xl. A rendered page wants the
        // opposite - it was authored for a browser window, and 1024px makes
        // every layout look broken. The width is therefore part of the mode,
        // and it eases between the two so the switch reads as the frame opening
        // up rather than as the page jumping.
        //
        // Both ends are rem lengths on purpose: `max-w-none` is not a length,
        // so a transition to it would snap instead of animate.
        "transition-[max-width] motion-reduce:transition-none",
        // The easing is per DIRECTION, and it has to be. A single fast-out
        // curve looks right opening (the frame leaps out and settles) and wrong
        // closing: the box is clamped by the viewport, so the top of the range
        // is invisible, and a curve that front-loads its motion spends the whole
        // VISIBLE shrink in the first hundred milliseconds and then crawls
        // through a tail nobody can see. That reads as no animation at all.
        // Closing therefore uses a symmetric curve, which spreads the same
        // distance across the full duration.
        showPage
          ? "duration-500 ease-[cubic-bezier(0.32,0.72,0,1)]"
          : "duration-[420ms] ease-[cubic-bezier(0.4,0,0.2,1)]",
        showPage ? "max-w-[120rem]" : "max-w-5xl",
      )}
    >
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <div
          role="group"
          aria-label={t("label")}
          className="inline-flex shrink-0 rounded-lg border border-border/70 bg-card/60 p-0.5"
        >
          {readings.map((reading) => {
            const Icon = ICONS[reading.mode];
            const icon = <Icon className="size-3.5" aria-hidden />;
            const label = t(reading.mode);

            // `raw` is a link, not a mode. Re-printing the same bytes inside
            // this page with the decoration turned off would be a costume; the
            // point of the button is to reach the actual response, with its own
            // Content-Type and its own headers.
            if (reading.mode === "raw" && reading.href) {
              return (
                <a
                  key={reading.mode}
                  href={reading.href}
                  target="_blank"
                  rel="noreferrer"
                  title={t("rawTitle")}
                  className="inline-flex items-center gap-1.5 rounded-md px-2.5 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
                >
                  {icon}
                  {label}
                  <ExternalLink className="size-3 opacity-60" aria-hidden />
                </a>
              );
            }

            return (
              <ToggleButton
                key={reading.mode}
                active={mode === reading.mode}
                onClick={() => setMode(reading.mode)}
                icon={icon}
                label={label}
              />
            );
          })}
        </div>

        {language ? (
          <span
            aria-label={t("languageLabel", { language })}
            className="shrink-0 rounded-md border border-border/60 bg-card/60 px-2 py-1 text-[11px] font-medium text-muted-foreground"
          >
            {language}
          </span>
        ) : null}

        {renderPath ? (
          <p className="flex min-w-0 items-start gap-1.5 text-[11px] leading-snug text-muted-foreground">
            <ShieldAlert className="mt-px size-3.5 shrink-0 opacity-70" aria-hidden />
            <span>
              {t("notice")}
              {showPage ? <span className="ml-1 opacity-80">{t("inert")}</span> : null}
            </span>
          </p>
        ) : null}
      </div>

      <div className="min-h-0 flex-1">
        {showPage ? (
          <iframe
            // No allow-same-origin, ever: paired with allow-scripts it is not a
            // weaker sandbox but no sandbox at all, and the frame could simply
            // strip the attribute off itself through window.parent.
            sandbox={shape === "svg" ? "" : "allow-scripts"}
            src={renderPath}
            title={t("frameTitle", { filename })}
            referrerPolicy="no-referrer"
            allow=""
            className="h-full w-full rounded-xl border border-border/60 bg-card"
          />
        ) : (
          <TextView content={content} size={size} unavailable={unavailable} rawUrl={rawUrl} />
        )}
      </div>
    </div>
  );
}

function ToggleButton({
  active,
  onClick,
  icon,
  label,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  label: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={cn(
        "inline-flex items-center gap-1.5 rounded-md px-2.5 py-1.5 text-xs font-medium transition-colors",
        active
          ? "bg-primary text-primary-foreground shadow-sm"
          : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
      )}
    >
      {icon}
      {label}
    </button>
  );
}
