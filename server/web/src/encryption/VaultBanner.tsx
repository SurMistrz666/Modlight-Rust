/**
 * The library's banner while this browser is locked out of the account's own
 * encrypted files (docs/ENCRYPTION.md section 8). Not upstream.
 *
 * Names, sizes and folders show as always; thumbnails fall back to the type
 * mark because their bytes need the password. Unlocking reloads the page, so
 * every tile asks for its picture again.
 */
import * as React from "react";
import { useTranslations } from "next-intl";
import { Loader2, LockKeyhole, LockKeyholeOpen } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { unlockMyFiles } from "./actions";

export interface LibraryVault {
  state: "encrypting" | "on";
  unlocked: boolean;
}

export function VaultBanner({ vault }: { vault?: LibraryVault | null }) {
  const t = useTranslations("encryption");
  const [password, setPassword] = React.useState("");
  const [remember, setRemember] = React.useState(true);
  const [pending, setPending] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  if (!vault || vault.unlocked) return null;

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!password || pending) return;
    setPending(true);
    setError(null);
    try {
      const res = await unlockMyFiles(password, remember);
      if (res.ok) {
        window.location.reload();
        return;
      }
      setError(t(`settings.error.${res.reason}`));
      setPassword("");
    } catch {
      setError(t("settings.error.failed"));
    }
    setPending(false);
  }

  return (
    <form
      onSubmit={submit}
      data-vault-banner
      className="border-border/80 bg-card flex flex-col gap-3 rounded-xl border p-4 sm:flex-row sm:items-center"
    >
      <div className="flex min-w-0 flex-1 items-start gap-3">
        <span className="bg-primary/10 text-primary grid size-9 shrink-0 place-items-center rounded-lg">
          <LockKeyhole className="size-4" />
        </span>
        <div className="min-w-0 space-y-0.5">
          <p className="text-sm font-medium">{t("banner.title")}</p>
          <p className="text-muted-foreground text-xs">{t("banner.text")}</p>
          {error ? (
            <p className="text-destructive text-xs" role="alert">
              {error}
            </p>
          ) : null}
        </div>
      </div>
      <div className="flex flex-col gap-2 sm:w-[22rem]">
        <div className="flex gap-2">
          <Input
            type="password"
            autoComplete="off"
            aria-label={t("gate.passwordLabel")}
            placeholder={t("gate.passwordPlaceholder")}
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            aria-invalid={!!error}
          />
          <Button type="submit" disabled={pending || !password}>
            {pending ? <Loader2 className="size-4 animate-spin" /> : <LockKeyholeOpen className="size-4" />}
            {t("banner.unlock")}
          </Button>
        </div>
        <label className="text-muted-foreground flex items-center gap-2 text-xs">
          <Checkbox checked={remember} onCheckedChange={(v) => setRemember(v === true)} />
          {t("gate.remember")}
        </label>
      </div>
    </form>
  );
}

/** Admin -> a user's files: why nothing of them opens (docs/ENCRYPTION.md). */
export function AdminVaultNotice({ encryption }: { encryption?: string | null }) {
  const t = useTranslations("encryption.admin");
  if (!encryption) return null;
  return (
    <div
      data-admin-vault-notice
      className="border-border/80 bg-card text-muted-foreground flex items-start gap-3 rounded-xl border p-4 text-sm"
    >
      <LockKeyhole className="text-primary mt-0.5 size-4 shrink-0" />
      <p>{t("notice")}</p>
    </div>
  );
}
