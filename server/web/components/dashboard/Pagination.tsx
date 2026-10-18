"use client";

import { useRouter, useSearchParams, usePathname } from "next/navigation";
import { useState, useTransition } from "react";
import { useTranslations } from "next-intl";
import { ChevronLeft, ChevronRight, CornerDownLeft, Rows3 } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

/** Page sizes offered in the toolbar. Mirrored by the dashboard's validation. */
export const PER_PAGE_OPTIONS = [30, 60, 120] as const;
export const DEFAULT_PER_PAGE = 30;

/** Compact page list: 1 … 4 5 [6] 7 8 … 228 */
function pageRange(current: number, totalPages: number): (number | "…")[] {
  if (totalPages <= 7) {
    return Array.from({ length: totalPages }, (_, i) => i + 1);
  }
  const out: (number | "…")[] = [1];
  const start = Math.max(2, current - 1);
  const end = Math.min(totalPages - 1, current + 1);
  if (start > 2) out.push("…");
  for (let p = start; p <= end; p++) out.push(p);
  if (end < totalPages - 1) out.push("…");
  out.push(totalPages);
  return out;
}

export function Pagination({
  total,
  page,
  perPage,
}: {
  total: number;
  page: number;
  perPage: number;
}) {
  const t = useTranslations("dashboard");
  const router = useRouter();
  const pathname = usePathname();
  const params = useSearchParams();
  const [pending, startTransition] = useTransition();

  const totalPages = Math.max(1, Math.ceil(total / perPage));
  const from = (page - 1) * perPage + 1;
  const to = Math.min(total, page * perPage);

  function navigate(mutate: (p: URLSearchParams) => void) {
    const next = new URLSearchParams(params.toString());
    mutate(next);
    startTransition(() => {
      router.push(`${pathname}?${next.toString()}`, { scroll: false });
    });
  }

  function go(p: number) {
    const clamped = Math.min(totalPages, Math.max(1, p));
    navigate((next) => {
      if (clamped <= 1) next.delete("page");
      else next.set("page", String(clamped));
    });
  }

  function setPerPage(value: string) {
    navigate((next) => {
      // Anchor on the first item currently on screen so changing the page size
      // keeps the user roughly where they were instead of teleporting to 1.
      const size = Number(value) || DEFAULT_PER_PAGE;
      const anchorPage = Math.max(1, Math.ceil(from / size));
      if (size === DEFAULT_PER_PAGE) next.delete("per");
      else next.set("per", String(size));
      if (anchorPage <= 1) next.delete("page");
      else next.set("page", String(anchorPage));
    });
  }

  if (total === 0) return null;

  const items = pageRange(page, totalPages);
  const showControls = totalPages > 1;

  return (
    <div
      aria-busy={pending || undefined}
      className={cn(
        "flex flex-col items-center justify-between gap-3 sm:flex-row",
        pending && "opacity-70",
      )}
    >
      <div className="flex items-center gap-3 max-sm:w-full max-sm:justify-between">
        <p className="text-muted-foreground text-xs">
          {t.rich("pagination.showing", {
            from,
            to,
            total: total.toLocaleString(),
            v: (chunks) => <span className="text-foreground font-medium tabular-nums">{chunks}</span>,
          })}
        </p>

        {total > PER_PAGE_OPTIONS[0] ? (
          <Select value={String(perPage)} onValueChange={setPerPage}>
            <SelectTrigger
              className="h-8 w-auto gap-1.5 max-sm:h-11"
              aria-label={t("pagination.perPage")}
            >
              <Rows3 className="size-3.5" />
              <SelectValue />
            </SelectTrigger>
            <SelectContent align="start" position="popper">
              {PER_PAGE_OPTIONS.map((n) => (
                <SelectItem key={n} value={String(n)}>
                  {t("pagination.perPageOption", { count: n })}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        ) : null}
      </div>

      {showControls ? (
        <div className="flex items-center gap-1 max-sm:w-full max-sm:justify-between">
          <Button
            variant="outline"
            size="sm"
            onClick={() => go(page - 1)}
            disabled={page <= 1 || pending}
            aria-label={t("pagination.previousPage")}
            className="max-sm:h-11 max-sm:min-w-11"
          >
            <ChevronLeft className="size-4" />
            <span className="hidden sm:inline">{t("pagination.prev")}</span>
          </Button>

          {/* Phone: one tappable "3 / 228" instead of a numbered strip that
              would either wrap or push the page sideways. */}
          <div className="sm:hidden">
            <JumpToPage page={page} totalPages={totalPages} onJump={go} compact />
          </div>

          <div className="hidden items-center gap-1 sm:flex">
            {items.map((it, i) =>
              it === "…" ? (
                // The gap is the jump affordance: 228 pages is too many to walk.
                <JumpToPage
                  key={`gap-${i}`}
                  page={page}
                  totalPages={totalPages}
                  onJump={go}
                />
              ) : (
                <button
                  key={it}
                  type="button"
                  onClick={() => go(it)}
                  disabled={pending}
                  aria-current={it === page ? "page" : undefined}
                  className={cn(
                    "grid size-8 place-items-center rounded-lg text-sm font-medium tabular-nums transition-colors",
                    it === page
                      ? "bg-primary text-primary-foreground"
                      : "text-muted-foreground hover:bg-muted hover:text-foreground",
                  )}
                >
                  {it}
                </button>
              ),
            )}
          </div>

          <Button
            variant="outline"
            size="sm"
            onClick={() => go(page + 1)}
            disabled={page >= totalPages || pending}
            aria-label={t("pagination.nextPage")}
            className="max-sm:h-11 max-sm:min-w-11"
          >
            <span className="hidden sm:inline">{t("pagination.next")}</span>
            <ChevronRight className="size-4" />
          </Button>
        </div>
      ) : null}
    </div>
  );
}

/**
 * "…" doubles as a jump box. With 228 pages, stepping through the numbered
 * strip is theatre — typing the page you want is the only real navigation.
 */
function JumpToPage({
  page,
  totalPages,
  onJump,
  compact = false,
}: {
  page: number;
  totalPages: number;
  onJump: (page: number) => void;
  /** Renders as "page / total" — the whole navigator on a narrow screen. */
  compact?: boolean;
}) {
  const t = useTranslations("dashboard");
  const [open, setOpen] = useState(false);
  const [value, setValue] = useState(String(page));

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (next) setValue(String(page));
      }}
    >
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={t("pagination.jumpTo")}
          title={t("pagination.jumpTo")}
          className={cn(
            "text-muted-foreground hover:bg-muted hover:text-foreground grid place-items-center rounded-lg text-sm transition-colors",
            compact
              ? "ring-foreground/10 h-11 px-3 font-medium tabular-nums ring-1"
              : "size-8",
          )}
        >
          {compact ? t("pagination.pageOf", { page, total: totalPages.toLocaleString() }) : "…"}
        </button>
      </PopoverTrigger>
      <PopoverContent align="center" className="w-56 gap-2 p-2.5">
        <p className="text-muted-foreground px-0.5 text-xs font-medium">
          {t("pagination.jumpHint", { total: totalPages.toLocaleString() })}
        </p>
        <form
          className="flex items-center gap-1.5"
          onSubmit={(e) => {
            e.preventDefault();
            const n = Number(value);
            if (!Number.isFinite(n) || n < 1) return;
            setOpen(false);
            onJump(Math.trunc(n));
          }}
        >
          <Input
            autoFocus
            type="number"
            inputMode="numeric"
            min={1}
            max={totalPages}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            aria-label={t("pagination.jumpTo")}
            className="h-8 flex-1 tabular-nums"
          />
          <Button type="submit" size="sm" className="h-8">
            <CornerDownLeft className="size-3.5" />
            {t("pagination.go")}
          </Button>
        </form>
      </PopoverContent>
    </Popover>
  );
}
