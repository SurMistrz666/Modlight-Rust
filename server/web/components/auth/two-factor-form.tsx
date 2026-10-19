"use client";

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

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (pending) return;
    setError(null);
    setPending(true);

    const { error: err } = useBackup
      ? await authClient.twoFactor.verifyBackupCode({ code: code.trim() })
      : await authClient.twoFactor.verifyTotp({ code: code.trim(), trustDevice });

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
