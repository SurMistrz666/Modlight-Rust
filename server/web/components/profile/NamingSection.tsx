"use client";

import { useEffect, useMemo, useState, useTransition } from "react";
import { useRouter } from "next/navigation";
import { Check, Loader2, Sparkles, TriangleAlert, Wand2 } from "lucide-react";
import { toast } from "sonner";
import { useLocale, useTranslations } from "next-intl";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { updateNaming } from "@/app/actions/account";
import { cn } from "@/lib/utils";
import {
  NAMING_SCHEME_IDS,
  WORD_COUNTS,
  RANDOM_LENGTHS,
  poolFor,
  previewCode,
  type NamingSchemeId,
} from "./naming-preview";

interface NamingSectionProps {
  /** Base URL of the user's links, so the preview shows the real thing. */
  baseUrl: string;
  /** The user's tag, or null when their links skip the tag segment. */
  tag: string | null;
  scheme: NamingSchemeId;
  pattern: string | null;
  wordCount: number;
  randomLength: number;
}

/**
 * How NEW uploads are named — the second segment of every link.
 *
 * The preview is regenerated on every change (and on demand) rather than shown
 * as a static example, because the whole question the user is answering is
 * "what will my links actually look like", and word/character schemes only
 * answer that convincingly when you can watch them roll.
 */
