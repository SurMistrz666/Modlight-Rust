"use client";

import { useEffect } from "react";
import Link from "next/link";
import { useTranslations } from "next-intl";
import { RotateCw, TriangleAlert } from "lucide-react";
import { BrandTile } from "@/components/auth/brand-mark";
import { Button } from "@/components/ui/button";

/**
 * Application-wide boundary. Anything that escapes a nested error.tsx lands
 * here — including a broken public link someone was sent — so it has to look
 * like the product rather than like a stack trace, and it has to offer a way
 * out even to a visitor who has no account.
 */
export default function GlobalRouteError({
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
    <main className="relative grid min-h-dvh w-full place-items-center overflow-hidden px-6 py-16">
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 -z-10"
        style={{
          backgroundImage:
            "radial-gradient(48rem 32rem at 50% -10%, color-mix(in oklch, var(--destructive) 16%, transparent), transparent 70%)",
        }}
      />
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 -z-10 opacity-[0.05] [background-image:linear-gradient(to_right,currentColor_1px,transparent_1px),linear-gradient(to_bottom,currentColor_1px,transparent_1px)] [background-size:32px_32px] [mask-image:radial-gradient(ellipse_at_center,black,transparent_75%)]"
      />

      <div className="flex w-full max-w-md flex-col items-center gap-6 text-center">
        <Link
          href="/"
          className="text-muted-foreground hover:text-foreground inline-flex items-center gap-2 text-sm font-semibold tracking-tight transition-colors"
        >
          <BrandTile className="size-6" />
          Modlight
        </Link>

        <div className="bg-destructive/10 text-destructive ring-destructive/20 grid size-16 place-items-center rounded-3xl ring-1">
          <TriangleAlert className="size-8" />
        </div>

        <div className="space-y-2">
          <h1 className="font-heading text-2xl font-semibold tracking-tight text-balance">
            {t("errors.title")}
          </h1>
          <p className="text-muted-foreground text-sm text-pretty">{t("errors.desc")}</p>
        </div>

        {error.digest ? (
          <p className="text-muted-foreground/80 bg-muted/60 ring-foreground/10 rounded-full px-3 py-1 font-mono text-[0.7rem] ring-1">
            {t("errors.reference")}: {error.digest}
          </p>
        ) : null}

        <div className="flex flex-col items-center gap-2 sm:flex-row">
          <Button size="lg" onClick={reset}>
            <RotateCw className="size-4" />
            {t("errors.retry")}
          </Button>
          <Button size="lg" variant="ghost" asChild>
            <Link href="/">{t("errors.home")}</Link>
          </Button>
        </div>
      </div>
    </main>
  );
}
