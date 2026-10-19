"use client";

import { useMemo, useState, useTransition } from "react";
import {
  FileCode2,
  Type,
  Clock3,
  KeyRound,
  Sparkles,
  Copy,
  Check,
  ExternalLink,
  RotateCcw,
  CheckCircle2,
  Code2,
  Eye,
  Flame,
  Info,
} from "lucide-react";
import { toast } from "sonner";
import { useTranslations } from "next-intl";
import { cn } from "@/lib/utils";
import { createPaste, type PasteResult } from "@/app/actions/pastes";
import { PASTE_LANGUAGES } from "@/lib/paste-languages";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

const EXPIRY_OPTIONS: { value: string; labelKey: string; minutes: number | null }[] = [
  { value: "never", labelKey: "never", minutes: null },
  { value: "1h", labelKey: "expiry1Hour", minutes: 60 },
  { value: "24h", labelKey: "expiry1Day", minutes: 60 * 24 },
  { value: "7d", labelKey: "expiry7Days", minutes: 60 * 24 * 7 },
  { value: "30d", labelKey: "expiry30Days", minutes: 60 * 24 * 30 },
];

/**
 * View-budget presets. `custom` reveals a number field instead of setting a
 * value, so the common cases stay one click and the uncommon one stays possible.
 */
const VIEW_LIMIT_OPTIONS: { value: string; labelKey: string; views: number | null }[] = [
  { value: "none", labelKey: "viewsUnlimited", views: null },
  { value: "1", labelKey: "viewsOnce", views: 1 },
  { value: "5", labelKey: "viewsN", views: 5 },
  { value: "10", labelKey: "viewsN", views: 10 },
  { value: "50", labelKey: "viewsN", views: 50 },
  { value: "custom", labelKey: "viewsCustom", views: null },
];

/** A small inline copy button that flips to a check on success. */
function CopyButton({
  value,
  label,
  className,
}: {
  value: string;
  label?: string;
  className?: string;
}) {
  const t = useTranslations("paste");
  const tCommon = useTranslations("common");
  const [copied, setCopied] = useState(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      toast.success(t("copiedToClipboard"));
      setTimeout(() => setCopied(false), 1600);
    } catch {
      toast.error(t("clipboardError"));
    }
  }

  return (
    <Button type="button" variant="default" size="sm" onClick={copy} className={className}>
      {copied ? <Check className="text-emerald-300" /> : <Copy />}
      {copied ? tCommon("copied") : (label ?? t("copyLink"))}
    </Button>
  );
}

