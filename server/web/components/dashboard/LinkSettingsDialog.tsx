"use client";

import { useEffect, useState, useTransition } from "react";
import { useRouter } from "next/navigation";
import { useTranslations } from "next-intl";
import {
  Clock,
  Eye,
  KeyRound,
  Loader2,
  Check,
  X,
  Flame,
  Infinity as InfinityIcon,
} from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  setUploadExpiry,
  setUploadMaxViews,
  setUploadPassword,
} from "@/app/actions/uploads";
import type { FileItem } from "./types";

const EXPIRY_PRESETS: { key: string; ms: number | null }[] = [
  { key: "preset1h", ms: 60 * 60 * 1000 },
  { key: "preset24h", ms: 24 * 60 * 60 * 1000 },
  { key: "preset7d", ms: 7 * 24 * 60 * 60 * 1000 },
  { key: "preset30d", ms: 30 * 24 * 60 * 60 * 1000 },
  { key: "presetNever", ms: null },
];

/** Format a future Date for a `datetime-local` input value (local time, no seconds/zone). */
function toLocalInputValue(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return (
    `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}` +
    `T${pad(d.getHours())}:${pad(d.getMinutes())}`
  );
}

function formatExpiry(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

export function LinkSettingsDialog({
  open,
  onOpenChange,
  item,
}: {
  open: boolean;
  onOpenChange: (v: boolean) => void;
  item: Pick<
    FileItem,
    "id" | "filename" | "expiresAt" | "maxViews" | "hasPassword"
  >;
}) {
  const t = useTranslations("dashboard");
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      {/* min-w-0 lets the header shrink instead of being widened by its own
          content: without it a long filename pushes the dialog past its
          max-width and the panel spills outside the popup. */}
      <DialogContent className="w-[calc(100vw-2rem)] min-w-0 sm:max-w-md">
        <DialogHeader className="min-w-0">
          <DialogTitle>{t("linkSettings.title")}</DialogTitle>
          {/* `truncate` alone is not enough here — an unbroken 200-character
              filename has no space to wrap at, so allow a hard break too. */}
          <DialogDescription className="min-w-0 truncate break-all" title={item.filename}>
            {item.filename}
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-5">
          <ExpirySection item={item} />
          <Separator />
          <ViewLimitSection item={item} />
          <Separator />
          <PasswordSection item={item} />
        </div>

        <DialogFooter showCloseButton />
      </DialogContent>
    </Dialog>
  );
}

/* ── Expiry ─────────────────────────────────────────────────────────── */

