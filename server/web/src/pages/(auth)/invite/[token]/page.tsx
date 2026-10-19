// Port of upstream app/(auth)/invite/[token]/page.tsx.
// @upstream app/(auth)/invite/[token]/page.tsx
//
// Server (Rust): previewInvite(token) (app/actions/invite.ts -> readInvite);
// the token is only READ here. generateMetadata: { title: t("auth.invite.title") }.
import Link from "next/link";
import { MailX } from "lucide-react";
import { useTranslations } from "next-intl";
import { AuthCard } from "@/components/auth/auth-card";
import { InviteForm } from "@/components/auth/invite-form";
import { config } from "@/lib/config";
import type { InvitePreview } from "@/app/actions/invite";

export interface InvitePageProps {
  /** params.token, as the server read it */
  token: string;
  /** previewInvite(token) */
  preview: InvitePreview | null;
}

/**
 * Landing page for the link an administrator mails when creating an account.
 *
 * The token is resolved before anything is rendered, so an expired or already
 * used invitation says so instead of presenting a form that cannot work. It is
 * only READ here; it is consumed when the password is submitted.
 */
export default function InvitePage({ props }: { props: InvitePageProps }) {
  const t = useTranslations("auth");
  const { token, preview } = props;

  if (!preview) {
    return (
      <AuthCard title={t("invite.expiredTitle")} description={t("invite.expiredDescription")}>
        <div className="space-y-5 text-center">
          <div className="bg-muted text-muted-foreground ring-foreground/10 mx-auto grid size-12 place-items-center rounded-2xl ring-1">
            <MailX className="size-6" />
          </div>
          <p className="text-muted-foreground text-sm leading-relaxed">
            {t("invite.expiredBody")}
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
      title={t("invite.title")}
      description={t("invite.description", { appName: config.appName })}
    >
      <InviteForm token={token} email={preview.email} />
    </AuthCard>
  );
}
