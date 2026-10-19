"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import Link from "next/link";
import nextDynamic from "next/dynamic";
import { useTheme } from "next-themes";
import { useTranslations } from "next-intl";
import Dashboard from "@uppy/react/dashboard";
import { ArrowRight, AlertTriangle, Check, Copy, FolderOpen, UploadCloud } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { copyText } from "@/components/transfers/clipboard";
import { useTransfers } from "@/components/transfers/TransfersProvider";

import "@uppy/core/css/style.min.css";
import "@uppy/dashboard/css/style.min.css";

/**
 * The full-page dropzone.
 *
 * It does NOT own an Uppy instance: the engine lives in the app shell
 * (components/transfers/TransfersProvider) so a transfer survives navigation
 * and so a file dropped anywhere in the panel joins the same queue. This
 * component is the rich view of that queue; the floating dock is the compact
 * one, and it steps aside (`setDockHidden`) while this page is mounted rather
 * than showing the same progress bars twice.
 */
function UppyPanel() {
  const t = useTranslations("upload");
  const { resolvedTheme } = useTheme();
  const { uppy, ensure, setDockHidden, items, stats, target } = useTransfers();

  useEffect(() => {
    void ensure();
  }, [ensure]);

  useEffect(() => {
    setDockHidden(true);
    return () => setDockHidden(false);
  }, [setDockHidden]);

  const [copiedId, setCopiedId] = useState<string | null>(null);

  // flatMap rather than filter so `url` narrows to a string for the JSX below.
  const links = useMemo(
    () =>
      items.flatMap((item) =>
        item.status === "done" && item.url
          ? [{ id: item.id, name: item.name, url: item.url }]
          : [],
      ),
    [items],
  );

  const copyOne = useCallback(
    async (id: string, url: string, name: string) => {
      if (await copyText(url)) {
        setCopiedId(id);
        window.setTimeout(() => setCopiedId((cur) => (cur === id ? null : cur)), 1500);
      } else {
        toast.error(t("results.copyFailed"), { description: name });
      }
    },
    [t],
  );

  const copyAll = useCallback(async () => {
    const text = links.map((l) => l.url).join("\n");
    if (await copyText(text)) toast.success(t("results.copiedAll", { count: links.length }));
    else toast.error(t("results.copyFailed"));
  }, [links, t]);

  // Map app theme onto Uppy's Dashboard. resolvedTheme is undefined until
  // mounted, so fall back to 'auto' to avoid a light flash.
  const dashboardTheme =
    resolvedTheme === "light" ? "light" : resolvedTheme === "dark" ? "dark" : "auto";

  const destination = target.isDefault
    ? target.folderName
      ? t("target.folder", { folder: target.folderName })
      : t("target.library")
    : target.folderId
      ? t("target.folder", { folder: target.folderName ?? t("target.thisFolder") })
      : t("target.unsorted");

  return (
    <div className="space-y-5">
      <p className="text-muted-foreground flex items-center gap-1.5 text-sm">
        <FolderOpen className="size-4 shrink-0" />
        <span className="min-w-0 truncate">{destination}</span>
      </p>

      <div
        className={cn(
          "uppy-dashboard-shell ring-foreground/10 overflow-hidden rounded-xl ring-1",
          "bg-card shadow-sm",
        )}
      >
        {uppy ? (
          <Dashboard
            uppy={uppy}
            theme={dashboardTheme}
            width="100%"
            height={460}
            proudlyDisplayPoweredByUppy={false}
            note={t("dashboardNote")}
          />
        ) : (
          <DashboardSkeleton />
        )}
      </div>

      {links.length > 0 ? (
        <div className="ring-primary/20 bg-primary/5 rounded-xl ring-1">
          <div className="border-primary/15 flex flex-wrap items-center justify-between gap-3 border-b px-4 py-3">
            <p className="text-sm font-medium">{t("results.title", { count: links.length })}</p>
            <div className="flex items-center gap-2">
              {links.length > 1 ? (
                <Button variant="outline" size="sm" onClick={copyAll}>
                  <Copy className="size-4" />
                  {t("results.copyAll")}
                </Button>
              ) : null}
              <Button asChild size="sm">
                <Link href="/dashboard">
                  {t("results.viewFiles")}
                  <ArrowRight className="size-4" />
                </Link>
              </Button>
            </div>
          </div>

          <ul className="divide-primary/10 divide-y">
            {links.map((link) => (
              <li key={link.id} className="flex items-center gap-3 px-4 py-2.5">
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm">{link.name}</p>
                  <p className="text-muted-foreground truncate font-mono text-xs">{link.url}</p>
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => void copyOne(link.id, link.url, link.name)}
                  aria-label={t("results.copyOne", { name: link.name })}
                >
                  {copiedId === link.id ? (
                    <Check className="text-primary size-4" />
                  ) : (
                    <Copy className="size-4" />
                  )}
                </Button>
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      {stats.failed > 0 ? (
        <div className="border-destructive/20 bg-destructive/5 flex items-center gap-3 rounded-xl border p-4 text-sm">
          <span className="bg-destructive/10 text-destructive grid size-9 shrink-0 place-items-center rounded-lg">
            <AlertTriangle className="size-5" />
          </span>
          <p className="text-muted-foreground">{t("results.failed", { count: stats.failed })}</p>
        </div>
      ) : null}
    </div>
  );
}

function DashboardSkeleton() {
  const t = useTranslations("upload");
  return (
    <div className="bg-card/40 ring-foreground/5 grid h-[460px] place-items-center rounded-xl border border-dashed ring-1">
      <div className="text-muted-foreground flex flex-col items-center gap-3">
        <UploadCloud className="size-8 animate-pulse" />
        <p className="text-sm">{t("preparing")}</p>
      </div>
    </div>
  );
}

// ssr:false keeps Uppy's browser code out of the prerender. Legal here
// because the surrounding module is a Client Component.
const LazyUppyPanel = nextDynamic(() => Promise.resolve(UppyPanel), {
  ssr: false,
  loading: () => <DashboardSkeleton />,
});

export function Uploader() {
  return <LazyUppyPanel />;
}

export default Uploader;
