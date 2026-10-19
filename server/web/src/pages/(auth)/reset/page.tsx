// Port of upstream app/(auth)/reset/page.tsx.
// @upstream app/(auth)/reset/page.tsx
//
// No page data: `token` and `error` come from the URL (Better Auth's reset
// link callback redirects here with ?token= or ?error=INVALID_TOKEN).
// generateMetadata: { title: t("auth.reset.title") }.
import Link from "next/link";
import { ShieldAlert } from "lucide-react";
import { useTranslations } from "next-intl";
import { AuthCard } from "@/components/auth/auth-card";
import { ResetForm } from "@/components/auth/reset-form";
import { useNextSearchParams } from "../../_lib/search-params";

export type ResetPageProps = Record<string, never>;
function first(value: string | string[] | undefined): string | undefined {
  return Array.isArray(value) ? value[0] : value;
}

export default function ResetPage(_: { props: ResetPageProps }) {
  const sp = useNextSearchParams();
  const token = first(sp.token);
  const error = first(sp.error);
  const t = useTranslations("auth");

  // better-auth redirects here with ?error=INVALID_TOKEN when the link is bad.
  if (!token || error) {
    return (
      <AuthCard
        title={t("reset.expiredTitle")}
        description={t("reset.expiredDescription")}
      >
        <div className="space-y-5 text-center">
          <div className="bg-destructive/10 text-destructive mx-auto grid size-12 place-items-center rounded-2xl ring-1 ring-destructive/20">
            <ShieldAlert className="size-6" />
          </div>
          <p className="text-muted-foreground text-sm leading-relaxed">
            {t("reset.expiredBody")}
          </p>
          <Link
            href="/forgot"
            className="text-foreground inline-block text-sm font-medium underline-offset-4 hover:underline"
          >
            {t("reset.requestNewLink")}
          </Link>
        </div>
      </AuthCard>
    );
  }

  return (
    <AuthCard
      title={t("reset.title")}
      description={t("reset.description")}
    >
      <ResetForm token={token} />
    </AuthCard>
  );
}
