"use client";

import * as React from "react";
import { useRouter } from "next/navigation";
import { useTranslations } from "next-intl";
import { Lock, Loader2, KeyRound } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { verifyMediaPassword } from "@/app/actions/unlock";

interface PasswordGateProps {
  mediaId: string;
  appName: string;
  /**
   * This file carries the owner's current default password, so remembering the
   * unlock can cover their other files too. False = the password belongs to
   * this file alone and the promise shrinks to match.
   */
  onDefaultPassword: boolean;
}

/** Password unlock form shown in place of protected media. */
export function PasswordGate({ mediaId, appName, onDefaultPassword }: PasswordGateProps) {
  const router = useRouter();
  const t = useTranslations("viewer");
  const [password, setPassword] = React.useState("");
  const [remember, setRemember] = React.useState(false);
  const [pending, setPending] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (pending || !password) return;
    setPending(true);
    setError(null);
    try {
      // What `remember` is worth is re-decided on the server; this flag only
      // says the visitor asked for it.
      const res = await verifyMediaPassword(mediaId, password, remember);
      if (res.ok) {
        router.refresh();
      } else {
        setError(t("password.incorrect"));
        setPassword("");
        setPending(false);
      }
    } catch {
      setError(t("password.error"));
      setPending(false);
    }
  }

  return (
    <div className="mx-auto w-full max-w-md">
      <div className="overflow-hidden rounded-2xl border border-border/80 bg-card/60 shadow-2xl shadow-black/20 backdrop-blur supports-[backdrop-filter]:bg-card/50">
        <div className="flex flex-col items-center gap-4 border-b border-border/70 bg-[color-mix(in_oklch,var(--background)_70%,#000_8%)] px-6 py-10 text-center">
          <div className="relative flex size-16 items-center justify-center rounded-2xl border border-border/70 bg-gradient-to-br from-primary/20 to-primary/5">
            <Lock className="size-7 text-primary" />
            <span className="absolute inset-0 -z-10 rounded-2xl bg-primary/20 blur-xl" />
          </div>
          <div className="space-y-1">
            <h1 className="text-lg font-semibold tracking-tight">{t("password.protectedFile")}</h1>
            <p className="text-sm text-muted-foreground">
              {t("password.prompt")}
            </p>
          </div>
          {/* The filename is deliberately NOT shown. A name like
              "raport-finansowy-2026.xlsx" can reveal more than the contents,
              and this page is reachable by anyone holding the link. */}
        </div>

        <form onSubmit={onSubmit} className="flex flex-col gap-4 p-6">
          <div className="space-y-2">
            <Label htmlFor="ml-password">{t("password.passwordLabel")}</Label>
            <div className="relative">
              <KeyRound className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
              <Input
                id="ml-password"
                type="password"
                autoFocus
                autoComplete="off"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder={t("password.passwordPlaceholder")}
                aria-invalid={!!error}
                className="pl-9"
              />
            </div>
            {error ? (
              <p className="text-sm text-destructive" role="alert">
                {error}
              </p>
            ) : null}
          </div>

          {/* Remembering the unlock. The wording is not cosmetic: an owner-wide
              consent and a single-file one are genuinely different promises, and
              the visitor is the one giving it. */}
          <div className="flex items-start gap-2.5 rounded-lg border border-border/70 bg-muted/30 p-3">
            <Checkbox
              id="ml-remember"
              checked={remember}
              disabled={pending}
              onCheckedChange={(v) => setRemember(v === true)}
              className="mt-0.5"
            />
            <div className="space-y-1">
              <Label htmlFor="ml-remember" className="text-sm font-normal leading-snug">
                {onDefaultPassword ? t("password.remember30") : t("password.rememberThisFile")}
              </Label>
              <p className="text-xs text-muted-foreground">
                {onDefaultPassword
                  ? t("password.remember30Help")
                  : t("password.rememberThisFileHelp")}
              </p>
            </div>
          </div>

          <Button type="submit" disabled={pending || !password} className="w-full">
            {pending ? <Loader2 className="animate-spin" /> : <Lock />}
            {pending ? t("password.unlocking") : t("password.unlock")}
          </Button>
        </form>
      </div>

      <p className="mt-4 text-center text-xs text-muted-foreground/70">
        {t("password.protectedBy")} <span className="font-medium text-muted-foreground">{appName}</span>
      </p>
    </div>
  );
}
