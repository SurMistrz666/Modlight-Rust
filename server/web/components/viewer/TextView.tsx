"use client";

import * as React from "react";
import { AlertCircle, Copy, Check, ExternalLink } from "lucide-react";
import { toast } from "sonner";
import { useTranslations } from "next-intl";
import { cn } from "@/lib/utils";
import { humanFileSize } from "@/lib/format";

/**
 * Why the contents are not on screen.
 *
 * The two cases used to share one boolean and one sentence, which meant a
 * storage read that threw told the visitor the file was too large - a statement
 * about a file we had in fact failed to open. They are different facts and they
 * deserve different sentences.
 */
export type TextUnavailable = "too-large" | "unreadable" | null | undefined;

interface TextViewProps {
  /** Raw text content, already read server-side. */
  content?: string | null;
  /** Original byte size of the file (named in the "too large" message). */
  size: number;
  /** Set when the contents could not be inlined, and why. */
  unavailable?: TextUnavailable;
  /**
   * The raw response for this file.
   *
   * Load-bearing in the unavailable branch: over the ceiling the page has
   * nothing to draw, and a dead end that only says "too large" leaves the
   * visitor no way to the bytes at all. The link is the way out.
   */
  rawUrl?: string;
}

/**
 * Renders text content (read server-side) in a styled monospace block with soft
 * line numbers, horizontal scroll, and a Copy button.
 *
 * THIS is where every byte of user content that reaches the page is escaped, and
 * it is escaped exactly once: the content arrives as a string and leaves as
 * React text children (`{line}` below), which React escapes on the way into the
 * HTML. There is no dangerouslySetInnerHTML here and there must never be one, so
 * `<script>alert(1)</script>` stored in a paste renders as those characters.
 * Running an uploaded document for real is a separate route with its own opaque
 * origin (lib/media/render-ticket.ts); it never happens on this page.
 */
export function TextView({ content, size, unavailable, rawUrl }: TextViewProps) {
  const t = useTranslations("viewer");
  const tCommon = useTranslations("common");
  const [copied, setCopied] = React.useState(false);
  const timer = React.useRef<ReturnType<typeof setTimeout> | null>(null);

  React.useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );

  if (unavailable || content == null) {
    const reason = unavailable === "unreadable" ? "unreadable" : "too-large";
    return (
      <div className="flex h-full w-full flex-col items-center justify-center gap-3 px-6 text-center text-sm text-muted-foreground">
        <AlertCircle className="size-5 text-muted-foreground" />
        <p className="max-w-md">
          {reason === "unreadable"
            ? t("text.unreadable")
            : t("text.tooLarge", { size: humanFileSize(size) })}
        </p>
        {rawUrl ? (
          <a
            href={rawUrl}
            target="_blank"
            rel="noreferrer"
            className="inline-flex items-center gap-1.5 rounded-md border border-border/70 bg-card/70 px-3 py-1.5 text-xs font-medium text-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
          >
            {t("text.openRaw")}
            <ExternalLink className="size-3 opacity-60" aria-hidden />
          </a>
        ) : null}
      </div>
    );
  }

  const lines = content.length === 0 ? [""] : content.replace(/\n$/, "").split("\n");
  const gutterWidth = String(lines.length).length;

  async function onCopy() {
    try {
      await navigator.clipboard.writeText(content ?? "");
      setCopied(true);
      toast.success(tCommon("copied"));
      if (timer.current) clearTimeout(timer.current);
      timer.current = setTimeout(() => setCopied(false), 1600);
    } catch {
      toast.error(t("copyButton.copyFailedBlocked"));
    }
  }

  return (
    // Fills the stage and scrolls inside itself — the page never scrolls, so the
    // floating action bar stays put while a long file is read.
    <div className="relative mx-auto h-full w-full max-w-5xl overflow-hidden rounded-xl border border-border/60 bg-card/40">
      <button
        type="button"
        onClick={onCopy}
        title={t("text.copyContents")}
        aria-label={t("text.copyContents")}
        className={cn(
          "absolute top-3 right-3 z-10 inline-flex items-center gap-1.5 rounded-md border border-border/70 bg-card/80 px-2.5 py-1.5 text-xs font-medium text-foreground shadow-sm backdrop-blur transition-colors hover:bg-accent hover:text-accent-foreground",
        )}
      >
        {copied ? (
          <Check className="size-3.5 text-emerald-500" />
        ) : (
          <Copy className="size-3.5" />
        )}
        {copied ? tCommon("copied") : tCommon("copy")}
      </button>

      <div className="h-full overflow-auto bg-[color-mix(in_oklch,var(--muted)_40%,transparent)]">
        <pre className="m-0 flex font-mono text-[13px] leading-relaxed text-foreground/90">
          {/* Soft line-number gutter. aria-hidden + select-none so a Copy or a
              manual text selection grabs only the content, not the numbers. */}
          <code
            aria-hidden
            className="sticky left-0 z-[1] shrink-0 select-none border-r border-border/60 bg-[color-mix(in_oklch,var(--muted)_55%,transparent)] py-4 pr-3 pl-4 text-right text-muted-foreground/60 sm:pl-6"
            style={{ minWidth: `${gutterWidth + 3}ch` }}
          >
            {lines.map((_, i) => (
              <span key={i} className="block">
                {i + 1}
              </span>
            ))}
          </code>
          <code className="block min-w-0 grow py-4 pr-6 pl-4 whitespace-pre">
            {lines.map((line, i) => (
              <span key={i} className="block">
                {line.length === 0 ? "​" : line}
              </span>
            ))}
          </code>
        </pre>
      </div>
    </div>
  );
}
