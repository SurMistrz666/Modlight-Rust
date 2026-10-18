"use client";

import { useEffect } from "react";
import Link from "next/link";
import { useTranslations } from "next-intl";
import { AlertTriangle, RotateCw } from "lucide-react";
import { Button } from "@/components/ui/button";

/**
 * Segment-level boundary: the navigation shell survives, only the content area
 * is replaced. Recovering is one click — `reset()` re-renders the segment, so a
 * transient database or storage hiccup doesn't cost the user their place.
 */
export default function DashboardError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  useEffect(() => {
    console.error(error);
  }, [error]);

  const t = useTranslations("common");

  return (
    <div className="mx-auto w-full max-w-2xl py-10">
      <div className="bg-card/40 ring-foreground/10 relative flex flex-col items-center gap-5 overflow-hidden rounded-xl px-6 py-16 text-center ring-1">
        <div
          aria-hidden
          className="pointer-events-none absolute inset-0 opacity-[0.04] [background-image:linear-gradient(to_right,currentColor_1px,transparent_1px),linear-gradient(to_bottom,currentColor_1px,transparent_1px)] [background-size:24px_24px]"
        />

        <div className="bg-destructive/10 text-destructive ring-destructive/20 relative grid size-14 place-items-center rounded-2xl ring-1">
          <AlertTriangle className="size-7" />
        </div>

        <div className="relative space-y-1.5">
          <h2 className="font-heading text-lg font-semibold tracking-tight">
            {t("errors.title")}
          </h2>
          <p className="text-muted-foreground mx-auto max-w-sm text-sm">{t("errors.desc")}</p>
        </div>

        {error.digest ? (
          <p className="text-muted-foreground/80 bg-muted/60 ring-foreground/10 relative rounded-full px-3 py-1 font-mono text-[0.7rem] ring-1">
            {t("errors.reference")}: {error.digest}
          </p>
        ) : null}

        <div className="relative flex flex-col items-center gap-2 sm:flex-row">
          <Button onClick={reset}>
            <RotateCw className="size-4" />
            {t("errors.retry")}
          </Button>
          <Button variant="ghost" asChild>
            <Link href="/dashboard">{t("errors.backToFiles")}</Link>
          </Button>
        </div>
      </div>
    </div>
  );
}
