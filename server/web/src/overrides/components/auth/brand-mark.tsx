// Port of upstream components/auth/brand-mark.tsx.
// @upstream components/auth/brand-mark.tsx
// Deviation: BrandTile shows the app icon (src/assets/modlight-icon.png)
// instead of the violet gradient tile; ModlightGlyph and BrandMark are unchanged.
import Link from "next/link";
import { cn } from "@/lib/utils";
import modlightIcon from "../../../assets/modlight-icon.png";

/**
 * The Modlight glyph — a beam rising from two dispersing bars.
 *
 * Read as: a file going up, and light spreading out below it. Three shapes, no
 * hairlines, nothing thinner than ~2px once scaled to a 16px favicon, which is
 * the size that decides whether a mark survives. Drawn in `currentColor`, and
 * the geometry already carries its own padding inside the 32×32 box, so it can
 * be dropped into `BrandTile` at 100% and land exactly where it lands in
 * `app/icon.svg`.
 *
 * The chevron is filled *and* stroked with the same paint: the stroke is doing
 * the corner rounding (`linejoin: round`), which keeps the outline a single
 * path instead of a dozen arc commands.
 *
 * Prefer `BrandTile` — this bare glyph is only for the rare surface that wants
 * a monochrome mark in the surrounding text colour.
 */
export function ModlightGlyph({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 32 32"
      fill="none"
      aria-hidden="true"
      focusable="false"
      className={className}
    >
      <path
        d="M16 5.4 L25.6 15.6 L19.6 15.6 L16 11 L12.4 15.6 L6.4 15.6 Z"
        fill="currentColor"
        stroke="currentColor"
        strokeWidth={1.6}
        strokeLinejoin="round"
      />
      <rect x="9.2" y="18.8" width="13.6" height="3.4" rx="1.7" fill="currentColor" />
      <rect x="12.4" y="24" width="7.2" height="3.4" rx="1.7" fill="currentColor" opacity="0.85" />
    </svg>
  );
}

/**
 * The brand mark: the app icon, i.e. the favicon.
 *
 * A picture rather than paint: the icon is the same object the desktop app and
 * the browser tab show (public/icon.svg and public/apple-icon.png are rendered
 * from the same source), so it never re-tints per skin. The image already
 * carries its own rounded tile with transparent corners, so nothing here adds a
 * background, ring or radius. Pass a size through `className` (`size-9`,
 * `size-6`, ...).
 */
export function BrandTile({ className }: { className?: string }) {
  return (
    <img
      src={modlightIcon}
      alt=""
      aria-hidden="true"
      draggable={false}
      decoding="async"
      className={cn("shrink-0 select-none", className)}
    />
  );
}

/**
 * The Modlight wordmark used across the auth shell. Mirrors the AppShell brand
 * so the product reads consistently before and after sign-in.
 */
export function BrandMark({
  appName = "Modlight",
  href = "/",
  className,
}: {
  appName?: string;
  href?: string;
  className?: string;
}) {
  return (
    <Link
      href={href}
      className={cn(
        "group inline-flex items-center gap-2.5 outline-none",
        className,
      )}
    >
      <BrandTile className="group-focus-visible:ring-ring size-9 transition-transform group-hover:scale-105 group-focus-visible:ring-2" />
      <span className="text-lg font-semibold tracking-tight">{appName}</span>
    </Link>
  );
}
