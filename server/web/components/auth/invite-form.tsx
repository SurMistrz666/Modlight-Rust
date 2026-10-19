"use client";

import { useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { toast } from "sonner";
import { useTranslations } from "next-intl";
import { acceptInvitation } from "@/app/actions/invite";
import { Field, FormError } from "@/components/auth/form-bits";
import { PasswordInput } from "@/components/auth/password-input";
import { SubmitButton } from "@/components/auth/submit-button";

const MIN_PASSWORD = 8;

/**
 * Choose the password an invitation was issued for.
 *
 * The account already exists and has no password at all until this form is
 * submitted, so there is nothing to guess in the meantime. On success the
 * action signs the person in itself, which is why this redirects straight to
 * the dashboard instead of to /login.
 */
export function InviteForm({ token, email }: { token: string; email: string }) {
  const t = useTranslations("auth");
  const router = useRouter();
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (pending) return;
    setError(null);

    if (password.length < MIN_PASSWORD) {
      setError(t("passwordTooShort", { count: MIN_PASSWORD }));
      return;
    }
    if (password !== confirm) {
      setError(t("reset.passwordsDontMatch"));
      return;
    }

    setPending(true);
    const res = await acceptInvitation(token, password);

    if (!res.ok) {
      // "sign-in-failed" is a special case: the password IS set, so telling the
      // person to try again from /login is the truthful instruction.
      const msg = t(`invite.error.${res.reason}`);
      setError(msg);
      toast.error(msg);
      setPending(false);
      if (res.reason === "sign-in-failed") router.push("/login");
      return;
    }

    toast.success(t("invite.done"));
    router.push("/dashboard");
    router.refresh();
  }

  return (
    <form onSubmit={onSubmit} className="space-y-4" noValidate>
      <p className="bg-muted/60 text-muted-foreground ring-foreground/10 rounded-lg px-3 py-2 text-sm ring-1">
        {t.rich("invite.forAddress", {
          email: () => <span className="text-foreground font-medium">{email}</span>,
        })}
      </p>

      <Field
        label={t("invite.choosePassword")}
        htmlFor="password"
        hint={t("minLength", { count: MIN_PASSWORD })}
      >
        <PasswordInput
          id="password"
          name="password"
          autoComplete="new-password"
          placeholder={t("reset.newPasswordPlaceholder")}
          required
          minLength={MIN_PASSWORD}
          autoFocus
          disabled={pending}
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          aria-invalid={!!error}
        />
      </Field>

      <Field label={t("reset.confirmPassword")} htmlFor="confirm">
        <PasswordInput
          id="confirm"
          name="confirm"
          autoComplete="new-password"
          placeholder={t("reset.confirmPasswordPlaceholder")}
          required
          disabled={pending}
          value={confirm}
          onChange={(e) => setConfirm(e.target.value)}
          aria-invalid={!!error}
        />
      </Field>

      <FormError message={error} />

      <SubmitButton pending={pending} pendingText={t("invite.activating")}>
        {t("invite.activate")}
      </SubmitButton>

      <p className="text-muted-foreground pt-1 text-center text-sm">
        <Link
          href="/login"
          className="text-foreground font-medium underline-offset-4 hover:underline"
        >
          {t("backToSignIn")}
        </Link>
      </p>
    </form>
  );
}
