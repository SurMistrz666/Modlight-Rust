// Port of upstream app/(auth)/2fa/page.tsx.
// @upstream app/(auth)/2fa/page.tsx
//
// No page data: `next` comes from the URL and is sanitized here exactly as
// upstream does on the server (safeNext). generateMetadata:
// { title: t("auth.twoFactor.title") }.
import { useTranslations } from "next-intl";
import { AuthCard } from "@/components/auth/auth-card";
import { TwoFactorForm } from "@/components/auth/two-factor-form";
import { useNextSearchParams } from "../../_lib/search-params";

export type TwoFactorPageProps = Record<string, never>;
function safeNext(value: string | string[] | undefined): string {
  const raw = Array.isArray(value) ? value[0] : value;
  if (raw && raw.startsWith("/") && !raw.startsWith("//")) return raw;
  return "/dashboard";
}

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