export function PasteEditor() {
  const t = useTranslations("paste");
  const tCommon = useTranslations("common");
  const [content, setContent] = useState("");
  const [title, setTitle] = useState("");
  const [language, setLanguage] = useState("text");
  const [expiry, setExpiry] = useState("never");
  const [password, setPassword] = useState("");
  const [viewLimit, setViewLimit] = useState("none");
  const [customViews, setCustomViews] = useState("");
  const [pending, startTransition] = useTransition();
  const [result, setResult] = useState<PasteResult | null>(null);

  const lineCount = useMemo(
    () => (content ? content.split("\n").length : 0),
    [content],
  );
  const charCount = content.length;

  /**
   * The view budget the form currently describes, or null for "no limit".
   * `undefined` means the custom field holds something unusable, which is a
   * refusal rather than a silent fall back to unlimited: a paste meant to burn
   * after one open must never be created without its cap.
   */
  function resolveMaxViews(): number | null | undefined {
    if (viewLimit === "custom") {
      const raw = customViews.trim();
      if (!raw) return undefined;
      const n = Number(raw);
      if (!Number.isFinite(n) || n < 1) return undefined;
      return Math.floor(n);
    }
    return VIEW_LIMIT_OPTIONS.find((o) => o.value === viewLimit)?.views ?? null;
  }

  function handleCreate() {
    if (!content.trim()) {
      toast.error(t("pasteEmpty"));
      return;
    }

    const maxViews = resolveMaxViews();
    if (maxViews === undefined) {
      toast.error(t("viewsInvalid"));
      return;
    }

    const expiresInMinutes = EXPIRY_OPTIONS.find((o) => o.value === expiry)?.minutes ?? null;
    startTransition(async () => {
      try {
        const res = await createPaste({
          content,
          language,
          title: title.trim() || undefined,
          expiresInMinutes,
          password: password ? password : undefined,
          maxViews,
        });
        setResult(res);
        toast.success(t("pasteCreated"));
      } catch (err) {
        toast.error(err instanceof Error ? err.message : t("createError"));
      }
    });
  }

  function reset() {
    setResult(null);
    setContent("");
    setTitle("");
    setLanguage("text");
    setExpiry("never");
    setPassword("");
    setViewLimit("none");
    setCustomViews("");
  }

  if (result) {
    return (
      <div className="rounded-xl bg-card p-6 ring-1 ring-foreground/10">
        <div className="flex flex-col items-center gap-3 text-center">
          <span className="grid size-12 place-items-center rounded-2xl bg-emerald-500/10 text-emerald-400 ring-1 ring-emerald-500/20">
            <CheckCircle2 className="size-6" />
          </span>
          <div className="space-y-1">
            <h2 className="text-lg font-semibold tracking-tight">{t("successHeading")}</h2>
            <p className="text-sm text-muted-foreground">
              {t("successDescription")}
            </p>
          </div>
        </div>

        {/* What the link will actually do. Stated from the stored values, so a
            burn-after-view paste says so before it is shared, not after. */}
        {(result.maxViews != null || result.expiresAt || result.hasPassword) && (
          <div className="mt-5 flex flex-wrap justify-center gap-1.5">
            {result.maxViews != null && (
              <Badge variant="secondary" className="gap-1">
                <Flame className="size-3" />
                {result.maxViews === 1
                  ? t("badgeOneTime")
                  : t("badgeViews", { count: result.maxViews })}
              </Badge>
            )}
            {result.expiresAt && (
              <Badge variant="secondary" className="gap-1">
                <Clock3 className="size-3" />
                {t("badgeExpires", {
                  date: new Date(result.expiresAt).toLocaleString(undefined, {
                    dateStyle: "medium",
                    timeStyle: "short",
                  }),
                })}
              </Badge>
            )}
            {result.hasPassword && (
              <Badge variant="secondary" className="gap-1">
                <KeyRound className="size-3" />
                {t("badgePassword")}
              </Badge>
            )}
          </div>
        )}

        <div className="mt-6 space-y-1.5">
          <Label htmlFor="paste-url" className="text-xs">
            {t("shareableLink")}
          </Label>
          <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
            <Input
              id="paste-url"
              readOnly
              value={result.url}
              onFocus={(e) => e.currentTarget.select()}
              className="font-mono text-xs"
            />
            <div className="flex shrink-0 gap-2">
              <CopyButton value={result.url} />
              <Button asChild type="button" variant="outline" size="sm">
                <a href={result.url} target="_blank" rel="noopener noreferrer">
                  <ExternalLink />
                  {tCommon("open")}
                </a>
              </Button>
            </div>
          </div>
        </div>

        {/* Opening the link here would spend one of the budgeted views, so the
            warning sits right next to the buttons that do it. */}
        {result.maxViews != null && (
          <p className="mt-2 flex items-start gap-1.5 text-xs text-muted-foreground">
            <Info className="mt-0.5 size-3.5 shrink-0" />
            {t("viewsOwnerExempt")}
          </p>
        )}

        <div className="mt-4 flex flex-wrap items-center justify-between gap-3 border-t border-foreground/10 pt-4">
          <a
            href={result.rawUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-1.5 text-xs text-muted-foreground transition-colors hover:text-foreground"
          >
            <Code2 className="size-3.5" />
            {t("viewRaw")}
          </a>
          <Button type="button" variant="ghost" size="sm" onClick={reset}>
            <RotateCcw />
            {t("newPaste")}
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-5">
      <div className="overflow-hidden rounded-xl bg-card ring-1 ring-foreground/10">
        <div className="flex items-center justify-between gap-3 border-b border-foreground/10 px-3 py-2">
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <FileCode2 className="size-3.5" />
            <span className="font-medium">{t("content")}</span>
          </div>
          <span className="font-mono text-[11px] text-muted-foreground tabular-nums">
            {lineCount} {lineCount === 1 ? t("lineSingular") : t("linePlural")} ·{" "}
            {charCount.toLocaleString()}{" "}
            {charCount === 1 ? t("charSingular") : t("charPlural")}
          </span>
        </div>
        <Textarea
          value={content}
          onChange={(e) => setContent(e.target.value)}
          placeholder={t("contentPlaceholder")}
          spellCheck={false}
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          className={cn(
            "min-h-[340px] resize-y rounded-none border-0 bg-transparent font-mono text-[13px] leading-relaxed",
            "px-4 py-3 focus-visible:ring-0 md:text-[13px]",
          )}
        />
      </div>

      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-1.5">
          <Label htmlFor="paste-title" className="text-xs">
            <Type className="size-3.5 text-muted-foreground" />
            {t("title")} <span className="text-muted-foreground">{t("optional")}</span>
          </Label>
          <Input
            id="paste-title"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder={t("titlePlaceholder")}
            maxLength={120}
            disabled={pending}
          />
        </div>

        <div className="space-y-1.5">
          <Label htmlFor="paste-language" className="text-xs">
            <FileCode2 className="size-3.5 text-muted-foreground" />
            {t("language")}
          </Label>
          <Select value={language} onValueChange={setLanguage} disabled={pending}>
            <SelectTrigger id="paste-language" className="h-8 w-full">
              <SelectValue placeholder={t("selectLanguage")} />
            </SelectTrigger>
            <SelectContent>
              {PASTE_LANGUAGES.map((lang) => (
                <SelectItem key={lang} value={lang}>
                  {lang}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        <div className="space-y-1.5">
          <Label htmlFor="paste-expiry" className="text-xs">
            <Clock3 className="size-3.5 text-muted-foreground" />
            {t("expires")} <span className="text-muted-foreground">{t("optional")}</span>
          </Label>
          <Select value={expiry} onValueChange={setExpiry} disabled={pending}>
            <SelectTrigger id="paste-expiry" className="h-8 w-full">
              <SelectValue placeholder={t("never")} />
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

        <div className="space-y-1.5">
          <Label htmlFor="paste-password" className="text-xs">
            <KeyRound className="size-3.5 text-muted-foreground" />
            {t("password")} <span className="text-muted-foreground">{t("optional")}</span>
          </Label>
          <Input
            id="paste-password"
            type="password"
            autoComplete="new-password"
            placeholder="••••••••"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            disabled={pending}
          />
        </div>
      </div>

      {/* View budget. Given its own block rather than a fourth cell in the grid:
          it needs the explanation underneath it to be read, because "expires
          after N views" is meaningless without knowing what counts as a view. */}
      <div className="space-y-2 rounded-xl bg-card p-4 ring-1 ring-foreground/10">
        <div className="flex flex-col gap-2 sm:flex-row sm:items-end sm:gap-3">
          <div className="flex-1 space-y-1.5">
            <Label htmlFor="paste-views" className="text-xs">
              <Eye className="size-3.5 text-muted-foreground" />
              {t("viewLimit")} <span className="text-muted-foreground">{t("optional")}</span>
            </Label>
            <Select value={viewLimit} onValueChange={setViewLimit} disabled={pending}>
              <SelectTrigger id="paste-views" className="h-8 w-full">
                <SelectValue placeholder={t("viewsUnlimited")} />
              </SelectTrigger>
              <SelectContent>
                {VIEW_LIMIT_OPTIONS.map((o) => (
                  <SelectItem key={o.value} value={o.value}>
                    {/* Only the plural label takes a count; the fixed ones read
                        better written out ("1 view (one time link)"). */}
                    {o.labelKey === "viewsN" ? t("viewsN", { count: o.views ?? 0 }) : t(o.labelKey)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          {viewLimit === "custom" && (
            <div className="w-full space-y-1.5 sm:w-40">
              <Label htmlFor="paste-views-custom" className="text-xs">
                {t("viewsCustomLabel")}
              </Label>
              <Input
                id="paste-views-custom"
                type="number"
                min={1}
                step={1}
                inputMode="numeric"
                placeholder="25"
                value={customViews}
                onChange={(e) => setCustomViews(e.target.value)}
                disabled={pending}
              />
            </div>
          )}
        </div>

        <p className="flex items-start gap-1.5 text-xs leading-relaxed text-muted-foreground">
          <Info className="mt-0.5 size-3.5 shrink-0" />
          <span>
            {t("viewLimitDesc")} <span className="text-foreground">{t("viewLimitCounts")}</span>{" "}
            {t("viewLimitDoesNotCount")}
          </span>
        </p>
      </div>

      <div className="flex items-center justify-between gap-3">
        <p className="text-xs text-muted-foreground">
          {t("helperText")}
        </p>
        <Button type="button" onClick={handleCreate} disabled={pending || !content.trim()}>
          <Sparkles />
          {pending ? t("creating") : t("createPaste")}
        </Button>
      </div>
    </div>
  );
}
