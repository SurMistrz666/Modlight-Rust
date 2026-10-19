"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslations } from "next-intl";
import { decode } from "blurhash";
import { cn } from "@/lib/utils";
import { extLabel, FAMILY_EMBLEM, FAMILY_PLATE, typeFamily } from "./media";

/**
 * Image thumbnail with a blurhash canvas placeholder that fades out once the
 * real image loads. Falls back to a neutral skeleton when no blurhash exists.
 */
export function Thumbnail({
  src,
  hoverSrc,
  active,
  alt,
  blurhash,
  className,
  imgClassName,
  fallback,
}: {
  src: string;
  /**
   * Optional animated/full-res source to swap in while `active` is true
   * (e.g. the raw animated GIF behind a static first-frame thumbnail).
   */
  hoverSrc?: string;
  /** When true and `hoverSrc` is set, render `hoverSrc` instead of `src`. */
  active?: boolean;
  alt: string;
  blurhash: string | null;
  className?: string;
  imgClassName?: string;
  /**
   * What to draw when the image never arrives.
   *
   * This component used to render NOTHING on error, so a row whose bytes the
   * thumbnailer cannot open - a .heic, a .psd, a truncated upload - left a bare
   * muted rectangle indistinguishable from every other unopenable file. The tile
   * passes its type badge here, so a failed thumbnail still says what the file is.
   */
  fallback?: React.ReactNode;
}) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const imgRef = useRef<HTMLImageElement | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [errored, setErrored] = useState(false);

  const displaySrc = active && hoverSrc ? hoverSrc : src;

  /**
   * Same trap as the full-size viewer: a `load` event that fired before React
   * hydrated is gone, so `onLoad` alone leaves an already-decoded image stuck at
   * `opacity-0` — an invisible tile over a perfectly healthy file.
   * `loading="lazy"` makes this rarer here than in the viewer, but tiles above
   * the fold decode immediately on a warm cache and hit it every time. Ref
   * callbacks run during commit, before paint, so this cannot flash.
   */
  const attachImg = useCallback((node: HTMLImageElement | null) => {
    imgRef.current = node;
    if (node?.complete) setLoaded(true);
  }, []);

  // Second net: covers the hover swap between `src` and `hoverSrc`.
  useEffect(() => {
    if (imgRef.current?.complete) setLoaded(true);
  }, [displaySrc]);

  useEffect(() => {
    if (!blurhash || !canvasRef.current) return;
    try {
      const w = 32;
      const h = 32;
      const pixels = decode(blurhash, w, h);
      const ctx = canvasRef.current.getContext("2d");
      if (!ctx) return;
      const imageData = ctx.createImageData(w, h);
      imageData.data.set(pixels);
      ctx.putImageData(imageData, 0, 0);
    } catch {
      /* invalid hash — skeleton shows through */
    }
  }, [blurhash]);

  return (
    <div className={cn("bg-muted relative overflow-hidden", className)}>
      {blurhash ? (
        <canvas
          ref={canvasRef}
          width={32}
          height={32}
          aria-hidden
          className={cn(
            "absolute inset-0 size-full scale-110 transition-opacity duration-500",
            loaded ? "opacity-0" : "opacity-100",
          )}
        />
      ) : (
        <div
          aria-hidden
          className={cn(
            "from-muted to-muted/40 absolute inset-0 animate-pulse bg-gradient-to-br transition-opacity duration-500",
            loaded ? "opacity-0" : "opacity-100",
          )}
        />
      )}
      {!errored ? (
        // eslint-disable-next-line @next/next/no-img-element
        <img
          src={displaySrc}
          alt={alt}
          loading="lazy"
          decoding="async"
          // The tile itself owns dragging (files → folders); a native image drag
          // would hijack the gesture and replace our payload with an image URL.
          draggable={false}
          ref={attachImg}
          onLoad={() => setLoaded(true)}
          onError={() => {
            setErrored(true);
            setLoaded(true);
          }}
          className={cn(
            "relative size-full object-cover transition-opacity duration-500",
            loaded ? "opacity-100" : "opacity-0",
            imgClassName,
          )}
        />
      ) : (
        <div className="relative grid size-full place-items-center">{fallback}</div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// The mark a file gets when there is no picture of it
// ---------------------------------------------------------------------------

/**
 * How wide the extension may be drawn before it has to shrink. Real extensions
 * are three to five characters; "markdown" and "webmanifest" exist, so the size
 * steps down rather than cutting a word in half and reading as a typo.
 */
function extTextClass(label: string): string {
  if (label.length <= 3) return "text-[13px]";
  if (label.length === 4) return "text-[11px]";
  if (label.length === 5) return "text-[10px]";
  return "text-[8px]";
}

/**
 * A recognisable type mark for a file the tile cannot show a picture of.
 *
 * Shape first, then colour, then the extension - in that order of legibility.
 * The silhouette is a sheet with a folded corner, which reads as "a file" at any
 * size and in any skin; the plate carries the family colour; the extension is
 * printed on it, because a colour alone can only ever say "some kind of
 * document" and cannot be perceived at all by a part of the audience. The family
 * is spelled out underneath for the same reason (WCAG 1.4.1: colour is never the
 * only carrier of meaning).
 *
 * The plate is opaque, and that is what makes one badge work across fourteen
 * skins: the surface it sits on stops mattering, so the only contrast to
 * guarantee is white-on-plate, which is a constant this project measures in
 * tests/unit/type-badge.test.ts. Everything around the plate - the caption, the
 * tile behind it - stays on tokens.
 */
export function TypeBadge({
  mime,
  ext,
  showFamily = true,
  className,
}: {
  mime: string;
  ext: string;
  /** Print the family name under the mark. Off inside very small containers. */
  showFamily?: boolean;
  className?: string;
}) {
  const t = useTranslations("files");
  const family = typeFamily(mime, ext);
  const Emblem = FAMILY_EMBLEM[family];
  const label = extLabel({ ext, mimeType: mime }).slice(0, 8);
  const familyName = t(`type.family.${family}`);

  return (
    <div className={cn("flex flex-col items-center gap-1.5", className)}>
      <div
        role="img"
        aria-label={t("type.aria", { ext: label, family: familyName })}
        className="relative aspect-[4/5] w-14 transition-transform duration-200 group-hover/file:scale-105"
      >
        {/* The sheet and its folded corner. One SVG rather than a rounded div
            plus a faked triangle: a clip-path would throw the rounded corners
            away, and a border trick has to be painted in the colour of whatever
            is behind it, which on fourteen skins is not a knowable colour. */}
        <svg
          viewBox="0 0 64 80"
          className="text-foreground/35 absolute inset-0 size-full"
          aria-hidden
          focusable="false"
        >
          <path
            d="M8 0h32l24 24v48a8 8 0 0 1-8 8H8a8 8 0 0 1-8-8V8a8 8 0 0 1 8-8z"
            fill={FAMILY_PLATE[family]}
          />
          <path d="M40 0l24 24H48a8 8 0 0 1-8-8V0z" fill="#ffffff" fillOpacity="0.26" />
          {/*
           * The rim, and the ONE part of the mark that has to know about the
           * skin. The plate is opaque so the label can be guaranteed legible on
           * it, but that same opacity is what lets the sheet SHAPE disappear:
           * measured against the tile surface, the darkest plates come out at
           * 1.42:1 on win98 dark and 1.50:1 on cartoon dark, so what the eye
           * gets there is white text floating in a dark rectangle rather than a
           * file. A fixed white rim only half-fixes it (worst case 2.07:1) and
           * does nothing at all on a light skin, where white-on-white is not an
           * edge. `currentColor` off a foreground token inverts with the skin -
           * light rim on dark skins, dark rim on light ones - which takes the
           * worst pairing across all 14 skins in both modes to 3.20:1, over the
           * 3:1 WCAG asks of a graphic that carries meaning.
           * tests/unit/type-badge.test.ts measures every pair.
           *
           * Drawn along the path's own edge, so the viewBox clips the outer half
           * and the rim reads as an inset bevel that cannot spill past the mark.
           */}
          <path
            d="M8 0h32l24 24v48a8 8 0 0 1-8 8H8a8 8 0 0 1-8-8V8a8 8 0 0 1 8-8z"
            fill="none"
            stroke="currentColor"
            strokeWidth={2}
          />
        </svg>

        <span className="absolute inset-0 flex flex-col items-center justify-center gap-1 pt-2">
          <Emblem className="size-5 text-white/85" aria-hidden />
          <span
            className={cn(
              "rounded-[3px] bg-black/25 px-1 font-bold tracking-tight text-white uppercase",
              extTextClass(label),
            )}
          >
            {label}
          </span>
        </span>
      </div>

      {showFamily ? (
        <span className="text-muted-foreground text-[10px] font-medium tracking-wider uppercase">
          {familyName}
        </span>
      ) : null}
    </div>
  );
}

/**
 * A small label laid over a preview that DOES render.
 *
 * Token-driven, unlike the badge: this one sits on top of the user's picture, so
 * it has to belong to the skin rather than to a palette of its own. It is used
 * for one thing only - saying that a still-looking tile actually moves - because
 * that is the single fact the picture itself cannot show.
 */
export function TypeChip({ label, className }: { label: string; className?: string }) {
  return (
    <span
      className={cn(
        "bg-background/80 text-foreground ring-foreground/10 rounded-md px-1.5 py-0.5 text-[10px] font-semibold tracking-wider uppercase ring-1 backdrop-blur",
        className,
      )}
    >
      {label}
    </span>
  );
}

/**
 * Reports whether an element is near the viewport.
 *
 * Video tiles render a real `<video preload="metadata">` to get a first frame,
 * because the image endpoint can't rasterize video. With 120 tiles on a page
 * (and libraries here run to 6800+ files) mounting them all opens 120 media
 * connections and stalls the tab - so tiles only mount their video once this
 * says they're close to being seen, and drop it again when they scroll far away.
 */
export function useInView<T extends HTMLElement>(
  ref: React.RefObject<T | null>,
  { rootMargin = "300px", once = false }: { rootMargin?: string; once?: boolean } = {},
) {
  const [inView, setInView] = useState(false);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (typeof IntersectionObserver === "undefined") {
      setInView(true);
      return;
    }

    const io = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          setInView(true);
          if (once) io.disconnect();
        } else if (!once) {
          setInView(false);
        }
      },
      { rootMargin },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [ref, rootMargin, once]);

  return inView;
}