export function NamingSection({
  baseUrl,
  tag,
  scheme: initialScheme,
  pattern: initialPattern,
  wordCount: initialWordCount,
  randomLength: initialRandomLength,
}: NamingSectionProps) {
  const t = useTranslations("profile.naming");
  const locale = useLocale();
  const router = useRouter();

  // "551 mln" for a Polish reader, "551M" for an English one. The pool is here
  // to convey scale, and a ten-digit string does the opposite.
  const compact = useMemo(
    () => new Intl.NumberFormat(locale, { notation: "compact", maximumFractionDigits: 1 }),
    [locale],
  );

  const [scheme, setScheme] = useState<NamingSchemeId>(initialScheme);
  const [pattern, setPattern] = useState(initialPattern ?? "{words2}_{rand5}");
  const [wordCount, setWordCount] = useState(initialWordCount);
  const [randomLength, setRandomLength] = useState(initialRandomLength);
  const [nonce, setNonce] = useState(0);
  const [saving, startSave] = useTransition();

  const dirty =
    scheme !== initialScheme ||
    wordCount !== initialWordCount ||
    randomLength !== initialRandomLength ||
    (scheme === "custom" && pattern.trim() !== (initialPattern ?? "").trim());

  // The preview is random, so it MUST NOT be computed during render: the server
  // and the client would roll different codes and React would report a
  // hydration mismatch on every visit to this page. Generating it in an effect
  // means the markup starts identical on both sides and only then fills in.
  // `nonce` is the re-roll button — it lets the sample change with no input change.
  const [sample, setSample] = useState<string | null>(null);
  useEffect(() => {
    setSample(previewCode({ scheme, pattern, wordCount, randomLength }));
  }, [scheme, pattern, wordCount, randomLength, nonce]);

  const previewUrl = sample
    ? tag
      ? `${baseUrl}/${tag}/${sample}`
      : `${baseUrl}/${sample}`
    : null;
  const patternEmpty = scheme === "custom" && !pattern.trim();

  function save() {
    if (!dirty || saving || patternEmpty) return;
    startSave(async () => {
      try {
        await updateNaming({ scheme, pattern, wordCount, randomLength });
        toast.success(t("saved"));
        router.refresh();
      } catch {
        toast.error(t("error"));
      }
    });
  }

  return (
    <Card>
      <CardHeader className="border-b">
        <CardTitle className="flex items-center gap-2">
          <Wand2 className="text-primary size-4" />
          {t("title")}
        </CardTitle>
        <CardDescription>{t("desc")}</CardDescription>
      </CardHeader>

      {/* flex-1 keeps the footer pinned to the bottom when this card is stretched
          to match a taller neighbour in the same grid row. */}
      <CardContent className="flex-1 space-y-5">
        <div className="space-y-2">
          <Label htmlFor="naming-scheme">{t("label")}</Label>
          <Select value={scheme} onValueChange={(v) => setScheme(v as NamingSchemeId)}>
            <SelectTrigger id="naming-scheme" className="w-full sm:w-96">
              <SelectValue>{t(`scheme.${scheme}.name`)}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              {NAMING_SCHEME_IDS.map((id) => (
                <SelectItem key={id} value={id}>
                  <span className="flex flex-col gap-0.5 py-0.5">
                    <span>{t(`scheme.${id}.name`)}</span>
                    <span className="text-muted-foreground text-xs">
                      {t(`scheme.${id}.hint`)}
                    </span>
                  </span>
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        {/* Words: how many */}
        {scheme === "words" && (
          <div className="space-y-2">
            <Label>{t("words.label")}</Label>
            <div className="flex flex-wrap gap-2">
              {WORD_COUNTS.map((n) => (
                <Button
                  key={n}
                  type="button"
                  size="sm"
                  variant={wordCount === n ? "default" : "outline"}
                  onClick={() => setWordCount(n)}
                >
                  {t("words.count", { count: n })}
                </Button>
              ))}
            </div>
            <p className="text-muted-foreground text-xs">
              {t("words.pool", { pool: compact.format(poolFor(scheme, { wordCount })) })}
            </p>
          </div>
        )}

        {/* Random characters: how long */}
        {scheme === "random-chars" && (
          <div className="space-y-2">
            <Label>{t("chars.label")}</Label>
            <div className="flex flex-wrap gap-2">
              {RANDOM_LENGTHS.map((n) => (
                <Button
                  key={n}
                  type="button"
                  size="sm"
                  variant={randomLength === n ? "default" : "outline"}
                  onClick={() => setRandomLength(n)}
                >
                  {n}
                </Button>
              ))}
            </div>
            <p className="text-muted-foreground text-xs">
              {t("chars.pool", {
                length: randomLength,
                pool: compact.format(poolFor(scheme, { randomLength })),
              })}
            </p>
            <div className="border-primary/25 bg-primary/5 flex items-start gap-2.5 rounded-lg border px-3 py-2.5">
              <Sparkles className="text-primary mt-0.5 size-4 shrink-0" />
              <div className="space-y-0.5">
                <p className="text-sm font-medium">{t("chars.unambiguousTitle")}</p>
                <p className="text-muted-foreground text-xs leading-relaxed">
                  {t("chars.unambiguousBody")}
                </p>
              </div>
            </div>
            <p className="text-muted-foreground flex items-start gap-1.5 text-xs leading-relaxed">
              <TriangleAlert className="mt-0.5 size-3.5 shrink-0" />
              {t("chars.growWarning")}
            </p>
          </div>
        )}

        {/* Custom pattern */}
        {scheme === "custom" && (
          <div className="space-y-2">
            <Label htmlFor="naming-pattern">{t("custom.label")}</Label>
            <Input
              id="naming-pattern"
              value={pattern}
              onChange={(e) => setPattern(e.target.value)}
              placeholder="{words2}_{rand5}"
              spellCheck={false}
              autoComplete="off"
              maxLength={120}
              aria-invalid={patternEmpty}
              className={cn("font-mono", patternEmpty && "border-destructive")}
            />
            <div className="bg-muted/40 ring-foreground/10 space-y-1.5 rounded-lg px-3 py-2.5 ring-1">
              <p className="text-muted-foreground text-xs font-medium">{t("custom.tokensTitle")}</p>
              <ul className="text-muted-foreground grid gap-x-4 gap-y-1 text-xs sm:grid-cols-2">
                {TOKEN_KEYS.map((key) => (
                  <li key={key} className="flex items-baseline gap-2">
                    <code className="bg-muted text-foreground rounded px-1 py-0.5 font-mono text-[11px]">
                      {`{${key}}`}
                    </code>
                    <span className="min-w-0 flex-1">{t(`custom.token.${key}`)}</span>
                  </li>
                ))}
              </ul>
            </div>
          </div>
        )}

        {/* Live preview */}
        <div className="bg-muted/40 ring-foreground/10 space-y-1.5 rounded-lg px-3 py-2.5 ring-1">
          <div className="flex items-center justify-between gap-2">
            <p className="text-muted-foreground text-xs font-medium">{t("previewLabel")}</p>
            <Button
              type="button"
              variant="ghost"
              size="xs"
              onClick={() => setNonce((n) => n + 1)}
              aria-label={t("reroll")}
            >
              <Sparkles />
              {t("reroll")}
            </Button>
          </div>
          <p className="text-foreground min-h-4 font-mono text-xs break-all">
            {previewUrl ?? <span className="text-muted-foreground">{baseUrl}/…</span>}
          </p>
        </div>

        <p className="text-muted-foreground flex items-start gap-1.5 text-xs leading-relaxed">
          <TriangleAlert className="mt-0.5 size-3.5 shrink-0" />
          {t("existingSafe")}
        </p>
      </CardContent>

      <CardFooter className="justify-end gap-2">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={!dirty || saving}
          onClick={() => {
            setScheme(initialScheme);
            setPattern(initialPattern ?? "{words2}_{rand5}");
            setWordCount(initialWordCount);
            setRandomLength(initialRandomLength);
          }}
        >
          {t("discard")}
        </Button>
        <Button type="button" size="sm" onClick={save} disabled={!dirty || saving || patternEmpty}>
          {saving ? <Loader2 className="size-4 animate-spin" /> : <Check className="size-4" />}
          {t("save")}
        </Button>
      </CardFooter>
    </Card>
  );
}

/** Tokens documented under the custom-pattern field. */
const TOKEN_KEYS = [
  "words2",
  "rand5",
  "name",
  "ext",
  "date",
  "time",
  "year",
  "month",
  "day",
  "random",
] as const;
