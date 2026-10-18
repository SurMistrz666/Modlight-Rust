"use client";

import { useRouter, useSearchParams, usePathname } from "next/navigation";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState, useTransition } from "react";
import { useTranslations } from "next-intl";
import {
  ArrowDownUp,
  CalendarDays,
  Check,
  Eye,
  HardDrive,
  LayoutGrid,
  List,
  Loader2,
  Search,
  SlidersHorizontal,
  UploadCloud,
  X,
} from "lucide-react";
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
import type { SortKey } from "@/lib/data/uploads";

const SORT_KEYS: SortKey[] = ["newest", "oldest", "largest", "smallest", "name", "views"];

/** Kinds offered as quick chips, in the order they matter for this library. */
const KIND_CHIPS = ["image", "video", "audio", "document", "archive", "paste", "other"] as const;

/** Minimum-size presets. Values are the literal `size` URL parameter. */
export const SIZE_PRESETS = {
  "10mb": 10 * 1024 * 1024,
  "100mb": 100 * 1024 * 1024,
  "1gb": 1024 * 1024 * 1024,
} as const;

export type SizePreset = keyof typeof SIZE_PRESETS;

const DATE_PRESETS = ["today", "7d", "30d", "365d"] as const;
type DatePreset = (typeof DATE_PRESETS)[number];

