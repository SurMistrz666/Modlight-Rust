// Port of upstream app/(auth)/register/page.tsx.
// @upstream app/(auth)/register/page.tsx
//
// Server (Rust): registrationPageState(). generateMetadata: { title: t("auth.signUp") }.
import Link from "next/link";
import { Lock } from "lucide-react";
import { useTranslations } from "next-intl";
import { AuthCard } from "@/components/auth/auth-card";
import { RegisterForm } from "@/components/auth/register-form";
import { config } from "@/lib/config";

export interface RegisterPageProps {
  /** registrationPageState() */
  state: { open: boolean; bootstrap: boolean; keyRequired: boolean };
}

/**
 * The page and the endpoint ask the SAME question, through the same function
 * (lib/registration.ts). They used to disagree: this page read an environment
 * variable while the panel wrote a database row, so an administrator could turn
 * registration on, see it turned on, and still be shown a closed door.
 */
export default function RegisterPage({ props }: { props: RegisterPageProps }) {
  const t = useTranslations("auth");
  const { state } = props;

  if (!state.open) {
    return (
      <AuthCard
        title={t("register.closedTitle")}
        description={t("register.closedDescription")}
      >
        <div className="space-y-5 text-center">
          <div className="bg-muted text-muted-foreground mx-auto grid size-12 place-items-center rounded-2xl ring-1 ring-foreground/10">
            <Lock className="size-6" />
          </div>
          <p className="text-muted-foreground text-sm leading-relaxed">
            {t("register.closedBody")}
          </p>
          <Link
            href="/login"
            className="text-foreground inline-block text-sm font-medium underline-offset-4 hover:underline"
          >
            {t("backToSignIn")}
          </Link>
        </div>
      </AuthCard>
    );
  }

  return (
    <AuthCard
      title={state.bootstrap ? t("register.bootstrapTitle") : t("register.title")}
      description={
        state.bootstrap
          ? t("register.bootstrapDescription", { appName: config.appName })
          : t("register.description", { appName: config.appName })
      }
      footer={
        <>
          {t("haveAccount")}{" "}
          <Link
            href="/login"
            className="text-foreground font-medium underline-offset-4 hover:underline"
          >
            {t("signIn")}
          </Link>
        </>
      }
    >
      <RegisterForm
        appName={config.appName}
        keyRequired={state.keyRequired}
        bootstrap={state.bootstrap}
      />
    </AuthCard>
  );
}
