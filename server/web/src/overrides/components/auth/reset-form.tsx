"use client";

// Port of upstream components/auth/reset-form.tsx. Everything is upstream's,
// line for line, except one fix marked "No answer": a new password whose
// request fails outright no longer leaves the form disabled (bug hunt
// 2026-11-11, WEB-07; the four other sign-in forms got it as WEB-03).
// @upstream components/auth/reset-form.tsx

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
  // No answer: Better Auth's client throws when fetch does (no connection, a
  // server restarting behind its proxy). Upstream awaited it without a catch,
  // so the form stayed on "Updating" until the page was reloaded.
  const tc = useTranslations("common");

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
    let err: unknown;
    try {
      ({ error: err } = await authClient.resetPassword({
        newPassword: password,
        token,
      }));
    } catch {
      // No answer
      const msg = tc("error");
      setError(msg);
      toast.error(msg);
      setPending(false);
      return;
    }

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
