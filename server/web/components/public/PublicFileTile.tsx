"use client";

import { useEffect, useRef, useState } from "react";
import { useTranslations } from "next-intl";
import { Play } from "lucide-react";
import { cn } from "@/lib/utils";
import { humanFileSize } from "@/lib/format";
/**
 * The dashboard tile internals are reused verbatim on purpose: a shared folder
 * should look and behave exactly like the panel it was shared from, so the same
 * blurhash thumbnail, the same viewport gate and the same icon set are used here
 * rather than a second, slowly diverging implementation.
 */
import { Thumbnail, useInView } from "@/components/dashboard/Thumbnail";
import {
  buildPublicUrl,
  buildRawUrl,
  extLabel,
  isGif,
  isImage,
  isVideo,
  KindIcon,
  kindTint,
} from "@/components/dashboard/media";
import { usePrefersReducedMotion } from "./motion";
import type { PublicTileItem } from "./types";

/**
 * Thumbnail width. The serving route only renders a fixed set of sizes
 * (see THUMB_SIZES in app/[userCode]/[mediaCode]/[action]/route.ts); 480 is the
 * one the dashboard grid asks for, so a visitor and the owner hit the same
 * rendered variant instead of doubling the work.
 */
const THUMB = 480;

export function PublicFileTile({
  item,
  userCode,
  base,
}: {
  item: PublicTileItem;
  /** Owner tag, the `{userCode}` segment of every link on this tile. */
  userCode: string;
  /**
   * Origin the owner publishes on (`userBaseUrl(owner.preferredDomain)`).
   * Links and thumbnails must be built on it, not on whatever host the visitor
   * happens to be browsing, so a gallery stays consistent with the links its
   * owner hands out.
   */
  base: string;
}) {
  const t = useTranslations("public.gallery");
  const [hovered, setHovered] = useState(false);
  const reducedMotion = usePrefersReducedMotion();
  const rootRef = useRef<HTMLAnchorElement | null>(null);
  const videoRef = useRef<HTMLVideoElement | null>(null);

  // A video tile mounts a real <video> to get its first frame, because the image
  // endpoint cannot rasterize video. Only do that near the viewport: a gallery of
  // 200 items would otherwise open 200 media connections at once and stall the tab.
  const nearViewport = useInView(rootRef, { rootMargin: "400px" });

  const video = isVideo(item.mimeType, item.ext);
  const gif = isGif(item.mimeType, item.ext);
  const image = isImage(item.mimeType, item.ext);

  const href = buildPublicUrl(base, userCode, item.code, item.ext);
  const raw = buildRawUrl(base, userCode, item.code, item.ext);

  // Motion is opt-out: a still first frame is shown to everyone, and only a
  // visitor who has not asked for reduced motion gets playback on hover.
  const previewActive = hovered && !reducedMotion;

  // Stop decoding as soon as the pointer leaves.
  useEffect(() => {
    if (video && !previewActive && videoRef.current) videoRef.current.pause();
  }, [video, previewActive]);

  return (
    <a
      ref={rootRef}
      href={href}
      aria-label={t("open", { filename: item.filename })}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onFocus={() => setHovered(true)}
      onBlur={() => setHovered(false)}
      className={cn(
        "group/file bg-card ring-foreground/10 relative flex h-full flex-col overflow-hidden rounded-xl ring-1 outline-none",
        "transition-[box-shadow,transform,background-color] duration-200",
        "hover:ring-foreground/20 hover:shadow-foreground/5 hover:-translate-y-0.5 hover:shadow-lg",
        "focus-visible:ring-primary/50 focus-visible:ring-2",
      )}
    >
      {/* Preview: dominant, nothing laid over the picture itself. */}
      <div className="bg-muted/40 relative aspect-[4/3] overflow-hidden">
        {video ? (
          <>
            {/* Static poster from a muted, metadata-only <video>. */}
            {nearViewport ? (
              <video
                src={raw}
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

            {/* Hover playback, mounted only while the pointer is on the tile. */}
            {previewActive ? (
              <video
                ref={videoRef}
                src={raw}
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
        ) : gif ? (
          <>
            <Thumbnail
              src={`${raw}?w=${THUMB}&h=${THUMB}`}
              hoverSrc={previewActive ? raw : undefined}
              active={previewActive}
              alt={item.filename}
              blurhash={item.blurhash}
              className="size-full"
              imgClassName="transition-transform duration-500 group-hover/file:scale-105"
            />
            <span className="bg-background/80 text-foreground ring-foreground/10 absolute right-2 bottom-2 z-[2] rounded-md px-1.5 py-0.5 text-[10px] font-semibold tracking-wider uppercase ring-1 backdrop-blur">
              GIF
            </span>
          </>
        ) : image ? (
          <Thumbnail
            src={`${raw}?w=${THUMB}&h=${THUMB}`}
            alt={item.filename}
            blurhash={item.blurhash}
            className="size-full"
            imgClassName="transition-transform duration-500 group-hover/file:scale-105"
          />
        ) : (
          <div className="grid size-full place-items-center">
            <div
              className={cn(
                "grid size-16 place-items-center rounded-2xl transition-transform group-hover/file:scale-105",
                kindTint(item.mimeType),
              )}
            >
              <KindIcon kind={item.mimeType} mime={item.mimeType} className="size-7" />
            </div>
          </div>
        )}
      </div>

      {/* Meta */}
      <div className="flex flex-1 flex-col justify-center gap-0.5 p-3">
        <p className="truncate text-sm font-medium" title={item.filename}>
          {item.filename}
        </p>
        <p className="text-muted-foreground text-xs tabular-nums">
          {humanFileSize(item.size)}
          <span aria-hidden className="mx-1.5 opacity-40">
            ·
          </span>
          {extLabel(item)}
        </p>
      </div>
    </a>
  );
}
