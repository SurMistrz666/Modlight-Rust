"use client";

import { useState, useRef, useMemo, useEffect } from "react";
import { useRouter } from "next/navigation";
import {
  ShieldCheck,
  ShieldOff,
  KeyRound,
  Loader2,
  Copy,
  Check,
  Lock,
  FileLock,
  MessageSquare,
  Smartphone,
  TriangleAlert,
  Eye,
  EyeOff,
  ExternalLink,
} from "lucide-react";
import { toast } from "sonner";
import { useTranslations } from "next-intl";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  CardDescription,
  CardFooter,
} from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { Checkbox } from "@/components/ui/checkbox";
import { Alert, AlertTitle, AlertDescription } from "@/components/ui/alert";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import { authClient } from "@/lib/auth-client";
import { cn } from "@/lib/utils";
import { myDefaultFilePassword, updateDefaultFilePassword } from "@/app/actions/account";
import { makeQr, qrToSvg } from "./qr";

interface Props {
  twoFactorEnabled: boolean;
  emailVerified: boolean;
}

/** Format a base32 secret into space-separated 4-char groups for easy reading. */
function groupSecret(secret: string): string {
  return secret.replace(/\s+/g, "").replace(/(.{4})/g, "$1 ").trim();
}

/** Extract the raw secret from an otpauth:// URI. */
function parseSecret(uri: string): string {
  try {
    const u = new URL(uri);
    return u.searchParams.get("secret") ?? "";
  } catch {
    return "";
  }
}

function CopyField({ label, value, mono = true }: { label: string; value: string; mono?: boolean }) {
  const t = useTranslations("profile.security");
  const tc = useTranslations("common");
  const [copied, setCopied] = useState(false);
  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      toast.success(t("copyLabelCopied", { label }));
      setTimeout(() => setCopied(false), 1500);
    } catch {
      toast.error(t("copyError"));
    }
  }
  return (
    <div className="flex items-stretch gap-2">
      <div
        className={cn(
          "bg-muted/50 flex flex-1 items-center overflow-x-auto rounded-lg px-3 py-2 text-sm ring-1 ring-foreground/10",
          mono && "font-mono tracking-tight",
        )}
      >
        <span className="break-all">{value}</span>
      </div>
      <Button type="button" variant="outline" size="icon" onClick={copy} aria-label={`${tc("copy")} ${label}`}>
        {copied ? <Check className="size-4 text-emerald-500" /> : <Copy className="size-4" />}
      </Button>
    </div>
  );
}

function PasswordInput({
  id,
  value,
  onChange,
  placeholder,
  autoComplete,
  disabled,
}: {
  id: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  autoComplete?: string;
  disabled?: boolean;
}) {
  const t = useTranslations("profile.security");
  const [show, setShow] = useState(false);
  return (
    <div className="relative">
      <Input
        id={id}
        type={show ? "text" : "password"}
        value={value}
        placeholder={placeholder}
        autoComplete={autoComplete}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value)}
        className="pr-9"
      />
      <button
        type="button"
        onClick={() => setShow((s) => !s)}
        className="text-muted-foreground hover:text-foreground absolute inset-y-0 right-0 grid w-9 place-items-center"
        aria-label={show ? t("hidePassword") : t("showPassword")}
        tabIndex={-1}
      >
        {show ? <EyeOff className="size-4" /> : <Eye className="size-4" />}
      </button>
    </div>
  );
}

type TfaStep = "idle" | "setup" | "verify";

