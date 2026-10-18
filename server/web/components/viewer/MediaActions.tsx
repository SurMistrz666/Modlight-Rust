"use client";

import { Fragment } from "react";
import { Download, ExternalLink, Link2, FileCode2, Code2, FileDown } from "lucide-react";
import { toast } from "sonner";
import { useTranslations } from "next-intl";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/viewer/CopyButton";
import type { LinkFormats } from "@/lib/media/link-formats";
import { LINK_FORMAT_LABELS } from "@/lib/media/link-formats";
import {
  downloadHintKey,
  downloadLabelKey,
  downloadOptionsFor,
  sectionHeadingKey,
} from "@/lib/media/download-menu";
import { estimateConvertedSize, formatEstimate } from "@/lib/media/convert-size";
import { useConvertDownload } from "@/components/dashboard/use-convert-download";

/**
 * Derive the `/{userCode}/{code}` convert base from the public download URL,
 * which is always shaped `{appUrl}/{userCode}/{code}/download`.
 */
function convertBase(downloadUrl: string): string | null {
  const m = downloadUrl.match(/\/([^/]+)\/([^/]+)\/download(?:[?#].*)?$/);
  return m ? `/${m[1]}/${m[2]}` : null;
}

/** Last-resort extension, for callers that pass neither `ext` nor a MIME. */
function extFromUrl(rawUrl: string): string {
  const m = /\.([a-z0-9]+)(?:[?#].*)?$/i.exec(rawUrl);
  return m ? m[1].toLowerCase() : "";
}

interface MediaActionsProps {
  publicUrl: string;
  rawUrl: string;
  downloadUrl: string;
  formats: LinkFormats;
  /** Owner preference — when true, the primary copy yields the raw URL. */
  copyRaw: boolean;
  /** Stored type, which decides what "Download as…" can offer. */
  mimeType?: string;
  /** Extension, the better evidence whenever the stored type is octet-stream. */
  ext?: string;
  /**
   * Frames in the file (1 = still, >1 = animation, null = never counted).
   * The route admits exactly the targets this produces, so the two cannot drift.
   */
  frames?: number | null;
  /**
   * True when the viewer owns this file (or is an admin).
   *
   * Decides whether the encoder-backed conversions appear at all. A stranger is
   * offered the stream copies and the frame grabs; the rest would be menu
   * entries that answer 401, because work whose size the caller chooses has to
   * belong to an account. See av-targets.ts.
   */
  canEncode?: boolean;
  /** Stored size, which is what a container copy will produce, to the byte. */
  size?: number;
  /** Pixel dimensions: the only usable predictor for a PNG or GIF result. */
  width?: number | null;
  height?: number | null;
  /**
   * Where to open this same file on the app's own hostname.
   *
   * Set only when the page is being served on a hostname that is NOT the one the
   * session cookie belongs to, and the viewer is therefore anonymous here. The
   * cookie is host-only by design, so an owner reading their file on a secondary
   * domain genuinely has no session and genuinely cannot re-encode - and the
   * honest answer is one link, not a silently shorter menu.
   */
  signInUrl?: string | null;
}

/** The action cluster in the info bar: copy link, copy raw, more formats, download, open raw. */
export function MediaActions({
  publicUrl,
  rawUrl,
  downloadUrl,
  formats,
  copyRaw,
  mimeType,
  ext,
  frames,
  canEncode = false,
  size = 0,
  width,
  height,
  signInUrl,
}: MediaActionsProps) {
  const t = useTranslations("viewer");
  const tCommon = useTranslations("common");
  const { start: startDownload } = useConvertDownload();

  const primaryValue = copyRaw ? rawUrl : publicUrl;
  const primaryLabel = copyRaw ? t("actions.copyRaw") : t("actions.copyLink");
  const primaryToast = copyRaw ? t("actions.rawLinkCopied") : t("actions.directLinkCopied");
  const secondaryValue = copyRaw ? publicUrl : rawUrl;
  const secondaryLabel = copyRaw ? t("actions.copyLink") : t("actions.copyRaw");
  const secondaryToast = copyRaw ? t("actions.directLinkCopied") : t("actions.rawLinkCopied");

  // One shared answer to "what can this become", instead of the regex over the
  // URL that used to live here and disagree with the dashboard menu.
  const convertTargets = downloadOptionsFor(mimeType, ext ?? extFromUrl(rawUrl), frames, {
    canEncode,
    // Only the web-playable copy reads these, and only to stay absent for the
    // vast majority of files that do not need it.
    width,
    height,
  });
  const base = convertBase(downloadUrl);
  const showConvert = (convertTargets.length > 0 || !!signInUrl) && base !== null;

  const resolvedExt = ext ?? extFromUrl(rawUrl);

  /** What to print for a target; see downloadLabelKey. */
  const label = (to: string): string => {
    const key = downloadLabelKey(to);
    return key ? t(key) : to.toUpperCase();
  };

  /** How big the result will be, under the honesty rules of convert-size.ts. */
  const sizeLabel = (option: (typeof convertTargets)[number]): string | null => {
    const estimate = estimateConvertedSize(
      { mimeType, ext: resolvedExt, size, width, height, frames },
      option,
    );
    if (!estimate) return null;
    const text = formatEstimate(estimate.bytes);
    return estimate.confidence === "exact" ? text : t("actions.approxSize", { size: text });
  };

  return (
    <div className="flex flex-wrap items-center gap-2">
      <CopyButton
        value={primaryValue}
        label={primaryLabel}
        toastMessage={primaryToast}
        variant="default"
      >
        <Link2 />
      </CopyButton>

      <CopyButton
        value={secondaryValue}
        label={secondaryLabel}
        toastMessage={secondaryToast}
        variant="outline"
      >
        <FileCode2 />
      </CopyButton>

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="outline" size="sm" aria-label={t("actions.moreCopyFormats")}>
            <Code2 />
            <span className="hidden sm:inline">{t("actions.formats")}</span>
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-56">
          <DropdownMenuLabel>{t("actions.copyAs")}</DropdownMenuLabel>
          <DropdownMenuSeparator />
          {(Object.keys(formats) as (keyof LinkFormats)[]).map((key) => (
            <CopyFormatItem key={key} label={LINK_FORMAT_LABELS[key]} value={formats[key]} />
          ))}
        </DropdownMenuContent>
      </DropdownMenu>

      {showConvert ? (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" size="sm" aria-label={t("actions.downloadAsFormat")}>
              <FileDown />
              <span className="hidden sm:inline">{t("actions.downloadAs")}</span>
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-56">
            <DropdownMenuLabel>{t("actions.downloadAsLabel")}</DropdownMenuLabel>
            <DropdownMenuSeparator />
            {convertTargets.map((option, i) => {
              // An animated source must never come back as a still without the
              // menu saying so first, and never above something that keeps it.
              const hint = downloadHintKey(option);
              const heading = sectionHeadingKey(option, convertTargets[i - 1]);
              return (
                <Fragment key={option.to}>
                  {heading ? (
                    <>
                      <DropdownMenuSeparator />
                      <DropdownMenuLabel className="text-muted-foreground text-[11px] tracking-wide uppercase">
                        {t(heading)}
                      </DropdownMenuLabel>
                    </>
                  ) : null}
                  {/* Asks before it downloads - see use-convert-download.ts.
                      This menu is the public one, so it is also where the
                      owner-only refusal lands most often. */}
                  <DropdownMenuItem
                    onSelect={() =>
                      void startDownload(`${base}/convert?to=${option.to}`, label(option.to))
                    }
                    className="flex flex-col items-start gap-0"
                  >
                    <span className="flex w-full items-baseline justify-between gap-3">
                      <span>{label(option.to)}</span>
                      {sizeLabel(option) ? (
                        <span className="text-muted-foreground text-[11px] tabular-nums">
                          {sizeLabel(option)}
                        </span>
                      ) : null}
                    </span>
                    {hint ? (
                      <span className="text-muted-foreground text-[11px]">{t(hint)}</span>
                    ) : null}
                  </DropdownMenuItem>
                </Fragment>
              );
            })}
            {/* The formats that need an encoder are missing because this hostname
                carries no session, not because the file cannot produce them.
                Saying so, with the link that fixes it, beats a shorter menu. */}
            {signInUrl ? (
              <>
                <DropdownMenuSeparator />
                <DropdownMenuItem asChild>
                  <a href={signInUrl}>
                    <span className="text-muted-foreground text-[13px]">
                      {t("actions.moreFormatsSignedIn")}
                    </span>
                  </a>
                </DropdownMenuItem>
              </>
            ) : null}
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}

      <div className="ml-auto flex items-center gap-2">
        <Button asChild variant="outline" size="sm">
          <a href={rawUrl} target="_blank" rel="noopener noreferrer">
            <ExternalLink />
            <span className="hidden sm:inline">{t("actions.openRaw")}</span>
          </a>
        </Button>
        <Button asChild variant="default" size="sm">
          <a href={downloadUrl}>
            <Download />
            {tCommon("download")}
          </a>
        </Button>
      </div>
    </div>
  );
}

function CopyFormatItem({ label, value }: { label: string; value: string }) {
  const t = useTranslations("viewer");
  return (
    <DropdownMenuItem
      onSelect={(e) => {
        e.preventDefault();
      }}
      asChild
    >
      <button
        type="button"
        className="flex w-full cursor-pointer flex-col items-start gap-0.5"
        onClick={async () => {
          try {
            await navigator.clipboard.writeText(value);
            toast.success(t("actions.formatCopied", { label }));
          } catch {
            toast.error(t("actions.copyFailed"));
          }
        }}
      >
        <span className="text-sm">{label}</span>
        <span className="max-w-full truncate font-mono text-[11px] text-muted-foreground">
          {value}
        </span>
      </button>
    </DropdownMenuItem>
  );
}
