// Port of upstream app/(app)/dashboard/folders/page.tsx (the folder explorer).
// @upstream app/(app)/dashboard/folders/page.tsx
//
// Server (Rust), as upstream: requireUser(); folderId = first(sp.folder) || null;
// listUserFolders(), folderPath() (when a folder is open), unfiledStats() (at the
// root only); buildFolderTree(); notFound() when the folder id is not in the
// tree; children mapped with toBrowserFolder().
import Link from "next/link";
import { useTranslations } from "next-intl";
import { ExternalLink, FileStack, Globe, UploadCloud } from "lucide-react";
import { Button } from "@/components/ui/button";
import { FolderBrowser } from "@/components/dashboard/FolderBrowser";
import type { BrowserFolder } from "@/components/dashboard/FolderCard";

export interface FoldersPageProps {
  /** The open folder (findNode(tree, folderId)) as far as the page reads it, or null at the root. */
  current: { id: string; name: string; slug: string; isPublic: boolean; count: number; size: number } | null;
  /** folderId ? folderPath(user.id, folderId) : [] */
  breadcrumb: { id: string; name: string; slug: string }[];
  /** (current ? current.children : tree).map(toBrowserFolder) */
  children: BrowserFolder[];
  /** rows.length: every folder of the account */
  totalFolders: number;
  /** folderId ? null : unfiledStats(user.id) */
  unsorted: { count: number; size: number } | null;
  /** user.userCode ?? "" */
  userCode: string;
}

/**
 * The folder explorer — the view someone who organises their library actually
 * wants to open first. It shows folders as tiles (name, file count, size),
 * never the flat file list; that still lives at /dashboard and is reachable
 * from here in one click.
 */
export default function FoldersPage({ props }: { props: FoldersPageProps }) {
  const t = useTranslations("dashboard.folderBrowser");
  const tNav = useTranslations("nav");
  const { current, breadcrumb, children, totalFolders, unsorted, userCode } = props;
  const filesHere = current ? { count: current.count, size: current.size } : null;
  return (
    <div className="mx-auto flex w-full max-w-7xl flex-col gap-5">
      {/* Location */}
      <div className="flex flex-col gap-2">
        <nav aria-label={t("breadcrumbAria")} className="flex flex-wrap items-center gap-1.5 text-xs">
          {current ? (
            <Link
              href="/dashboard/folders"
              className="text-muted-foreground hover:text-foreground transition-colors"
            >
              {t("title")}
            </Link>
          ) : (
            <span className="text-foreground font-medium">{t("title")}</span>
          )}
          {breadcrumb.map((crumb, i) => (
            <span key={crumb.id} className="flex items-center gap-1.5">
              <span className="text-muted-foreground/40">/</span>
              {i === breadcrumb.length - 1 ? (
                <span className="text-foreground font-medium">{crumb.name}</span>
              ) : (
                <Link
                  href={`/dashboard/folders?folder=${crumb.id}`}
                  className="text-muted-foreground hover:text-foreground transition-colors"
                >
                  {crumb.name}
                </Link>
              )}
            </span>
          ))}
        </nav>

        <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
          <h1 className="font-heading text-2xl font-semibold tracking-tight">
            {current ? current.name : t("title")}
          </h1>
          <span className="bg-muted text-muted-foreground rounded-full px-2 py-0.5 text-xs font-medium tabular-nums">
            {t("countFolders", { count: children.length })}
          </span>

          {current?.isPublic && userCode ? (
            <Link
              href={`/a/${userCode}/${current.slug}`}
              target="_blank"
              rel="noreferrer"
              className="bg-primary/10 text-primary ring-primary/20 hover:bg-primary/15 inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-xs font-medium ring-1 transition-colors"
            >
              <Globe className="size-3" />
              {t("public")}
              <ExternalLink className="size-3 opacity-70" />
            </Link>
          ) : null}
        </div>

        <div className="flex flex-wrap items-center justify-between gap-3">
          <p className="text-muted-foreground text-sm">
            {current
              ? t("subtitleInside", { name: current.name })
              : totalFolders > 0
                ? t("subtitle")
                : t("subtitleEmpty")}
          </p>
          <div className="flex shrink-0 items-center gap-2">
            <Button variant="outline" size="sm" asChild>
              <Link href={current ? `/dashboard?folder=${current.id}` : "/dashboard?all=1"}>
                <FileStack className="size-4" />
                {current ? t("showFiles") : tNav("files")}
              </Link>
            </Button>
            <Button size="sm" asChild>
              <Link href="/dashboard/upload">
                <UploadCloud className="size-4" />
                {tNav("upload")}
              </Link>
            </Button>
          </div>
        </div>
      </div>

      <FolderBrowser
        folders={children}
        currentFolderId={current?.id ?? null}
        currentFolderName={current?.name ?? null}
        filesHere={filesHere}
        unsorted={unsorted}
        userCode={userCode}
      />
    </div>
  );
}
