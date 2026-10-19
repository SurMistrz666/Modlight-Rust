// Port of upstream app/(app)/dashboard/paste/page.tsx.
// @upstream app/(app)/dashboard/paste/page.tsx
//
// No page data (the (app) layout already required a session).
// generateMetadata: { title: t("paste.metaTitle"), description: t("paste.metaDescription") }.
import { FileCode2 } from "lucide-react";
import { useTranslations } from "next-intl";
import { PasteEditor } from "@/components/paste/PasteEditor";

export type PastePageProps = Record<string, never>;

export default function PastePage(_: { props: PastePageProps }) {
  const t = useTranslations("paste");
  return (
    <div className="mx-auto w-full max-w-3xl space-y-8">
      <header className="space-y-2">
        <div className="flex items-center gap-3">
          <span className="grid size-10 place-items-center rounded-xl bg-primary/10 text-primary ring-1 ring-primary/20">
            <FileCode2 className="size-5" />
          </span>
          <h1 className="text-2xl font-semibold tracking-tight">{t("pageTitle")}</h1>
        </div>
        <p className="max-w-prose text-sm leading-relaxed text-muted-foreground">
          {t("pageDescription")}
        </p>
      </header>

      <PasteEditor />
    </div>
  );
}
