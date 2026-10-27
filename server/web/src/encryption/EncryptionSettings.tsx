/**
 * Settings -> Security: the "File encryption" card (docs/ENCRYPTION.md section
 * 8). Not upstream.
 *
 * Off: what turning it on means, and the button. On: whether this browser is
 * unlocked (with the password field when it is not), the background pass's
 * progress, the browsers and apps that remember the password, and the three
 * things the password guards: locking this browser, changing the password,
 * switching encryption off. Turning it on needs the password twice and an
 * explicit "I understand that my files cannot be recovered without it".
 */
import * as React from "react";
import { useLocale, useTranslations } from "next-intl";
import { toast } from "sonner";
import {
  AppWindow,
  Check,
  Globe,
  KeyRound,
  Loader2,
  Lock,
  LockKeyhole,
  LockKeyholeOpen,
  Monitor,
  ShieldCheck,
  TriangleAlert,
} from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import { Progress } from "@/components/ui/progress";
import { Separator } from "@/components/ui/separator";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { relativeTime } from "@/lib/format";
import {
  changeFilePassword,
  disableEncryption,
  enableEncryption,
  loadEncryptionStatus,
  lockMyFiles,
  revokeAllVaultGrants,
  revokeVaultGrant,
  unlockMyFiles,
  type VaultGrantView,
  type VaultResult,
  type VaultStatus,
} from "./actions";

const OFF: VaultStatus = {
  state: "off",
  unlocked: false,
  progress: { done: 0, total: 0 },
  error: false,
  since: null,
  grants: [],
  minLength: 8,
};

/** How often the card asks for progress while the pass is running. */
const POLL_MS = 2000;

type Dialogs = "enable" | "change" | "disable" | null;

export function EncryptionSettings({ initial }: { initial?: VaultStatus }) {
  const t = useTranslations("encryption.settings");
  const [status, setStatus] = React.useState<VaultStatus>(initial ?? OFF);
  const [dialog, setDialog] = React.useState<Dialogs>(null);
  const running = status.state === "encrypting" || status.state === "decrypting";

  const refresh = React.useCallback(async () => {
    try {
      setStatus(await loadEncryptionStatus());
    } catch {
      /* the next poll or action tries again */
    }
  }, []);

  React.useEffect(() => {
    if (!running) return;
    const id = window.setInterval(refresh, POLL_MS);
    return () => window.clearInterval(id);
  }, [running, refresh]);

  const gated = status.state === "on" || status.state === "encrypting";

  return (
    <Card className="xl:col-span-2" data-encryption-state={status.state}>
      <CardHeader className="border-b">
        <CardTitle className="flex items-center gap-2">
          <LockKeyhole className="text-primary size-4" />
          {t("title")}
          <StateBadge status={status} />
        </CardTitle>
        <CardDescription>{t("description")}</CardDescription>
      </CardHeader>

      <CardContent className="space-y-6">
        {status.state === "off" ? (
          <OffPanel onEnable={() => setDialog("enable")} />
        ) : (
          <>
            {running ? <ProgressPanel status={status} /> : null}
            {status.state === "on" ? (
              <p className="text-muted-foreground flex items-center gap-2 text-sm">
                <ShieldCheck className="size-4 text-emerald-500" />
                {t("allEncrypted", { total: status.progress.total })}
              </p>
            ) : null}
            {status.error && running ? (
              <p className="text-muted-foreground text-xs">{t("pending")}</p>
            ) : null}
            {gated ? <LockPanel status={status} onChanged={refresh} /> : null}
            {gated ? (
              <>
                <Separator />
                <GrantList status={status} onChanged={refresh} />
                <Separator />
                <div className="flex flex-wrap gap-2">
                  <Button type="button" variant="outline" size="sm" onClick={() => setDialog("change")}>
                    <KeyRound className="size-4" />
                    {t("changePassword")}
                  </Button>
                  <Button type="button" variant="outline" size="sm" onClick={() => setDialog("disable")}>
                    <LockKeyholeOpen className="size-4" />
                    {t("disable")}
                  </Button>
                </div>
              </>
            ) : null}
          </>
        )}
      </CardContent>

      <EnableDialog
        open={dialog === "enable"}
        minLength={status.minLength}
        onClose={() => setDialog(null)}
        onDone={refresh}
      />
      <ChangeDialog
        open={dialog === "change"}
        minLength={status.minLength}
        onClose={() => setDialog(null)}
        onDone={refresh}
      />
      <DisableDialog open={dialog === "disable"} onClose={() => setDialog(null)} onDone={refresh} />
    </Card>
  );
}

