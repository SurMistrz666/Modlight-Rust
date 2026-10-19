// Port of upstream app/(app)/dashboard/upload/page.tsx.
// @upstream app/(app)/dashboard/upload/page.tsx
//
// No page data. generateMetadata:
// { title: t("upload.page.title"), description: t("upload.page.metaDescription") }.
import { useTranslations } from "next-intl";
import { UploadCloud, Infinity as InfinityIcon, ClipboardPaste, Wifi } from "lucide-react";
import { UploadModes } from "@/components/upload/UploadModes";

export type UploadPageProps = Record<string, never>;
const HIGHLIGHTS = [
  { key: "resume", icon: Wifi },
  { key: "huge", icon: InfinityIcon },
  { key: "anywhere", icon: ClipboardPaste },
] as const;

export default function UploadPage(_: { props: UploadPageProps }) {
  const t = useTranslations("upload");

  return (
    <div className="mx-auto w-full max-w-3xl space-y-8">
      <header className="space-y-2">
        <div className="flex items-center gap-3">
          <span className="bg-primary/10 text-primary ring-primary/20 grid size-10 place-items-center rounded-xl ring-1">
            <UploadCloud className="size-5" />
          </span>
          <h1 className="text-2xl font-semibold tracking-tight">{t("page.title")}</h1>
        </div>
        <p className="text-muted-foreground max-w-prose text-sm leading-relaxed">
          {t("page.lead")}
        </p>
      </header>

      {/*
        Two uploaders, one choice, made before a file is picked.

        The standard one is a Client Component that lazy-loads the Uppy
        Dashboard with `next/dynamic({ ssr: false })` internally — the dynamic
        boundary lives inside the client module because `ssr: false` is illegal
        in a Server Component. Its engine belongs to the shell's transfers
        provider, so leaving this page does not cancel anything.

        The encrypted one cannot make that promise and does not pretend to: its
        key exists only in this tab. See components/upload/UploadModes.tsx.
      */}
      <UploadModes />

      <section className="grid gap-3 sm:grid-cols-3">
        {HIGHLIGHTS.map((item) => (
          <div key={item.key} className="bg-card ring-foreground/10 rounded-xl p-4 ring-1">
            <span className="bg-primary/10 text-primary mb-3 grid size-9 place-items-center rounded-lg">
              <item.icon className="size-4" />
            </span>
            <h2 className="text-sm font-medium">{t(`highlights.${item.key}.title`)}</h2>
            <p className="text-muted-foreground mt-1 text-xs leading-relaxed">
              {t(`highlights.${item.key}.body`)}
            </p>
          </div>
        ))}
      </section>
    </div>
  );
}
