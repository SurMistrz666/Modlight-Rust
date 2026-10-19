"use client";

import { PublicFileTile } from "./PublicFileTile";
import type { PublicTileItem } from "./types";

/**
 * The grid both public galleries render: a shared folder (/a/{tag}/{slug}) and
 * a public profile (/u/{tag}). One component so the two never drift apart, and
 * so a visitor gets the same tiles the owner sees in the panel.
 */
export function PublicGallery({
  items,
  userCode,
  base,
  label,
}: {
  items: PublicTileItem[];
  /** Owner tag used to build every link in the grid. */
  userCode: string;
  /** Origin the owner publishes on (see PublicFileTile). */
  base: string;
  /** Accessible name for the grid region. */
  label: string;
}) {
  return (
    <section
      aria-label={label}
      className="grid grid-cols-2 gap-3 sm:grid-cols-3 sm:gap-4 lg:grid-cols-4"
    >
      {items.map((item) => (
        <PublicFileTile key={item.id} item={item} userCode={userCode} base={base} />
      ))}
    </section>
  );
}
