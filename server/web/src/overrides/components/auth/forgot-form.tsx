"use client";

// Port of upstream components/auth/forgot-form.tsx with the optional Cloudflare
// Turnstile check added (server/src/turnstile.rs; not upstream). Everything
// else is upstream's, line for line; the additions are marked "Turnstile".
// With no site key configured `captcha.enabled` is false, no widget renders,
// no header is sent and the form behaves exactly as upstream's. One fix, marked
// "No answer": a request that fails outright, or that the rate limit refused,
// is no longer confirmed as sent.
// @upstream components/auth/forgot-form.tsx

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
// Turnstile
import { captchaErrorKey, useCaptcha } from "../../../turnstile/widget";

export function ForgotForm() {
  const t = useTranslations("auth");
  const [email, setEmail] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sent, setSent] = useState(false);
  // Turnstile
  const captcha = useCaptcha();
  const tCaptcha = useTranslations("turnstile.widget");
  // No answer: Better Auth's client throws when fetch does (no connection, a
  // server restarting behind its proxy). Upstream awaited it without a catch,
  // so the form stayed disabled until a reload.
  const tc = useTranslations("common");
  function failed(msg: string) {
    setError(msg);
    toast.error(msg);
    setPending(false);
  }

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (pending) return;
    setError(null);
    // Turnstile: no token yet, nothing to send.
    if (captcha.enabled && !captcha.token) {
      setError(tCaptcha("required"));
      return;
    }
    setPending(true);

    let err: unknown;
    try {
      ({ error: err } = await authClient.requestPasswordReset({
        email: email.trim(),
        redirectTo: "/reset",
        // Turnstile: the token travels in a header; it is spent either way.
        fetchOptions: { headers: captcha.headers() },
      }));
    } catch {
      // No answer
      captcha.reset();
      failed(tc("error"));
      return;
    }
    captcha.reset();

    // Turnstile: a refused check is no confirmation (403 and 503 included).
    const captchaKey = captchaErrorKey(err);
    if (captchaKey) {
      const msg = tCaptcha(captchaKey);
      setError(msg);
      toast.error(msg);
      setPending(false);
      return;
    }

    // No answer: the rate limit (3 a minute) refuses before the address is
    // looked at, so saying so tells nobody whether the account exists, and
    // "check your inbox" would promise a link that was never sent.
    if (err && (err as { status?: number }).status === 429) {
      failed(t("login.tooManyAttempts"));
      return;
    }

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

      {captcha.widget}

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