export function SecuritySection({ twoFactorEnabled, emailVerified }: Props) {
  const router = useRouter();
  const t = useTranslations("profile.security");
  const tc = useTranslations("common");

  // ---- Change password ----
  const [pwCurrent, setPwCurrent] = useState("");
  const [pwNew, setPwNew] = useState("");
  const [pwConfirm, setPwConfirm] = useState("");
  const [pwSaving, setPwSaving] = useState(false);

  async function changePassword(e: React.FormEvent) {
    e.preventDefault();
    if (pwNew.length < 8) {
      toast.error(t("passwordTooShort"));
      return;
    }
    if (pwNew !== pwConfirm) {
      toast.error(t("passwordMismatch"));
      return;
    }
    setPwSaving(true);
    const { error } = await authClient.changePassword({
      currentPassword: pwCurrent,
      newPassword: pwNew,
      revokeOtherSessions: true,
    });
    setPwSaving(false);
    if (error) {
      toast.error(error.message || t("passwordChangeError"));
      return;
    }
    toast.success(t("passwordChanged"));
    setPwCurrent("");
    setPwNew("");
    setPwConfirm("");
  }

  // ---- Default file password ----
  //
  // Loaded from the server rather than passed in, so this section can be added
  // to the profile without every caller having to thread the state through.
  const [dpLoaded, setDpLoaded] = useState(false);
  const [dpEnabled, setDpEnabled] = useState(false);
  const [dpPassword, setDpPassword] = useState("");
  const [dpSaving, setDpSaving] = useState(false);
  /** Files currently on the default password — the blast radius of the checkbox. */
  const [dpAffected, setDpAffected] = useState(0);
  /**
   * Defaults ON: rotating a password and leaving the files that use it on the
   * old one is almost never what someone means by "change my password". The
   * count next to it, and the confirm step, are what keep that honest.
   */
  const [dpApplyExisting, setDpApplyExisting] = useState(true);
  /** Second click required before anything is rewritten. */
  const [dpConfirm, setDpConfirm] = useState(false);
  /** Confirmation for switching the feature off across files already shared. */
  const [dpOffOpen, setDpOffOpen] = useState(false);

  useEffect(() => {
    let cancelled = false;
    myDefaultFilePassword()
      .then((s) => {
        if (cancelled) return;
        setDpEnabled(s.enabled);
        setDpAffected(s.affected);
        setDpLoaded(true);
      })
      .catch(() => {
        if (!cancelled) setDpLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  /** Does this action rewrite files already in circulation? */
  const dpRewrites = dpApplyExisting && dpAffected > 0;

  /** Turning the switch off clears the password; turning it on only reveals the
   *  field, because there is nothing to save until one is typed. */
  async function toggleDefaultPassword(next: boolean) {
    if (next) {
      setDpEnabled(true);
      return;
    }
    // Switching off while "apply to existing" is ticked strips the password
    // from files already shared, turning gated links back into open ones. That
    // needs its own confirmation: the form's confirm button SAVES a password,
    // so it cannot stand in for "clear it".
    if (dpRewrites) {
      setDpOffOpen(true);
      return;
    }
    await disableDefaultPassword(false);
  }

  async function disableDefaultPassword(alsoExisting: boolean) {
    setDpSaving(true);
    const res = await updateDefaultFilePassword(null, alsoExisting);
    setDpSaving(false);
    setDpOffOpen(false);
    if (!res.ok) {
      toast.error(t("defaultPassword.saveError"));
      return;
    }
    setDpEnabled(false);
    setDpPassword("");
    setDpConfirm(false);
    if (alsoExisting) setDpAffected(0);
    toast.success(
      res.updated > 0
        ? t("defaultPassword.clearedFiles", { count: res.updated })
        : t("defaultPassword.disabled"),
    );
  }

  async function saveDefaultPassword(e: React.FormEvent) {
    e.preventDefault();
    if (!dpPassword) return;

    if (dpRewrites && !dpConfirm) {
      setDpConfirm(true);
      return;
    }

    setDpSaving(true);
    const res = await updateDefaultFilePassword(dpPassword, dpApplyExisting);
    setDpSaving(false);
    setDpConfirm(false);
    if (!res.ok) {
      toast.error(t("defaultPassword.tooShort"));
      return;
    }
    setDpPassword("");
    setDpEnabled(true);
    toast.success(
      res.updated > 0
        ? t("defaultPassword.savedFiles", { count: res.updated })
        : t("defaultPassword.saved"),
    );
  }

  // ---- Two-factor ----
  const [step, setStep] = useState<TfaStep>("idle");
  const [tfaPassword, setTfaPassword] = useState("");
  const [tfaBusy, setTfaBusy] = useState(false);
  const [totpUri, setTotpUri] = useState("");
  const [backupCodes, setBackupCodes] = useState<string[]>([]);
  const [verifyCode, setVerifyCode] = useState("");

  // Password gate for enabling / disabling.
  const [pwDialogOpen, setPwDialogOpen] = useState(false);
  const dialogMode = useRef<"enable" | "disable">("enable");
  const [dialogPassword, setDialogPassword] = useState("");

  const secret = parseSecret(totpUri);

  // Build a scannable QR of the otpauth URI. Always dark-on-light so any
  // authenticator camera can read it regardless of the app theme.
  const qrSvg = useMemo(() => {
    if (!totpUri) return "";
    try {
      return qrToSvg(makeQr(totpUri), { fg: "#0a0a0a", bg: "#ffffff" });
    } catch {
      return "";
    }
  }, [totpUri]);

  function openGate(mode: "enable" | "disable") {
    dialogMode.current = mode;
    setDialogPassword("");
    setPwDialogOpen(true);
  }

  async function submitGate(e: React.FormEvent) {
    e.preventDefault();
    if (!dialogPassword) {
      toast.error(t("enterPasswordToContinue"));
      return;
    }
    setTfaBusy(true);
    if (dialogMode.current === "enable") {
      const { data, error } = await authClient.twoFactor.enable({ password: dialogPassword });
      setTfaBusy(false);
      if (error || !data) {
        toast.error(error?.message || t("enableError"));
        return;
      }
      setTotpUri(data.totpURI);
      setBackupCodes(data.backupCodes ?? []);
      setTfaPassword(dialogPassword);
      setStep("setup");
      setPwDialogOpen(false);
      setDialogPassword("");
    } else {
      const { error } = await authClient.twoFactor.disable({ password: dialogPassword });
      setTfaBusy(false);
      if (error) {
        toast.error(error.message || t("disableError"));
        return;
      }
      toast.success(t("twoFactorDisabled"));
      setPwDialogOpen(false);
      setDialogPassword("");
      resetTfa();
      router.refresh();
    }
  }

  function resetTfa() {
    setStep("idle");
    setTotpUri("");
    setBackupCodes([]);
    setVerifyCode("");
    setTfaPassword("");
  }

  async function verifyTotp(e: React.FormEvent) {
    e.preventDefault();
    const code = verifyCode.replace(/\s+/g, "");
    if (code.length !== 6) {
      toast.error(t("enterSixDigit"));
      return;
    }
    setTfaBusy(true);
    const { error } = await authClient.twoFactor.verifyTotp({ code });
    setTfaBusy(false);
    if (error) {
      toast.error(error.message || t("codeMismatch"));
      return;
    }
    toast.success(t("twoFactorEnabled"));
    resetTfa();
    router.refresh();
  }

  return (
    <>
      <Card>
        <CardHeader className="border-b">
          <CardTitle className="flex items-center gap-2">
            <Lock className="text-primary size-4" />
            {t("title")}
          </CardTitle>
          <CardDescription>{t("desc")}</CardDescription>
        </CardHeader>

        <CardContent className="flex-1 space-y-8">
          {/* Change password */}
          <form onSubmit={changePassword} className="space-y-4">
            <div className="flex items-center gap-2">
              <KeyRound className="text-muted-foreground size-4" />
              <h3 className="text-sm font-medium">{t("password")}</h3>
            </div>

            <div className="grid gap-4 sm:grid-cols-2">
              <div className="space-y-2 sm:col-span-2">
                <Label htmlFor="pw-current">{t("currentPassword")}</Label>
                <PasswordInput
                  id="pw-current"
                  value={pwCurrent}
                  onChange={setPwCurrent}
                  autoComplete="current-password"
                  disabled={pwSaving}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="pw-new">{t("newPassword")}</Label>
                <PasswordInput
                  id="pw-new"
                  value={pwNew}
                  onChange={setPwNew}
                  placeholder={t("newPasswordPlaceholder")}
                  autoComplete="new-password"
                  disabled={pwSaving}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="pw-confirm">{t("confirmPassword")}</Label>
                <PasswordInput
                  id="pw-confirm"
                  value={pwConfirm}
                  onChange={setPwConfirm}
                  autoComplete="new-password"
                  disabled={pwSaving}
                />
              </div>
            </div>

            <div className="flex items-center justify-between gap-3">
              <p className="text-muted-foreground text-xs">
                {t("passwordSignOutNote")}
              </p>
              <Button
                type="submit"
                size="sm"
                disabled={pwSaving || !pwCurrent || !pwNew || !pwConfirm}
              >
                {pwSaving ? <Loader2 className="size-4 animate-spin" /> : <Check className="size-4" />}
                {t("updatePassword")}
              </Button>
            </div>
          </form>

          <Separator />

          {/* Default password on new files */}
          <div className="space-y-4">
            <div className="flex items-start justify-between gap-3">
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  <FileLock className="text-muted-foreground size-4" />
                  <h3 className="text-sm font-medium">{t("defaultPassword.title")}</h3>
                  {dpEnabled && (
                    <Badge variant="secondary" className="gap-1">
                      <ShieldCheck className="size-3 text-emerald-500" />
                      {t("on")}
                    </Badge>
                  )}
                </div>
                <p className="text-muted-foreground text-sm">{t("defaultPassword.desc")}</p>
              </div>

              <Switch
                checked={dpEnabled}
                disabled={!dpLoaded || dpSaving}
                onCheckedChange={toggleDefaultPassword}
                aria-label={t("defaultPassword.title")}
              />
            </div>

            {dpEnabled && (
              <form onSubmit={saveDefaultPassword} className="space-y-4">
                <div className="grid gap-4 sm:grid-cols-2">
                  <div className="space-y-2">
                    <Label htmlFor="dp-password">
                      {t("defaultPassword.newLabel")}
                    </Label>
                    <PasswordInput
                      id="dp-password"
                      value={dpPassword}
                      onChange={(v) => {
                        setDpPassword(v);
                        setDpConfirm(false);
                      }}
                      placeholder={t("defaultPassword.placeholder")}
                      autoComplete="off"
                      disabled={dpSaving}
                    />
                  </div>
                </div>

                {/*
                  Stated up front rather than discovered later, because it is the
                  one visible cost of switching this on and it looks like a bug
                  when it happens.

                  It is also not fixable, only choosable: Discord fetches the
                  preview onto its own servers and re-hosts it publicly on
                  media.discordapp.net, so an embed of a protected file is that
                  file published without its password. Suppressing the preview IS
                  the protection working.
                */}
                <div className="border-border/70 bg-muted/30 flex items-start gap-2.5 rounded-lg border border-dashed p-3">
                  <MessageSquare className="text-muted-foreground mt-0.5 size-4 shrink-0" />
                  <div className="space-y-1">
                    <p className="text-sm font-medium">{t("defaultPassword.embedTitle")}</p>
                    <p className="text-muted-foreground text-xs leading-relaxed">
                      {t("defaultPassword.embedBody")}
                    </p>
                  </div>
                </div>

                {/* Nothing to apply to = no choice to make, so the option only
                    appears once at least one file is on the default. */}
                {dpAffected > 0 && (
                  <div className="bg-muted/40 flex items-start gap-3 rounded-lg border p-3">
                    <Checkbox
                      id="dp-apply-existing"
                      checked={dpApplyExisting}
                      disabled={dpSaving}
                      onCheckedChange={(v) => {
                        setDpApplyExisting(v === true);
                        setDpConfirm(false);
                      }}
                      className="mt-0.5"
                    />
                    <div className="space-y-1">
                      <Label htmlFor="dp-apply-existing" className="text-sm font-medium">
                        {t("defaultPassword.applyExisting", { count: dpAffected })}
                      </Label>
                      <p className="text-muted-foreground text-xs leading-relaxed">
                        {dpApplyExisting
                          ? t("defaultPassword.applyExistingOn")
                          : t("defaultPassword.applyExistingOff")}
                      </p>
                    </div>
                  </div>
                )}

                <Alert>
                  <TriangleAlert className="text-amber-500" />
                  <AlertTitle>{t("defaultPassword.warningTitle")}</AlertTitle>
                  <AlertDescription>
                    {dpRewrites
                      ? t("defaultPassword.warningRewrite", { count: dpAffected })
                      : t("defaultPassword.warningBody")}
                  </AlertDescription>
                </Alert>

                <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
                  <p className="text-muted-foreground text-xs">
                    {t("defaultPassword.appliesNote")}
                  </p>
                  <Button
                    type="submit"
                    size="sm"
                    variant={dpConfirm ? "destructive" : "default"}
                    disabled={dpSaving || !dpPassword}
                  >
                    {dpSaving ? (
                      <Loader2 className="size-4 animate-spin" />
                    ) : dpConfirm ? (
                      <TriangleAlert className="size-4" />
                    ) : (
                      <Check className="size-4" />
                    )}
                    {dpConfirm
                      ? t("defaultPassword.confirmRewrite", { count: dpAffected })
                      : t("defaultPassword.save")}
                  </Button>
                </div>
              </form>
            )}

            <Dialog open={dpOffOpen} onOpenChange={(o) => !dpSaving && setDpOffOpen(o)}>
              <DialogContent>
                <DialogHeader>
                  <DialogTitle>{t("defaultPassword.offTitle")}</DialogTitle>
                  <DialogDescription>
                    {t("defaultPassword.offBody", { count: dpAffected })}
                  </DialogDescription>
                </DialogHeader>
                <DialogFooter className="gap-2 sm:gap-2">
                  {/* Keeping the files protected is still a valid outcome: the
                      switch means "stop protecting NEW uploads", which does not
                      have to imply opening up what is already shared. */}
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={dpSaving}
                    onClick={() => disableDefaultPassword(false)}
                  >
                    {t("defaultPassword.offKeep")}
                  </Button>
                  <Button
                    variant="destructive"
                    size="sm"
                    disabled={dpSaving}
                    onClick={() => disableDefaultPassword(true)}
                  >
                    {dpSaving && <Loader2 className="size-4 animate-spin" />}
                    {t("defaultPassword.offStrip", { count: dpAffected })}
                  </Button>
                </DialogFooter>
              </DialogContent>
            </Dialog>
          </div>

          <Separator />

          {/* Two-factor */}
          <div className="space-y-4">
            <div className="flex items-start justify-between gap-3">
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  <Smartphone className="text-muted-foreground size-4" />
                  <h3 className="text-sm font-medium">{t("twoFactor")}</h3>
                  {twoFactorEnabled ? (
                    <Badge variant="secondary" className="gap-1">
                      <ShieldCheck className="size-3 text-emerald-500" />
                      {t("on")}
                    </Badge>
                  ) : (
                    <Badge variant="outline" className="text-muted-foreground">
                      {t("off")}
                    </Badge>
                  )}
                </div>
                <p className="text-muted-foreground text-sm">
                  {t("twoFactorDesc")}
                </p>
              </div>

              {step === "idle" && (
                twoFactorEnabled ? (
                  <Button
                    type="button"
                    variant="destructive"
                    size="sm"
                    onClick={() => openGate("disable")}
                  >
                    <ShieldOff className="size-4" />
                    {t("disable")}
                  </Button>
                ) : (
                  <Button type="button" size="sm" onClick={() => openGate("enable")}>
                    <ShieldCheck className="size-4" />
                    {t("enable")}
                  </Button>
                )
              )}
            </div>

            {!emailVerified && step === "idle" && !twoFactorEnabled && (
              <Alert>
                <TriangleAlert className="text-amber-500" />
                <AlertTitle>{t("verifyEmailFirst")}</AlertTitle>
                <AlertDescription>
                  {t("verifyEmailDesc")}
                </AlertDescription>
              </Alert>
            )}

            {(step === "setup" || step === "verify") && (
              <div className="bg-muted/30 space-y-5 rounded-xl border p-4">
                <div className="space-y-1.5">
                  <p className="text-sm font-medium">{t("scanTitle")}</p>
                  <p className="text-muted-foreground text-xs">
                    {t("scanDesc")}
                  </p>
                </div>

                {qrSvg && (
                  <div className="flex justify-center">
                    <div
                      className="rounded-xl bg-white p-3 shadow-sm ring-1 ring-foreground/10"
                      aria-label={t("qrLabel")}
                      role="img"
                    >
                      <img
                        src={`data:image/svg+xml;utf8,${encodeURIComponent(qrSvg)}`}
                        alt={t("qrAlt")}
                        width={196}
                        height={196}
                        className="size-44 [image-rendering:pixelated]"
                        draggable={false}
                      />
                    </div>
                  </div>
                )}

                <div className="space-y-2">
                  <Label className="text-muted-foreground text-xs uppercase tracking-wide">
                    {t("setupKey")}
                  </Label>
                  <CopyField label={t("setupKey")} value={groupSecret(secret)} />
                </div>

                <div className="space-y-2">
                  <Label className="text-muted-foreground text-xs uppercase tracking-wide">
                    {t("otpauthUri")}
                  </Label>
                  <CopyField label={t("otpauthUri")} value={totpUri} mono={false} />
                  <a
                    href={totpUri}
                    className="text-primary inline-flex items-center gap-1 text-xs hover:underline"
                  >
                    <ExternalLink className="size-3" />
                    {t("openInApp")}
                  </a>
                </div>

                {backupCodes.length > 0 && (
                  <div className="space-y-2">
                    <Label className="text-muted-foreground text-xs uppercase tracking-wide">
                      {t("backupCodes")}
                    </Label>
                    <Alert>
                      <KeyRound />
                      <AlertTitle>{t("saveBackupTitle")}</AlertTitle>
                      <AlertDescription>
                        {t("saveBackupDesc")}
                      </AlertDescription>
                    </Alert>
                    <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
                      {backupCodes.map((code) => (
                        <code
                          key={code}
                          className="bg-background rounded-md px-2 py-1.5 text-center font-mono text-xs tracking-wider ring-1 ring-foreground/10"
                        >
                          {code}
                        </code>
                      ))}
                    </div>
                    <Button
                      type="button"
                      variant="ghost"
                      size="xs"
                      onClick={() => {
                        navigator.clipboard.writeText(backupCodes.join("\n"));
                        toast.success(t("backupCodesCopied"));
                      }}
                    >
                      <Copy className="size-3" />
                      {t("copyAllCodes")}
                    </Button>
                  </div>
                )}

                <Separator />

                <form onSubmit={verifyTotp} className="space-y-3">
                  <div className="space-y-1.5">
                    <Label htmlFor="tfa-code" className="text-sm font-medium">
                      {t("enterCodeTitle")}
                    </Label>
                    <p className="text-muted-foreground text-xs">
                      {t("enterCodeDesc")}
                    </p>
                  </div>
                  <div className="flex items-center gap-2">
                    <Input
                      id="tfa-code"
                      inputMode="numeric"
                      autoComplete="one-time-code"
                      maxLength={6}
                      placeholder="000000"
                      value={verifyCode}
                      disabled={tfaBusy}
                      onChange={(e) => setVerifyCode(e.target.value.replace(/[^\d]/g, ""))}
                      className="w-32 text-center font-mono text-base tracking-[0.4em]"
                    />
                    <Button type="submit" size="sm" disabled={tfaBusy || verifyCode.length !== 6}>
                      {tfaBusy ? <Loader2 className="size-4 animate-spin" /> : <ShieldCheck className="size-4" />}
                      {t("verifyAndEnable")}
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      disabled={tfaBusy}
                      onClick={resetTfa}
                    >
                      {tc("cancel")}
                    </Button>
                  </div>
                </form>
              </div>
            )}
          </div>
        </CardContent>

        {twoFactorEnabled && step === "idle" && (
          <CardFooter>
            <p className="text-muted-foreground flex items-center gap-1.5 text-xs">
              <ShieldCheck className="size-3.5 text-emerald-500" />
              {t("protectedNote")}
            </p>
          </CardFooter>
        )}
      </Card>

      {/* Password gate dialog */}
      <Dialog
        open={pwDialogOpen}
        onOpenChange={(o) => {
          if (!tfaBusy) {
            setPwDialogOpen(o);
            if (!o) setDialogPassword("");
          }
        }}
      >
        <DialogContent>
          <form onSubmit={submitGate}>
            <DialogHeader>
              <DialogTitle>
                {dialogMode.current === "enable" ? t("confirmPasswordTitle") : t("disableTitle")}
              </DialogTitle>
              <DialogDescription>
                {dialogMode.current === "enable"
                  ? t("confirmPasswordDesc")
                  : t("disableDesc")}
              </DialogDescription>
            </DialogHeader>

            <div className="space-y-2 py-4">
              <Label htmlFor="gate-pw">{t("passwordLabel")}</Label>
              <PasswordInput
                id="gate-pw"
                value={dialogPassword}
                onChange={setDialogPassword}
                autoComplete="current-password"
                disabled={tfaBusy}
              />
            </div>

            <DialogFooter>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={tfaBusy}
                onClick={() => {
                  setPwDialogOpen(false);
                  setDialogPassword("");
                }}
              >
                {tc("cancel")}
              </Button>
              <Button
                type="submit"
                size="sm"
                variant={dialogMode.current === "disable" ? "destructive" : "default"}
                disabled={tfaBusy || !dialogPassword}
              >
                {tfaBusy ? <Loader2 className="size-4 animate-spin" /> : null}
                {dialogMode.current === "enable" ? t("continue") : t("disable2fa")}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </>
  );
}
