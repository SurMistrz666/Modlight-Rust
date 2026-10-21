/**
 * Admin -> Settings: the "Cloudflare Turnstile" card. Not upstream (see
 * widget.tsx and server/src/turnstile.rs).
 *
 * Built like the SMTP card (components/admin/MailSettings.tsx), for the same
 * reasons:
 *
 *  1. **The secret key is write-only.** The server reports a state ("set",
 *     "unset", "undecryptable") and never the value; a blank field keeps the
 *     stored one.
 *  2. **Panel or environment, never a mixture.** With TURNSTILE_* in the
 *     environment and no panel block, the fields are read-only until the
 *     operator takes the configuration over; "use the environment" deletes the
 *     panel block again.
 */
import { useState, useTransition } from "react";
import {
  CheckCircle2,
  ExternalLink,
  KeyRound,
  Loader2,
  Pencil,
  Save,
  Server,
  ShieldCheck,
  TriangleAlert,
  Undo2,
  XCircle,
} from "lucide-react";
import { toast } from "sonner";
import { useTranslations } from "next-intl";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  clearTurnstileSettings,
  saveTurnstileSettings,
  testTurnstileSecret,
  type TurnstileStatus,
  type TurnstileTestResult,
} from "./actions";

interface Draft {
  enabled: boolean;
  siteKey: string;
  secret: string;
}

function draftFrom(status: TurnstileStatus): Draft {
  // The secret is never seeded: it does not come back, by design.
  return { enabled: status.enabled, siteKey: status.siteKey, secret: "" };
}

const DASHBOARD = "https://dash.cloudflare.com/?to=/:account/turnstile";

