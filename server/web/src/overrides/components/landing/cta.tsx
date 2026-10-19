// Client port of upstream components/landing/cta.tsx (an async Server Component there).
// @upstream components/landing/cta.tsx
// Deviation: `async` dropped and getTranslations (next-intl/server) replaced by
// useTranslations (next-intl); nothing else differs.
import Link from "next/link";
import { useTranslations } from "next-intl";
import { ArrowRight, LayoutDashboard, Terminal } from "lucide-react";
import { Button } from "@/components/ui/button";

interface CtaProps {
  appName: string;
  isAuthed: boolean;
}

export function Cta({ appName, isAuthed }: CtaProps) {
  const t = useTranslations("landing");
  return (
    <section className="mx-auto max-w-6xl px-6 pb-24">
      <div className="border-border/70 bg-card/60 relative overflow-hidden rounded-3xl border px-6 py-16 text-center shadow-2xl shadow-black/30 sm:px-12 sm:py-20">
        <div
          aria-hidden
          className="pointer-events-none absolute inset-0 -z-10 bg-[radial-gradient(ellipse_at_center,oklch(0.672_0.169_277/0.18),transparent_70%)]"
        />
        <h2 className="mx-auto max-w-2xl text-3xl font-semibold tracking-tight text-balance sm:text-4xl">
          {t("cta.heading", { appName })}
        </h2>
        <p className="text-muted-foreground mx-auto mt-4 max-w-xl text-lg text-balance">
          {t("cta.subhead")}
        </p>

        <div className="mt-8 flex flex-col items-center justify-center gap-3 sm:flex-row">
          {isAuthed ? (
            <Button asChild size="lg" className="h-11 px-6 text-base">
              <Link href="/dashboard">
                <LayoutDashboard className="size-4" /> {t("cta.openDashboard")}
              </Link>
            </Button>
          ) : (
            <Button asChild size="lg" className="h-11 px-6 text-base">
              <Link href="/register">
                {t("cta.getStarted")} <ArrowRight className="size-4" />
              </Link>
            </Button>
          )}
          <Button asChild size="lg" variant="outline" className="h-11 px-6 text-base">
            <Link href="https://github.com/Nimfek/Modlight/blob/main/docs/deployment.md">
              <Terminal className="size-4" /> {t("cta.deploymentGuide")}
            </Link>
          </Button>
        </div>

        <div className="border-border/60 bg-background/70 text-muted-foreground mx-auto mt-10 inline-flex items-center gap-3 rounded-xl border px-4 py-2.5 font-mono text-sm">
          <Terminal className="text-primary size-4" />
          <span>
            <span className="text-muted-foreground/60">$ </span>docker compose up -d
          </span>
        </div>
      </div>
    </section>
  );
}
