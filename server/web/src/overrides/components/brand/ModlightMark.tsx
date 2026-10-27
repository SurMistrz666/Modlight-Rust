// Port of upstream components/brand/ModlightMark.tsx.
// @upstream components/brand/ModlightMark.tsx
// Deviation: draws the app icon (the same picture as the favicon) instead of
// the inline violet SVG tile.
import modlightIcon from "../../../assets/modlight-icon.png";

/**
 * The Modlight mark, inline: the app icon, the same picture the favicon and
 * the desktop app show. The image carries its own rounded tile, so nothing is
 * painted around it.
 */
export function ModlightMark({ className }: { className?: string }) {
  return (
    <img
      src={modlightIcon}
      alt=""
      aria-hidden="true"
      draggable={false}
      decoding="async"
      className={className}
    />
  );
}
