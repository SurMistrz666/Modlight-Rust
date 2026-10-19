"use client";

import * as React from "react";
import { FileText, ShieldAlert } from "lucide-react";
import { useTranslations } from "next-intl";
import { CopyButton } from "@/components/viewer/CopyButton";
import { humanFileSize } from "@/lib/format";

interface DocumentViewProps {
  /**
   * Sandboxed render URL with its ticket, or null when no preview is on offer -
   * the file is too big, or it is a format we cannot read.
   */
  renderPath: string | null;
  /** Why there is no preview, when there is none. */
  unavailable?: "legacy-doc" | "too-large" | null;
  filename: string;
  rawUrl: string;
  size: number;
}

/**
 * A Word document, shown as a document.
 *
 * Deliberately NOT the SourceView used for uploaded HTML and SVG, even though
 * both end up in the same sandboxed frame. That component's whole shape is a
 * Source/Page toggle, and "source" for a .docx would mean a zip archive full of
 * XML - an offer that cannot be honoured and would not be wanted if it could.
 *
 * The notice is load-bearing, not decoration. mammoth reads the document BODY
 * and produces semantic HTML; it does not read page geometry, so margins, page
 * breaks, columns, headers and footers are not represented. Somebody checking
 * whether a contract fits on two pages must not be able to mistake this for the
 * answer, which is also why nothing here says "PDF" or offers to produce one.
 */
export function DocumentView({
  renderPath,
  unavailable,
  filename,
  rawUrl,
  size,
}: DocumentViewProps) {
  const t = useTranslations("viewer.document");

  if (!renderPath) {
    return (
      <div className="flex h-full w-full flex-col items-center justify-center gap-5 px-6 text-center">
        <div className="flex size-28 items-center justify-center rounded-3xl border border-border/70 bg-gradient-to-br from-primary/15 via-primary/5 to-transparent">
          <FileText className="size-12 text-primary" />
        </div>
        <div className="space-y-1">
          <p className="max-w-md truncate font-medium text-foreground" title={filename}>
            {filename}
          </p>
          <p className="mx-auto max-w-sm text-sm text-muted-foreground">
            {/* Naming the reason beats a blank stage. The legacy .doc case in
                particular is a format limit, not a broken file, and saying so
                stops it reading as "the document is empty". */}
            {unavailable === "legacy-doc" ? t("legacyDoc") : t("tooLarge", { size: humanFileSize(size) })}
          </p>
        </div>
        <CopyButton value={rawUrl} label={t("copyLink")} variant="outline" />
      </div>
    );
  }

  return (
    <div className="mx-auto flex h-full w-full min-h-0 max-w-5xl flex-col gap-2">
      <p className="flex min-w-0 items-start gap-1.5 px-1 text-[11px] leading-snug text-muted-foreground">
        <ShieldAlert className="mt-px size-3.5 shrink-0 opacity-70" aria-hidden />
        <span>{t("notice")}</span>
      </p>

      <div className="min-h-0 flex-1">
        <iframe
          // No allow-scripts and no allow-same-origin. A converted document has
          // nothing to run, and sandbox flags from this attribute intersect with
          // the response policy, so the config-level CSP that grants scripts to
          // uploaded HTML cannot loosen this one.
          sandbox=""
          src={renderPath}
          title={t("frameTitle", { filename })}
          referrerPolicy="no-referrer"
          allow=""
          className="h-full w-full rounded-xl border border-border/60 bg-card"
        />
      </div>
    </div>
  );
}
