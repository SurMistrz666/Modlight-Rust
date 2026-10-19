"use client";

import { useState, useTransition } from "react";
import { useRouter } from "next/navigation";
import { Globe, Link2, ShieldCheck, TriangleAlert } from "lucide-react";
import { toast } from "sonner";
import { useTranslations } from "next-intl";
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
import { Label } from "@/components/ui/label";
import { updatePreferredDomain } from "@/app/actions/account";

interface DomainSectionProps {
  /** Every hostname the deployment answers on, default first. */
  domains: readonly string[];
  /** The default, used when the user expresses no preference. */
  defaultDomain: string;
  /** Stored preference (null = follow the default). */
  preferredDomain: string | null;
  /** The user's tag, or null when their links skip the tag segment. */
  tag: string | null;
}

const FOLLOW_DEFAULT = "__default__";

/** Strip the scheme so the dropdown reads "h.example.com", not "https://h.example.com". */
function hostOf(origin: string): string {
  try {
    return new URL(origin).host;
  } catch {
    return origin.replace(/^https?:\/\//, "");
  }
}

/**
 * Which of the deployment's hostnames this account's links are built from.
 *
 * Hidden entirely when there is only one — a picker with a single option is
 * just noise on an install that answers on one name.
 */
export function DomainSection({
  domains,
  defaultDomain,
  preferredDomain,
  tag,
}: DomainSectionProps) {
  const t = useTranslations("profile.domain");
  const router = useRouter();
  const [value, setValue] = useState(preferredDomain ?? FOLLOW_DEFAULT);
  const [pending, startTransition] = useTransition();

  if (domains.length < 2) return null;

  const active = value === FOLLOW_DEFAULT ? defaultDomain : value;
  const preview = tag ? `${active}/${tag}/screenshot.png` : `${active}/screenshot.png`;

  function pick(next: string) {
    if (next === value || pending) return;
    const previous = value;
    setValue(next);
    startTransition(async () => {
      try {
        const result = await updatePreferredDomain(next === FOLLOW_DEFAULT ? null : next);
        if (!result.ok) {
          setValue(previous);
          toast.error(t("error"));
          return;
        }
        toast.success(t("saved"));
        router.refresh();
      } catch {
        setValue(previous);
        toast.error(t("error"));
      }
    });
  }

  return (
    <Card>
      <CardHeader className="border-b">
        <CardTitle className="flex items-center gap-2">
          <Globe className="text-primary size-4" />
          {t("title")}
        </CardTitle>
        <CardDescription>{t("desc")}</CardDescription>
      </CardHeader>

      <CardContent className="flex-1 space-y-5">
        <div className="space-y-2">
          <Label htmlFor="preferred-domain">{t("label")}</Label>
          <Select value={value} onValueChange={pick} disabled={pending}>
            <SelectTrigger id="preferred-domain" className="w-full font-mono sm:w-80">
              <SelectValue>
                {value === FOLLOW_DEFAULT
                  ? t("followDefault", { host: hostOf(defaultDomain) })
                  : hostOf(value)}
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={FOLLOW_DEFAULT}>
                {t("followDefault", { host: hostOf(defaultDomain) })}
              </SelectItem>
              {domains.map((d) => (
                <SelectItem key={d} value={d} className="font-mono">
                  {hostOf(d)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        <div className="bg-muted/40 ring-foreground/10 space-y-1.5 rounded-lg px-3 py-2.5 ring-1">
          <p className="text-muted-foreground flex items-center gap-1.5 text-xs font-medium">
            <Link2 className="size-3.5" />
            {t("previewLabel")}
          </p>
          <p className="text-foreground font-mono text-xs break-all">{preview}</p>
        </div>

        {/* The reassurance that makes this switch safe to touch. */}
        <div className="border-primary/25 bg-primary/5 flex items-start gap-2.5 rounded-lg border px-3 py-2.5">
          <ShieldCheck className="text-primary mt-0.5 size-4 shrink-0" />
          <div className="space-y-0.5">
            <p className="text-sm font-medium">{t("safeTitle")}</p>
            <p className="text-muted-foreground text-xs leading-relaxed">
              {t("safeBody", { count: domains.length })}
            </p>
          </div>
        </div>

        {/* ShareX embeds the hostname in its config file, so it needs a re-download. */}
        <div className="border-amber-500/30 bg-amber-500/10 flex items-start gap-2.5 rounded-lg border px-3 py-2.5">
          <TriangleAlert className="mt-0.5 size-4 shrink-0 text-amber-600 dark:text-amber-400" />
          <div className="space-y-0.5">
            <p className="text-sm font-medium">{t("sharexTitle")}</p>
            <p className="text-muted-foreground text-xs leading-relaxed">
              {t.rich("sharexBody", {
                link: (chunks) => (
                  <a
                    href="/dashboard/tokens"
                    className="text-foreground font-medium underline underline-offset-2"
                  >
                    {chunks}
                  </a>
                ),
              })}
            </p>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}
