// Client port of upstream components/landing/large-files.tsx (an async Server Component there).
// @upstream components/landing/large-files.tsx
// Deviation: `async` dropped and getTranslations (next-intl/server) replaced by
// useTranslations (next-intl); nothing else differs.
import { RefreshCw, GaugeCircle, Waves, CheckCircle2 } from "lucide-react";
import { useTranslations } from "next-intl";

export function LargeFiles() {
  const t = useTranslations("landing");

  const points = [
    {
      icon: RefreshCw,
      title: t("largeFiles.resumableTitle"),
      description: t("largeFiles.resumableDesc"),
    },
    {
      icon: Waves,
      title: t("largeFiles.streamedTitle"),
      description: t("largeFiles.streamedDesc"),
    },
    {
      icon: GaugeCircle,
      title: t("largeFiles.proxyTitle"),
      description: t("largeFiles.proxyDesc"),
    },
  ];

  const comparison = [
    { label: t("largeFiles.row1Label"), legacy: t("largeFiles.row1Legacy"), modlight: t("largeFiles.row1Modlight") },
    { label: t("largeFiles.row2Label"), legacy: t("largeFiles.row2Legacy"), modlight: t("largeFiles.row2Modlight") },
    { label: t("largeFiles.row3Label"), legacy: t("largeFiles.row3Legacy"), modlight: t("largeFiles.row3Modlight") },
    { label: t("largeFiles.row4Label"), legacy: t("largeFiles.row4Legacy"), modlight: t("largeFiles.row4Modlight") },
  ];

  return (
    <section
      id="large-files"
      className="border-border/60 relative border-y bg-[radial-gradient(ellipse_at_top,oklch(0.672_0.169_277/0.08),transparent_70%)]"
    >
      <div className="mx-auto max-w-6xl px-6 py-20 sm:py-28">
        <div className="grid items-start gap-12 lg:grid-cols-2 lg:gap-16">
          <div>
            <p className="text-primary text-sm font-semibold tracking-wide uppercase">
              {t("largeFiles.eyebrow")}
            </p>
            <h2 className="mt-3 text-3xl font-semibold tracking-tight text-balance sm:text-4xl">
              {t("largeFiles.heading")}
            </h2>
            <p className="text-muted-foreground mt-4 text-lg leading-relaxed">
              {t("largeFiles.subhead")}
            </p>

            <ul className="mt-8 space-y-6">
              {points.map((p) => (
                <li key={p.title} className="flex gap-4">
                  <div className="bg-primary/10 text-primary ring-primary/20 mt-0.5 grid size-10 shrink-0 place-items-center rounded-lg ring-1">
                    <p.icon className="size-5" />
                  </div>
                  <div>
                    <h3 className="font-semibold tracking-tight">{p.title}</h3>
                    <p className="text-muted-foreground mt-1 text-sm leading-relaxed">
                      {p.description}
                    </p>
                  </div>
                </li>
              ))}
            </ul>
          </div>

          {/* Comparison card */}
          <div className="border-border/70 bg-card/70 overflow-hidden rounded-2xl border shadow-xl shadow-black/20 backdrop-blur">
            <div className="border-border/60 grid grid-cols-[1.4fr_1fr_1fr] gap-2 border-b px-5 py-4 text-xs font-semibold tracking-wide uppercase">
              <span className="text-muted-foreground">{t("largeFiles.colScenario")}</span>
              <span className="text-muted-foreground/70">{t("largeFiles.colLegacy")}</span>
              <span className="text-primary">{t("largeFiles.colModlight")}</span>
            </div>
            <div className="divide-border/50 divide-y">
              {comparison.map((row) => (
                <div
                  key={row.label}
                  className="grid grid-cols-[1.4fr_1fr_1fr] items-center gap-2 px-5 py-4 text-sm"
                >
                  <span className="text-foreground font-medium">{row.label}</span>
                  <span className="text-muted-foreground/80 text-xs">{row.legacy}</span>
                  <span className="text-foreground inline-flex items-center gap-1.5 text-xs font-medium">
                    <CheckCircle2 className="text-primary size-3.5 shrink-0" />
                    {row.modlight}
                  </span>
                </div>
              ))}
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