function StateBadge({ status }: { status: VaultStatus }) {
  const t = useTranslations("encryption.settings");
  if (status.state === "off") {
    return (
      <Badge variant="outline" className="text-muted-foreground ml-1">
        {t("state.off")}
      </Badge>
    );
  }
  if (status.state === "on") {
    return (
      <Badge variant="secondary" className="ml-1 gap-1">
        <ShieldCheck className="size-3 text-emerald-500" />
        {t("state.on")}
      </Badge>
    );
  }
  return (
    <Badge variant="secondary" className="ml-1 gap-1">
      <Loader2 className="size-3 animate-spin" />
      {t(`state.${status.state}`)}
    </Badge>
  );
}

function OffPanel({ onEnable }: { onEnable: () => void }) {
  const t = useTranslations("encryption.settings");
  return (
    <div className="space-y-4">
      <p className="text-muted-foreground text-sm">{t("offIntro")}</p>
      <ul className="space-y-2 text-sm">
        {(["uploads", "links", "admins"] as const).map((k) => (
          <li key={k} className="flex items-start gap-2">
            <Check className="mt-0.5 size-4 shrink-0 text-emerald-500" />
            <span>{t(`points.${k}`)}</span>
          </li>
        ))}
      </ul>
      <Button type="button" size="sm" onClick={onEnable}>
        <Lock className="size-4" />
        {t("enable")}
      </Button>
    </div>
  );
}

function ProgressPanel({ status }: { status: VaultStatus }) {
  const t = useTranslations("encryption.settings");
  const { done, total } = status.progress;
  const pct = total > 0 ? Math.round((done / total) * 100) : 100;
  return (
    <div className="space-y-2" data-encryption-progress={`${done}/${total}`}>
      <div className="flex items-center justify-between gap-3 text-sm">
        <span className="font-medium">
          {status.state === "decrypting"
            ? t("progressDecrypting", { done, total })
            : t("progressEncrypting", { done, total })}
        </span>
        <span className="text-muted-foreground tabular-nums">{pct}%</span>
      </div>
      <Progress value={pct} />
      <p className="text-muted-foreground text-xs">{t("progressNote")}</p>
    </div>
  );
}

function reasonText(t: ReturnType<typeof useTranslations>, res: VaultResult): string {
  if (res.ok) return "";
  return t(`error.${res.reason}`);
}

