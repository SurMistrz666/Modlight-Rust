"use client";

import { useState, useTransition } from "react";
import { useRouter } from "next/navigation";
import { Check, Loader2, Palette, Languages, Eye } from "lucide-react";
import { toast } from "sonner";
import { useTranslations } from "next-intl";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  CardDescription,
} from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { THEMES, resolveTheme, type ThemeId } from "@/lib/themes";
import { updatePreferences } from "@/app/actions/account";
import { cn } from "@/lib/utils";

interface AppearanceSectionProps {
  /** Current theme id from session.user.theme. May still be a retired id. */
  theme: string;
  /** Current locale: 'pl' | 'en' | null (null = auto). */
  locale: string | null;
}

/**
 * A live miniature of a skin: a scrap of app furniture (rail, card, heading,
 * body line, primary button) rendered by the skin's own CSS.
 *
 * This used to be a hand-written table of three hex dots per theme, which had
 * two problems. It drifted - a token could change and the swatch would keep
 * advertising the old colour - and it could only ever show colour, so the six
 * themed skins, whose whole point is texture, radius, shadow and typeface,
 * would all have reduced to "three dots on a rectangle".
 *
 * `data-theme` here declares the skin's `--l-*` / `--d-*` pairs on this
 * element, and the `.skin-scope` class (see globals.css) is what resolves them
 * into live tokens for the subtree. So everything inside is written in plain
 * token classes - `bg-card`, `text-muted-foreground`, `rounded-lg` - and each
 * tile paints itself: wallpaper, radius, bevel, panel texture and font all
 * arrive for free, and a new skin needs no change here at all.
 *
 * The contents are rendered at double size and scaled to half, because a skin's
 * wallpaper is built at page scale: a 190px starfield tile or a 26px deco
 * cornice would otherwise be cropped to a flat wash by a tile this small.
 */
function SkinPreview({ id }: { id: ThemeId }) {
  return (
    <span
      className="ring-foreground/10 relative block h-20 overflow-hidden rounded-[min(var(--radius-md),10px)] ring-1"
      aria-hidden
    >
      <span
        data-theme={id}
        className="skin-scope absolute top-0 left-0 flex h-[200%] w-[200%] origin-top-left scale-50"
      >
        {/* the rail */}
        <span className="bg-sidebar text-sidebar-foreground flex w-14 shrink-0 flex-col gap-2 p-3">
          <span className="bg-sidebar-primary h-4 w-4 rounded-sm" />
          <span className="bg-sidebar-foreground/40 h-1.5 w-full rounded-full" />
          <span className="bg-sidebar-foreground/25 h-1.5 w-3/4 rounded-full" />
        </span>

        {/* the content */}
        <span className="flex min-w-0 flex-1 items-center p-3">
          <span className="bg-card text-card-foreground flex w-full min-w-0 flex-col gap-2 rounded-lg border p-3">
            <span className="truncate text-[13px] leading-none font-semibold">screenshot.png</span>
            <span className="bg-muted-foreground/50 h-1.5 w-2/3 rounded-full" />
            <span className="bg-primary text-primary-foreground w-fit rounded-md px-2 py-1 text-[10px] leading-none font-semibold">
              Aa
            </span>
          </span>
        </span>
      </span>
    </span>
  );
}

const AUTO_VALUE = "auto";

/**
 * Repaint the document with a skin right now.
 *
 * `data-theme` on <html> is what every token in globals.css keys off, so setting
 * it directly is what makes a click feel instant — the server round-trip, the
 * cookie and the re-render then confirm it. Without this the picker only moved
 * its own checkmark and the page kept its old colours until a reload, which read
 * as "clicking themes does nothing".
 */
function applySkin(id: string): void {
  document.documentElement.setAttribute("data-theme", id);
}

