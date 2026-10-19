// Port of upstream app/(app)/dashboard/trash/page.tsx.
// @upstream app/(app)/dashboard/trash/page.tsx
//
// Server (Rust), as upstream: requireUser(); listUploads({ trashed: true,
// perPage: 120, sort: "newest" }), getUserPrefs(), trashTotalsFor() (only when
// the account has a tag); the rows become TrashItems with `daysLeft` computed
// from one server clock (trashDaysLeft(deletedAt, now)).
import Link from "next/link";
import { useTranslations } from "next-intl";
import { ArrowLeft, Trash2 } from "lucide-react";
import { TRASH_RETENTION_DAYS } from "@/lib/maintenance/trash";
import { humanFileSize } from "@/lib/format";
import { Button } from "@/components/ui/button";
import { TrashList, type TrashItem } from "@/components/dashboard/TrashList";

export interface TrashPageProps {
  /** user.userCode ?? "" */
  tag: string;
  /** userBaseUrl(prefs?.preferredDomain ?? null) */
  linkBase: string;
  /** tag ? trashTotalsFor({ userId, email }, tag) : null */
  totals: { files: number; bytes: number } | null;
  /** result.items mapped to TrashItem as upstream maps them */
  items: TrashItem[];
}

export default function TrashPage({ props }: { props: TrashPageProps }) {
  const t = useTranslations("dashboard.trash");
  const { tag, linkBase, totals, items } = props;
  return (
    <div className="mx-auto flex w-full max-w-5xl flex-col gap-6">
      <div className="flex flex-col gap-3">
        <Button variant="ghost" size="sm" asChild className="text-muted-foreground -ml-2 w-fit">
          <Link href="/dashboard">
            <ArrowLeft className="size-4" /> {t("backToFiles")}
          </Link>
        </Button>
        <div className="flex items-center gap-3">
          <div className="bg-destructive/10 text-destructive grid size-10 place-items-center rounded-lg">
            <Trash2 className="size-5" />
          </div>
          <div>
            <div className="flex flex-wrap items-center gap-2">
              <h1 className="font-heading text-2xl font-semibold tracking-tight">{t("title")}</h1>
              {/* The list below stops at 120 rows; this is the only place that
                  says how much is really in here, and it is the same number the
                  confirmation will quote back. */}
              {totals && totals.files > 0 ? (
                <span className="bg-muted text-muted-foreground rounded-full px-2 py-0.5 text-[0.6875rem] font-medium tabular-nums">
                  {t("totals", { count: totals.files, size: humanFileSize(totals.bytes) })}
                </span>
              ) : null}
            </div>
            <p className="text-muted-foreground text-sm">{t("subtitle")}</p>
          </div>
        </div>
      </div>

      <TrashList
        items={items}
        userCode={tag}
        appUrl={linkBase}
        retentionDays={TRASH_RETENTION_DAYS}
      />
    </div>
  );
}
