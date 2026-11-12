// Port of upstream app/(auth)/login/page.tsx.
// @upstream app/(auth)/login/page.tsx
//
// Server (Rust): registrationPageState() (only `open` is read). `next` comes
// from the URL and is sanitized here (safeNext), more strictly than upstream:
// see src/pages/_lib/safe-next.ts.
// generateMetadata: { title: t("auth.login.title") }.
import { useTranslations } from "next-intl";
import { AuthCard } from "@/components/auth/auth-card";
import { LoginForm } from "@/components/auth/login-form";
import { config } from "@/lib/config";
import { useNextSearchParams } from "../../_lib/search-params";
import { safeNext } from "../../_lib/safe-next";

export interface LoginPageProps {
  /** registrationPageState(), the field the page reads */
  registration: { open: boolean };
}

export default function LoginPage({ props }: { props: LoginPageProps }) {
  const { next } = useNextSearchParams();
  const target = safeNext(next);
  const t = useTranslations("auth");
  const { registration } = props;

  return (
    <AuthCard
      title={t("login.title")}
      description={t("login.description", { appName: config.appName })}
    >
      <LoginForm next={target} registrationEnabled={registration.open} />
    </AuthCard>
  );
}
