"use client";

import { useState, useTransition } from "react";
import { useRouter } from "next/navigation";
import {
  Link2,
  Plus,
  Trash2,
  Copy,
  Check,
  ExternalLink,
  Clock3,
  MousePointerClick,
  CalendarClock,
  Sparkles,
  Wand2,
  Lock,
} from "lucide-react";
import { toast } from "sonner";
import { useLocale, useTranslations } from "next-intl";
import { relativeTime } from "@/lib/format";
import { createShortLink, deleteShortLink } from "@/app/actions/shortlinks";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

interface LinkView {
  id: string;
  code: string;
  target: string;
  clicks: number;
  hasPassword: boolean;
  lastUsedAt: string | null;
  expiresAt: string | null;
  createdAt: string;
}

interface LinkManagerProps {
  links: LinkView[];
  appUrl: string;
  /**
   * Base the shortener is published under (SHORTLINK_URL, e.g. https://s.example.com).
   * Falls back to appUrl — the same app answers on both hosts, so an older link
   * built on APP_URL keeps resolving either way.
   */
  shortBase?: string;
  /** Owner's tag, used as the /s/{userCode}/ prefix. */
  userCode?: string | null;
  /** May this account create links at the root of the shortener domain? */
  canUseRoot?: boolean;
}

const EXPIRY_OPTIONS = [
  { value: "0", labelKey: "neverExpires" },
  { value: "1", labelKey: "expiry1Day" },
  { value: "7", labelKey: "expiry7Days" },
  { value: "30", labelKey: "expiry30Days" },
  { value: "90", labelKey: "expiry90Days" },
  { value: "365", labelKey: "expiry1Year" },
];

/** Strip protocol/trailing slash for a compact target preview. */
function prettyTarget(url: string): string {
  return url.replace(/^https?:\/\//i, "").replace(/\/+$/, "");
}

/**
 * Mirrors `slugify()` in lib/ids.ts so the preview matches what the server will
 * actually store. That module pulls in node:crypto and cannot be imported here.
 */
function slugPreview(input: string): string {
  return input
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

function isExpired(expiresAt: string | null): boolean {
  return expiresAt != null && new Date(expiresAt).getTime() <= Date.now();
}

function CopyButton({
  value,
  size = "icon-sm",
  variant = "outline",
  label,
}: {
  value: string;
  size?: "sm" | "default" | "xs" | "icon-sm";
  variant?: "outline" | "default" | "ghost" | "secondary";
  label?: string;
}) {
  const t = useTranslations("links");
  const tCommon = useTranslations("common");
  const [copied, setCopied] = useState(false);
  const isIcon = size === "icon-sm";

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      toast.success(t("shortLinkCopied"));
      setTimeout(() => setCopied(false), 1600);
    } catch {
      toast.error(t("clipboardError"));
    }
  }

  return (
    <Button
      type="button"
      variant={variant}
      size={size}
      onClick={copy}
      aria-label={t("copyShortLink")}
      title={t("copyShortLink")}
    >
      {copied ? <Check className="text-emerald-500" /> : <Copy />}
      {!isIcon && (copied ? tCommon("copied") : (label ?? tCommon("copy")))}
    </Button>
  );
}

