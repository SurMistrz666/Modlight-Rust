"use client";

// Port of upstream components/auth/two-factor-form.tsx. Everything is
// upstream's, line for line, except one fix marked "No answer": a check whose
// request fails outright no longer leaves the form disabled (bug hunt
// 2026-11-11, WEB-07; the four other sign-in forms got it as WEB-03).
// @upstream components/auth/two-factor-form.tsx

import { useState } from "react";
import { useRouter } from "next/navigation";
import { toast } from "sonner";
import { useTranslations } from "next-intl";
import { authClient } from "@/lib/auth-client";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Field, FormError } from "@/components/auth/form-bits";
import { SubmitButton } from "@/components/auth/submit-button";
import { authErrorMessage } from "@/components/auth/auth-error";

export function TwoFactorForm({ next }: { next: string }) {
  const t = useTranslations("auth");
  const router = useRouter();
  const [code, setCode] = useState("");
  const [trustDevice, setTrustDevice] = useState(true);
  const [useBackup, setUseBackup] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // No answer: Better Auth's client throws when fetch does (no connection, a
  // server restarting behind its proxy). Upstream awaited it without a catch,
  // so the form stayed on "Verifying" until the page was reloaded.
  const tc = useTranslations("common");

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (pending) return;
    setError(null);
    setPending(true);

    let err: unknown;
    try {
      ({ error: err } = useBackup
        ? await authClient.twoFactor.verifyBackupCode({ code: code.trim() })
        : await authClient.twoFactor.verifyTotp({ code: code.trim(), trustDevice }));
    } catch {
      // No answer
      const msg = tc("error");
      setError(msg);
      toast.error(msg);
      setPending(false);
      return;
    }

    if (err) {
      const msg = authErrorMessage(err, t("twoFactor.codeMismatch"));
      setError(msg);
      toast.error(msg);
      setPending(false);
      return;
    }

    toast.success(t("twoFactor.verified"));
    router.push(next);
    router.refresh();
  }

  return (
    <form onSubmit={onSubmit} className="space-y-4" noValidate>
      <Field
        label={useBackup ? t("twoFactor.backupCode") : t("twoFactor.authCode")}
        htmlFor="code"
      >
        <Input
          id="code"
          name="code"
          inputMode={useBackup ? "text" : "numeric"}
          autoComplete="one-time-code"
          placeholder={
            useBackup
              ? t("twoFactor.backupCodePlaceholder")
              : t("twoFactor.authCodePlaceholder")
          }
          required
          autoFocus
          disabled={pending}
          value={code}
          onChange={(e) => setCode(e.target.value)}
          aria-invalid={!!error}
          className="h-10 tracking-widest"
        />
      </Field>

      {!useBackup ? (
        <label className="text-muted-foreground flex items-center gap-2 text-sm">
          <Checkbox
            checked={trustDevice}
            onCheckedChange={(v) => setTrustDevice(v === true)}
          />
          {t("twoFactor.trustDevice")}
        </label>
      ) : null}

      <FormError message={error} />

      <SubmitButton pending={pending} pendingText={t("twoFactor.verifying")}>
        {t("twoFactor.verify")}
      </SubmitButton>

      <button
        type="button"
        onClick={() => {
          setUseBackup((v) => !v);
          setCode("");
          setError(null);
        }}
        className="text-muted-foreground hover:text-foreground w-full text-center text-sm underline-offset-4 hover:underline"
      >
        {useBackup
          ? t("twoFactor.useAuthenticator")
          : t("twoFactor.useBackupCode")}
      </button>
    </form>
  );
}
