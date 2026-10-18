"use client";

import dynamic from "next/dynamic";
import { useState } from "react";
import { useFormatter, useTranslations } from "next-intl";
import { AlertTriangle } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { HARDWARE_DECODER_MAX_EDGE, type PlaybackDiagnosis } from "@/lib/media/playback-health";
import type { FileItem } from "@/components/dashboard/types";

/**
 * Everything the repair needs, or null when there is nobody to offer it to.
 *
 * Present ONLY for the owner. A visitor holding a public link cannot re-encode
 * somebody else's file and does not need to be told that the recording they are
 * about to watch is a poor one - they can see that for themselves, and the
 * sentence would read as the site apologising for its own user.
 */
export interface PlaybackRepair {
  item: FileItem;
  userCode: string;
  appUrl: string;
}

/**
 * The dialog is the existing one, and that is the point.
 *
 * "Repair" is the same operation as Convert to `webmp4`, so it goes through the
 * same component: the same replace-or-copy choice, the same sentence about which
 * link survives, the same limiter, the same retry. A second confirmation UI for
 * one target would be a second place for the destructive path to be described
 * differently from the first.
 *
 * Lazily imported because this note is rare (12 of the 464 videos on production)
 * and the dialog pulls in the whole conversion planner. The public file page
 * must not carry that in its first load for every visitor who will never see it.
 */
const ConvertDialog = dynamic(
  () => import("@/components/dashboard/ConvertDialog").then((m) => m.ConvertDialog),
  { ssr: false },
);

/**
 * The quiet warning on a video whose SHAPE means it will not play well.
 *
 * Owner-only, on the file's own page and in the dashboard's detail sheet. Not a
 * modal and not a banner on every video: it fires on `diagnosePlayback`, which
 * answers for 2.6% of the videos on production, so most libraries never see it.
 *
 * LAYOUT: the breakpoints here are CONTAINER queries, not viewport ones, and
 * that is load-bearing rather than fashionable. The same component renders in
 * two boxes of very different widths - a bar spanning four fifths of the window,
 * and a 28rem sheet - on the same screen. Keyed to the viewport, the sheet
 * inherited the wide layout and squeezed the sentences into a 90px column beside
 * the button. Keyed to its own box, each placement gets the right one.
 */
export function PlaybackNote({
  diagnosis,
  repair,
  className,
}: {
  diagnosis: PlaybackDiagnosis;
  repair: PlaybackRepair | null;
  className?: string;
}) {
  const t = useTranslations("files");
  const format = useFormatter();
  const [open, setOpen] = useState(false);

  // Formatted here rather than in the message so the reader's own locale decides
  // the decimal separator: 0.0042 in English, 0,0042 in Polish.
  const bpp =
    diagnosis.bitsPerPixel != null
      ? format.number(diagnosis.bitsPerPixel, {
          minimumFractionDigits: 4,
          maximumFractionDigits: 4,
        })
      : null;

  const canRepair = diagnosis.repairable && repair !== null;

  return (
    <>
      <div className={cn("@container", className)}>
        <div className="ring-foreground/10 bg-muted/40 flex flex-col gap-2.5 rounded-lg p-3 ring-1 @md:flex-row @md:items-start @md:gap-4">
          <div className="min-w-0 flex-1">
            <p className="text-foreground flex items-center gap-1.5 text-sm font-medium">
              <AlertTriangle className="text-muted-foreground size-4 shrink-0" aria-hidden />
              {t("health.title")}
            </p>
            <div className="text-muted-foreground mt-1 space-y-1 text-xs leading-relaxed">
              {diagnosis.faults.includes("oversized") ? (
                <p>
                  {t("health.oversized", {
                    edge: diagnosis.edge,
                    wall: HARDWARE_DECODER_MAX_EDGE,
                  })}
                </p>
              ) : null}
              {diagnosis.faults.includes("starved") && bpp ? (
                <p>
                  {t("health.starved", { bpp, factor: diagnosis.starvationFactor ?? 0 })}
                </p>
              ) : null}
              {/* Nothing can put back detail the recorder never wrote, so a file
                  that is starved but already small gets the diagnosis and an
                  honest "there is nothing to press" rather than a button that
                  would hand back the same picture at the same size. */}
              {!canRepair ? <p>{t("health.noRepair")}</p> : null}
            </div>
          </div>
          {canRepair ? (
            <div className="shrink-0 @md:max-w-[14rem] @md:text-right">
              <Button variant="outline" size="sm" onClick={() => setOpen(true)}>
                {t("health.repair")}
              </Button>
              <p className="text-muted-foreground mt-1 text-[11px] leading-snug">
                {t("health.repairHint", { edge: diagnosis.repairEdge })}
              </p>
            </div>
          ) : null}
        </div>
      </div>

      {canRepair && repair ? (
        <ConvertDialog
          open={open}
          onOpenChange={setOpen}
          items={[repair.item]}
          to="webmp4"
          userCode={repair.userCode}
          appUrl={repair.appUrl}
        />
      ) : null}
    </>
  );
}
