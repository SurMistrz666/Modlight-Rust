// Client port of upstream components/dashboard/EmptyState.tsx (an async Server Component there).
// @upstream components/dashboard/EmptyState.tsx
// Deviation: `async` dropped and getTranslations (next-intl/server) replaced by
// useTranslations (next-intl); nothing else differs.
import Link from "next/link";
import { useTranslations } from "next-intl";
import {
  UploadCloud,
  ImageIcon,
  FileText,
  Film,
  SearchX,
  FolderOpen,
  MousePointer2,
} from "lucide-react";
import { Button } from "@/components/ui/button";

export type EmptyStateMode = "empty" | "filtered" | "folder";

/**
 * Three genuinely different dead ends, because "nothing here" is useless advice
 * when the cause differs:
 *   • empty    — the account has no files at all → onboarding.
 *   • filtered — a search/filter matched nothing → offer to clear it.
 *   • folder   — the folder exists but is empty → explain how files get in.
 */
export function EmptyState({
  filtering,
  mode,
  folderName,
  clearHref = "/dashboard",
}: {
  /** Legacy flag: true renders the "no matches" variant. */
  filtering?: boolean;
  mode?: EmptyStateMode;
  /** Name of the folder being viewed, for the folder variant. */
  folderName?: string;
  /** Where "clear filters" points. */
  clearHref?: string;
}) {
  const t = useTranslations("files");
  const resolved: EmptyStateMode = mode ?? (filtering ? "filtered" : "empty");

  if (resolved === "filtered") {
    return (
      <Shell>
        <div className="bg-muted text-muted-foreground relative grid size-14 place-items-center rounded-2xl">
          <SearchX className="size-7" />
        </div>
        <div className="relative space-y-1">
          <h3 className="text-base font-medium">{t("empty.noMatchingTitle")}</h3>
          <p className="text-muted-foreground mx-auto max-w-sm text-sm">
            {t("empty.noMatchingDesc")}
          </p>
        </div>
        <Button variant="outline" asChild className="relative">
          <Link href={clearHref}>{t("empty.clearFilters")}</Link>
        </Button>
      </Shell>
    );
  }

  if (resolved === "folder") {
    return (
      <Shell>
        <div className="bg-primary/10 text-primary ring-primary/15 relative grid size-14 place-items-center rounded-2xl ring-1">
          <FolderOpen className="size-7" />
        </div>
        <div className="relative space-y-1">
          <h3 className="text-base font-medium">
            {folderName
              ? t("empty.folderTitleNamed", { name: folderName })
              : t("empty.folderTitle")}
          </h3>
          <p className="text-muted-foreground mx-auto max-w-md text-sm">
            {t("empty.folderDesc")}
          </p>
        </div>
        <div className="text-muted-foreground/90 bg-muted/60 ring-foreground/10 relative flex items-center gap-2 rounded-full px-3 py-1.5 text-xs ring-1">
          <MousePointer2 className="size-3.5" />
          {t("empty.folderDragHint")}
        </div>
        <div className="relative flex flex-col items-center gap-2 sm:flex-row">
          <Button asChild>
            <Link href="/dashboard/upload">
              <UploadCloud className="size-4" /> {t("empty.uploadHere")}
            </Link>
          </Button>
          <Button asChild variant="ghost">
            <Link href="/dashboard">{t("empty.browseAll")}</Link>
          </Button>
        </div>
      </Shell>
    );
  }

  return (
    <Shell>
      {/* floating file chips illustration */}
      <div className="relative flex items-end justify-center gap-3">
        <FloatTile
          className="-rotate-6 [animation-delay:0ms]"
          tint="bg-muted text-muted-foreground"
        >
          <FileText className="size-6" />
        </FloatTile>
        <FloatTile
          className="z-10 scale-110 [animation-delay:150ms]"
          tint="bg-primary/10 text-primary"
        >
          <ImageIcon className="size-7" />
        </FloatTile>
        <FloatTile
          className="rotate-6 [animation-delay:300ms]"
          tint="bg-accent text-accent-foreground"
        >
          <Film className="size-6" />
        </FloatTile>
      </div>

      <div className="relative space-y-1">
        <h3 className="text-lg font-semibold tracking-tight">{t("empty.noFilesTitle")}</h3>
        <p className="text-muted-foreground mx-auto max-w-sm text-sm">
          {t("empty.noFilesDesc")}
        </p>
      </div>

      <div className="relative flex flex-col items-center gap-2 sm:flex-row">
        <Button asChild size="lg" className="shadow-sm">
          <Link href="/dashboard/upload">
            <UploadCloud className="size-4" /> {t("empty.uploadFirst")}
          </Link>
        </Button>
        <Button asChild variant="ghost" size="lg">
          <Link href="/dashboard/tokens">{t("empty.setUpShareX")}</Link>
        </Button>
      </div>
    </Shell>
  );
}

function Shell({ children }: { children: React.ReactNode }) {
  return (
    <div className="bg-card/40 ring-foreground/10 relative flex flex-col items-center justify-center gap-5 overflow-hidden rounded-xl px-6 py-20 text-center ring-1">
      {/* subtle decorative grid */}
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 opacity-[0.04] [background-image:linear-gradient(to_right,currentColor_1px,transparent_1px),linear-gradient(to_bottom,currentColor_1px,transparent_1px)] [background-size:24px_24px]"
      />
      {children}
    </div>
  );
}

function FloatTile({
  children,
  className,
  tint,
}: {
  children: React.ReactNode;
  className?: string;
  tint: string;
}) {
  return (
    <div
      className={`bg-card ring-foreground/10 grid size-16 animate-bounce place-items-center rounded-2xl shadow-md ring-1 [animation-duration:3s] ${className}`}
    >
      <div className={`grid size-full place-items-center rounded-2xl ${tint}`}>{children}</div>
    </div>
  );
}
