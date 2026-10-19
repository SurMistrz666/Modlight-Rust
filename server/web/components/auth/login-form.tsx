"use client";

import { useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { toast } from "sonner";
import { MailCheck } from "lucide-react";
import { useTranslations } from "next-intl";
import { signInWithIdentifier } from "@/app/actions/login-identifier";
import { Input } from "@/components/ui/input";
import { Field, FormError } from "@/components/auth/form-bits";
import { PasswordInput } from "@/components/auth/password-input";
import { SubmitButton } from "@/components/auth/submit-button";

export function LoginForm({
  next,
  registrationEnabled,
}: {
  next: string;
  registrationEnabled: boolean;
}) {
  const t = useTranslations("auth");
  const router = useRouter();
  const [identifier, setIdentifier] = useState("");
  const [password, setPassword] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** Correct password, unconfirmed address — a state, not an error. */
  const [unverified, setUnverified] = useState(false);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (pending) return;
    setError(null);
    setUnverified(false);
    setPending(true);

    // Sign-in happens entirely on the server: it accepts a username or an
    // email, resolves the former without ever handing the address back here,
    // and returns only whether the credentials worked.
    const result = await signInWithIdentifier(identifier, password);

    // The password was right and a fresh confirmation link is already on its
    // way (see emailVerification.sendOnSignIn), so this is an instruction, not
    // a rejection — hence no red error and no error toast.
    if (result.status === "unverified") {
      setUnverified(true);
      setPending(false);
      return;
    }

    if (result.status !== "ok" && result.status !== "two-factor") {
      const msg =
        result.status === "rate-limited"
          ? t("login.tooManyAttempts")
          : t("login.invalidCredentials");
      setError(msg);
      toast.error(msg);
      setPending(false);
      return;
    }

    if (result.status === "two-factor") {
      router.push(`/2fa?next=${encodeURIComponent(next)}`);
      return;
    }

    toast.success(t("welcomeBack"));
    router.push(next);
    router.refresh();
  }

  return (
    <form onSubmit={onSubmit} className="space-y-4" noValidate>
      <Field label={t("identifier")} htmlFor="identifier">
        <Input
          id="identifier"
          name="identifier"
          type="text"
          autoComplete="username"
          placeholder={t("identifierPlaceholder")}
          required
          autoFocus
          disabled={pending}
          value={identifier}
          onChange={(e) => setIdentifier(e.target.value)}
          aria-invalid={!!error}
          className="h-10"
        />
      </Field>

      <Field
        label={t("password")}
        htmlFor="password"
        hint={
          <Link
            href="/forgot"
            className="text-muted-foreground hover:text-foreground underline-offset-4 hover:underline"
          >
            {t("forgotLink")}
          </Link>
        }
      >
        <PasswordInput
          id="password"
          name="password"
          autoComplete="current-password"
          placeholder={t("passwordPlaceholder")}
          required
          disabled={pending}
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          aria-invalid={!!error}
        />
      </Field>

      {unverified ? (
        <div className="bg-primary/10 ring-primary/20 flex gap-3 rounded-lg px-3 py-2.5 text-sm ring-1">
          <MailCheck className="text-primary mt-0.5 size-4 shrink-0" />
          <div className="space-y-0.5">
            <p className="text-foreground font-medium">{t("login.unverifiedTitle")}</p>
            <p className="text-muted-foreground leading-relaxed">
              {t("login.unverifiedBody")}
            </p>
          </div>
        </div>
      ) : null}

      <FormError message={error} />

      <SubmitButton pending={pending} pendingText={t("login.signingIn")}>
        {t("signIn")}
      </SubmitButton>

      {registrationEnabled ? (
        <p className="text-muted-foreground pt-1 text-center text-sm">
          {t("noAccount")}{" "}
          <Link
            href="/register"
            className="text-foreground font-medium underline-offset-4 hover:underline"
          >
            {t("createOne")}
          </Link>
        </p>
      ) : null}
    </form>
  );
}