export function AppearanceSection({ theme, locale }: AppearanceSectionProps) {
  const router = useRouter();
  const t = useTranslations("profile");
  const localeLabel: Record<string, string> = {
    [AUTO_VALUE]: t("auto"),
    pl: "Polski",
    en: "English",
  };
  const [pending, startTransition] = useTransition();
  // Normalized, so an account still holding a retired id ("modern") lights up
  // the swatch it actually renders as instead of showing nothing selected.
  const [activeTheme, setActiveTheme] = useState<string>(() => resolveTheme(theme));
  const [savingTheme, setSavingTheme] = useState<string | null>(null);
  const [activeLocale, setActiveLocale] = useState<string>(locale ?? AUTO_VALUE);

  function pickTheme(id: ThemeId) {
    if (id === activeTheme || pending) return;
    const previous = activeTheme;
    setActiveTheme(id);
    setSavingTheme(id);
    applySkin(id);
    startTransition(async () => {
      try {
        // The action reports what the database actually holds now, so a write
        // that stored something else (or nothing) can't be mistaken for success.
        const saved = await updatePreferences({ theme: id });
        setActiveTheme(saved.theme);
        applySkin(saved.theme);
        toast.success(t("themeUpdated"));
        router.refresh();
      } catch {
        setActiveTheme(previous);
        applySkin(previous);
        toast.error(t("themeError"));
      } finally {
        setSavingTheme(null);
      }
    });
  }

  function pickLocale(value: string) {
    if (value === activeLocale || pending) return;
    const previous = activeLocale;
    setActiveLocale(value);
    startTransition(async () => {
      try {
        await updatePreferences({ locale: value === AUTO_VALUE ? null : value });
        toast.success(t("languageUpdated"));
        router.refresh();
      } catch {
        setActiveLocale(previous);
        toast.error(t("languageError"));
      }
    });
  }

  return (
    <Card>
      <CardHeader className="border-b">
        <CardTitle className="flex items-center gap-2">
          <Palette className="text-primary size-4" />
          {t("appearance")}
        </CardTitle>
        <CardDescription>
          {t("appearanceDesc")}
        </CardDescription>
      </CardHeader>

      <CardContent className="space-y-6">
        <div className="space-y-3">
          <div className="space-y-1">
            <h3 className="text-sm font-medium">{t("theme")}</h3>
            <p className="text-muted-foreground flex items-start gap-1.5 text-xs leading-relaxed">
              <Eye className="mt-0.5 size-3.5 shrink-0" />
              {t("themeNote")}
            </p>
          </div>

          <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
            {THEMES.map((themeOption) => {
              const selected = activeTheme === themeOption.id;
              const isSaving = savingTheme === themeOption.id;
              return (
                <button
                  key={themeOption.id}
                  type="button"
                  onClick={() => pickTheme(themeOption.id)}
                  disabled={pending}
                  aria-pressed={selected}
                  title={t(`themes.${themeOption.id}.desc`)}
                  className={cn(
                    "group relative flex flex-col gap-2 rounded-lg border p-3 text-left transition-colors",
                    "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
                    selected
                      ? "border-primary ring-1 ring-primary"
                      : "hover:border-foreground/20 hover:bg-accent/40",
                    pending && !isSaving && "opacity-60",
                  )}
                >
                  <SkinPreview id={themeOption.id} />

                  <span className="flex items-center justify-between gap-2">
                    <span className="truncate text-sm font-medium">
                      {t(`themes.${themeOption.id}.name`)}
                    </span>
                    {isSaving ? (
                      <Loader2 className="text-muted-foreground size-4 shrink-0 animate-spin" />
                    ) : selected ? (
                      <span className="bg-primary text-primary-foreground grid size-4 shrink-0 place-items-center rounded-full">
                        <Check className="size-3" />
                      </span>
                    ) : null}
                  </span>

                  <span className="text-muted-foreground line-clamp-2 text-xs leading-snug">
                    {t(`themes.${themeOption.id}.desc`)}
                  </span>
                </button>
              );
            })}
          </div>
        </div>

        <Separator />

        <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
          <div className="space-y-0.5">
            <Label htmlFor="locale" className="flex items-center gap-1.5">
              <Languages className="text-muted-foreground size-4" />
              {t("language")}
            </Label>
            <p className="text-muted-foreground text-sm leading-snug">
              {t("languageHelp")}
            </p>
          </div>
          <Select value={activeLocale} onValueChange={pickLocale} disabled={pending}>
            <SelectTrigger id="locale" className="w-full sm:w-48">
              <SelectValue>{localeLabel[activeLocale] ?? localeLabel[AUTO_VALUE]}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={AUTO_VALUE}>{t("auto")}</SelectItem>
              <SelectItem value="pl">Polski</SelectItem>
              <SelectItem value="en">English</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </CardContent>
    </Card>
  );
}
