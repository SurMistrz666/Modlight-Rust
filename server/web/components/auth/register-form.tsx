"use client";

import { useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { toast } from "sonner";
import { MailCheck } from "lucide-react";
import { useTranslations } from "next-intl";
import { signUp, authClient } from "@/lib/auth-client";
import { resendVerification } from "@/app/actions/invite";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Field, FormError } from "@/components/auth/form-bits";
import { PasswordInput } from "@/components/auth/password-input";
import { SubmitButton } from "@/components/auth/submit-button";
import { authErrorMessage } from "@/components/auth/auth-error";

const MIN_PASSWORD = 8;

export function RegisterForm({
  appName,
  keyRequired = false,
  bootstrap = false,
}: {
  appName: string;
  /**
   * Whether this install asks for a shared registration key.
   *
   * Decided on the server and used to render the field CONDITIONALLY, never to
   * hide it with CSS: an input that exists in the DOM still announces that a key
   * exists, and still submits whatever is in it.
   */
  keyRequired?: boolean;
  /** No account exists yet: this form creates the administrator. */
  bootstrap?: boolean;
}) {
  const t = useTranslations("auth");
  const router = useRouter();
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [registrationKey, setRegistrationKey] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [verifyEmail, setVerifyEmail] = useState<string | null>(null);
  const [resending, setResending] = useState(false);

  /** Send the confirmation link again. Throttled by caller on the server. */
  async function onResend(address: string) {
    setResending(true);
    try {
      const res = await resendVerification(address);
      if (res.ok) toast.success(t("register.verifyResent"));
      else toast.error(t(`register.resendError.${res.reason}`));
    } finally {
      setResending(false);
    }
  }

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (pending) return;
    setError(null);

    if (password.length < MIN_PASSWORD) {
      setError(t("passwordTooShort", { count: MIN_PASSWORD }));
      return;
    }

    setPending(true);
    const { data, error: err } = await signUp.email({
      name: name.trim(),
      email: email.trim(),
      password,
      // Extra fields reach the endpoint handler but are dropped before the user
      // row is written, so the key is checked and never stored on the account.
      ...(keyRequired ? { registrationKey: registrationKey.trim() } : {}),
    });

    if (err) {
      const msg = authErrorMessage(err, t("register.couldNotCreate"));
      setError(msg);
      toast.error(msg);
      setPending(false);
      return;
    }

    // Determine login state robustly: prefer the session `token` returned in the
    // signUp response (synchronous, no race); if it's absent, fall back to a
    // getSession() check. Either way, avoid showing "verify email" when the user
    // was actually auto-signed-in (email verification disabled).
    let signedIn = !!(
      data &&
      typeof data === "object" &&
      "token" in data &&
      (data as { token?: unknown }).token
    );
    if (!signedIn) {
      const { data: sess } = await authClient.getSession();
      signedIn = !!sess?.session;
    }

    if (signedIn) {
      toast.success(t("register.welcome", { appName }));
      router.push("/dashboard");
      router.refresh();
      return;
    }

    // No session => email verification is required.
    setVerifyEmail(email.trim());
    setPending(false);
  }

  if (verifyEmail) {
    return (
      <div className="space-y-5 text-center">
        <div className="bg-primary/10 text-primary mx-auto grid size-12 place-items-center rounded-2xl ring-1 ring-primary/20">
          <MailCheck className="size-6" />
        </div>
        <div className="space-y-1.5">
          <p className="font-medium">{t("register.verifyTitle")}</p>
          <p className="text-muted-foreground text-sm leading-relaxed">
            {t.rich("register.verifyBody", {
              email: () => (
                <span className="text-foreground font-medium">
                  {verifyEmail}
                </span>
              ),
            })}
          </p>
        </div>
        <Button
          type="button"
          variant="outline"
          className="w-full"
          disabled={resending}
          onClick={() => onResend(verifyEmail)}
        >
          {resending ? t("register.verifyResending") : t("register.verifyResend")}
        </Button>
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
      {bootstrap ? (
        <p className="bg-primary/10 text-primary ring-primary/20 rounded-lg px-3 py-2 text-sm ring-1">
          {t("register.bootstrapNotice")}
        </p>
      ) : null}

      <Field label={t("name")} htmlFor="name">
        <Input
          id="name"
          name="name"
          autoComplete="name"
          placeholder={t("namePlaceholder")}
          required
          autoFocus
          disabled={pending}
          value={name}
          onChange={(e) => setName(e.target.value)}
          className="h-10"
        />
      </Field>

      <Field label={t("email")} htmlFor="email">
        <Input
          id="email"
          name="email"
          type="email"
          inputMode="email"
          autoComplete="email"
          placeholder={t("emailPlaceholder")}
          required
          disabled={pending}
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          aria-invalid={!!error}
          className="h-10"
        />
      </Field>

      <Field
        label={t("password")}
        htmlFor="password"
        hint={t("minLength", { count: MIN_PASSWORD })}
      >
        <PasswordInput
          id="password"
          name="password"
          autoComplete="new-password"
          placeholder={t("register.createPasswordPlaceholder")}
          required
          minLength={MIN_PASSWORD}
          disabled={pending}
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          aria-invalid={!!error}
        />
      </Field>

      {keyRequired ? (
        <Field
          label={t("register.registrationKey")}
          htmlFor="registrationKey"
          hint={t("register.registrationKeyHint")}
        >
          <Input
            id="registrationKey"
            name="registrationKey"
            autoComplete="off"
            placeholder={t("register.registrationKeyPlaceholder")}
            required
            disabled={pending}
            value={registrationKey}
            onChange={(e) => setRegistrationKey(e.target.value)}
            aria-invalid={!!error}
            className="h-10"
          />
        </Field>
      ) : null}

      <FormError message={error} />

      <SubmitButton pending={pending} pendingText={t("register.creatingAccount")}>
        {t("register.createAccount")}
      </SubmitButton>
    </form>
  );
}
