"use client";

import { useState, useTransition } from "react";
import { Loader2, Check, Globe, MousePointerClick } from "lucide-react";
import { toast } from "sonner";
import { useTranslations } from "next-intl";
import { createUploadToken } from "@/app/actions/tokens";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { cn } from "@/lib/utils";

interface OneClickShareXProps {
  /** The hostname this user publishes on — baked into the .sxcu RequestURL. */
  appUrl: string;
  appName: string;
  /** Used to name the downloaded file: "<username>-Modlight.sxcu". */
  username: string;
  /**
   * Whether the deployment answers on more than one hostname. When it does, the
   * config file is domain-specific, and changing that choice later means
   * downloading a fresh one — ShareX has no way to learn about it otherwise.
   */
  showDomainNote?: boolean;
}

/**
 * The ShareX mark: four interlocking arcs in blue, green, red and yellow.
 *
 * Drawn rather than shipped as a bitmap so it stays sharp at every size this
 * page renders it (20 px in the button, 56 px in the tile) and needs no asset.
 *
 * Each arc is one stroked circle whose centre is pushed a little way out from
 * the middle, which is what opens the aperture between them; the 260° dash and
 * the 90° rotation between colours produce the pinwheel. The weave is faked by
 * paint order — a true over-under needs four clip paths, and at these sizes the
 * difference is invisible.
 */
function ShareXLogo({ className }: { className?: string }) {
  // The real mark, as shipped by the project. Three attempts at redrawing the
  // four interlocking arcs by hand got recognisable and never right — a logo is
  // an identity, and "close enough" is the one thing it cannot be.
  //
  // Plain <img>: next/image would rewrite this through the optimiser for a
  // 20 px icon that is already a small PNG, which costs a round trip and buys
  // nothing.
  // eslint-disable-next-line @next/next/no-img-element
  return <img src="/brand/sharex.png" alt="ShareX" className={className} />;
}

/** Bare host for display: "h.example.com", not "https://h.example.com". */
function hostOf(origin: string): string {
  try {
    return new URL(origin).host;
  } catch {
    return origin.replace(/^https?:\/\//, "");
  }
}

/** Build the ShareX custom uploader (.sxcu) JSON from an app URL + token. */
function buildSxcu(appUrl: string, appName: string, token: string): string {
  return JSON.stringify(
    {
      Version: "16.1.0",
      Name: appName,
      DestinationType: "ImageUploader, TextUploader, FileUploader",
      RequestMethod: "POST",
      RequestURL: `${appUrl}/upload`,
      Headers: { "x-api-key": token },
      Body: "MultipartFormData",
      FileFormName: "upload",
      URL: "{json:url}",
      ThumbnailURL: "{json:raw_url}",
      DeletionURL: "{json:deletionUrl}",
    },
    null,
    2,
  );
}

export function OneClickShareX({
  appUrl,
  appName,
  username,
  showDomainNote = false,
}: OneClickShareXProps) {
  const t = useTranslations("tokens");
  const [pending, startTransition] = useTransition();
  const [done, setDone] = useState(false);

  function handleDownload() {
    startTransition(async () => {
      try {
        // `upload` and nothing wider. A .sxcu sits in plain text in a settings
        // folder that syncs between machines and shows up whole in a screenshot
        // of the ShareX destinations window, and the only thing it ever does is
        // POST a file. This button is where most tokens on this deployment come
        // from, so it is where the default actually gets decided.
        const { token } = await createUploadToken("ShareX", "upload");
        const content = buildSxcu(appUrl, appName, token);
        const safeUser =
          (username || "user").replace(/[^a-z0-9-_]+/gi, "-").replace(/^-+|-+$/g, "") || "user";

        const blob = new Blob([content], { type: "application/json" });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = `${safeUser}-${appName}.sxcu`;
        document.body.appendChild(a);
        a.click();
        a.remove();
        URL.revokeObjectURL(url);

        setDone(true);
        setTimeout(() => setDone(false), 4000);
        toast.success(t("oneClick.downloadSuccess"));
      } catch {
        toast.error(t("oneClick.downloadError"));
      }
    });
  }

  return (
    <Card className="from-primary/10 via-card to-card relative overflow-hidden bg-gradient-to-br ring-1 ring-foreground/5">
      <CardContent className="flex flex-col items-center gap-6 px-6 py-10 text-center sm:px-10">
        <div className="grid size-20 place-items-center rounded-[var(--radius)] bg-background/60 shadow-sm ring-1 ring-foreground/10">
          <ShareXLogo className="size-14" />
        </div>

        <div className="space-y-1.5">
          <h2 className="font-heading text-xl font-semibold tracking-tight sm:text-2xl">
            {t("oneClick.title")}
          </h2>
          <p className="text-muted-foreground mx-auto max-w-md text-sm leading-relaxed">
            {t("oneClick.desc")}
          </p>
        </div>

        <Button
          size="lg"
          onClick={handleDownload}
          disabled={pending}
          className="h-12 gap-2.5 px-7 text-base shadow-sm"
        >
          {pending ? (
            <Loader2 className="size-5 animate-spin" />
          ) : done ? (
            <Check className="size-5" />
          ) : (
            <ShareXLogo className="size-5" />
          )}
          <span className={cn(pending && "opacity-90")}>
            {pending
              ? t("oneClick.preparing")
              : done
                ? t("oneClick.downloaded")
                : t("oneClick.download")}
          </span>
        </Button>

        <p className="text-muted-foreground flex items-center justify-center gap-1.5 text-xs">
          <MousePointerClick className="text-primary size-3.5" />
          {t.rich("oneClick.downloadsHint", {
            fileName: () => (
              <span className="font-mono">
                {username || "user"}-{appName}.sxcu
              </span>
            ),
          })}
        </p>

        {showDomainNote && (
          <div className="border-primary/25 bg-primary/5 flex max-w-md items-start gap-2.5 rounded-lg border px-3 py-2.5 text-left">
            <Globe className="text-primary mt-0.5 size-4 shrink-0" />
            <p className="text-muted-foreground text-xs leading-relaxed">
              {t.rich("oneClick.domainNote", {
                // A tag rather than a `{host}` value: the placeholder carries
                // styling, and next-intl only accepts functions for those.
                host: () => <span className="text-foreground font-mono">{hostOf(appUrl)}</span>,
                link: (chunks) => (
                  <a
                    href="/dashboard/profile"
                    className="text-foreground font-medium underline underline-offset-2"
                  >
                    {chunks}
                  </a>
                ),
              })}
            </p>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
