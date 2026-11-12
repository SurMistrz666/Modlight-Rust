// Port of upstream app/(auth)/2fa/page.tsx.
// @upstream app/(auth)/2fa/page.tsx
//
// No page data: `next` comes from the URL and is sanitized here (safeNext),
// more strictly than upstream: see src/pages/_lib/safe-next.ts.
// generateMetadata: { title: t("auth.twoFactor.title") }.
import { useTranslations } from "next-intl";
import { AuthCard } from "@/components/auth/auth-card";
import { TwoFactorForm } from "@/components/auth/two-factor-form";
import { useNextSearchParams } from "../../_lib/search-params";
import { safeNext } from "../../_lib/safe-next";

export type TwoFactorPageProps = Record<string, never>;

export default function TwoFactorPage(_: { props: TwoFactorPageProps }) {
  const { next } = useNextSearchParams();
  const t = useTranslations("auth");
  return (
    <AuthCard
      title={t("twoFactor.title")}
      description={t("twoFactor.description")}
    >
      <TwoFactorForm next={safeNext(next)} />
    </AuthCard>
  );
}
