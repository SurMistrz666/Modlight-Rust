"use client";

import { useState } from "react";
import Link from "next/link";
import { toast } from "sonner";
import { MailCheck } from "lucide-react";
import { useTranslations } from "next-intl";
import { authClient } from "@/lib/auth-client";
import { Input } from "@/components/ui/input";
import { Field, FormError } from "@/components/auth/form-bits";
import { SubmitButton } from "@/components/auth/submit-button";
import { authErrorMessage } from "@/components/auth/auth-error";

export function ForgotForm() {
  const t = useTranslations("auth");
  const [email, setEmail] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sent, setSent] = useState(false);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (pending) return;
    setError(null);
    setPending(true);

    const { error: err } = await authClient.requestPasswordReset({
      email: email.trim(),
      redirectTo: "/reset",
    });

    // Only surface hard failures (network/validation). For a missing account we
    // still show the neutral confirmation to avoid leaking which emails exist.
    if (err && (err as { status?: number }).status === 400) {
      const msg = authErrorMessage(err, t("forgot.invalidEmail"));
      setError(msg);
      toast.error(msg);
      setPending(false);
      return;
    }

    setSent(true);
    setPending(false);
  }

  if (sent) {
    return (
      <div className="space-y-5 text-center">
        <div className="bg-primary/10 text-primary mx-auto grid size-12 place-items-center rounded-2xl ring-1 ring-primary/20">
          <MailCheck className="size-6" />
        </div>
        <div className="space-y-1.5">
          <p className="font-medium">{t("checkInbox")}</p>
          <p className="text-muted-foreground text-sm leading-relaxed">
            {t.rich("forgot.sentBody", {
              email: () => (
                <span className="text-foreground font-medium">
                  {email.trim()}
                </span>
              ),
            })}
          </p>
        </div>
        <Link
          href="/login"
          className="text-foreground inline-block text-sm font-medium underline-offset-4 hover:underline"
        >
          {t("backToSignIn")}
        </Link>
      </div>
    );
  }

  return (
    <form onSubmit={onSubmit} className="space-y-4" noValidate>
      <Field label={t("email")} htmlFor="email">
        <Input
          id="email"
          name="email"
          type="email"
          inputMode="email"
          autoComplete="email"
          placeholder={t("emailPlaceholder")}
          required
          autoFocus
          disabled={pending}
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          aria-invalid={!!error}
          className="h-10"
        />
      </Field>

      <FormError message={error} />

      <SubmitButton pending={pending} pendingText={t("forgot.sendingLink")}>
        {t("forgot.sendResetLink")}
      </SubmitButton>

      <p className="text-muted-foreground pt-1 text-center text-sm">
        {t("forgot.rememberedIt")}{" "}
        <Link
          href="/login"
          className="text-foreground font-medium underline-offset-4 hover:underline"
        >
          {t("signIn")}
        </Link>
      </p>
    </form>
  );
}
