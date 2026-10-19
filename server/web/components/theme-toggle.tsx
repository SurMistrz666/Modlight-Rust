"use client";

import { useEffect, useSyncExternalStore } from "react";
import { useTheme } from "next-themes";
import { useTranslations } from "next-intl";
import { Moon, Sun } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { MODE_COOKIE, THEME_COOKIE_MAX_AGE, resolveMode, type Mode } from "@/lib/theme-cookies";

const CYCLE: Record<Mode, Mode> = { light: "dark", dark: "system", system: "light" };

/** Hydration-safe "are we on the client yet" — no setState in an effect. */
const noopSubscribe = () => () => {};
const useMounted = () =>
  useSyncExternalStore(
    noopSubscribe,
    () => true,
    () => false,
  );

/**
 * Light → Dark → System, in that order.
 *
 * The icons are swapped in CSS (`dark:` follows the same `data-mode` signal the
 * tokens do) rather than from React state: the server cannot know the resolved
 * mode, so a state-driven icon would either mismatch on hydration or need a
 * blank placeholder. This way the very first paint is already right.
 */
export function ThemeToggle() {
  const t = useTranslations("common");
  const { theme, setTheme } = useTheme();
  const mounted = useMounted();

  // Keep the cookie the server reads in step with next-themes' localStorage —
  // including on first mount, which heals a cookie that was cleared.
  useEffect(() => {
    if (!theme) return;
    document.cookie = `${MODE_COOKIE}=${resolveMode(theme)};path=/;max-age=${THEME_COOKIE_MAX_AGE};samesite=lax`;
  }, [theme]);

  const mode = resolveMode(theme);
  const next = CYCLE[mode];

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          aria-label={t("theme.toggle")}
          className="relative"
          onClick={() => setTheme(next)}
        >
          <Sun className="size-4 dark:hidden" />
          <Moon className="hidden size-4 dark:block" />
          {/* Marks "follow the OS". Appears after mount only — the server has
              no way to know, and a 6px dot popping in beats a hydration error. */}
          <span
            aria-hidden
            data-visible={mounted && mode === "system"}
            className="bg-primary ring-background absolute right-1 bottom-1 size-1.5 rounded-full ring-2 opacity-0 transition-opacity data-[visible=true]:opacity-100"
          />
        </Button>
      </TooltipTrigger>
      <TooltipContent>
        <span className="flex flex-col gap-0.5">
          <span>{mounted ? t("theme.current", { mode: t(`theme.${mode}`) }) : t("theme.toggle")}</span>
          <span className="opacity-70">{t("theme.switchTo", { mode: t(`theme.${next}`) })}</span>
        </span>
      </TooltipContent>
    </Tooltip>
  );
}