export function LinkManager({
  links,
  appUrl,
  shortBase,
  userCode = null,
  canUseRoot = false,
}: LinkManagerProps) {
  const t = useTranslations("links");
  const tCommon = useTranslations("common");
  const router = useRouter();
  // relativeTime formats through Intl, so it has to be told whose language
  // this is; without it a Polish reader gets English ages.
  const locale = useLocale();
  const [target, setTarget] = useState("");
  const [customCode, setCustomCode] = useState("");
  const [password, setPassword] = useState("");
  const [expiry, setExpiry] = useState("0");
  // Namespacing is the default: it is what keeps one user's "test" from
  // blocking everyone else's.
  const [useUserCode, setUseUserCode] = useState(true);
  const [creating, startCreate] = useTransition();
  const [deletePending, startDelete] = useTransition();
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [confirmId, setConfirmId] = useState<string | null>(null);

  const base = shortBase || appUrl;
  const shortUrl = (code: string) => `${base}/s/${code}`;

  // Without a tag there is nothing to prefix with, so the choice disappears.
  const prefixAvailable = Boolean(userCode);
  const prefixed = prefixAvailable && useUserCode;
  const codePrefix = prefixed ? `/s/${userCode}/` : "/s/";
  const previewCode = customCode.trim() ? slugPreview(customCode) : t("codePlaceholderPreview");
  const previewUrl = `${base}${codePrefix}${previewCode}`;

  function handleCreate() {
    const trimmed = target.trim();
    if (!trimmed) {
      toast.error(t("enterUrl"));
      return;
    }
    const days = Number(expiry);
    startCreate(async () => {
      try {
        const result = await createShortLink({
          target: trimmed,
          customCode: customCode.trim() || undefined,
          expiresInDays: days > 0 ? days : null,
          password: password.trim() || undefined,
          useUserCode: prefixed,
        });
        if (!result.ok) {
          toast.error(t(`issue.${result.reason}`));
          return;
        }
        try {
          await navigator.clipboard.writeText(result.url);
          toast.success(t("createdAndCopied"));
        } catch {
          toast.success(t("created"));
        }
        setTarget("");
        setCustomCode("");
        setPassword("");
        setExpiry("0");
        router.refresh();
      } catch {
        toast.error(t("createError"));
      }
    });
  }

  function handleDelete(id: string) {
    setDeletingId(id);
    startDelete(async () => {
      try {
        await deleteShortLink(id);
        toast.success(t("deleted"));
        setConfirmId(null);
        router.refresh();
      } catch {
        toast.error(t("deleteError"));
      } finally {
        setDeletingId(null);
      }
    });
  }

  return (
    <div className="space-y-6">
      {/* Create */}
      <Card>
        <CardHeader className="border-b">
          <div className="space-y-1">
            <CardTitle className="flex items-center gap-2">
              <Wand2 className="text-primary size-4" />
              {t("shortenUrl")}
            </CardTitle>
            <CardDescription>
              {t("shortenDescription")}
            </CardDescription>
          </div>
        </CardHeader>

        <CardContent className="space-y-4 pt-5">
          <div className="space-y-2">
            <Label htmlFor="link-target">{t("destinationUrl")}</Label>
            <Input
              id="link-target"
              type="url"
              inputMode="url"
              value={target}
              onChange={(e) => setTarget(e.target.value)}
              placeholder={t("destinationPlaceholder")}
              autoComplete="off"
              onKeyDown={(e) => {
                if (e.key === "Enter" && !creating) handleCreate();
              }}
            />
          </div>

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="link-code">
                {t("customCode")}{" "}
                <span className="text-muted-foreground font-normal">{t("optional")}</span>
              </Label>
              <div className="flex items-stretch">
                <span className="border-input bg-muted text-muted-foreground inline-flex max-w-[45%] items-center truncate rounded-l-lg border border-r-0 px-2.5 font-mono text-xs">
                  {codePrefix}
                </span>
                <Input
                  id="link-code"
                  value={customCode}
                  onChange={(e) => setCustomCode(e.target.value)}
                  placeholder={t("customCodePlaceholder")}
                  autoComplete="off"
                  maxLength={64}
                  className="rounded-l-none"
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && !creating) handleCreate();
                  }}
                />
              </div>
            </div>

            <div className="space-y-2">
              <Label htmlFor="link-expiry">{t("expiry")}</Label>
              <Select value={expiry} onValueChange={setExpiry}>
                <SelectTrigger id="link-expiry" className="w-full" size="default">
                  <CalendarClock className="text-muted-foreground size-4" />
                  <SelectValue placeholder={t("neverExpires")} />
                </SelectTrigger>
                <SelectContent>
                  {EXPIRY_OPTIONS.map((o) => (
                    <SelectItem key={o.value} value={o.value}>
                      {t(o.labelKey)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="link-password">
              {t("password")}{" "}
              <span className="text-muted-foreground font-normal">{t("optional")}</span>
            </Label>
            <div className="relative">
              <Lock className="text-muted-foreground pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2" />
              <Input
                id="link-password"
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder={t("passwordPlaceholder")}
                autoComplete="new-password"
                className="pl-9"
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !creating) handleCreate();
                }}
              />
            </div>
            <p className="text-muted-foreground text-xs">
              {t("passwordHelper")}
            </p>
          </div>

          {prefixAvailable && (
            <div className="space-y-2">
              <div className="flex items-start gap-2.5">
                <Checkbox
                  id="link-prefix"
                  checked={useUserCode}
                  onCheckedChange={(v) => setUseUserCode(v === true)}
                  disabled={!canUseRoot}
                  aria-describedby="link-prefix-help"
                />
                <div className="space-y-0.5">
                  <Label htmlFor="link-prefix" className="cursor-pointer">
                    {t("useUserCode")}
                  </Label>
                  <p id="link-prefix-help" className="text-muted-foreground text-xs leading-relaxed">
                    {canUseRoot ? t("useUserCodeHelp") : t("useUserCodeLocked")}
                  </p>
                </div>
              </div>
            </div>
          )}

          {/* What the finished link will look like. */}
          <div className="bg-muted/40 ring-foreground/10 space-y-1 rounded-lg px-3 py-2.5 ring-1">
            <p className="text-muted-foreground text-xs font-medium">{t("previewLabel")}</p>
            <p className="text-foreground font-mono text-xs break-all">{previewUrl}</p>
          </div>

          <div className="flex justify-end">
            <Button onClick={handleCreate} disabled={creating}>
              <Plus />
              {creating ? t("creating") : t("createShortLink")}
            </Button>
          </div>
        </CardContent>
      </Card>

      {/* List */}
      <Card>
        <CardHeader className="border-b">
          <div className="flex items-center justify-between gap-4">
            <CardTitle className="flex items-center gap-2">
              <Link2 className="text-primary size-4" />
              {t("yourLinks")}
            </CardTitle>
            {links.length > 0 && (
              <Badge variant="secondary">{links.length}</Badge>
            )}
          </div>
        </CardHeader>

        <CardContent className="px-0 pb-0">
          {links.length === 0 ? (
            <div className="flex flex-col items-center justify-center gap-3 px-6 py-14 text-center">
              <div className="bg-primary/10 text-primary grid size-12 place-items-center rounded-2xl">
                <Link2 className="size-6" />
              </div>
              <div className="space-y-1">
                <p className="text-sm font-medium">{t("emptyTitle")}</p>
                <p className="text-muted-foreground mx-auto max-w-xs text-sm">
                  {t("emptyDescription")}
                </p>
              </div>
              <p className="text-muted-foreground/70 mt-1 inline-flex items-center gap-1.5 text-xs">
                <Sparkles className="size-3" />
                {t("emptyTip")}
              </p>
            </div>
          ) : (
            <ul className="divide-border divide-y">
              {links.map((l) => {
                const url = shortUrl(l.code);
                const expired = isExpired(l.expiresAt);
                const isDeleting = deletePending && deletingId === l.id;
                return (
                  <li
                    key={l.id}
                    className="flex flex-wrap items-center gap-x-4 gap-y-3 px-4 py-3.5 sm:px-6"
                  >
                    <div className="bg-muted text-muted-foreground grid size-9 shrink-0 place-items-center rounded-lg">
                      <Link2 className="size-4" />
                    </div>

                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center gap-2">
                        {/* The full destination, so it can be read (and trusted)
                            without hovering — short links live on their own host. */}
                        <code className="text-foreground bg-muted truncate rounded px-1.5 py-0.5 font-mono text-xs">
                          {prettyTarget(url)}
                        </code>
                        {l.hasPassword && (
                          <Badge
                            variant="secondary"
                            className="shrink-0 gap-1"
                            title={t("passwordProtected")}
                          >
                            <Lock className="size-3" />
                            {t("protected")}
                          </Badge>
                        )}
                        {expired && (
                          <Badge variant="destructive" className="shrink-0">
                            {t("expired")}
                          </Badge>
                        )}
                      </div>
                      <a
                        href={l.target}
                        target="_blank"
                        rel="noreferrer noopener"
                        className="text-muted-foreground hover:text-foreground mt-1 block truncate text-xs transition-colors"
                        title={l.target}
                      >
                        → {prettyTarget(l.target)}
                      </a>
                      <div className="text-muted-foreground mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs">
                        <span className="inline-flex items-center gap-1">
                          <MousePointerClick className="size-3" />
                          {l.clicks} {l.clicks === 1 ? t("clickSingular") : t("clickPlural")}
                        </span>
                        <span aria-hidden className="opacity-40">
                          •
                        </span>
                        <span>{t("createdAt", { time: relativeTime(l.createdAt, locale) })}</span>
                        <span aria-hidden className="opacity-40">
                          •
                        </span>
                        <span className="inline-flex items-center gap-1">
                          <Clock3 className="size-3" />
                          {l.lastUsedAt
                            ? t("usedAt", { time: relativeTime(l.lastUsedAt, locale) })
                            : t("neverUsed")}
                        </span>
                        {l.expiresAt && !expired && (
                          <>
                            <span aria-hidden className="opacity-40">
                              •
                            </span>
                            <span className="inline-flex items-center gap-1">
                              <CalendarClock className="size-3" />
                              {t("expiresAt", { time: relativeTime(l.expiresAt, locale) })}
                            </span>
                          </>
                        )}
                      </div>
                    </div>

                    <div className="flex items-center gap-1.5">
                      <CopyButton value={url} />
                      <Button
                        type="button"
                        variant="outline"
                        size="icon-sm"
                        asChild
                        aria-label={t("openShortLink")}
                        title={t("openShortLink")}
                      >
                        <a href={url} target="_blank" rel="noreferrer noopener">
                          <ExternalLink />
                        </a>
                      </Button>
                      {confirmId === l.id ? (
                        <div className="flex items-center gap-1.5">
                          <Button
                            type="button"
                            variant="destructive"
                            size="sm"
                            onClick={() => handleDelete(l.id)}
                            disabled={isDeleting}
                          >
                            <Trash2 />
                            {isDeleting ? t("deleting") : tCommon("confirm")}
                          </Button>
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            onClick={() => setConfirmId(null)}
                            disabled={isDeleting}
                          >
                            {tCommon("cancel")}
                          </Button>
                        </div>
                      ) : (
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-sm"
                          onClick={() => setConfirmId(l.id)}
                          className="text-muted-foreground hover:text-destructive"
                          aria-label={t("deleteShortLink")}
                          title={t("deleteShortLink")}
                        >
                          <Trash2 />
                        </Button>
                      )}
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