function ExpirySection({
  item,
}: {
  item: Pick<FileItem, "id" | "expiresAt">;
}) {
  const t = useTranslations("dashboard");
  const router = useRouter();
  const [pending, startTransition] = useTransition();
  const [custom, setCustom] = useState("");

  // Seed the custom field from the current expiry whenever it changes.
  useEffect(() => {
    if (item.expiresAt) {
      const d = new Date(item.expiresAt);
      if (!Number.isNaN(d.getTime())) {
        setCustom(toLocalInputValue(d));
        return;
      }
    }
    setCustom("");
  }, [item.expiresAt]);

  function apply(iso: string | null, label: string) {
    startTransition(async () => {
      try {
        await setUploadExpiry(item.id, iso);
        toast.success(iso ? t("linkSettings.expirySetTo", { label }) : t("linkSettings.expiryRemoved"));
        router.refresh();
      } catch {
        toast.error(t("linkSettings.couldntUpdateExpiry"));
      }
    });
  }

  function applyPreset(ms: number | null, label: string) {
    if (ms == null) {
      apply(null, label);
      return;
    }
    apply(new Date(Date.now() + ms).toISOString(), label);
  }

  function applyCustom() {
    const v = custom.trim();
    if (!v) {
      toast.error(t("linkSettings.pickDateTime"));
      return;
    }
    const d = new Date(v);
    if (Number.isNaN(d.getTime())) {
      toast.error(t("linkSettings.invalidDateTime"));
      return;
    }
    apply(d.toISOString(), formatExpiry(d.toISOString()));
  }

  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <Clock className="text-muted-foreground size-4" />
          <h3 className="text-sm font-medium">{t("linkSettings.expiry")}</h3>
        </div>
        {pending ? <Loader2 className="text-muted-foreground size-3.5 animate-spin" /> : null}
      </div>
      <p className="text-muted-foreground text-xs">
        {t("linkSettings.expiryDesc")}
      </p>

      <div className="flex flex-wrap gap-1.5">
        {EXPIRY_PRESETS.map((p) => {
          const label = t(`linkSettings.${p.key}`);
          return (
            <Button
              key={p.key}
              type="button"
              variant="outline"
              size="xs"
              disabled={pending}
              onClick={() => applyPreset(p.ms, label)}
            >
              {p.ms === null ? <X className="size-3" /> : null}
              {label}
            </Button>
          );
        })}
      </div>

      <div className="flex items-end gap-2">
        <div className="flex flex-1 flex-col gap-1.5">
          <Label htmlFor={`expiry-${item.id}`} className="text-xs">
            {t("linkSettings.customTime")}
          </Label>
          <Input
            id={`expiry-${item.id}`}
            type="datetime-local"
            value={custom}
            onChange={(e) => setCustom(e.target.value)}
            disabled={pending}
          />
        </div>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          disabled={pending || !custom.trim()}
          onClick={applyCustom}
        >
          <Check className="size-3.5" /> {t("linkSettings.set")}
        </Button>
      </div>

      <p className="text-xs">
        {item.expiresAt ? (
          <span className="text-foreground">
            {t.rich("linkSettings.expiresOn", {
              date: formatExpiry(item.expiresAt),
              strong: (chunks) => <span className="font-medium">{chunks}</span>,
            })}
          </span>
        ) : (
          <span className="text-muted-foreground">{t("linkSettings.neverExpires")}</span>
        )}
      </p>
    </section>
  );
}

/* ── View limit ─────────────────────────────────────────────────────── */

function ViewLimitSection({
  item,
}: {
  item: Pick<FileItem, "id" | "maxViews">;
}) {
  const t = useTranslations("dashboard");
  const router = useRouter();
  const [pending, startTransition] = useTransition();
  const [value, setValue] = useState(item.maxViews != null ? String(item.maxViews) : "");

  useEffect(() => {
    setValue(item.maxViews != null ? String(item.maxViews) : "");
  }, [item.maxViews]);

  function apply(maxViews: number | null, message: string) {
    startTransition(async () => {
      try {
        await setUploadMaxViews(item.id, maxViews);
        toast.success(message);
        router.refresh();
      } catch {
        toast.error(t("linkSettings.couldntUpdateViewLimit"));
      }
    });
  }

  function applyInput() {
    const n = Number(value);
    if (!value.trim() || !Number.isFinite(n) || n < 1) {
      toast.error(t("linkSettings.enterNumberOrMore"));
      return;
    }
    const capped = Math.floor(n);
    apply(capped, t("linkSettings.limitedToViewsToast", { count: capped }));
  }

  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <Eye className="text-muted-foreground size-4" />
          <h3 className="text-sm font-medium">{t("linkSettings.viewLimit")}</h3>
        </div>
        {pending ? <Loader2 className="text-muted-foreground size-3.5 animate-spin" /> : null}
      </div>
      <p className="text-muted-foreground text-xs">
        {t("linkSettings.viewLimitDesc")}
      </p>

      <div className="flex items-end gap-2">
        <div className="flex flex-1 flex-col gap-1.5">
          <Label htmlFor={`views-${item.id}`} className="text-xs">
            {t("linkSettings.maxViews")}
          </Label>
          <Input
            id={`views-${item.id}`}
            type="number"
            min={1}
            step={1}
            inputMode="numeric"
            placeholder={t("linkSettings.noLimitPlaceholder")}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") applyInput();
            }}
            disabled={pending}
          />
        </div>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          disabled={pending || !value.trim()}
          onClick={applyInput}
        >
          <Check className="size-3.5" /> {t("linkSettings.set")}
        </Button>
      </div>

      <div className="flex flex-wrap gap-1.5">
        <Button
          type="button"
          variant="outline"
          size="xs"
          disabled={pending}
          onClick={() => apply(1, t("linkSettings.oneTimeLinkSet"))}
        >
          <Flame className="size-3" /> {t("linkSettings.oneTimeLink")}
        </Button>
        <Button
          type="button"
          variant="outline"
          size="xs"
          disabled={pending}
          onClick={() => apply(null, t("linkSettings.viewLimitRemoved"))}
        >
          <InfinityIcon className="size-3" /> {t("linkSettings.noLimit")}
        </Button>
      </div>

      <p className="text-xs">
        {item.maxViews != null ? (
          <span className="text-foreground">
            {t.rich("linkSettings.limitedToViews", {
              count: item.maxViews,
              strong: (chunks) => <span className="font-medium">{chunks}</span>,
            })}
          </span>
        ) : (
          <span className="text-muted-foreground">{t("linkSettings.noViewLimit")}</span>
        )}
      </p>
    </section>
  );
}

