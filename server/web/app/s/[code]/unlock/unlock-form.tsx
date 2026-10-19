"use client";

import { useState } from "react";
import { Lock } from "lucide-react";
import { toast } from "sonner";
import { useTranslations } from "next-intl";
import { verifyShortlinkPassword } from "@/app/actions/shortlink-unlock";
import { PasswordInput } from "@/components/auth/password-input";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";

/**
 * Client form for the short-link gate. Verifies the password via the server
 * action; on success the unlock cookie is set, so navigating to /s/{code}
 * passes the gate and redirects to the target.
 */
export function UnlockForm({ code }: { code: string }) {
  const t = useTranslations("public");
  const [password, setPassword] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (pending) return;
    const pw = password.trim();
    if (!pw) {
      setError(t("shortlink.enterPassword"));
      return;
    }
    setError(null);
    setPending(true);
    try {
      const { ok } = await verifyShortlinkPassword(code, pw);
      if (!ok) {
        setError(t("shortlink.incorrect"));
        setPending(false);
        return;
      }
      // Cookie is set; the redirect route will now forward to the target.
      window.location.assign(`/s/${code}`);
    } catch {
      toast.error(t("shortlink.error"));
      setPending(false);
    }
  }

  return (
    <form onSubmit={onSubmit} className="space-y-4" noValidate>
      <div className="space-y-2">
        <Label htmlFor="sl-password">{t("shortlink.passwordLabel")}</Label>
        <PasswordInput
          id="sl-password"
          name="password"
          autoComplete="current-password"
          placeholder="••••••••"
          required
          autoFocus
          disabled={pending}
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          aria-invalid={!!error}
        />
        {error ? (
          <p className="text-destructive text-sm" role="alert">
            {error}
          </p>
        ) : null}
      </div>

      <Button type="submit" className="w-full" disabled={pending}>
        <Lock />
        {pending ? t("shortlink.unlocking") : t("shortlink.unlockLink")}
      </Button>
    </form>
  );
}
