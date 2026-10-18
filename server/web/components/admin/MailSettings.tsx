"use client";

import { useState, useTransition } from "react";
import {
  Mail,
  Save,
  Send,
  Loader2,
  CheckCircle2,
  XCircle,
  ShieldCheck,
  KeyRound,
  Server,
  Undo2,
  Pencil,
  TriangleAlert,
} from "lucide-react";
import { toast } from "sonner";
import { useTranslations } from "next-intl";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  clearSmtpPassword,
  clearSmtpSettings,
  saveSmtpSettings,
  sendTestMail,
  setVerificationRequired,
  type MailTestResult,
} from "@/app/actions/mail";
import type { MailStatus, SmtpSecurity } from "@/lib/mail-settings";

const SECURITY_OPTIONS: SmtpSecurity[] = ["none", "starttls", "tls"];

interface Draft {
  host: string;
  port: string;
  security: SmtpSecurity;
  user: string;
  password: string;
  from: string;
  fromName: string;
}

function draftFrom(status: MailStatus): Draft {
  return {
    host: status.host,
    port: String(status.port),
    security: status.security,
    user: status.user,
    // Never seeded from the server: the password does not come back, by design.
    password: "",
    from: status.from,
    fromName: status.fromName,
  };
}

/**
 * SMTP configuration for operators who do not have shell access to the install.
 *
 * Two rules shape everything on this card:
 *
 *  1. **The password is write-only.** The server reports a STATE ("set",
 *     "unset", "undecryptable") and never the value, so this component has
 *     nothing to render even if it wanted to. A blank field therefore means
 *     "keep the stored one"; forgetting it is a separate button.
 *  2. **Panel or environment, never a mixture.** If SMTP_HOST is in the
 *     environment and the panel rows are empty, the whole block comes from the
 *     environment and the fields are read-only until the operator says
 *     otherwise. Editing one field into a half-panel, half-env configuration
 *     would mean sending the .env credentials to a server typed in here.
 */
