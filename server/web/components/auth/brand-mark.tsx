import Link from "next/link";
import { cn } from "@/lib/utils";

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
 * The brand mark: the violet tile plus the white glyph, i.e. the favicon.
 *
 * This is the *only* correct way to show the logo. The previous pattern —
 * wrapping the bare glyph in a `bg-primary text-primary-foreground` chip —
 * repainted the mark from the active skin, so it came out navy under win98 and,
 * worse, near-black-on-violet in every dark mode (`--primary-foreground` is a
 * dark ink there, since its job is to sit on top of a light primary). The tab
 * showed one logo and the header showed another.
 *
 * Brand paint therefore lives in the `brand-tile` utility (app/globals.css),
 * literal and skin-independent, mirroring the gradient in `app/icon.svg`:
 * the corner radius here (22%) is the CSS equivalent of that file's `rx="7"`
 * on a 32-wide tile. Pass a size through `className` (`size-9`, `size-6`, …);
 * the glyph fills the tile and keeps the favicon's proportions at every size.
 */
export function BrandTile({ className }: { className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "brand-tile grid shrink-0 place-items-center overflow-hidden rounded-[22%] shadow-sm ring-1 ring-white/15 ring-inset",
        className,
      )}
    >
      <ModlightGlyph className="size-full" />
    </span>
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
