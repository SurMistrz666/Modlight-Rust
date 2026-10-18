// Client port of upstream components/tokens/AppCard.tsx (an async Server Component there).
// @upstream components/tokens/AppCard.tsx
// Deviations: `async` dropped, getTranslations -> useTranslations, and the
// release manifest arrives as a prop (the tokens page payload carries
// `release`, upstream read it with `await currentRelease()` here).
import Link from "next/link";
import { Download, KeyRound, MonitorDown } from "lucide-react";
import { useTranslations } from "next-intl";
import { humanFileSize } from "@/lib/format";
import { APP_NAME, type ReleaseManifest } from "@/lib/releases";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { ModlightMark } from "@/components/brand/ModlightMark";

/**
 * The primary integration: our own desktop client.
 *
 * Deliberately the first and largest thing on the page. ShareX still works and
 * still has its card below, but it is now the fallback rather than the headline
 * — somebody arriving here should install our app, sign in, and be finished.
 *
 * There is no config file to download beside it, and that absence is the point:
 * the app signs in through the browser and mints its own token, so the whole
 * "generate a token, save a file, find the app's import menu" ritual disappears.
 */
export function AppCard({ release }: { release: ReleaseManifest | null }) {
  const t = useTranslations("tokens");

  const steps = [
    { icon: MonitorDown, key: "install" },
    { icon: KeyRound, key: "signIn" },
    { icon: Download, key: "use" },
  ] as const;

  return (
    <Card className="from-primary/10 via-card to-card ring-foreground/5 relative overflow-hidden bg-gradient-to-br ring-1">
      <CardContent className="flex flex-col gap-7 px-6 py-9 sm:px-10">
        <div className="flex flex-col items-center gap-5 text-center">
          <div className="bg-background/60 ring-foreground/10 grid size-20 place-items-center rounded-[var(--radius)] shadow-sm ring-1">
            <ModlightMark className="size-14" />
          </div>

          <div className="space-y-1.5">
            <h3 className="font-heading text-xl font-semibold tracking-tight sm:text-2xl">
              {APP_NAME}
            </h3>
            <p className="text-muted-foreground mx-auto max-w-md text-sm leading-relaxed">
              {t("appCard.desc")}
            </p>
          </div>

          {release ? (
            <div className="flex flex-col items-center gap-2">
              <Button size="lg" asChild className="h-12 gap-2.5 px-7 text-base shadow-sm">
                <a href="/download/app" download>
                  <Download className="size-5" />
                  {t("appCard.download")}
                </a>
              </Button>
              <p className="text-muted-foreground text-xs tabular-nums">
                {t("app.version", { version: release.version })}
                {" · "}
                {humanFileSize(release.size)}
              </p>
            </div>
          ) : (
            <div className="flex flex-col items-center gap-2">
              <Button size="lg" disabled className="h-12 gap-2.5 px-7 text-base">
                <Download className="size-5" />
                {t("appCard.download")}
              </Button>
              <p className="text-muted-foreground text-xs">{t("app.noBuild")}</p>
            </div>
          )}
        </div>

        {/* Three steps, because the whole selling point is that there are only
            three. Numbered: this IS a sequence and the order carries meaning. */}
        <ol className="border-foreground/10 grid gap-4 border-t pt-6 sm:grid-cols-3">
          {steps.map(({ icon: Icon, key }, i) => (
            <li key={key} className="flex gap-3">
              <span className="bg-primary/10 text-primary grid size-7 shrink-0 place-items-center rounded-full text-xs font-semibold tabular-nums">
                {i + 1}
              </span>
              <div className="min-w-0">
                <p className="flex items-center gap-1.5 text-sm font-medium">
                  <Icon className="text-muted-foreground size-3.5 shrink-0" />
                  {t(`appCard.steps.${key}.title`)}
                </p>
                <p className="text-muted-foreground mt-0.5 text-xs leading-relaxed">
                  {t(`appCard.steps.${key}.desc`)}
                </p>
              </div>
            </li>
          ))}
        </ol>

        <p className="text-muted-foreground text-center text-xs">
          {t.rich("appCard.moreInfo", {
            link: (chunks) => (
              <Link
                href="/download"
                className="text-foreground font-medium underline underline-offset-2"
              >
                {chunks}
              </Link>
            ),
          })}
        </p>
      </CardContent>
    </Card>
  );
}
