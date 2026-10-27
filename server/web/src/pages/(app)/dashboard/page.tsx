// Port of upstream app/(app)/dashboard/page.tsx (the file library).
// @upstream app/(app)/dashboard/page.tsx
//
// Server (Rust), as upstream: requireUser(); listUserFolders(); a bare
// /dashboard (no query string at all) of an account that has folders redirects
// to /dashboard/folders; the query string is parsed exactly as upstream does
// (page, per, search, sort, view, folder, sub, kind, visibility, size, after,
// before); kindCounts(), listUploads(), folderPath(), getUserPrefs(); the rows
// become FileItems. Everything below "Heading" is the upstream JSX verbatim.
import Link from "next/link";
import { useTranslations } from "next-intl";
import { ExternalLink, Globe } from "lucide-react";
import type { FileKind, SortKey } from "@/lib/data/uploads";
import type { FolderRow } from "@/lib/data/folders";
import { Toolbar } from "@/components/dashboard/Toolbar";
import { FileGrid } from "@/components/dashboard/FileGrid";
import { FileList } from "@/components/dashboard/FileList";
import { Pagination } from "@/components/dashboard/Pagination";
import { EmptyState } from "@/components/dashboard/EmptyState";
import type { FileItem } from "@/components/dashboard/types";
import { VaultBanner, type LibraryVault } from "../../../encryption/VaultBanner";

export interface DashboardPageProps {
  /** first(sp.search)?.trim() || undefined */
  search?: string;
  /** first(sp.sort) when it is one of SORTS, else "newest" */
  sort: SortKey;
  /** first(sp.view) === "list" ? "list" : "grid" */
  view: "grid" | "list";
  /** first(sp.kind) when it is one of FILE_KINDS */
  kind?: FileKind;
  /** first(sp.visibility) when "public" | "private" */
  visibility?: "public" | "private";
  /** first(sp.size), raw (the Toolbar echoes it) */
  rawSize?: string;
  /** first(sp.after), raw */
  rawAfter?: string;
  /** first(sp.before), raw */
  rawBefore?: string;
  /** "none" -> null (loose files), another value -> that folder id, absent -> everything */
  folderId?: string | null;
  /** first(sp.sub) === "1" */
  includeSubfolders: boolean;
  /** Boolean(search || kind || visibility || minSize || after || before), with the parsed values */
  filtering: boolean;
  /** kindCounts(user.id) */
  counts: Record<FileKind, number>;
  /** listUploads(...) totals: total rows, the page and page size it actually used */
  result: { total: number; page: number; perPage: number };
  /** result.items mapped to FileItem exactly as upstream maps them */
  items: FileItem[];
  /** folderId ? folderPath(user.id, folderId) : [] */
  breadcrumb: { id: string; name: string; slug: string }[];
  /** allFolders.find(f => f.id === folderId) when a folder id was given and exists */
  currentFolder?: Pick<FolderRow, "id" | "name" | "slug" | "isPublic">;
  /** userBaseUrl(prefs?.preferredDomain ?? null) */
  linkBase: string;
  /** user.userCode ?? "" */
  userCode: string;
  /** user.copyRaw ?? false */
  copyRaw: boolean;
  /** Not upstream: the account's file vault (docs/ENCRYPTION.md); absent without one. */
  vault?: LibraryVault;
}