export function TurnstileSettings({ initial }: { initial: TurnstileStatus }) {
  const t = useTranslations("turnstile.admin");
  const [status, setStatus] = useState<TurnstileStatus>(initial);
  const [draft, setDraft] = useState<Draft>(() => draftFrom(initial));
  const [editing, setEditing] = useState(initial.source !== "env");
  const [saving, startSaving] = useTransition();
  const [testing, setTesting] = useState(false);
  const [result, setResult] = useState<TurnstileTestResult | null>(null);
  const locked = !editing;
  const host = typeof window === "undefined" ? "" : window.location.hostname;

  function apply(next: TurnstileStatus) {
    setStatus(next);
    setDraft(draftFrom(next));
    setEditing(next.source !== "env");
  }

  function set<K extends keyof Draft>(key: K, value: Draft[K]) {
    setDraft((d) => ({ ...d, [key]: value }));
  }

  function onSave() {
    startSaving(async () => {
      const res = await saveTurnstileSettings({
        enabled: draft.enabled,
        siteKey: draft.siteKey.trim(),
        secret: draft.secret.trim() || undefined,
      });
      if (!res.ok) {
        toast.error(t("saveErrorTitle"), { description: t(`invalid.${res.reason}`) });
        return;
      }
      apply(res.status);
      setResult(null);
      toast.success(t("savedTitle"), { description: res.status.active ? t("savedOn") : t("savedOff") });
    });
  }

  function onUseEnvironment() {
    startSaving(async () => {
      apply(await clearTurnstileSettings());
      setResult(null);
      toast.success(t("revertedTitle"), { description: t("revertedDesc") });
    });
  }

  async function onTest() {
    setTesting(true);
    setResult(null);
    try {
      const typed = draft.secret.trim();
      setResult(await (typed ? testTurnstileSecret(typed) : testTurnstileSecret()));
    } catch {
      toast.error(t("testError.unreachable"));
    } finally {
      setTesting(false);
    }
  }

  const secretPlaceholder =
    status.source === "panel" && status.secretState === "set"
      ? t("secretKeep")
      : status.secretState === "undecryptable"
        ? t("secretUnreadablePlaceholder")
        : t("secretEmpty");
  const secretNote =
    status.source === "env"
      ? t("secretFromEnv")
      : status.secretState === "undecryptable"
        ? t("secretUnreadable")
        : status.secretState === "set"
          ? t("secretSet")
          : t("secretUnset");

  return (
    <Card>
      <CardHeader className="flex flex-col gap-3 border-b sm:flex-row sm:items-center sm:justify-between">
        <CardTitle className="flex items-center gap-2 text-sm">
          <ShieldCheck className="text-muted-foreground size-4" /> {t("title")}
        </CardTitle>
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="outline" className="text-muted-foreground gap-1.5">
            <Server className="size-3" /> {t(`source.${status.source}`)}
          </Badge>
          {status.active ? (
            <Badge className="gap-1.5 bg-emerald-500/15 text-emerald-600 dark:text-emerald-400">
              <CheckCircle2 className="size-3" /> {t("active")}
            </Badge>
          ) : (
            <Badge variant="outline" className="text-muted-foreground gap-1.5">
              <XCircle className="size-3" /> {t("inactive")}
            </Badge>
          )}
        </div>
      </CardHeader>

      <CardContent className="flex flex-col gap-5 py-5">
        <p className="text-muted-foreground text-xs leading-relaxed">{t("intro")}</p>

        {status.source === "env" && locked ? (
          <div className="ring-foreground/10 flex flex-col gap-2 rounded-lg p-3 ring-1 sm:flex-row sm:items-center sm:justify-between">
            <p className="text-muted-foreground text-xs leading-relaxed">{t("fromEnv")}</p>
            <Button type="button" variant="outline" size="sm" className="shrink-0" onClick={() => setEditing(true)}>
              <Pencil className="size-4" /> {t("configureHere")}
            </Button>
          </div>
        ) : null}

        {status.source === "panel" && status.envAvailable ? (
          <div className="ring-foreground/10 flex flex-col gap-2 rounded-lg p-3 ring-1 sm:flex-row sm:items-center sm:justify-between">
            <p className="text-muted-foreground text-xs leading-relaxed">{t("overridesEnv")}</p>
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="shrink-0"
              disabled={saving}
              onClick={onUseEnvironment}
            >
              <Undo2 className="size-4" /> {t("useEnv")}
            </Button>
          </div>
        ) : null}

        <div className={cn("flex items-start gap-4", locked && "opacity-55")}>
          <div className="min-w-0 flex-1">
            <Label htmlFor="turnstile-enabled" className="font-medium">
              {t("enabledLabel")}
            </Label>
            <p className="text-muted-foreground mt-0.5 text-xs leading-relaxed">{t("enabledDesc")}</p>
          </div>
          <Switch
            id="turnstile-enabled"
            checked={draft.enabled}
            disabled={locked || saving}
            onCheckedChange={(v) => set("enabled", v)}
          />
        </div>

        <div className="grid gap-4 sm:grid-cols-2">
          <div className="flex flex-col gap-2">
            <Label htmlFor="turnstile-site-key" className="text-xs">
              {t("siteKey")}
            </Label>
            <Input
              id="turnstile-site-key"
              value={draft.siteKey}
              disabled={locked}
              spellCheck={false}
              autoComplete="off"
              className="font-mono"
              placeholder={t("siteKeyPlaceholder")}
              onChange={(e) => set("siteKey", e.target.value)}
            />
          </div>
          <div className="flex flex-col gap-2">
            <Label htmlFor="turnstile-secret" className="text-xs">
              {t("secret")}
            </Label>
            <Input
              id="turnstile-secret"
              type="password"
              value={draft.secret}
              disabled={locked}
              autoComplete="new-password"
              spellCheck={false}
              className="font-mono"
              placeholder={secretPlaceholder}
              onChange={(e) => set("secret", e.target.value)}
            />
          </div>
        </div>

        <p className="text-muted-foreground flex items-start gap-1.5 text-xs leading-relaxed">
          <KeyRound className="mt-0.5 size-3.5 shrink-0" />
          {secretNote}
        </p>

        <div className="flex flex-wrap items-center justify-end gap-2">
          <Button type="button" disabled={saving || locked} onClick={onSave}>
            {saving ? <Loader2 className="size-4 animate-spin" /> : <Save className="size-4" />}
            {t("save")}
          </Button>
        </div>

        <Separator />

        <div className="flex flex-col gap-3">
          <div>
            <p className="text-sm font-medium">{t("testTitle")}</p>
            <p className="text-muted-foreground mt-0.5 text-xs leading-relaxed">{t("testDesc")}</p>
          </div>
          <div>
            <Button type="button" variant="outline" size="sm" disabled={testing} onClick={onTest}>
              {testing ? <Loader2 className="size-4 animate-spin" /> : <ShieldCheck className="size-4" />}
              {testing ? t("testing") : t("test")}
            </Button>
          </div>
          {result ? (
            <div
              role="status"
              className={cn(
                "flex flex-col gap-1.5 rounded-lg border p-3 text-xs leading-relaxed",
                result.ok ? "border-emerald-500/30 bg-emerald-500/5" : "border-destructive/30 bg-destructive/5",
              )}
            >
              <p className="text-foreground flex items-center gap-2 font-medium">
                {result.ok ? (
                  <CheckCircle2 className="size-4 text-emerald-500" />
                ) : (
                  <XCircle className="text-destructive size-4" />
                )}
                {result.ok ? t("testOk") : t(`testError.${result.kind}`)}
              </p>
              {result.ok && result.alwaysPasses ? (
                <p className="text-muted-foreground flex items-start gap-1.5">
                  <TriangleAlert className="mt-0.5 size-3.5 shrink-0 text-amber-500" />
                  {t("testAlwaysPasses")}
                </p>
              ) : null}
              <p className="text-muted-foreground tabular-nums">{t("testTook", { ms: result.tookMs })}</p>
            </div>
          ) : null}
        </div>

        <Separator />

        <div className="text-muted-foreground flex flex-col gap-2 text-xs leading-relaxed">
          <p className="text-foreground text-sm font-medium">{t("howToTitle")}</p>
          <p>
            {t("howTo", { host })}{" "}
            <a
              href={DASHBOARD}
              target="_blank"
              rel="noreferrer noopener"
              className="text-foreground inline-flex items-center gap-1 underline-offset-4 hover:underline"
            >
              dash.cloudflare.com <ExternalLink className="size-3" />
            </a>
          </p>
          <p>{t("testKeys")}</p>
          <p className="flex items-start gap-1.5">
            <TriangleAlert className="mt-0.5 size-3.5 shrink-0 text-amber-500" />
            {t("lockout")}
          </p>
        </div>
      </CardContent>
    </Card>
  );
}