/** Whether this browser may open the files, and the field that unlocks it. */
function LockPanel({ status, onChanged }: { status: VaultStatus; onChanged: () => Promise<void> }) {
  const t = useTranslations("encryption.settings");
  const [password, setPassword] = React.useState("");
  const [remember, setRemember] = React.useState(true);
  const [pending, setPending] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  async function unlock(e: React.FormEvent) {
    e.preventDefault();
    if (!password || pending) return;
    setPending(true);
    setError(null);
    try {
      const res = await unlockMyFiles(password, remember);
      if (res.ok) {
        setPassword("");
        toast.success(t("toast.unlocked"));
        await onChanged();
      } else {
        setError(reasonText(t, res));
      }
    } catch {
      setError(t("error.failed"));
    }
    setPending(false);
  }

  async function lock() {
    setPending(true);
    try {
      await lockMyFiles();
      toast.success(t("toast.locked"));
      await onChanged();
    } catch {
      toast.error(t("error.failed"));
    }
    setPending(false);
  }

  if (status.unlocked) {
    return (
      <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-emerald-500/25 bg-emerald-500/5 px-3 py-2.5">
        <span className="flex items-center gap-2 text-sm">
          <LockKeyholeOpen className="size-4 text-emerald-500" />
          {t("unlockedHere")}
        </span>
        <Button type="button" variant="ghost" size="sm" disabled={pending} onClick={lock}>
          <Lock className="size-4" />
          {t("lockNow")}
        </Button>
      </div>
    );
  }

  return (
    <form onSubmit={unlock} className="space-y-3 rounded-lg border border-amber-500/30 bg-amber-500/5 p-3">
      <div className="space-y-1">
        <p className="flex items-center gap-2 text-sm font-medium">
          <Lock className="size-4 text-amber-500" />
          {t("lockedHere")}
        </p>
        <p className="text-muted-foreground text-xs">{t("lockedHint")}</p>
      </div>
      <div className="flex flex-col gap-2 sm:flex-row">
        <Input
          type="password"
          autoComplete="off"
          aria-label={t("password")}
          placeholder={t("password")}
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          aria-invalid={!!error}
        />
        <Button type="submit" size="sm" className="sm:h-9" disabled={pending || !password}>
          {pending ? <Loader2 className="size-4 animate-spin" /> : <LockKeyholeOpen className="size-4" />}
          {pending ? t("unlocking") : t("unlock")}
        </Button>
      </div>
      <label className="flex items-center gap-2 text-xs">
        <Checkbox checked={remember} onCheckedChange={(v) => setRemember(v === true)} />
        {/* Same promise as the unlock page. */}
        <RememberLabel />
      </label>
      {error ? (
        <p className="text-destructive text-sm" role="alert">
          {error}
        </p>
      ) : null}
    </form>
  );
}

function RememberLabel() {
  const t = useTranslations("encryption.gate");
  return <span>{t("remember")}</span>;
}

function KindIcon({ grant }: { grant: VaultGrantView }) {
  const cls = "text-muted-foreground size-4 shrink-0";
  if (grant.kind === "app") return <AppWindow className={cls} />;
  if (grant.kind === "visitor") return <Globe className={cls} />;
  return <Monitor className={cls} />;
}

