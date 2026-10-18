"use client";

import { Eye, PenLine, ShieldAlert, Upload, Check } from "lucide-react";
import { useTranslations } from "next-intl";
import { TOKEN_SCOPES, type TokenScope } from "@/lib/api-scopes";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

/**
 * Choosing what a token may do, and showing what it may do afterwards.
 *
 * The picker is a list of four rows rather than a `<select>` on purpose. A
 * dropdown shows one option at a time, so the person choosing sees the name of
 * the scope they landed on and never the sentence that explains it — and a
 * choice made without reading the sentence is the choice that ends up as `full`
 * for everything, which is the state this whole feature exists to leave behind.
 *
 * Imported from lib/api-scopes.ts, not lib/api-token.ts: this is a client
 * component, and the latter reaches @/lib/db.
 */

const ICONS: Record<TokenScope, React.ComponentType<{ className?: string }>> = {
  upload: Upload,
  read: Eye,
  write: PenLine,
  full: ShieldAlert,
};

/** `full` is the one that deserves a warning colour: it is the whole account. */
const TONE: Record<TokenScope, string> = {
  upload: "text-emerald-600 dark:text-emerald-400",
  read: "text-sky-600 dark:text-sky-400",
  write: "text-amber-600 dark:text-amber-400",
  full: "text-destructive",
};

export function ScopeBadge({ scope, className }: { scope: TokenScope; className?: string }) {
  const t = useTranslations("tokens.scopes");
  const Icon = ICONS[scope];
  return (
    <Badge
      variant={scope === "full" ? "destructive" : "outline"}
      className={cn("gap-1", className)}
      title={t(`${scope}.desc`)}
    >
      <Icon className="size-3" />
      {t(`${scope}.label`)}
    </Badge>
  );
}

interface ScopePickerProps {
  value: TokenScope;
  onChange: (scope: TokenScope) => void;
  /**
   * Which scopes may be picked. Omitted means all four (creating a token);
   * supplied means "only these" (narrowing an existing one, where anything
   * wider is refused by the server anyway and must not be offered here).
   */
  allowed?: readonly TokenScope[];
  disabled?: boolean;
}

export function ScopePicker({ value, onChange, allowed, disabled }: ScopePickerProps) {
  const t = useTranslations("tokens.scopes");
  const options = allowed ?? TOKEN_SCOPES;

  return (
    <div className="grid gap-1.5" role="radiogroup" aria-label={t("legend")}>
      {options.map((scope) => {
        const Icon = ICONS[scope];
        const selected = scope === value;
        return (
          <button
            key={scope}
            type="button"
            role="radio"
            aria-checked={selected}
            disabled={disabled}
            onClick={() => onChange(scope)}
            className={cn(
              "flex w-full items-start gap-3 rounded-lg border p-3 text-left transition-colors",
              "focus-visible:ring-ring/50 focus-visible:ring-[3px] focus-visible:outline-none",
              selected ? "border-primary bg-primary/5" : "border-border hover:bg-muted/50",
              disabled && "pointer-events-none opacity-60",
            )}
          >
            <Icon className={cn("mt-0.5 size-4 shrink-0", TONE[scope])} />
            <span className="min-w-0 flex-1">
              <span className="flex items-center gap-2 text-sm font-medium">
                {t(`${scope}.label`)}
                {selected && <Check className="text-primary size-3.5" />}
              </span>
              <span className="text-muted-foreground mt-0.5 block text-xs leading-relaxed">
                {t(`${scope}.desc`)}
              </span>
            </span>
          </button>
        );
      })}
    </div>
  );
}
