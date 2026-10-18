// Port of upstream app/(app)/dashboard/admin/users/[id]/files/page.tsx.
// @upstream app/(app)/dashboard/admin/users/[id]/files/page.tsx
//
// Server (Rust), as upstream: requireAdmin(); the query string parsed as
// upstream (page, per, search, sort, kind, view, trashed);
// adminUserFiles(adminId, id, {...}) - which also writes the deduplicated
// `admin.user.files_viewed` audit entry - and notFound() when it returns null.
// generateMetadata: { title: t("admin.files.pageTitle") }.
import Link from "next/link";
import { useTranslations } from "next-intl";
import { ArrowLeft, Ban, Eye, HardDrive, ShieldCheck, Trash2 } from "lucide-react";
import type { AdminFilesView } from "@/lib/data/admin-files";
import type { FileKind, SortKey } from "@/lib/data/uploads";
import { humanFileSize } from "@/lib/format";
import { isUnlimited } from "@/lib/quota";
import { Badge } from "@/components/ui/badge";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import { TRASH_RETENTION_DAYS } from "@/lib/maintenance/trash";
import { AdminFileToolbar } from "@/components/admin/AdminFileToolbar";
import { FileGrid } from "@/components/dashboard/FileGrid";
import { FileList } from "@/components/dashboard/FileList";
import { TrashList } from "@/components/dashboard/TrashList";
import { Pagination } from "@/components/dashboard/Pagination";

export interface AdminUserFilesPageProps {
  /** first(sp.search)?.trim() || undefined */
  search?: string;
  /** first(sp.sort) when in SORTS, else "newest" */
  sort: SortKey;
  /** first(sp.kind) when in FILE_KINDS */
  kind?: FileKind;
  /** first(sp.view) === "list" ? "list" : "grid" */
  view: "grid" | "list";
  /** first(sp.trashed) === "1" */
  trashed: boolean;
  /** adminUserFiles(session.user.id, id, { page, perPage, search, sort, kind, trashed }) */
  data: AdminFilesView;
}

export default function AdminUserFilesPage({ props }: { props: AdminUserFilesPageProps }) {
  const t = useTranslations("admin.files");
  const { search, sort, kind, view, trashed, data } = props;
  const { target, items } = data;
  const initials = (target.name?.trim() || target.email).slice(0, 2).toUpperCase();

  return (
    <div className="mx-auto flex w-full max-w-7xl flex-col gap-5">
      <Link
        href="/dashboard/admin"
        className="text-muted-foreground hover:text-foreground inline-flex w-fit items-center gap-1.5 text-xs transition-colors"
      >
        <ArrowLeft className="size-3.5" /> {t("backToAdmin")}
      </Link>

      {/* Who is being looked at */}
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex min-w-0 items-center gap-3">
          <Avatar className="size-10">
            {target.avatar ? <AvatarImage src={target.avatar} alt="" /> : null}
            <AvatarFallback className="text-sm">{initials}</AvatarFallback>
          </Avatar>
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <h1 className="font-heading truncate text-xl font-semibold tracking-tight">
                {target.name || target.email}
              </h1>
              {target.role === "admin" ? (
                <Badge className="gap-1 bg-violet-500/15 text-violet-500">
                  <ShieldCheck className="size-3" /> {t("roleAdmin")}
                </Badge>
              ) : null}
              {target.banned ? (
                <Badge variant="destructive" className="gap-1">
                  <Ban className="size-3" /> {t("banned")}
                </Badge>
              ) : null}
            </div>
            <p className="text-muted-foreground truncate text-sm">{target.email}</p>
          </div>
        </div>

        <div className="text-muted-foreground flex shrink-0 flex-wrap items-center gap-x-4 gap-y-1 text-xs">
          <span className="inline-flex items-center gap-1.5 tabular-nums">
            <Eye className="size-3.5" /> {t("countFiles", { count: data.liveTotal })}
          </span>
          <span className="inline-flex items-center gap-1.5 tabular-nums">
            <HardDrive className="size-3.5" />
            {humanFileSize(target.usedQuota)}
            {isUnlimited(target.maxQuota) ? " / ∞" : ` / ${humanFileSize(target.maxQuota)}`}
          </span>
          <code className="bg-muted text-foreground/80 rounded-md px-1.5 py-0.5 font-mono">
            /{target.tag}
          </code>
        </div>
      </div>

      {/* Standing notice. An administrator can already read any file; what was
          missing was saying so out loud and writing it down. */}
      <div className="ring-foreground/10 bg-muted/40 rounded-lg px-3 py-2 text-xs leading-relaxed ring-1">
        {t("banner", { name: target.name || target.email })}
      </div>

      <AdminFileToolbar
        search={search ?? ""}
        sort={sort}
        kind={kind ?? ""}
        view={view}
        trashed={trashed}
      />

      {items.length > 0 ? (
        <>
          {data.trashItems ? (
            /* The trash is a different place with different verbs. Drawing it
               with the ordinary file components gave links that 404 and a
               "move to trash" action on files already there, which only reset
               their thirty-day countdown. */
            <TrashList
              items={data.trashItems}
              userCode={target.tag}
              appUrl={target.linkBase}
              retentionDays={TRASH_RETENTION_DAYS}
            />
          ) : view === "list" ? (
            <FileList
              items={items}
              userCode={target.tag}
              appUrl={target.linkBase}
              copyRaw={target.copyRaw}
              mode="admin"
            />
          ) : (
            <FileGrid
              items={items}
              userCode={target.tag}
              appUrl={target.linkBase}
              copyRaw={target.copyRaw}
              mode="admin"
            />
          )}
          <Pagination total={data.total} page={data.page} perPage={data.perPage} />
        </>
      ) : (
        <div className="text-muted-foreground ring-foreground/10 flex flex-col items-center gap-2 rounded-xl px-6 py-16 text-center ring-1">
          <span className="bg-muted grid size-11 place-items-center rounded-full">
            <Trash2 className="size-5" />
          </span>
          <p className="text-foreground text-sm font-medium">
            {trashed ? t("emptyTrash") : search || kind ? t("emptyFiltered") : t("empty")}
          </p>
          <p className="text-xs">{t("emptyHint")}</p>
        </div>
      )}
    </div>
  );
}