function GrantList({ status, onChanged }: { status: VaultStatus; onChanged: () => Promise<void> }) {
  const t = useTranslations("encryption.settings");
  const locale = useLocale();
  const [busy, setBusy] = React.useState<string | null>(null);

  async function revoke(id: string) {
    setBusy(id);
    try {
      const res = await revokeVaultGrant(id);
      if (res.ok) toast.success(t("toast.revoked"));
      else toast.error(reasonText(t, res));
      await onChanged();
    } catch {
      toast.error(t("error.failed"));
    }
    setBusy(null);
  }

  async function revokeAll() {
    setBusy("all");
    try {
      await revokeAllVaultGrants();
      toast.success(t("toast.revoked"));
      await onChanged();
    } catch {
      toast.error(t("error.failed"));
    }
    setBusy(null);
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-3">
        <h3 className="text-sm font-medium">{t("grantsTitle")}</h3>
        {status.grants.length > 1 ? (
          <Button type="button" variant="ghost" size="sm" disabled={busy !== null} onClick={revokeAll}>
            {t("revokeAll")}
          </Button>
        ) : null}
      </div>
      {status.grants.length === 0 ? (
        <p className="text-muted-foreground text-sm">{t("grantsEmpty")}</p>
      ) : (
        <ul className="divide-border/70 divide-y rounded-lg border">
          {status.grants.map((g) => (
            <li key={g.id} className="flex items-center gap-3 px-3 py-2.5" data-grant-kind={g.kind}>
              <KindIcon grant={g} />
              <div className="min-w-0 flex-1">
                <p className="flex flex-wrap items-center gap-1.5 text-sm">
                  <span className="truncate font-medium">{g.label || t("unknownDevice")}</span>
                  <Badge variant="outline" className="text-muted-foreground h-5 px-1.5 text-[0.65rem]">
                    {t(`kind.${g.kind}`)}
                  </Badge>
                  {g.current ? (
                    <Badge variant="secondary" className="h-5 px-1.5 text-[0.65rem]">
                      {t("thisBrowser")}
                    </Badge>
                  ) : null}
                </p>
                <p className="text-muted-foreground truncate text-xs">
                  {[
                    t("created", { date: relativeTime(g.createdAt, locale) }),
                    g.lastUsedAt ? t("lastUsed", { date: relativeTime(g.lastUsedAt, locale) }) : null,
                    t("expires", { date: relativeTime(g.expiresAt, locale) }),
                    g.ip,
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </p>
              </div>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={busy !== null}
                onClick={() => revoke(g.id)}
              >
                {busy === g.id ? <Loader2 className="size-4 animate-spin" /> : null}
                {t("revoke")}
              </Button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function PasswordField({
  id,
  label,
  value,
  onChange,
  help,
  autoFocus,
}: {
  id: string;
  label: string;
  value: string;
  onChange: (v: string) => void;
  help?: string;
  autoFocus?: boolean;
}) {
  return (
    <div className="space-y-2">
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        type="password"
        autoComplete="new-password"
        autoFocus={autoFocus}
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
      {help ? <p className="text-muted-foreground text-xs">{help}</p> : null}
    </div>
  );
}

function EnableDialog({
  open,
  minLength,
  onClose,
  onDone,
}: {
  open: boolean;
  minLength: number;
  onClose: () => void;
  onDone: () => Promise<void>;
}) {
  const t = useTranslations("encryption.settings");
  const [password, setPassword] = React.useState("");
  const [confirm, setConfirm] = React.useState("");
  const [ack, setAck] = React.useState(false);
  const [pending, setPending] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  function reset() {
    setPassword("");
    setConfirm("");
    setAck(false);
    setError(null);
  }

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (pending) return;
    if (password !== confirm) {
      setError(t("error.mismatch"));
      return;
    }
    setPending(true);
    setError(null);
    try {
      const res = await enableEncryption(password, confirm, ack);
      if (res.ok) {
        toast.success(t("toast.enabled"));
        reset();
        onClose();
        await onDone();
      } else {
        setError(reasonText(t, res));
      }
    } catch {
      setError(t("error.failed"));
    }
    setPending(false);
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (!o && !pending) {
          reset();
          onClose();
        }
      }}
    >
      <DialogContent className="sm:max-w-lg">
        <form onSubmit={submit} className="space-y-5">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <LockKeyhole className="size-4" />
              {t("enableTitle")}
            </DialogTitle>
            <DialogDescription>{t("enableIntro")}</DialogDescription>
          </DialogHeader>
          <PasswordField
            id="ml-vault-new"
            label={t("password")}
            value={password}
            onChange={setPassword}
            help={t("passwordHelp", { min: minLength })}
            autoFocus
          />
          <PasswordField id="ml-vault-confirm" label={t("confirm")} value={confirm} onChange={setConfirm} />
          <Alert variant="destructive" className="border-destructive/40">
            <TriangleAlert className="size-4" />
            <AlertTitle>{t("warningTitle")}</AlertTitle>
            <AlertDescription>{t("warning")}</AlertDescription>
          </Alert>
          <label className="flex items-start gap-2.5 text-sm">
            <Checkbox checked={ack} onCheckedChange={(v) => setAck(v === true)} className="mt-0.5" />
            <span>{t("acknowledge")}</span>
          </label>
          {error ? (
            <p className="text-destructive text-sm" role="alert">
              {error}
            </p>
          ) : null}
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              disabled={pending}
              onClick={() => {
                reset();
                onClose();
              }}
            >
              {t("cancel")}
            </Button>
            <Button type="submit" disabled={pending || !password || !confirm || !ack}>
              {pending ? <Loader2 className="size-4 animate-spin" /> : <Lock className="size-4" />}
              {t("turnOn")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function ChangeDialog({
  open,
  minLength,
  onClose,
  onDone,
}: {
  open: boolean;
  minLength: number;
  onClose: () => void;
  onDone: () => Promise<void>;
}) {
  const t = useTranslations("encryption.settings");
  const [current, setCurrent] = React.useState("");
  const [next, setNext] = React.useState("");
  const [confirm, setConfirm] = React.useState("");
  const [pending, setPending] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  function close() {
    setCurrent("");
    setNext("");
    setConfirm("");
    setError(null);
    onClose();
  }

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (pending) return;
    if (next !== confirm) {
      setError(t("error.mismatch"));
      return;
    }
    setPending(true);
    setError(null);
    try {
      const res = await changeFilePassword(current, next, confirm);
      if (res.ok) {
        toast.success(t("toast.changed"));
        close();
        await onDone();
      } else {
        setError(reasonText(t, res));
      }
    } catch {
      setError(t("error.failed"));
    }
    setPending(false);
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && !pending && close()}>
      <DialogContent className="sm:max-w-lg">
        <form onSubmit={submit} className="space-y-5">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <KeyRound className="size-4" />
              {t("changeTitle")}
            </DialogTitle>
            <DialogDescription>{t("changeNote")}</DialogDescription>
          </DialogHeader>
          <PasswordField id="ml-vault-current" label={t("current")} value={current} onChange={setCurrent} autoFocus />
          <PasswordField
            id="ml-vault-next"
            label={t("newPassword")}
            value={next}
            onChange={setNext}
            help={t("passwordHelp", { min: minLength })}
          />
          <PasswordField id="ml-vault-next2" label={t("confirmNew")} value={confirm} onChange={setConfirm} />
          {error ? (
            <p className="text-destructive text-sm" role="alert">
              {error}
            </p>
          ) : null}
          <DialogFooter>
            <Button type="button" variant="outline" disabled={pending} onClick={close}>
              {t("cancel")}
            </Button>
            <Button type="submit" disabled={pending || !current || !next || !confirm}>
              {pending ? <Loader2 className="size-4 animate-spin" /> : <Check className="size-4" />}
              {t("save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function DisableDialog({
  open,
  onClose,
  onDone,
}: {
  open: boolean;
  onClose: () => void;
  onDone: () => Promise<void>;
}) {
  const t = useTranslations("encryption.settings");
  const [password, setPassword] = React.useState("");
  const [pending, setPending] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  function close() {
    setPassword("");
    setError(null);
    onClose();
  }

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (pending || !password) return;
    setPending(true);
    setError(null);
    try {
      const res = await disableEncryption(password);
      if (res.ok) {
        toast.success(t("toast.disabled"));
        close();
        await onDone();
      } else {
        setError(reasonText(t, res));
      }
    } catch {
      setError(t("error.failed"));
    }
    setPending(false);
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && !pending && close()}>
      <DialogContent className="sm:max-w-md">
        <form onSubmit={submit} className="space-y-5">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <LockKeyholeOpen className="size-4" />
              {t("disableTitle")}
            </DialogTitle>
            <DialogDescription>{t("disableNote")}</DialogDescription>
          </DialogHeader>
          <div className="space-y-2">
            <Label htmlFor="ml-vault-off">{t("current")}</Label>
            <Input
              id="ml-vault-off"
              type="password"
              autoComplete="off"
              autoFocus
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              aria-invalid={!!error}
            />
          </div>
          {error ? (
            <p className="text-destructive text-sm" role="alert">
              {error}
            </p>
          ) : null}
          <DialogFooter>
            <Button type="button" variant="outline" disabled={pending} onClick={close}>
              {t("cancel")}
            </Button>
            <Button type="submit" variant="destructive" disabled={pending || !password}>
              {pending ? <Loader2 className="size-4 animate-spin" /> : <LockKeyholeOpen className="size-4" />}
              {t("disableConfirm")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
