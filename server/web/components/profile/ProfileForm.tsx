"use client";

import { useState, useTransition } from "react";
import { useRouter } from "next/navigation";
import { Loader2, Check, AtSign, Copy } from "lucide-react";
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
import { Switch } from "@/components/ui/switch";
import { Separator } from "@/components/ui/separator";
import { updatePreferences } from "@/app/actions/account";
import { cn } from "@/lib/utils";

interface ProfileDefaults {
  name: string;
  email: string;
  userCode: string | null;
  hideUploads: boolean;
  copyRaw: boolean;
}

function ToggleRow({
  id,
  label,
  help,
  checked,
  onChange,
  disabled,
}: {
  id: string;
  label: string;
  help: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <div className="flex items-start justify-between gap-4 py-1">
      <div className="space-y-0.5">
        <Label htmlFor={id} className="cursor-pointer">
          {label}
        </Label>
        <p className="text-muted-foreground text-sm leading-snug">{help}</p>
      </div>
      <Switch id={id} checked={checked} onCheckedChange={onChange} disabled={disabled} />
    </div>
  );
}

export function ProfileForm({ defaults }: { defaults: ProfileDefaults }) {
  const router = useRouter();
  const t = useTranslations("profile.account");
  const tc = useTranslations("common");
  const [pending, startTransition] = useTransition();

  const [name, setName] = useState(defaults.name);
  const [hideUploads, setHideUploads] = useState(defaults.hideUploads);
  const [copyRaw, setCopyRaw] = useState(defaults.copyRaw);
  const [copied, setCopied] = useState(false);

  const dirty =
    name.trim() !== defaults.name.trim() ||
    hideUploads !== defaults.hideUploads ||
    copyRaw !== defaults.copyRaw;

  function reset() {
    setName(defaults.name);
    setHideUploads(defaults.hideUploads);
    setCopyRaw(defaults.copyRaw);
  }

  function save() {
    const trimmed = name.trim();
    if (!trimmed) {
      toast.error(t("nameEmpty"));
      return;
    }
    startTransition(async () => {
      try {
        await updatePreferences({ name: trimmed, hideUploads, copyRaw });
        toast.success(t("saved"));
        router.refresh();
      } catch {
        toast.error(t("saveError"));
        reset();
      }
    });
  }

  async function copyCode() {
    if (!defaults.userCode) return;
    try {
      await navigator.clipboard.writeText(defaults.userCode);
      setCopied(true);
      toast.success(t("userCodeCopied"));
      setTimeout(() => setCopied(false), 1500);
    } catch {
      toast.error(t("copyError"));
    }
  }

  return (
    <Card>
      <CardHeader className="border-b">
        <CardTitle>{t("title")}</CardTitle>
        <CardDescription>{t("desc")}</CardDescription>
      </CardHeader>

      {/* flex-1: the card is a grid item that stretches to its row's height, so
          the body has to absorb the slack or the footer floats mid-card. */}
      <CardContent className="flex-1 space-y-6">
        <div className="grid gap-5 sm:grid-cols-2">
          <div className="space-y-2">
            <Label htmlFor="name">{t("displayName")}</Label>
            <Input
              id="name"
              value={name}
              maxLength={120}
              placeholder={t("namePlaceholder")}
              onChange={(e) => setName(e.target.value)}
              disabled={pending}
            />
          </div>

          <div className="space-y-2">
            <Label htmlFor="email">{t("email")}</Label>
            <Input id="email" value={defaults.email} readOnly disabled className="text-muted-foreground" />
          </div>
        </div>

        {defaults.userCode && (
          <div className="space-y-2">
            <Label htmlFor="userCode">{t("userCode")}</Label>
            <div className="flex items-stretch gap-2">
              <div className="bg-muted/40 flex flex-1 items-center gap-2 rounded-lg px-2.5 ring-1 ring-foreground/10">
                <AtSign className="text-muted-foreground size-4 shrink-0" />
                <span className="font-mono text-sm tracking-tight">{defaults.userCode}</span>
              </div>
              <Button
                type="button"
                variant="outline"
                size="icon"
                onClick={copyCode}
                aria-label={t("copyUserCode")}
              >
                {copied ? <Check className="size-4 text-emerald-500" /> : <Copy className="size-4" />}
              </Button>
            </div>
            <p className="text-muted-foreground text-xs">
              {t("userCodeHint")} <span className="font-mono">/{defaults.userCode}/…</span>
            </p>
          </div>
        )}

        <Separator />

        <div className="space-y-4">
          <h3 className="text-sm font-medium">{t("uploadDefaults")}</h3>
          <ToggleRow
            id="hideUploads"
            label={t("privateByDefault")}
            help={t("privateByDefaultHelp")}
            checked={hideUploads}
            onChange={setHideUploads}
            disabled={pending}
          />
          <ToggleRow
            id="copyRaw"
            label={t("copyRaw")}
            help={t("copyRawHelp")}
            checked={copyRaw}
            onChange={setCopyRaw}
            disabled={pending}
          />
        </div>
      </CardContent>

      <CardFooter className="justify-end gap-2">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          onClick={reset}
          disabled={!dirty || pending}
        >
          {t("discard")}
        </Button>
        <Button type="button" size="sm" onClick={save} disabled={!dirty || pending}>
          {pending ? <Loader2 className="size-4 animate-spin" /> : <Check className="size-4" />}
          <span className={cn(pending && "opacity-90")}>{t("saveChanges")}</span>
        </Button>
      </CardFooter>
    </Card>
  );
}
