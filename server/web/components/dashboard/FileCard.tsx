"use client";

import { useEffect, useRef, useState } from "react";
import { useLocale, useTranslations } from "next-intl";
import { toast } from "sonner";
import {
  Eye,
  Globe,
  Lock,
  Copy,
  ExternalLink,
  MoreHorizontal,
  Clock,
  KeyRound,
  Flame,
  Play,
  Folder,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { humanFileSize, relativeTime } from "@/lib/format";
import { Thumbnail, TypeBadge, TypeChip, useInView } from "./Thumbnail";
import { FileMenu } from "./FileMenu";
import {
  buildOpenUrl,
  buildPublicUrl,
  buildRawUrl,
  canPlayVideo,
  canShowImage,
  extLabel,
  isGif,
} from "./media";
import type { FileItem, FileViewMode } from "./types";

/** Stop a control inside the tile from also triggering the tile's own click. */
function swallow(e: React.MouseEvent | React.KeyboardEvent) {
  e.stopPropagation();
}

export function FileCard({
  item,
  userCode,
  appUrl,
  copyRaw,
  selected = false,
  selecting = false,
  onToggleSelect,
  onOpenDetails,
  onTrashed,
  onTrashedMany,
  onMoved,
  targetIds,
  targetItems,
  currentFolderId,
  mode,
}: {
  item: FileItem;
  userCode: string;
  appUrl: string;
  copyRaw?: boolean;
  /** Owner's own library, or an administrator reviewing it. */
  mode?: FileViewMode;
  /** Part of the current multi-selection. */
  selected?: boolean;
  /** A selection is active somewhere in the grid — keep checkboxes visible. */
  selecting?: boolean;
  onToggleSelect?: () => void;
  onOpenDetails?: () => void;
  onTrashed: (id: string) => void;
  onTrashedMany?: (ids: string[]) => void;
  onMoved?: (ids: string[], folderId: string | null) => void;
  targetIds?: string[];
  /** The selected rows, so "Convert" can reason about a mixed selection. */
  targetItems?: FileItem[];
  currentFolderId?: string | null;
}) {
  const t = useTranslations("files");
  // relativeTime formats through Intl, so it has to be told whose language
  // this is; without it a Polish reader gets English ages.
  const locale = useLocale();
  const [hovered, setHovered] = useState(false);
  const reducedMotion = usePrefersReducedMotion();
  const rootRef = useRef<HTMLDivElement | null>(null);
  const videoRef = useRef<HTMLVideoElement | null>(null);

  // Video tiles mount a real <video> to get a first frame. Only do that near
  // the viewport: a page of 120 tiles would otherwise open 120 media streams.
  const nearViewport = useInView(rootRef, { rootMargin: "400px" });

  /*
   * Three questions, not two, and all three ask what the browser can actually
   * DRAW rather than what the file claims to be.
   *
   * `isImage` / `isVideo` were the wrong tests for a tile: `image/heic` and
   * `image/vnd.adobe.photoshop` are images the thumbnailer refuses, and
   * `video/x-matroska` is a video no browser can demux. Both used to mount a
   * preview element that quietly painted nothing, which is where the wall of
   * identical grey cards came from. Those rows now fall through to the type
   * badge instead, and no request is made for bytes that cannot be shown.
   */
  const video = canPlayVideo(item.mimeType, item.ext);
  const gif = isGif(item.mimeType, item.ext);
  const image = canShowImage(item.mimeType, item.ext);
  /*
   * The one thing a rendered preview cannot say about itself: that it moves.
   * `frames` is null for rows uploaded before it was counted, so an unknown
   * count is read as "still" - a missing chip is a smaller lie than a wrong one.
   */
  const animated = gif || (item.frames ?? 1) > 1;
  const typeBadge = <TypeBadge mime={item.mimeType} ext={item.ext} />;
  const publicUrl = buildPublicUrl(appUrl, userCode, item.code, item.ext);
  const rawUrl = buildRawUrl(appUrl, userCode, item.code, item.ext);
  // Opening stays on the host the panel is being read on, so the owner arrives
  // with their session and sees the full menu. See buildOpenUrl.
  const openUrl = buildOpenUrl(userCode, item.code, item.ext);

  // Only animate previews when the user hasn't asked to reduce motion.
  const previewActive = hovered && !reducedMotion;

  // Pause the hover-playing video as soon as the pointer leaves so we stop
  // decoding/streaming and free resources.
  useEffect(() => {
    if (video && !previewActive && videoRef.current) {
      videoRef.current.pause();
    }
  }, [video, previewActive]);

  async function quickCopy() {
    try {
      await navigator.clipboard.writeText(copyRaw ? rawUrl : publicUrl);
      toast.success(t("toast.linkCopied"));
    } catch {
      toast.error(t("toast.couldntCopy"));
    }
  }

  const chips = [
    item.hasPassword && {
      key: "pw",
      icon: <KeyRound className="size-3" />,
      label: t("card.passwordProtected"),
    },
    item.expiresAt && {
      key: "exp",
      icon: <Clock className="size-3" />,
      label: t("card.hasExpiry"),
    },
    item.maxViews != null && {
      key: "burn",
      icon: <Flame className="size-3" />,
      label: t("card.limitedToViews", { count: item.maxViews }),
    },
  ].filter(Boolean) as { key: string; icon: React.ReactNode; label: string }[];

  return (
    <div
      ref={rootRef}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      className={cn(
        "group/file bg-card ring-foreground/10 relative flex h-full flex-col overflow-hidden rounded-xl ring-1 transition-[box-shadow,transform,background-color] duration-200",
        "group-hover/tile:ring-foreground/20 group-hover/tile:shadow-foreground/5 group-hover/tile:-translate-y-0.5 group-hover/tile:shadow-lg",
        // Selected tiles stay put: the grid draws the selection ring around
        // them, and a lifting tile makes a multi-selection look unstable.
        selected && "bg-primary/[0.04] group-hover/tile:translate-y-0",
      )}
    >
      {/* Preview — dominant, and nothing is laid over the picture itself */}
      <div className="bg-muted/40 relative block aspect-[4/3] overflow-hidden">
        {video ? (
          <>
            {/* Static poster: a real first frame from a muted, metadata-only
                <video>, because the image endpoint can't rasterize video. */}
            {nearViewport ? (
              <video
                src={rawUrl}
                muted
                playsInline
                preload="metadata"
                draggable={false}
                className={cn(
                  "absolute inset-0 size-full object-cover transition-opacity duration-300",
                  previewActive && "opacity-0",
                )}
              />
            ) : (
              <div className="from-muted to-muted/40 absolute inset-0 bg-gradient-to-br" />
            )}

            {/* Hover playback, mounted only while the pointer is over the tile. */}
            {previewActive ? (
              <video
                ref={videoRef}
                src={rawUrl}
                muted
                loop
                playsInline
                autoPlay
                preload="none"
                draggable={false}
                className="absolute inset-0 size-full object-cover"
              />
            ) : null}

            {!previewActive ? (
              <span className="bg-background/70 text-foreground ring-foreground/10 absolute top-1/2 left-1/2 z-[2] grid size-11 -translate-x-1/2 -translate-y-1/2 place-items-center rounded-full ring-1 backdrop-blur transition-transform group-hover/file:scale-110">
                <Play className="size-5 translate-x-px fill-current" />
              </span>
            ) : null}
          </>
        ) : image ? (
          <>
            <Thumbnail
              src={`${rawUrl}?w=480&h=480`}
              /* Only a GIF swaps in the full file on hover; an animated WebP or
                 AVIF needs the motion endpoint, which is a separate concern. */
              hoverSrc={gif && previewActive ? rawUrl : undefined}
              active={previewActive}
              alt={item.filename}
              blurhash={item.blurhash}
              className="size-full"
              imgClassName="transition-transform duration-500 group-hover/file:scale-105"
              /* A thumbnail that never arrives still says what the file is. */
              fallback={typeBadge}
            />
            {animated ? (
              <TypeChip label={extLabel(item)} className="absolute right-2 bottom-2 z-[2]" />
            ) : null}
          </>
        ) : (
          /*
           * No picture to look at, so the tile draws the file's identity
           * instead. Nothing is laid over a preview that DID render: where there
           * is a picture, the picture is the identity, and a badge on top of it
           * would cover the one thing the owner opened the page to see. The only
           * exception is the chip above, which says the still frame moves.
           */
          <div className="grid size-full place-items-center">{typeBadge}</div>
        )}

        {/*
          There is deliberately no scrim over the preview. A top-down gradient
          used to sit here to keep the chips legible; it read as a light being
          shone on every single tile. The chips carry their own translucent,
          blurred plate instead, which is legible on any image without touching
          the picture.
        */}

        {/* Selection checkbox */}
        {onToggleSelect ? (
          <label
            onClick={(e) => {
              swallow(e);
              onToggleSelect();
            }}
            className={cn(
              "bg-background/85 ring-foreground/10 absolute top-2 left-2 z-[3] grid size-7 cursor-pointer place-items-center rounded-md ring-1 backdrop-blur transition-opacity",
              selected || selecting
                ? "opacity-100"
                : "opacity-0 group-hover/file:opacity-100 focus-within:opacity-100",
            )}
          >
            <Checkbox
              checked={selected}
              tabIndex={-1}
              aria-label={t("selection.selectFile", { filename: item.filename })}
              className="pointer-events-none"
            />
          </label>
        ) : null}

        {/* Hover quick actions */}
        <div className="absolute top-2 right-2 z-[3] flex items-center gap-1 opacity-0 transition-opacity group-hover/file:opacity-100 focus-within:opacity-100">
          <QuickBtn
            label={t("card.copyLink")}
            onClick={(e) => {
              swallow(e);
              quickCopy();
            }}
          >
            <Copy className="size-3.5" />
          </QuickBtn>
          <QuickBtn label={t("card.open")} asLink href={openUrl} onClick={swallow}>
            <ExternalLink className="size-3.5" />
          </QuickBtn>
        </div>

        {/* Status chips */}
        <div className="pointer-events-none absolute bottom-2 left-2 z-[2] flex items-center gap-1">
          <span
            title={item.published ? t("card.public") : t("card.private")}
            className={cn(
              "ring-foreground/10 grid size-6 place-items-center rounded-md ring-1 backdrop-blur",
              item.published
                ? "bg-background/80 text-foreground"
                : "bg-background/80 text-muted-foreground",
            )}
          >
            {item.published ? <Globe className="size-3" /> : <Lock className="size-3" />}
          </span>
          {chips.map((chip) => (
            <span
              key={chip.key}
              title={chip.label}
              className="bg-background/80 text-foreground ring-foreground/10 grid size-6 place-items-center rounded-md ring-1 backdrop-blur"
            >
              {chip.icon}
            </span>
          ))}
        </div>
      </div>

      {/* Meta */}
      <div className="flex flex-1 flex-col gap-2 p-3">
        <div className="flex items-start justify-between gap-2">
          <div className="min-w-0">
            <p className="truncate text-sm font-medium" title={item.filename}>
              {item.filename}
            </p>
            <p className="text-muted-foreground mt-0.5 text-xs tabular-nums">
              {humanFileSize(item.size)} · {relativeTime(item.createdAt, locale)}
            </p>
          </div>
          <div className="-mt-0.5 -mr-1 shrink-0" onClick={swallow}>
            <FileMenu
              item={item}
              userCode={userCode}
              appUrl={appUrl}
              copyRaw={copyRaw}
              targetIds={targetIds}
              targetItems={targetItems}
              currentFolderId={currentFolderId}
              mode={mode}
              onOpenDetails={onOpenDetails}
              onTrashed={onTrashed}
              onTrashedMany={onTrashedMany}
              onMoved={onMoved}
              trigger={
                <Button
                  variant="ghost"
                  size="icon-sm"
                  tabIndex={-1}
                  aria-label={t("card.fileActions")}
                >
                  <MoreHorizontal className="size-4" />
                </Button>
              }
            />
          </div>
        </div>

        <div className="text-muted-foreground mt-auto flex items-center gap-3 text-xs">
          <span className="flex items-center gap-1 tabular-nums">
            <Eye className="size-3" /> {item.views.toLocaleString()}
          </span>
          {item.folderName ? (
            <span className="flex min-w-0 items-center gap-1" title={item.folderName}>
              <Folder className="size-3 shrink-0" />
              <span className="truncate">{item.folderName}</span>
            </span>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function QuickBtn({
  label,
  children,
  onClick,
  asLink,
  href,
}: {
  label: string;
  children: React.ReactNode;
  onClick?: (e: React.MouseEvent) => void;
  asLink?: boolean;
  href?: string;
}) {
  const inner = (
    <span className="bg-background/85 text-foreground ring-foreground/10 hover:bg-background grid size-7 place-items-center rounded-md ring-1 backdrop-blur transition-colors">
      {children}
    </span>
  );
  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger asChild>
          {asLink ? (
            <a
              href={href}
              target="_blank"
              rel="noopener noreferrer"
              aria-label={label}
              tabIndex={-1}
              draggable={false}
              onClick={onClick}
            >
              {inner}
            </a>
          ) : (
            <button type="button" tabIndex={-1} onClick={onClick} aria-label={label}>
              {inner}
            </button>
          )}
        </TooltipTrigger>
        <TooltipContent>{label}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

/** Tracks the user's `prefers-reduced-motion` setting (SSR-safe). */
export function usePrefersReducedMotion() {
  const [reduced, setReduced] = useState(false);

  useEffect(() => {
    const mql = window.matchMedia("(prefers-reduced-motion: reduce)");
    const onChange = () => setReduced(mql.matches);
    onChange();
    mql.addEventListener("change", onChange);
    return () => mql.removeEventListener("change", onChange);
  }, []);

  return reduced;
}