export default function DashboardPage({ props }: { props: DashboardPageProps }) {
  const t = useTranslations("dashboard");
  const tNav = useTranslations("nav");
  const {
    search,
    sort,
    view,
    kind,
    visibility,
    rawSize,
    rawAfter,
    rawBefore,
    folderId,
    includeSubfolders,
    filtering,
    counts,
    result,
    items,
    breadcrumb,
    currentFolder,
    linkBase,
    userCode,
    copyRaw,
  } = props;
  const hasResults = items.length > 0;
  // ---- Heading -----------------------------------------------------------
  const title = currentFolder
    ? currentFolder.name
    : folderId === null
      ? t("folders.unsorted")
      : kind
        ? tNav(`kind.${kind}`)
        : t("yourFiles");

  const subtitle = currentFolder
    ? t("scope.inFolder", { name: currentFolder.name })
    : folderId === null
      ? t("scope.unsorted")
      : kind
        ? t("scope.kind", { kind: tNav(`kind.${kind}`) })
        : t("yourFilesDesc");

  return (
    <div className="mx-auto flex w-full max-w-7xl flex-col gap-5">
      {/* Location */}
      <div className="flex flex-col gap-2">
        {breadcrumb.length > 0 || folderId === null ? (
          <nav
            aria-label={t("breadcrumbAria")}
            className="flex flex-wrap items-center gap-1.5 text-xs"
          >
            <Link
              href="/dashboard?all=1"
              className="text-muted-foreground hover:text-foreground transition-colors"
            >
              {t("yourFiles")}
            </Link>
            {/* Inside a folder the trail also leads back up to the explorer,
                so the two views are never a one-way trip. */}
            {breadcrumb.length > 0 ? (
              <>
                <span className="text-muted-foreground/40">/</span>
                <Link
                  href="/dashboard/folders"
                  className="text-muted-foreground hover:text-foreground transition-colors"
                >
                  {tNav("folders")}
                </Link>
              </>
            ) : null}
            {folderId === null ? (
              <>
                <span className="text-muted-foreground/40">/</span>
                <span className="text-foreground font-medium">{t("folders.unsorted")}</span>
              </>
            ) : (
              breadcrumb.map((crumb, i) => (
                <span key={crumb.id} className="flex items-center gap-1.5">
                  <span className="text-muted-foreground/40">/</span>
                  {i === breadcrumb.length - 1 ? (
                    <span className="text-foreground font-medium">{crumb.name}</span>
                  ) : (
                    <Link
                      href={`/dashboard?folder=${crumb.id}`}
                      className="text-muted-foreground hover:text-foreground transition-colors"
                    >
                      {crumb.name}
                    </Link>
                  )}
                </span>
              ))
            )}
          </nav>
        ) : null}

        <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
          <h1 className="font-heading text-2xl font-semibold tracking-tight">{title}</h1>
          <span className="bg-muted text-muted-foreground rounded-full px-2 py-0.5 text-xs font-medium tabular-nums">
            {t("countFiles", { count: result.total })}
          </span>

          {currentFolder?.isPublic ? (
            <Link
              href={`/a/${userCode}/${currentFolder.slug}`}
              target="_blank"
              rel="noreferrer"
              className="bg-primary/10 text-primary ring-primary/20 hover:bg-primary/15 inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-xs font-medium ring-1 transition-colors"
            >
              <Globe className="size-3" />
              {t("folders.publicBadge")}
              <ExternalLink className="size-3 opacity-70" />
            </Link>
          ) : null}
        </div>

        <p className="text-muted-foreground text-sm">{subtitle}</p>
      </div>

      {/* Not upstream: the file password, while this browser lacks it. */}
      <VaultBanner vault={props.vault} />

      <Toolbar
        search={search ?? ""}
        sort={sort}
        view={view}
        kind={kind ?? ""}
        visibility={visibility ?? ""}
        size={rawSize ?? ""}
        after={rawAfter ?? ""}
        before={rawBefore ?? ""}
        counts={counts}
        folderActive={Boolean(folderId)}
        includeSubfolders={includeSubfolders}
      />

      {hasResults ? (
        <>
          {view === "list" ? (
            <FileList
              items={items}
              userCode={userCode}
              appUrl={linkBase}
              copyRaw={copyRaw}
              currentFolderId={folderId ?? null}
            />
          ) : (
            <FileGrid
              items={items}
              userCode={userCode}
              appUrl={linkBase}
              copyRaw={copyRaw}
              currentFolderId={folderId ?? null}
            />
          )}
          <Pagination total={result.total} page={result.page} perPage={result.perPage} />
        </>
      ) : (
        <EmptyState
          mode={filtering ? "filtered" : currentFolder ? "folder" : "empty"}
          folderName={currentFolder?.name}
          clearHref={currentFolder ? `/dashboard?folder=${currentFolder.id}` : "/dashboard?all=1"}
        />
      )}
    </div>
  );
}