export function MailSettings({
  initial,
  adminEmail,
}: {
  initial: MailStatus;
  /** Default recipient of the test message: whoever is configuring this. */
  adminEmail: string;
}) {
  const t = useTranslations("admin.mail");

  const [status, setStatus] = useState<MailStatus>(initial);
  const [draft, setDraft] = useState<Draft>(() => draftFrom(initial));
  // An environment-configured install starts locked, so nothing is changed by
  // a stray keystroke in a field that looks editable.
  const [editing, setEditing] = useState(initial.source !== "env");
  const [saving, startSaving] = useTransition();
  const [testTo, setTestTo] = useState("");
  const [testing, setTesting] = useState(false);
  const [result, setResult] = useState<MailTestResult | null>(null);

  const locked = !editing;

  function apply(next: MailStatus) {
    setStatus(next);
    setDraft(draftFrom(next));
    setEditing(next.source !== "env");
  }

  function set<K extends keyof Draft>(key: K, value: Draft[K]) {
    setDraft((d) => ({ ...d, [key]: value }));
  }

  function onSave() {
    const port = Number(draft.port);
    startSaving(async () => {
      const res = await saveSmtpSettings({
        host: draft.host,
        port,
        security: draft.security,
        user: draft.user,
        password: draft.password || undefined,
        from: draft.from,
        fromName: draft.fromName,
        // The confirmation switch is NOT sent from here. It has its own action,
        // and the value this card holds is the EFFECTIVE one, which is masked to
        // false whenever the block cannot currently send. Sending it back would
        // write "off" over a stored "on" on the first save of any new SMTP
        // configuration.
      });
      if (!res.ok) {
        toast.error(t("saveErrorTitle"), { description: t(`invalid.${res.reason}`) });
        return;
      }
      apply(res.status);
      setResult(null);
      toast.success(t("savedTitle"), { description: t("savedDesc") });
    });
  }

  function onUseEnvironment() {
    startSaving(async () => {
      apply(await clearSmtpSettings());
      setResult(null);
      toast.success(t("revertedTitle"), { description: t("revertedDesc") });
    });
  }

  function onForgetPassword() {
    startSaving(async () => {
      apply(await clearSmtpPassword());
      toast.success(t("passwordCleared"));
    });
  }

  function onToggleVerification(required: boolean) {
    startSaving(async () => {
      const next = await setVerificationRequired(required);
      setStatus(next);
      toast.success(required ? t("verificationOn") : t("verificationOff"));
    });
  }

  async function onTest() {
    setTesting(true);
    setResult(null);
    try {
      setResult(await sendTestMail(testTo.trim() || undefined));
    } catch {
      toast.error(t("testFailedTitle"));
    } finally {
      setTesting(false);
    }
  }

  return (
    <div className="flex flex-col gap-6">
      <Card>
        <CardHeader className="flex flex-col gap-3 border-b sm:flex-row sm:items-center sm:justify-between">
          <CardTitle className="flex items-center gap-2 text-sm">
            <Mail className="text-muted-foreground size-4" /> {t("title")}
          </CardTitle>
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant="outline" className="text-muted-foreground gap-1.5">
              <Server className="size-3" /> {t(`source.${status.source}`)}
            </Badge>
            {status.ready ? (
              <Badge className="gap-1.5 bg-emerald-500/15 text-emerald-600 dark:text-emerald-400">
                <CheckCircle2 className="size-3" /> {t("ready")}
              </Badge>
            ) : (
              <Badge variant="outline" className="text-muted-foreground gap-1.5">
                <XCircle className="size-3" /> {t("notReady")}
              </Badge>
            )}
          </div>
        </CardHeader>

        <CardContent className="flex flex-col gap-5 py-5">
          <p className="text-muted-foreground text-xs leading-relaxed">{t("intro")}</p>

          {status.source === "env" && locked ? (
            <div className="ring-foreground/10 flex flex-col gap-2 rounded-lg p-3 ring-1 sm:flex-row sm:items-center sm:justify-between">
              <p className="text-muted-foreground text-xs leading-relaxed">{t("fromEnv")}</p>
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="shrink-0"
                onClick={() => setEditing(true)}
              >
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

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="flex flex-col gap-2 sm:col-span-2">
              <Label htmlFor="smtp-host" className="text-xs">
                {t("host")}
              </Label>
              <Input
                id="smtp-host"
                value={draft.host}
                disabled={locked}
                spellCheck={false}
                autoComplete="off"
                placeholder={t("hostPlaceholder")}
                onChange={(e) => set("host", e.target.value)}
              />
            </div>

            <div className="flex flex-col gap-2">
              <Label htmlFor="smtp-port" className="text-xs">
                {t("port")}
              </Label>
              <Input
                id="smtp-port"
                inputMode="numeric"
                value={draft.port}
                disabled={locked}
                className="tabular-nums"
                onChange={(e) => set("port", e.target.value.replace(/[^0-9]/g, ""))}
              />
            </div>

            <div className="flex flex-col gap-2">
              <Label htmlFor="smtp-security" className="text-xs">
                {t("security")}
              </Label>
              <Select
                value={draft.security}
                disabled={locked}
                onValueChange={(v) => set("security", v as SmtpSecurity)}
              >
                <SelectTrigger id="smtp-security" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {SECURITY_OPTIONS.map((option) => (
                    <SelectItem key={option} value={option}>
                      {t(`securityOption.${option}`)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="flex flex-col gap-2">
              <Label htmlFor="smtp-user" className="text-xs">
                {t("user")}
              </Label>
              <Input
                id="smtp-user"
                value={draft.user}
                disabled={locked}
                spellCheck={false}
                autoComplete="off"
                placeholder={t("userPlaceholder")}
                onChange={(e) => set("user", e.target.value)}
              />
            </div>

            <div className="flex flex-col gap-2">
              <Label htmlFor="smtp-password" className="text-xs">
                {t("password")}
              </Label>
              <Input
                id="smtp-password"
                type="password"
                value={draft.password}
                disabled={locked}
                autoComplete="new-password"
                placeholder={
                  status.passwordState === "set"
                    ? t("passwordKeep")
                    : status.passwordState === "undecryptable"
                      ? t("passwordUnreadablePlaceholder")
                      : t("passwordEmpty")
                }
                onChange={(e) => set("password", e.target.value)}
              />
            </div>

            <div className="flex flex-col gap-2">
              <Label htmlFor="smtp-from" className="text-xs">
                {t("from")}
              </Label>
              <Input
                id="smtp-from"
                type="email"
                value={draft.from}
                disabled={locked}
                spellCheck={false}
                autoComplete="off"
                placeholder={t("fromPlaceholder")}
                onChange={(e) => set("from", e.target.value)}
              />
            </div>

            <div className="flex flex-col gap-2">
              <Label htmlFor="smtp-from-name" className="text-xs">
                {t("fromName")}
              </Label>
              <Input
                id="smtp-from-name"
                value={draft.fromName}
                disabled={locked}
                autoComplete="off"
                onChange={(e) => set("fromName", e.target.value)}
              />
            </div>
          </div>

          <p className="text-muted-foreground flex items-start gap-1.5 text-xs leading-relaxed">
            <KeyRound className="mt-0.5 size-3.5 shrink-0" />
            {status.passwordState === "undecryptable"
              ? t("passwordUnreadable")
              : status.passwordState === "set"
                ? t("passwordSet")
                : t("passwordUnset")}
          </p>

          <div className="flex flex-wrap items-center justify-end gap-2">
            {status.source === "panel" && status.passwordState !== "unset" ? (
              <Button
                type="button"
                variant="ghost"
                disabled={saving}
                onClick={onForgetPassword}
              >
                {t("forgetPassword")}
              </Button>
            ) : null}
            <Button type="button" disabled={saving || locked} onClick={onSave}>
              {saving ? <Loader2 className="size-4 animate-spin" /> : <Save className="size-4" />}
              {t("save")}
            </Button>
          </div>
        </CardContent>
      </Card>

      {/* The test: a real message, because the failure that matters most (the
          server refusing our From address) only happens once one is on the wire. */}
      <Card>
        <CardHeader className="border-b">
          <CardTitle className="flex items-center gap-2 text-sm">
            <Send className="text-muted-foreground size-4" /> {t("testTitle")}
          </CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-4 py-5">
          <p className="text-muted-foreground text-xs leading-relaxed">{t("testDesc")}</p>

          <div className="flex flex-col gap-2 sm:flex-row sm:items-end">
            <div className="flex min-w-0 flex-1 flex-col gap-2">
              <Label htmlFor="smtp-test-to" className="text-xs">
                {t("testRecipient")}
              </Label>
              <Input
                id="smtp-test-to"
                type="email"
                value={testTo}
                spellCheck={false}
                autoComplete="off"
                placeholder={adminEmail}
                onChange={(e) => setTestTo(e.target.value)}
              />
            </div>
            <Button
              type="button"
              variant="outline"
              className="shrink-0"
              disabled={testing || !status.ready}
              onClick={onTest}
            >
              {testing ? <Loader2 className="size-4 animate-spin" /> : <Send className="size-4" />}
              {testing ? t("testSending") : t("testSend")}
            </Button>
          </div>

          {!status.ready ? (
            <p className="text-muted-foreground text-xs leading-relaxed">{t("testUnavailable")}</p>
          ) : null}

          {result ? (
            <div
              className={cn(
                "flex flex-col gap-2 rounded-lg border p-3 text-xs leading-relaxed",
                result.ok
                  ? "border-emerald-500/30 bg-emerald-500/5"
                  : "border-destructive/30 bg-destructive/5",
              )}
            >
              <p className="text-foreground flex items-center gap-2 font-medium">
                {result.ok ? (
                  <CheckCircle2 className="size-4 text-emerald-500" />
                ) : (
                  <XCircle className="text-destructive size-4" />
                )}
                {result.ok
                  ? t("testOk", { recipients: result.accepted.join(", ") || (testTo.trim() || adminEmail) })
                  : t(`testError.${result.kind}`)}
              </p>
              <p className="text-muted-foreground tabular-nums">
                {t("testTook", { ms: result.tookMs })}
                {!result.ok && result.smtpCode ? ` · ${t("testCode", { code: result.smtpCode })}` : ""}
              </p>
              {/* The server's own sentence is the whole diagnostic value of this
                  button; it arrives already stripped of anything secret. */}
              {!result.ok && result.serverSays ? (
                <pre className="bg-background/60 text-muted-foreground max-h-40 overflow-auto rounded-md p-2 font-mono text-[11px] whitespace-pre-wrap">
                  {result.serverSays}
                </pre>
              ) : null}
            </div>
          ) : null}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="border-b">
          <CardTitle className="flex items-center gap-2 text-sm">
            <ShieldCheck className="text-muted-foreground size-4" /> {t("verificationTitle")}
          </CardTitle>
        </CardHeader>
        <CardContent className="flex items-start gap-4 py-5">
          <div className="min-w-0 flex-1">
            <Label htmlFor="set-email_verification" className="font-medium">
              {t("verificationLabel")}
            </Label>
            <p className="text-muted-foreground mt-0.5 text-xs leading-relaxed">
              {t("verificationDesc")}
            </p>
            {!status.ready ? (
              <p className="text-muted-foreground mt-1.5 flex items-start gap-1.5 text-xs leading-relaxed">
                <TriangleAlert className="mt-0.5 size-3.5 shrink-0 text-amber-500" />
                {t("verificationNeedsMail")}
              </p>
            ) : null}
          </div>
          <Switch
            id="set-email_verification"
            checked={status.verificationRequired}
            disabled={saving || !status.ready}
            onCheckedChange={onToggleVerification}
          />
        </CardContent>
      </Card>
    </div>
  );
}
