"use client";

import * as React from "react";
import { Maximize2 } from "lucide-react";
import { useTranslations } from "next-intl";
import { cn } from "@/lib/utils";

interface ImageViewProps {
  rawUrl: string;
  filename: string;
  width?: number | null;
  height?: number | null;
}

/**
 * The image, centered on the bare page background. Clicking it opens the
 * full-resolution raw bytes in a new tab.
 */
export function ImageView({ rawUrl, filename, width, height }: ImageViewProps) {
  const t = useTranslations("viewer");
  const [loaded, setLoaded] = React.useState(false);
  const imgRef = React.useRef<HTMLImageElement | null>(null);

  /**
   * Read the starting state off the element instead of waiting for `onLoad`.
   *
   * The image is server-rendered, so by the time React hydrates the browser has
   * very often already finished decoding it (a warm cache makes this the normal
   * case, not the edge case) — and a `load` event that fired before hydration is
   * gone forever. `onLoad` therefore never ran, `loaded` stayed false, and the
   * fade-in class left the picture at `opacity-0`: the visitor saw nothing but
   * an empty stage behind a perfectly healthy image. Ref callbacks run during
   * commit, before paint, so this cannot flash.
   */
  const attach = React.useCallback((node: HTMLImageElement | null) => {
    imgRef.current = node;
    if (node?.complete) setLoaded(true);
  }, []);

  // Second safety net: covers a src swap and any commit where the element
  // finished loading between the ref callback and this effect.
  React.useEffect(() => {
    if (imgRef.current?.complete) setLoaded(true);
  }, [rawUrl]);

  return (
    <div className="flex h-full w-full items-center justify-center px-3 py-3 sm:px-6">
      <a
        href={rawUrl}
        target="_blank"
        rel="noopener noreferrer"
        title={t("image.openFullSize")}
        className="group/img relative inline-flex max-w-full items-center justify-center"
      >
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img
          ref={attach}
          src={rawUrl}
          alt={filename}
          width={width ?? undefined}
          height={height ?? undefined}
          loading="eager"
          decoding="async"
          onLoad={() => setLoaded(true)}
          // A decode failure must never leave the picture invisible — showing the
          // broken-image/alt state beats an empty stage.
          onError={() => setLoaded(true)}
          /**
           * Sized against the viewport rather than the parent on purpose: the
           * anchor shrink-wraps the picture, so its height is indefinite and a
           * percentage `max-height` on the image would resolve to `none` and let
           * a tall image run under the floating bar. `--viewer-bar-space` is the
           * measured height of that bar (see FloatingBar); the extra 3rem covers
           * the stage padding plus the gap above the bar.
           */
          style={{ maxHeight: "calc(100dvh - var(--viewer-bar-space, 7rem) - 3rem)" }}
          /**
           * The checkerboard behind transparent PNGs is deliberately gone — the
           * picture now simply sits on the neutral page background. The hairline
           * ring keeps a transparent image legible in both modes by drawing its
           * bounds, which the alpha areas no longer do on their own.
           */
          className={cn(
            "max-w-full rounded-lg object-contain shadow-2xl shadow-black/20 ring-1 ring-foreground/10 transition-opacity duration-300",
            loaded ? "opacity-100" : "opacity-0",
          )}
        />
        <span className="pointer-events-none absolute right-3 bottom-3 flex items-center gap-1.5 rounded-md border border-white/10 bg-black/55 px-2 py-1 text-xs font-medium text-white opacity-0 backdrop-blur-sm transition-opacity duration-200 group-hover/img:opacity-100">
          <Maximize2 className="size-3" />
          {t("image.openFullSize")}
        </span>
      </a>
    </div>
  );
}
