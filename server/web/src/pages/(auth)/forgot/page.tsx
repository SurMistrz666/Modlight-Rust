// Port of upstream app/(auth)/forgot/page.tsx.
// @upstream app/(auth)/forgot/page.tsx
//
// Server (Rust): mailReady() (lib/mail-settings.ts). generateMetadata:
// { title: t("auth.forgot.title") }.
import Link from "next/link";
import { MailX } from "lucide-react";
import { useTranslations } from "next-intl";
import { AuthCard } from "@/components/auth/auth-card";
import { ForgotForm } from "@/components/auth/forgot-form";

export interface ForgotPageProps {
  /** await mailReady() */
  mailReady: boolean;
}

export default function ForgotPage({ props }: { props: ForgotPageProps }) {
  const t = useTranslations("auth");

  /**
   * Checked BEFORE the form is rendered, not after it is submitted.
   *
   * With no SMTP server the reset request succeeded, the message went to the
   * container's console, and the visitor was told to check an inbox that would
   * never receive anything. Asking somebody to type their address in order to
   * be lied to is worse than telling them the feature is off.
   */
  if (!props.mailReady) {
    return (
      <AuthCard title={t("forgot.title")} description={t("forgot.unavailableDescription")}>
        <div className="space-y-5 text-center">
          <div className="bg-muted text-muted-foreground mx-auto grid size-12 place-items-center rounded-2xl ring-1 ring-foreground/10">
            <MailX className="size-6" />
          </div>
          <p className="text-muted-foreground text-sm leading-relaxed">
            {t("forgot.unavailableBody")}
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
    <AuthCard title={t("forgot.title")} description={t("forgot.description")}>
      <ForgotForm />
    </AuthCard>
  );
}
