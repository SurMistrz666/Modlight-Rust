// Client port of upstream components/landing/hero.tsx (an async Server Component there).
// @upstream components/landing/hero.tsx
// Deviation: `async` dropped and getTranslations (next-intl/server) replaced by
// useTranslations (next-intl); nothing else differs.
import Link from "next/link";
import { useTranslations } from "next-intl";
import { ArrowRight, LayoutDashboard, Star, Zap } from "lucide-react";
import { Button } from "@/components/ui/button";

interface HeroProps {
  appName: string;
  isAuthed: boolean;
}

export function Hero({ appName, isAuthed }: HeroProps) {
  const t = useTranslations("landing");
  return (
    <section className="relative overflow-hidden">
      {/* Ambient glow */}
      <div
        aria-hidden
        className="pointer-events-none absolute inset-x-0 -top-40 -z-10 mx-auto h-[40rem] w-[80rem] max-w-none bg-[radial-gradient(closest-side,oklch(0.672_0.169_277/0.22),transparent)] blur-2xl"
      />

      <div className="mx-auto max-w-5xl px-6 pt-20 pb-16 text-center sm:pt-28 sm:pb-24">
        <Link
          href="https://github.com/Nimfek/Modlight"
          className="border-border/70 bg-card/60 text-muted-foreground hover:border-primary/40 hover:text-foreground group mx-auto inline-flex items-center gap-2 rounded-full border px-3.5 py-1.5 text-xs font-medium backdrop-blur transition-colors"
        >
          <span className="bg-primary/15 text-primary inline-flex items-center gap-1 rounded-full px-2 py-0.5">
            <Zap className="size-3" /> {t("hero.badgeNew")}
          </span>
          {t("hero.badgeText")}
          <ArrowRight className="size-3 transition-transform group-hover:translate-x-0.5" />
        </Link>

        <h1 className="mx-auto mt-8 max-w-4xl text-balance text-4xl font-semibold tracking-tight sm:text-6xl lg:text-7xl">
          {t("hero.headlineStart")}
          <span className="from-primary bg-gradient-to-r to-fuchsia-400 bg-clip-text text-transparent">
            {" "}
            {t("hero.headlineHighlight")}
          </span>
        </h1>

        <p className="text-muted-foreground mx-auto mt-6 max-w-2xl text-balance text-lg leading-relaxed sm:text-xl">
          {t("hero.subhead", { appName })}
        </p>

        <div className="mt-10 flex flex-col items-center justify-center gap-3 sm:flex-row">
          {isAuthed ? (
            <Button asChild size="lg" className="h-11 px-6 text-base">
              <Link href="/dashboard">
                <LayoutDashboard className="size-4" /> {t("hero.openDashboard")}
              </Link>
            </Button>
          ) : (
            <>
              <Button asChild size="lg" className="h-11 px-6 text-base">
                <Link href="/register">
                  {t("hero.getStarted")} <ArrowRight className="size-4" />
                </Link>
              </Button>
              <Button asChild size="lg" variant="outline" className="h-11 px-6 text-base">
                <Link href="/login">{t("hero.signIn")}</Link>
              </Button>
            </>
          )}
          <Button asChild size="lg" variant="ghost" className="h-11 px-5 text-base">
            <Link href="https://github.com/Nimfek/Modlight">
              <Star className="size-4" /> {t("hero.starOnGithub")}
            </Link>
          </Button>
        </div>

        <p className="text-muted-foreground/80 mt-5 text-sm">
          {t("hero.tagline")}
        </p>

        {/* Hero terminal/preview card */}
        <div className="relative mx-auto mt-16 max-w-3xl">
          <div
            aria-hidden
            className="from-primary/30 absolute -inset-x-8 -top-8 -bottom-8 -z-10 rounded-[2.5rem] bg-gradient-to-b to-transparent opacity-40 blur-3xl"
          />
          <div className="border-border/70 bg-card/80 overflow-hidden rounded-2xl border shadow-2xl shadow-black/40 backdrop-blur">
            <div className="border-border/60 flex items-center gap-2 border-b px-4 py-3">
              <span className="size-3 rounded-full bg-red-400/80" />
              <span className="size-3 rounded-full bg-yellow-400/80" />
              <span className="size-3 rounded-full bg-green-400/80" />
              <span className="text-muted-foreground ml-3 font-mono text-xs">
                {t("hero.previewUploading")}
              </span>
            </div>
            <div className="space-y-4 p-6 text-left">
              <div className="flex items-center justify-between text-sm">
                <span className="font-medium">{t("hero.previewResumable")}</span>
                <span className="text-primary font-mono text-xs">73%</span>
              </div>
              <div className="bg-muted h-2 w-full overflow-hidden rounded-full">
                <div className="from-primary h-full w-[73%] rounded-full bg-gradient-to-r to-fuchsia-400" />
              </div>
              <div className="text-muted-foreground grid grid-cols-3 gap-3 font-mono text-xs">
                <div className="border-border/50 bg-background/50 rounded-lg border p-3">
                  <div className="text-foreground text-sm font-semibold">128 MB/s</div>
                  {t("hero.previewThroughput")}
                </div>
                <div className="border-border/50 bg-background/50 rounded-lg border p-3">
                  <div className="text-foreground text-sm font-semibold">0</div>
                  {t("hero.previewFailedChunks")}
                </div>
                <div className="border-border/50 bg-background/50 rounded-lg border p-3">
                  <div className="text-foreground text-sm font-semibold">{t("hero.previewResume")}</div>
                  {t("hero.previewOnReconnect")}
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