/* ── Password ───────────────────────────────────────────────────────── */

function PasswordSection({
  item,
}: {
  item: Pick<FileItem, "id" | "hasPassword">;
}) {
  const t = useTranslations("dashboard");
  const router = useRouter();
  const [pending, startTransition] = useTransition();
  const [value, setValue] = useState("");

  function setPassword() {
    const v = value;
    if (!v) {
      toast.error(t("linkSettings.enterPassword"));
      return;
    }
    startTransition(async () => {
      try {
        await setUploadPassword(item.id, v);
        toast.success(t("linkSettings.passwordSet"));
        setValue("");
        router.refresh();
      } catch {
        toast.error(t("linkSettings.couldntSetPassword"));
      }
    });
  }

  function removePassword() {
    startTransition(async () => {
      try {
        await setUploadPassword(item.id, null);
        toast.success(t("linkSettings.passwordRemoved"));
        setValue("");
        router.refresh();
      } catch {
        toast.error(t("linkSettings.couldntRemovePassword"));
      }
    });
  }

  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <KeyRound className="text-muted-foreground size-4" />
          <h3 className="text-sm font-medium">{t("linkSettings.password")}</h3>
        </div>
        <div className="flex items-center gap-2">
          {pending ? <Loader2 className="text-muted-foreground size-3.5 animate-spin" /> : null}
          <Badge
            variant="secondary"
            className={cn(
              "gap-1",
              item.hasPassword
                ? "text-primary"
                : "text-muted-foreground",
            )}
          >
            {item.hasPassword ? t("linkSettings.protected") : t("linkSettings.off")}
          </Badge>
        </div>
      </div>
      <p className="text-muted-foreground text-xs">
        {t("linkSettings.passwordDesc")}
      </p>

      <div className="flex items-end gap-2">
        <div className="flex flex-1 flex-col gap-1.5">
          <Label htmlFor={`password-${item.id}`} className="text-xs">
            {item.hasPassword ? t("linkSettings.newPassword") : t("linkSettings.passwordLabel")}
          </Label>
          <Input
            id={`password-${item.id}`}
            type="password"
            autoComplete="new-password"
            placeholder="••••••••"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") setPassword();
            }}
            disabled={pending}
          />
        </div>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          disabled={pending || !value}
          onClick={setPassword}
        >
          <Check className="size-3.5" /> {t("linkSettings.set")}
        </Button>
      </div>

      {item.hasPassword ? (
        <Button
          type="button"
          variant="destructive"
          size="sm"
          className="w-fit"
          disabled={pending}
          onClick={removePassword}
        >
          <X className="size-3.5" /> {t("linkSettings.removePassword")}
        </Button>
      ) : null}
    </section>
  );
}
