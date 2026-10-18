/**
 * The Modlight tile mark, inline.
 *
 * Mirrors app/icon.svg, which is the favicon and cannot be imported as a
 * component. Kept as SVG rather than an <img> so it inherits the page's colour
 * handling, needs no network request on a public media page, and stays crisp at
 * the 20 px it is usually drawn at.
 *
 * The gradient ids are namespaced. Two copies of this on one page with a bare
 * `id="tile"` would both point at whichever definition the document parsed
 * last, which is the classic way an inline SVG logo turns black somewhere else
 * on the page.
 */
export function ModlightMark({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 32 32"
      role="img"
      aria-hidden="true"
      focusable="false"
      className={className}
    >
      <defs>
        <linearGradient id="ml-mark-tile" x1="0" y1="0" x2="0.55" y2="1">
          <stop offset="0" stopColor="#6E6BF5" />
          <stop offset="1" stopColor="#3B2FC2" />
        </linearGradient>
        <linearGradient id="ml-mark-gloss" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#fff" stopOpacity="0.22" />
          <stop offset="0.55" stopColor="#fff" stopOpacity="0" />
        </linearGradient>
      </defs>
      <rect width="32" height="32" rx="7" fill="url(#ml-mark-tile)" />
      <rect width="32" height="32" rx="7" fill="url(#ml-mark-gloss)" />
      <path
        d="M16 5.4 L25.6 15.6 L19.6 15.6 L16 11.0 L12.4 15.6 L6.4 15.6 Z"
        fill="#fff"
        stroke="#fff"
        strokeWidth="1.6"
        strokeLinejoin="round"
      />
      <rect x="9.2" y="18.8" width="13.6" height="3.4" rx="1.7" fill="#fff" />
      <rect x="12.4" y="24" width="7.2" height="3.4" rx="1.7" fill="#fff" opacity="0.85" />
    </svg>
  );
}
