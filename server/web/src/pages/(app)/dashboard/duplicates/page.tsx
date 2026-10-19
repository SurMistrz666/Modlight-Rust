// Port of upstream app/(app)/dashboard/duplicates/page.tsx.
// @upstream app/(app)/dashboard/duplicates/page.tsx
//
// Server (Rust), as upstream: requireUser(); scanDuplicates(user.id).
// generateMetadata: { title: t("profile.duplicates.page.metaTitle") }.
import Link from "next/link";
import { useTranslations } from "next-intl";
import { ArrowLeft, CopyCheck } from "lucide-react";
import type { DuplicateScan } from "@/lib/data/duplicates";
import { Button } from "@/components/ui/button";
import { DuplicatesScreen } from "@/components/duplicates/DuplicatesScreen";

export interface DuplicatesPageProps {
  /** scanDuplicates(session.user.id) */
  scan: DuplicateScan;
  /** session.user.userCode ?? "" */
  userCode: string;
}

/**
 * Duplicate cleanup, on a page of its own.
 *
 * The scan runs here rather than on the client: it is three queries against one
 * account, and every one of them is cheaper to run beside the database than to
 * hand a browser a button that asks for it. Rendering it server-side also means
 * the reminder in the shell and this list are produced by the same code path
 * (`dupesCte`), so the strip cannot advertise a number the page then contradicts.
 */
export default function DuplicatesPage({ props }: { props: DuplicatesPageProps }) {
  const t = useTranslations("profile.duplicates");
  const { scan } = props;
  return (
    <div className="mx-auto flex w-full max-w-5xl flex-col gap-6">
      <div className="flex flex-col gap-3">
        <Button variant="ghost" size="sm" asChild className="text-muted-foreground -ml-2 w-fit">
          <Link href="/dashboard">
            <ArrowLeft className="size-4" /> {t("page.back")}
          </Link>
        </Button>

        <div className="flex items-center gap-3">
          <div className="bg-primary/10 text-primary grid size-10 place-items-center rounded-lg">
            <CopyCheck className="size-5" />
          </div>
          <div>
            <h1 className="font-heading text-2xl font-semibold tracking-tight">
              {t("page.title")}
            </h1>
            <p className="text-muted-foreground text-sm">{t("page.subtitle")}</p>
          </div>
        </div>
      </div>

      <DuplicatesScreen scan={scan} userCode={props.userCode} />
    </div>
  );
}
