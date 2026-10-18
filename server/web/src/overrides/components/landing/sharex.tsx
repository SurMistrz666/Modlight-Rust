// Client port of upstream components/landing/sharex.tsx (an async Server Component there).
// @upstream components/landing/sharex.tsx
// Deviation: `async` dropped and getTranslations (next-intl/server) replaced by
// useTranslations (next-intl); nothing else differs.
import Link from "next/link";
import { useTranslations } from "next-intl";
import { Download, KeyRound, Camera, ArrowRight } from "lucide-react";
import { Button } from "@/components/ui/button";

interface ShareXProps {
  appUrl: string;
}

export function ShareX({ appUrl }: ShareXProps) {
  const t = useTranslations("landing");

  const steps = [
    {
      icon: KeyRound,
      title: t("shareX.step1Title"),
      description: t("shareX.step1Desc"),
    },
    {
      icon: Download,
      title: t("shareX.step2Title"),
      description: t("shareX.step2Desc"),
    },
    {
      icon: Camera,
      title: t("shareX.step3Title"),
      description: t("shareX.step3Desc"),
    },
  ];

  return (
    <section id="sharex" className="mx-auto max-w-6xl px-6 py-20 sm:py-28">
      <div className="grid items-center gap-12 lg:grid-cols-2 lg:gap-16">
        {/* Config preview */}
        <div className="order-2 lg:order-1">
          <div className="border-border/70 bg-card/80 overflow-hidden rounded-2xl border shadow-xl shadow-black/20">
            <div className="border-border/60 flex items-center gap-2 border-b px-4 py-3">
              <span className="size-3 rounded-full bg-red-400/80" />
              <span className="size-3 rounded-full bg-yellow-400/80" />
              <span className="size-3 rounded-full bg-green-400/80" />
              <span className="text-muted-foreground ml-3 font-mono text-xs">
                {t("shareX.configFile")}
              </span>
            </div>
            <pre className="overflow-x-auto p-5 font-mono text-xs leading-relaxed">
              <code>
                <span className="text-muted-foreground">{"{"}</span>
                {"\n  "}
                <span className="text-fuchsia-400">&quot;Name&quot;</span>:{" "}
                <span className="text-primary">&quot;Modlight&quot;</span>,{"\n  "}
                <span className="text-fuchsia-400">&quot;RequestMethod&quot;</span>:{" "}
                <span className="text-primary">&quot;POST&quot;</span>,{"\n  "}
                <span className="text-fuchsia-400">&quot;RequestURL&quot;</span>:{" "}
                <span className="text-primary">&quot;{appUrl}/upload&quot;</span>,{"\n  "}
                <span className="text-fuchsia-400">&quot;Body&quot;</span>:{" "}
                <span className="text-primary">&quot;MultipartFormData&quot;</span>,{"\n  "}
                <span className="text-fuchsia-400">&quot;FileFormName&quot;</span>:{" "}
                <span className="text-primary">&quot;upload&quot;</span>,{"\n  "}
                <span className="text-fuchsia-400">&quot;Headers&quot;</span>:{" "}
                <span className="text-muted-foreground">{"{"}</span> {"\n    "}
                <span className="text-fuchsia-400">&quot;x-api-key&quot;</span>:{" "}
                <span className="text-primary">&quot;••••••••••&quot;</span>
                {"\n  "}
                <span className="text-muted-foreground">{"}"}</span>,{"\n  "}
                <span className="text-fuchsia-400">&quot;URL&quot;</span>:{" "}
                <span className="text-primary">&quot;{"{json:url}"}&quot;</span>
                {"\n"}
                <span className="text-muted-foreground">{"}"}</span>
              </code>
            </pre>
          </div>
        </div>

        {/* Steps + copy */}
        <div className="order-1 lg:order-2">
          <p className="text-primary text-sm font-semibold tracking-wide uppercase">
            {t("shareX.eyebrow")}
          </p>
          <h2 className="mt-3 text-3xl font-semibold tracking-tight text-balance sm:text-4xl">
            {t("shareX.heading")}
          </h2>
          <p className="text-muted-foreground mt-4 text-lg leading-relaxed">
            {t("shareX.introBefore")}{" "}
            <code className="bg-muted text-foreground rounded px-1.5 py-0.5 font-mono text-sm">
              /upload
            </code>{" "}
            {t("shareX.introAfter")}
          </p>

          <ol className="mt-8 space-y-5">
            {steps.map((s, i) => (
              <li key={s.title} className="flex gap-4">
                <div className="border-primary/30 bg-primary/10 text-primary relative grid size-9 shrink-0 place-items-center rounded-full border font-mono text-sm font-semibold">
                  {i + 1}
                </div>
                <div className="pt-1">
                  <h3 className="font-semibold tracking-tight">{s.title}</h3>
                  <p className="text-muted-foreground mt-1 text-sm leading-relaxed">
                    {s.description}
                  </p>
                </div>
              </li>
            ))}
          </ol>

          <div className="mt-8">
            <Button asChild variant="outline" size="lg" className="h-11 px-5 text-base">
              <Link href="https://github.com/Nimfek/Modlight/blob/main/docs/sharex-setup.md">
                {t("shareX.readGuide")} <ArrowRight className="size-4" />
              </Link>
            </Button>
          </div>
        </div>
      </div>
    </section>
  );
}