/** Local YYYY-MM-DD — the shape the `after` / `before` params carry. */
function isoDay(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

function datePresetValue(preset: DatePreset): string {
  const d = new Date();
  d.setHours(0, 0, 0, 0);
  if (preset === "7d") d.setDate(d.getDate() - 7);
  if (preset === "30d") d.setDate(d.getDate() - 30);
  if (preset === "365d") d.setFullYear(d.getFullYear() - 1);
  return isoDay(d);
}

export interface ToolbarProps {
  search: string;
  sort: SortKey;
  view: "grid" | "list";
  /** "" = every kind. */
  kind: string;
  /** "" | "public" | "private" */
  visibility: string;
  /** "" | keyof SIZE_PRESETS */
  size: string;
  /** "" | YYYY-MM-DD */
  after: string;
  before: string;
  /** Per-kind counts for the chips. */
  counts: Record<string, number>;
  /** A folder view is open, so "include subfolders" is meaningful. */
  folderActive?: boolean;
  includeSubfolders?: boolean;
}

export function Toolbar({
  search,
  sort,
  view,
  kind,
  visibility,
  size,
  after,
  before,
  counts,
  folderActive = false,
  includeSubfolders = false,
}: ToolbarProps) {
  const t = useTranslations("dashboard");
  const tNav = useTranslations("nav");
  const router = useRouter();
  const pathname = usePathname();
  const params = useSearchParams();
  const [pending, startTransition] = useTransition();
  const [term, setTerm] = useState(search);
  const debounce = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Keep local input in sync when navigated externally (e.g. clearing).
  useEffect(() => setTerm(search), [search]);
  useEffect(() => () => {
    if (debounce.current) clearTimeout(debounce.current);
  }, []);

  const push = useCallback(
    (mutate: (p: URLSearchParams) => void) => {
      const next = new URLSearchParams(params.toString());
      mutate(next);
      next.delete("page"); // any filter change resets paging
      startTransition(() => {
        router.push(`${pathname}?${next.toString()}`, { scroll: false });
      });
    },
    [params, pathname, router],
  );

  const onSearch = useCallback(
    (value: string) => {
      setTerm(value);
      if (debounce.current) clearTimeout(debounce.current);
      debounce.current = setTimeout(() => {
        push((p) => {
          if (value.trim()) p.set("search", value.trim());
          else p.delete("search");
        });
      }, 300);
    },
    [push],
  );

  function setParam(key: string, value: string | null) {
    push((p) => {
      if (value) p.set(key, value);
      else p.delete(key);
    });
  }

  function setView(v: "grid" | "list") {
    setParam("view", v === "grid" ? null : v);
  }

  const activeDatePreset = useMemo<DatePreset | null>(() => {
    if (!after) return null;
    return DATE_PRESETS.find((p) => datePresetValue(p) === after) ?? null;
  }, [after]);

  const chips = useMemo(() => {
    const out: { key: string; label: string; clear: string[] }[] = [];
    if (search) {
      out.push({ key: "search", label: t("filters.chipSearch", { term: search }), clear: ["search"] });
    }
    if (kind) {
      out.push({ key: "kind", label: tNav(`kind.${kind}`), clear: ["kind"] });
    }
    if (visibility === "public" || visibility === "private") {
      out.push({
        key: "visibility",
        label: t(`filters.${visibility}`),
        clear: ["visibility"],
      });
    }
    if (size) {
      out.push({ key: "size", label: t("filters.chipSize", { size: sizeLabel(size) }), clear: ["size"] });
    }
    if (after || before) {
      out.push({
        key: "date",
        label: activeDatePreset
          ? t(`filters.date.${activeDatePreset}`)
          : after
            ? t("filters.chipSince", { date: after })
            : t("filters.chipUntil", { date: before }),
        clear: ["after", "before"],
      });
    }
    return out;
  }, [search, kind, visibility, size, after, before, activeDatePreset, t, tNav]);

  const advancedCount =
    (visibility ? 1 : 0) + (size ? 1 : 0) + (after || before ? 1 : 0) + (includeSubfolders ? 1 : 0);

  function clearAll() {
    push((p) => {
      for (const key of ["search", "kind", "visibility", "size", "after", "before", "sub"]) {
        p.delete(key);
      }
    });
    setTerm("");
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="bg-card/60 ring-foreground/10 relative flex flex-col gap-2 overflow-hidden rounded-xl p-2 ring-1">
        {/* Loading rail — the one honest signal that a filter is in flight. */}
        <div
          aria-hidden
          className={cn(
            "bg-primary/70 absolute inset-x-0 top-0 h-0.5 origin-left transition-opacity",
            pending ? "animate-pulse opacity-100" : "opacity-0",
          )}
        />

        <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
          {/* Search */}
          <div className="group/search relative flex-1">
            {pending ? (
              <Loader2 className="text-primary pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 animate-spin" />
            ) : (
              <Search className="text-muted-foreground group-focus-within/search:text-foreground pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 transition-colors" />
            )}
            <Input
              value={term}
              onChange={(e) => onSearch(e.target.value)}
              placeholder={t("toolbar.searchPlaceholder")}
              className="bg-background/80 h-9 border-transparent pr-8 pl-9 shadow-none focus-visible:border-transparent max-sm:h-11"
              aria-label={t("toolbar.searchAria")}
            />
            {term ? (
              <button
                type="button"
                onClick={() => onSearch("")}
                aria-label={t("toolbar.clearSearch")}
                className="text-muted-foreground hover:text-foreground hover:bg-muted absolute top-1/2 right-2 grid size-6 -translate-y-1/2 place-items-center rounded-md transition-colors"
              >
                <X className="size-4" />
              </button>
            ) : null}
          </div>

          {/* Wraps rather than overflows: on a 360 px phone the four controls
              below do not fit on one line, and a toolbar that pushes the page
              sideways is worse than one that takes two rows. */}
          <div className="flex flex-wrap items-center gap-2">
            {/* Advanced filters */}
            <Popover>
              <PopoverTrigger asChild>
                <Button
                  variant="outline"
                  size="sm"
                  aria-label={t("filters.title")}
                  className="h-9 max-sm:h-11 max-sm:min-w-11"
                >
                  <SlidersHorizontal className="size-3.5" />
                  <span className="hidden sm:inline">{t("filters.title")}</span>
                  {advancedCount > 0 ? (
                    <span className="bg-primary text-primary-foreground ml-0.5 grid size-4 place-items-center rounded-full text-[0.625rem] font-semibold tabular-nums">
                      {advancedCount}
                    </span>
                  ) : null}
                </Button>
              </PopoverTrigger>
              <PopoverContent align="end" className="w-72 gap-3 p-3">
                <FilterGroup
                  icon={<Eye className="size-3.5" />}
                  label={t("filters.visibility")}
                  options={[
                    { value: "", label: t("filters.any") },
                    { value: "public", label: t("filters.public") },
                    { value: "private", label: t("filters.private") },
                  ]}
                  value={visibility}
                  onChange={(v) => setParam("visibility", v || null)}
                />

                <FilterGroup
                  icon={<CalendarDays className="size-3.5" />}
                  label={t("filters.uploaded")}
                  options={[
                    { value: "", label: t("filters.anyTime") },
                    ...DATE_PRESETS.map((p) => ({ value: p, label: t(`filters.date.${p}`) })),
                  ]}
                  value={activeDatePreset ?? ""}
                  onChange={(v) =>
                    push((p) => {
                      p.delete("before");
                      if (v) p.set("after", datePresetValue(v as DatePreset));
                      else p.delete("after");
                    })
                  }
                />

                <FilterGroup
                  icon={<HardDrive className="size-3.5" />}
                  label={t("filters.size")}
                  options={[
                    { value: "", label: t("filters.anySize") },
                    ...Object.keys(SIZE_PRESETS).map((k) => ({
                      value: k,
                      label: `> ${sizeLabel(k)}`,
                    })),
                  ]}
                  value={size}
                  onChange={(v) => setParam("size", v || null)}
                />

                {folderActive ? (
                  <label className="hover:bg-accent/60 flex cursor-pointer items-center justify-between gap-3 rounded-lg px-2 py-1.5 text-sm transition-colors">
                    <span className="min-w-0">
                      <span className="font-medium">{t("filters.includeSubfolders")}</span>
                      <span className="text-muted-foreground block text-xs">
                        {t("filters.includeSubfoldersHint")}
                      </span>
                    </span>
                    <input
                      type="checkbox"
                      className="accent-primary size-4 shrink-0"
                      checked={includeSubfolders}
                      onChange={(e) => setParam("sub", e.target.checked ? "1" : null)}
                    />
                  </label>
                ) : null}

                {advancedCount > 0 ? (
                  <Button variant="ghost" size="sm" className="justify-start" onClick={clearAll}>
                    <X className="size-3.5" /> {t("filters.clearAll")}
                  </Button>
                ) : null}
              </PopoverContent>
            </Popover>

            {/* Sort */}
            <Select value={sort} onValueChange={(v) => setParam("sort", v === "newest" ? null : v)}>
              <SelectTrigger className="h-9 min-w-0 flex-1 max-sm:h-11 sm:min-w-[9.5rem] sm:flex-none">
                <ArrowDownUp className="size-3.5" />
                <SelectValue />
              </SelectTrigger>
              <SelectContent align="end" position="popper">
                {SORT_KEYS.map((k) => (
                  <SelectItem key={k} value={k}>
                    {t(`sort.${k}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>

            {/* View toggle */}
            <div className="bg-background/80 ring-foreground/10 inline-flex h-9 items-center gap-0.5 rounded-lg p-0.5 ring-1 max-sm:h-11">
              <ToggleBtn
                active={view === "grid"}
                label={t("toolbar.gridView")}
                onClick={() => setView("grid")}
              >
                <LayoutGrid className="size-4" />
              </ToggleBtn>
              <ToggleBtn
                active={view === "list"}
                label={t("toolbar.listView")}
                onClick={() => setView("list")}
              >
                <List className="size-4" />
              </ToggleBtn>
            </div>

            {/* Upload CTA */}
            <Button asChild size="lg" className="shadow-sm max-sm:h-11 max-sm:min-w-11">
              <Link href="/dashboard/upload" aria-label={t("toolbar.upload")}>
                <UploadCloud className="size-4" />
                <span className="hidden sm:inline">{t("toolbar.upload")}</span>
              </Link>
            </Button>
          </div>
        </div>

        {/* Kind chips — the fastest cut through 6 800 screenshots */}
        {/* Scrolls sideways inside its own box rather than widening the page —
            `overscroll-x-contain` keeps the swipe from turning into a browser
            back-gesture on a phone. */}
        <div className="-mx-1 flex items-center gap-1.5 overflow-x-auto overscroll-x-contain px-1 pb-0.5 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
          <KindChip
            label={t("filters.allTypes")}
            active={!kind}
            onClick={() => setParam("kind", null)}
          />
          {KIND_CHIPS.map((k) => {
            const n = counts[k] ?? 0;
            if (!n && kind !== k) return null;
            return (
              <KindChip
                key={k}
                label={tNav(`kind.${k}`)}
                count={n}
                active={kind === k}
                onClick={() => setParam("kind", kind === k ? null : k)}
              />
            );
          })}
        </div>
      </div>

      {/* Active filters */}
      {chips.length > 0 ? (
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="text-muted-foreground/70 text-[0.6875rem] font-semibold tracking-[0.08em] uppercase">
            {t("filters.active")}
          </span>
          {chips.map((chip) => (
            <button
              key={chip.key}
              type="button"
              onClick={() =>
                push((p) => {
                  chip.clear.forEach((k) => p.delete(k));
                })
              }
              className="bg-primary/10 text-foreground ring-primary/20 hover:bg-primary/15 inline-flex max-w-full items-center gap-1.5 rounded-full py-1 pr-1.5 pl-2.5 text-xs font-medium ring-1 transition-colors max-sm:min-h-9 sm:max-w-[16rem]"
            >
              <span className="truncate">{chip.label}</span>
              <X className="size-3 shrink-0 opacity-60" />
            </button>
          ))}
          <button
            type="button"
            onClick={clearAll}
            className="text-muted-foreground hover:text-foreground ml-1 text-xs font-medium underline-offset-2 transition-colors hover:underline"
          >
            {t("filters.clearAll")}
          </button>
        </div>
      ) : null}
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Bits                                                                        */
/* -------------------------------------------------------------------------- */

function sizeLabel(key: string): string {
  switch (key) {
    case "10mb":
      return "10 MB";
    case "100mb":
      return "100 MB";
    case "1gb":
      return "1 GB";
    default:
      return key;
  }
}

function KindChip({
  label,
  count,
  active,
  onClick,
}: {
  label: string;
  count?: number;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={cn(
        "inline-flex shrink-0 items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-medium whitespace-nowrap transition-colors max-sm:min-h-9 max-sm:px-3.5",
        active
          ? "bg-primary text-primary-foreground"
          : "text-muted-foreground hover:bg-accent hover:text-foreground ring-foreground/10 ring-1",
      )}
    >
      {label}
      {typeof count === "number" && count > 0 ? (
        <span className={cn("tabular-nums", active ? "opacity-80" : "opacity-60")}>
          {count.toLocaleString()}
        </span>
      ) : null}
    </button>
  );
}

function FilterGroup({
  icon,
  label,
  options,
  value,
  onChange,
}: {
  icon: React.ReactNode;
  label: string;
  options: { value: string; label: string }[];
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <div className="flex flex-col gap-1">
      <p className="text-muted-foreground flex items-center gap-1.5 px-1 text-[0.6875rem] font-semibold tracking-[0.08em] uppercase">
        {icon}
        {label}
      </p>
      <div className="flex flex-col">
        {options.map((opt) => {
          const active = opt.value === value;
          return (
            <button
              key={opt.value || "__any__"}
              type="button"
              onClick={() => onChange(opt.value)}
              className={cn(
                "flex items-center justify-between rounded-md px-2 py-1.5 text-left text-sm transition-colors max-sm:min-h-10",
                active ? "bg-accent text-foreground font-medium" : "hover:bg-accent/60",
              )}
            >
              {opt.label}
              {active ? <Check className="text-primary size-3.5" /> : null}
            </button>
          );
        })}
      </div>
    </div>
  );
}

function ToggleBtn({
  active,
  label,
  onClick,
  children,
}: {
  active: boolean;
  label: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "grid size-8 place-items-center rounded-md transition-colors max-sm:size-10",
        active
          ? "bg-background text-foreground shadow-sm"
          : "text-muted-foreground hover:text-foreground",
      )}
    >
      {children}
    </button>
  );
}
