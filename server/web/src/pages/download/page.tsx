// Port of upstream app/download/page.tsx.
// @upstream app/download/page.tsx
//
// Server: `release` = currentRelease() (lib/releases-core.ts). The metadata
// (generateMetadata) is computed server-side into the payload `meta`.
import Link from "next/link";
import { useTranslations } from "next-intl";
import { ArrowRight, Clock, Download, FolderTree, Link2, ShieldCheck } from "lucide-react";
import { config } from "@/lib/config";
import { APP_NAME, type ReleaseManifest } from "@/lib/releases";
import { humanFileSize } from "@/lib/format";
import { Button } from "@/components/ui/button";

export interface DownloadPageProps {
  /** currentRelease(): the published desktop build, or null. */
  release: ReleaseManifest | null;
}

/**
 * The public landing page for the companion app.
 *
 * Public because the point is to be linkable — somebody sharing "get the app"
 * should not be handing out a page that first demands a login. The download
 * itself is a normal published upload, so no account is needed to fetch it
 * either; an account is only needed to configure it, which is what the panel
 * link at the bottom is for.
 */
export default function DownloadPage({ props }: { props: DownloadPageProps }) {
  const t = useTranslations("download");
  const release = props.release;

  const features = [
    { icon: Link2, key: "clipboard" },
    { icon: FolderTree, key: "folders" },
    { icon: ShieldCheck, key: "resumable" },
  ] as const;

  return (
    <main className="page-surface min-h-dvh">
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-10 px-5 py-16 sm:py-24">
        <header className="flex flex-col items-center gap-5 text-center">
          <div className="bg-card ring-foreground/10 grid size-16 place-items-center rounded-2xl shadow-sm ring-1">
            <Download className="text-primary size-8" />
          </div>
          <div className="space-y-2">
            <h1 className="font-heading text-3xl font-semibold tracking-tight sm:text-4xl">
              {APP_NAME}
            </h1>
            <p className="text-muted-foreground mx-auto max-w-xl leading-relaxed">
              {t("tagline", { appName: config.appName })}
            </p>
          </div>
        </header>

        {release ? (
          <section className="bg-card ring-foreground/10 flex flex-col items-center gap-4 rounded-2xl px-6 py-8 text-center ring-1">
            <Button size="lg" asChild className="h-12 gap-2.5 px-7 text-base">
              <a href="/download/app" download>
                <Download className="size-5" />
                {t("downloadFor", { version: release.version })}
              </a>
            </Button>

            <dl className="text-muted-foreground flex flex-wrap items-center justify-center gap-x-5 gap-y-1 text-xs">
              <div className="flex items-center gap-1.5">
                <dt className="sr-only">{t("size")}</dt>
                <dd className="tabular-nums">{humanFileSize(release.size)}</dd>
              </div>
              {release.publishedAt ? (
                <div className="flex items-center gap-1.5">
                  <Clock className="size-3.5" />
                  <dt className="sr-only">{t("published")}</dt>
                  <dd>
                    <time dateTime={release.publishedAt}>
                      {release.publishedAt.slice(0, 10)}
                    </time>
                  </dd>
                </div>
              ) : null}
            </dl>

            {release.notes ? (
              <p className="text-muted-foreground max-w-lg text-sm leading-relaxed whitespace-pre-line">
                {release.notes}
              </p>
            ) : null}

            {/* The hash is printed rather than merely served, so anybody can
                check what they downloaded without trusting this page's own
                download button. */}
            {release.sha256 ? (
              <p className="text-muted-foreground/80 max-w-full text-[11px] break-all">
                <span className="font-medium">SHA-256</span>{" "}
                <code className="font-mono">{release.sha256}</code>
              </p>
            ) : null}
          </section>
        ) : (
          <section className="ring-foreground/10 rounded-2xl px-6 py-10 text-center ring-1">
            <p className="text-muted-foreground text-sm">{t("none")}</p>
          </section>
        )}

        <section className="grid gap-4 sm:grid-cols-3">
          {features.map(({ icon: Icon, key }) => (
            <div key={key} className="ring-foreground/10 rounded-xl px-4 py-4 ring-1">
              <Icon className="text-primary mb-2 size-5" />
              <h2 className="text-sm font-medium">{t(`features.${key}.title`)}</h2>
              <p className="text-muted-foreground mt-1 text-xs leading-relaxed">
                {t(`features.${key}.desc`)}
              </p>
            </div>
          ))}
        </section>

        <footer className="text-center">
          <Link
            href="/dashboard/tokens"
            className="text-muted-foreground hover:text-foreground inline-flex items-center gap-1.5 text-sm underline-offset-4 hover:underline"
          >
            {t("getConfig")} <ArrowRight className="size-3.5" />
          </Link>
        </footer>
      </div>
    </main>
  );
}
