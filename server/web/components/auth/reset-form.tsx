"use client";

import { useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { toast } from "sonner";
import { useTranslations } from "next-intl";
import { authClient } from "@/lib/auth-client";
import { Field, FormError } from "@/components/auth/form-bits";
import { PasswordInput } from "@/components/auth/password-input";
import { SubmitButton } from "@/components/auth/submit-button";
import { authErrorMessage } from "@/components/auth/auth-error";

const MIN_PASSWORD = 8;

export function ResetForm({ token }: { token: string }) {
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
    const { error: err } = await authClient.resetPassword({
      newPassword: password,
      token,
    });

    if (err) {
      const msg = authErrorMessage(err, t("reset.invalidLink"));
      setError(msg);
      toast.error(msg);
      setPending(false);
      return;
    }

    toast.success(t("reset.passwordUpdated"));
    router.push("/login");
    router.refresh();
  }

  return (
    <form onSubmit={onSubmit} className="space-y-4" noValidate>
      <Field
        label={t("reset.newPassword")}
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

      <SubmitButton pending={pending} pendingText={t("reset.updating")}>
        {t("reset.updatePassword")}
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
