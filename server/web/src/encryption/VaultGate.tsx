/**
 * The unlock page of an encrypted account's link (docs/ENCRYPTION.md section
 * 4): a file page, a public gallery or an album. Not upstream; built like the
 * viewer's password gate (components/viewer/PasswordGate.tsx), whose look it
 * shares, with the one difference the owner asked for: "remember on this
 * browser for 30 days" is ticked by default.
 *
 * Nothing about the files is shown: no name, no type, no size.
 */
import * as React from "react";
import { useRouter } from "next/navigation";
import { useTranslations } from "next-intl";
import { KeyRound, Loader2, Lock, LockKeyhole } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { unlockOwnerFiles, type UnlockTarget } from "./actions";

interface VaultGateProps {
  target: UnlockTarget;
  /** "file": one file's page; "files": a gallery or an album. */
  kind: "file" | "files";
  appName: string;
}

export function VaultGate({ target, kind, appName }: VaultGateProps) {
  const router = useRouter();
  const t = useTranslations("encryption.gate");
  const [password, setPassword] = React.useState("");
  const [remember, setRemember] = React.useState(true);
  const [pending, setPending] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (pending || !password) return;
    setPending(true);
    setError(null);
    try {
      const res = await unlockOwnerFiles(target, password, remember);
      if (res.ok) {
        // The page is asked for again: with the cookie it is the file itself.
        router.refresh();
        return;
      }
      setError(
        res.reason === "rate-limited"
          ? t("rateLimited", { minutes: Math.max(1, Math.ceil((res.retryAfter ?? 60) / 60)) })
          : t("wrong"),
      );
      setPassword("");
    } catch {
      setError(t("failed"));
    }
    setPending(false);
  }

  return (
    <div className="mx-auto w-full max-w-md" data-vault-gate={kind}>
      <div className="overflow-hidden rounded-2xl border border-border/80 bg-card/60 shadow-2xl shadow-black/20 backdrop-blur supports-[backdrop-filter]:bg-card/50">
        <div className="flex flex-col items-center gap-4 border-b border-border/70 bg-[color-mix(in_oklch,var(--background)_70%,#000_8%)] px-6 py-10 text-center">
          <div className="relative flex size-16 items-center justify-center rounded-2xl border border-border/70 bg-gradient-to-br from-primary/20 to-primary/5">
            <LockKeyhole className="size-7 text-primary" />
            <span className="absolute inset-0 -z-10 rounded-2xl bg-primary/20 blur-xl" />
          </div>
          <div className="space-y-1">
            <h1 className="text-lg font-semibold tracking-tight">
              {kind === "file" ? t("fileTitle") : t("galleryTitle")}
            </h1>
            <p className="text-sm text-muted-foreground">
              {kind === "file" ? t("filePrompt") : t("galleryPrompt")}
            </p>
          </div>
        </div>

        <form onSubmit={onSubmit} className="flex flex-col gap-4 p-6">
          <div className="space-y-2">
            <Label htmlFor="ml-vault-password">{t("passwordLabel")}</Label>
            <div className="relative">
              <KeyRound className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
              <Input
                id="ml-vault-password"
                type="password"
                autoFocus
                autoComplete="off"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder={t("passwordPlaceholder")}
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

          <div className="flex items-start gap-2.5 rounded-lg border border-border/70 bg-muted/30 p-3">
            <Checkbox
              id="ml-vault-remember"
              checked={remember}
              disabled={pending}
              onCheckedChange={(v) => setRemember(v === true)}
              className="mt-0.5"
            />
            <div className="space-y-1">
              <Label htmlFor="ml-vault-remember" className="text-sm font-normal leading-snug">
                {t("remember")}
              </Label>
              <p className="text-xs text-muted-foreground">{t("rememberHelp")}</p>
            </div>
          </div>

          <Button type="submit" disabled={pending || !password} className="w-full">
            {pending ? <Loader2 className="animate-spin" /> : <Lock />}
            {pending ? t("unlocking") : t("unlock")}
          </Button>
        </form>
      </div>

      <p className="mt-4 text-center text-xs text-muted-foreground/70">
        {t("protectedBy")} <span className="font-medium text-muted-foreground">{appName}</span>
      </p>
    </div>
  );
}
