// Port of upstream app/(app)/dashboard/loading.tsx (an async Server Component there).
// @upstream app/(app)/dashboard/loading.tsx
// Deviation: `async` dropped, getTranslations -> useTranslations.
import { useTranslations } from "next-intl";

/**
 * Streaming placeholder for the library. It mirrors the real page's rhythm —
 * heading block, toolbar, thumbnail-dominant grid — so the layout doesn't jump
 * when the data lands. With 6 800 files a page can take a moment; an honest
 * skeleton reads as "loading", a spinner reads as "stuck".
 */
export default function DashboardLoading() {
  const t = useTranslations("common");

  return (
    <div
      role="status"
      aria-busy="true"
      aria-live="polite"
      className="mx-auto flex w-full max-w-7xl flex-col gap-5"
    >
      <span className="sr-only">{t("loading")}</span>

      {/* Heading */}
      <div className="flex flex-col gap-2">
        <Bar className="h-3 w-40" />
        <div className="flex items-center gap-3">
          <Bar className="h-7 w-52 rounded-lg" />
          <Bar className="h-5 w-16 rounded-full" />
        </div>
        <Bar className="h-3.5 w-72" />
      </div>

      {/* Toolbar */}
      <div className="flex flex-col gap-3">
        <div className="flex flex-wrap items-center gap-2">
          <Bar className="h-9 min-w-56 flex-1 rounded-lg" />
          <Bar className="h-9 w-24 rounded-lg" />
          <Bar className="h-9 w-24 rounded-lg" />
          <Bar className="h-9 w-16 rounded-lg" />
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          {[64, 52, 72, 48, 60].map((w, i) => (
            <Bar key={i} className="h-6 rounded-full" style={{ width: w }} delay={i * 40} />
          ))}
        </div>
      </div>

      {/* Grid */}
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
        {Array.from({ length: 10 }).map((_, i) => (
          <div key={i} className="ring-foreground/10 bg-card overflow-hidden rounded-xl ring-1">
            <Bar className="aspect-[4/3] w-full rounded-none" delay={i * 45} />
            <div className="flex flex-col gap-2 p-3">
              <Bar className="h-3.5 w-4/5" delay={i * 45 + 60} />
              <div className="flex items-center gap-2">
                <Bar className="h-3 w-12" delay={i * 45 + 90} />
                <Bar className="h-3 w-10" delay={i * 45 + 120} />
              </div>
            </div>
          </div>
        ))}
      </div>

      {/* Pagination */}
      <div className="flex items-center justify-between gap-3 pt-1">
        <Bar className="h-4 w-40" />
        <div className="flex items-center gap-1.5">
          {[36, 36, 36, 36, 36].map((w, i) => (
            <Bar key={i} className="h-8 rounded-lg" style={{ width: w }} delay={i * 40} />
          ))}
        </div>
      </div>
    </div>
  );
}

function Bar({
  className = "",
  style,
  delay = 0,
}: {
  className?: string;
  style?: React.CSSProperties;
  delay?: number;
}) {
  return (
    <div
      aria-hidden
      className={`bg-muted skeleton-shimmer rounded-md ${className}`}
      style={{ ...style, ["--shimmer-delay" as string]: `${delay}ms` }}
    />
  );
}
