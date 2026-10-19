"use client";

import { useMemo, useState, useTransition } from "react";
import {
  UserPlus,
  HardDrive,
  Save,
  RotateCcw,
  KeyRound,
  Eye,
  Copy,
  Loader2,
  TriangleAlert,
  Undo2,
} from "lucide-react";
import { toast } from "sonner";
import { useTranslations } from "next-intl";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Separator } from "@/components/ui/separator";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  revealRegistrationKey,
  saveRegistrationKey,
  setRegistrationEnabled,
} from "@/app/actions/registration";
import type { RegistrationKeyState } from "@/lib/registration";

export interface RegistrationPanelState {
  enabled: boolean;
  keyState: RegistrationKeyState;
}

/**
 * The two registration decisions, in the order they are made.
 *
 *   1. May a stranger sign themselves up at all?
 *   2. If so, must they also quote a shared key?
 *
 * They are deliberately not merged into one control. The switch is the
 * emergency stop ("close the door now"); the key only narrows who comes through
 * an open door. Neither has anything to say about accounts an administrator
 * creates from the Users tab, which is what the copy here has to make obvious:
 * the whole reason this screen was rebuilt is that turning sign-up off used to
 * stop the administrator too.
 */
export function SettingsForm({
  registration,
  defaultQuota,
}: {
  registration: RegistrationPanelState;
  /** Humanised `DEFAULT_USER_QUOTA`, or null when new accounts are unlimited. */
  defaultQuota: string | null;
}) {
  const t = useTranslations("admin.settings");
  const tc = useTranslations("common");

  const [enabled, setEnabled] = useState(registration.enabled);
  const [savedEnabled, setSavedEnabled] = useState(registration.enabled);
  const [keyState, setKeyState] = useState<RegistrationKeyState>(registration.keyState);

  // Blank means "leave the stored key alone", exactly like the SMTP password:
  // an empty field is how a browser renders "we are not showing you this", so
  // treating it as "delete the key" would drop it every time somebody flipped
  // the switch. Removal is its own, explicit act.
  const [draftKey, setDraftKey] = useState("");
  const [removeKey, setRemoveKey] = useState(false);
  const [revealed, setRevealed] = useState<string | null>(null);
  const [revealing, setRevealing] = useState(false);
  const [pending, startTransition] = useTransition();

  const keySet = keyState !== "unset";
  const typedKey = draftKey.trim();

  const dirty = useMemo(
    () => enabled !== savedEnabled || removeKey || typedKey.length > 0,
    [enabled, savedEnabled, removeKey, typedKey],
  );

  function resetDraft() {
    setEnabled(savedEnabled);
    setDraftKey("");
    setRemoveKey(false);
  }

  function onSave() {
    const nextEnabled = enabled;
    const clearing = removeKey;
    const writing = clearing ? null : typedKey || null;

    startTransition(async () => {
      try {
        if (nextEnabled !== savedEnabled) {
          await setRegistrationEnabled(nextEnabled);
          setSavedEnabled(nextEnabled);
        }
        if (clearing || writing !== null) {
          const res = await saveRegistrationKey(writing);
          if (!res.ok) {
            toast.error(t("saveErrorTitle"), { description: t("keyTooLong") });
            return;
          }
          setKeyState(res.keyState);
          setDraftKey("");
          setRemoveKey(false);
          setRevealed(null);
        }
        toast.success(t("savedTitle"), { description: t("savedDesc") });
      } catch (err) {
        toast.error(t("saveErrorTitle"), {
          description: err instanceof Error ? err.message : t("saveErrorDesc"),
        });
      }
    });
  }

  async function onReveal() {
    setRevealing(true);
    try {
      const { key } = await revealRegistrationKey();
      if (key) setRevealed(key);
      else toast.error(t("keyUnreadable"));
    } catch {
      toast.error(t("saveErrorDesc"));
    } finally {
      setRevealing(false);
    }
  }

  async function copyRevealed() {
    if (!revealed) return;
    try {
      await navigator.clipboard.writeText(revealed);
      toast.success(tc("copied"));
    } catch {
      toast.error(t("saveErrorDesc"));
    }
  }

  return (
    <div className="flex flex-col gap-6">
      <Card>
        <CardHeader className="border-b">
          <CardTitle className="text-sm">{t("platform")}</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-5 py-5">
          {/* 1. The door */}
          <div className="flex items-start gap-4">
            <span className="bg-muted text-muted-foreground grid size-9 shrink-0 place-items-center rounded-lg">
              <UserPlus className="size-4.5" />
            </span>
            <div className="min-w-0 flex-1">
              <Label htmlFor="set-register_enabled" className="font-medium">
                {t("registerTitle")}
              </Label>
              <p className="text-muted-foreground mt-0.5 text-xs leading-relaxed">
                {t("registerDesc")}
              </p>
              <p className="text-muted-foreground mt-1.5 text-xs leading-relaxed">
                {t("registerAdminNote")}
              </p>
            </div>
            <Switch
              id="set-register_enabled"
              checked={enabled}
              onCheckedChange={setEnabled}
            />
          </div>

          <Separator />

          {/* 2. Who comes through it */}
          <div
            className={cn(
              "flex items-start gap-4 transition-opacity",
              !enabled && "opacity-55",
            )}
          >
            <span className="bg-muted text-muted-foreground grid size-9 shrink-0 place-items-center rounded-lg">
              <KeyRound className="size-4.5" />
            </span>
            <div className="flex min-w-0 flex-1 flex-col gap-2">
              <div>
                <Label htmlFor="set-registration_key" className="font-medium">
                  {t("keyTitle")}
                </Label>
                <p className="text-muted-foreground mt-0.5 text-xs leading-relaxed">
                  {t("keyDesc")}
                </p>
              </div>

              <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
                <Input
                  id="set-registration_key"
                  autoComplete="off"
                  spellCheck={false}
                  className="font-mono"
                  disabled={removeKey}
                  placeholder={keySet ? t("keyKeepPlaceholder") : t("keyPlaceholder")}
                  value={draftKey}
                  onChange={(e) => setDraftKey(e.target.value)}
                />
                {keySet ? (
                  <div className="flex shrink-0 items-center gap-2">
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      disabled={revealing || keyState === "undecryptable"}
                      onClick={onReveal}
                    >
                      {revealing ? (
                        <Loader2 className="size-4 animate-spin" />
                      ) : (
                        <Eye className="size-4" />
                      )}
                      {t("keyShow")}
                    </Button>
                    <Button
                      type="button"
                      variant={removeKey ? "secondary" : "ghost"}
                      size="sm"
                      onClick={() => {
                        setRemoveKey((v) => !v);
                        setDraftKey("");
                      }}
                    >
                      {removeKey ? <Undo2 className="size-4" /> : <RotateCcw className="size-4" />}
                      {removeKey ? tc("cancel") : t("keyRemove")}
                    </Button>
                  </div>
                ) : null}
              </div>

              {/* The disclosure itself. Shown on request and audited server-side:
                  this key exists to be handed out, so an administrator has to be
                  able to read it back, but never by accident. */}
              {revealed ? (
                <div className="ring-foreground/10 flex items-center gap-2 rounded-lg px-3 py-2 ring-1">
                  <code className="min-w-0 flex-1 truncate font-mono text-sm">{revealed}</code>
                  <Button type="button" variant="ghost" size="icon-sm" onClick={copyRevealed}>
                    <Copy className="size-4" />
                    <span className="sr-only">{tc("copy")}</span>
                  </Button>
                </div>
              ) : null}

              {removeKey ? (
                <p className="text-muted-foreground text-xs leading-relaxed">
                  {t("keyRemovePending")}
                </p>
              ) : null}

              {keyState === "undecryptable" ? (
                <p className="text-destructive flex items-start gap-1.5 text-xs leading-relaxed">
                  <TriangleAlert className="mt-0.5 size-3.5 shrink-0" />
                  {t("keyUndecryptable")}
                </p>
              ) : null}

              {!enabled ? (
                <p className="text-muted-foreground text-xs leading-relaxed">
                  {t("keyInactive")}
                </p>
              ) : null}
            </div>
          </div>
        </CardContent>
      </Card>

      {/* Read-only: storage limits are not a global switch any more — they are a
          per-account number, edited in the Users tab. This card exists so the
          operator can see where a brand-new account starts. */}
      <Card>
        <CardHeader className="border-b">
          <CardTitle className="flex items-center gap-2 text-sm">
            <HardDrive className="text-muted-foreground size-4" /> {t("storageDefaults")}
          </CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <div className="min-w-0">
            <p className="text-sm font-medium">{t("newAccountQuota")}</p>
            <p className="text-muted-foreground mt-0.5 text-xs leading-relaxed">
              {t.rich("newAccountQuotaHelp", {
                env: () => (
                  <code className="bg-muted rounded px-1 py-0.5 text-[0.7rem]">
                    DEFAULT_USER_QUOTA
                  </code>
                ),
              })}
            </p>
          </div>
          <span className="font-heading shrink-0 text-2xl leading-none font-semibold tracking-tight tabular-nums">
            {defaultQuota ?? "∞"}
          </span>
        </CardContent>
      </Card>

      <Separator />

      <div className="flex items-center justify-between gap-3">
        <p className="text-muted-foreground text-xs">
          {dirty ? t("unsavedChanges") : t("allSaved")}
        </p>
        <div className="flex items-center gap-2">
          <Button
            type="button"
            variant="ghost"
            disabled={!dirty || pending}
            onClick={resetDraft}
          >
            <RotateCcw className="size-4" /> {t("reset")}
          </Button>
          <Button type="button" disabled={!dirty || pending} onClick={onSave}>
            {pending ? <Loader2 className="size-4 animate-spin" /> : <Save className="size-4" />}
            {pending ? t("saving") : t("saveChanges")}
          </Button>
        </div>
      </div>
    </div>
  );
}
