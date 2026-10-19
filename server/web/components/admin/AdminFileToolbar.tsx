"use client";

import { useRouter, usePathname, useSearchParams } from "next/navigation";
import { useEffect, useState, useTransition } from "react";
import { useTranslations } from "next-intl";
import { LayoutGrid, List, Loader2, Search, Trash2, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { FileKind, SortKey } from "@/lib/data/uploads";

const SORT_KEYS: SortKey[] = ["newest", "oldest", "largest", "smallest", "name", "views"];
const KINDS: FileKind[] = ["image", "video", "audio", "document", "archive", "paste", "other"];

/**
 * Filters for the administrative file browser.
 *
 * Deliberately smaller than the owner's own Toolbar: this view is for answering
 * "what is on this account", not for working in it, and the owner's toolbar
 * carries an upload button and folder scoping that mean nothing here.
 */
export function AdminFileToolbar({
  search,
  sort,
  kind,
  view,
  trashed,
}: {
  search: string;
  sort: SortKey;
  /** "" = every kind. */
  kind: string;
  view: "grid" | "list";
  trashed: boolean;
}) {
  const t = useTranslations("admin.files");
  // The owner's own toolbar strings, reused so both views name the same things
  // the same way.
  const td = useTranslations("dashboard");
  const tNav = useTranslations("nav");
  const router = useRouter();
  const pathname = usePathname();
  const params = useSearchParams();
  const [pending, startTransition] = useTransition();
  const [term, setTerm] = useState(search);

  // The URL is the source of truth; the field follows it (back button, reset).
  useEffect(() => setTerm(search), [search]);

  function navigate(mutate: (p: URLSearchParams) => void) {
    const next = new URLSearchParams(params.toString());
    mutate(next);
    // Any change to the query invalidates the page number that came with it.
    next.delete("page");
    const query = next.toString();
    startTransition(() => {
      router.push(query ? `${pathname}?${query}` : pathname);
    });
  }

  function setParam(key: string, value: string) {
    navigate((p) => (value ? p.set(key, value) : p.delete(key)));
  }

  return (
    <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
      <form
        className="relative w-full lg:max-w-xs"
        onSubmit={(e) => {
          e.preventDefault();
          setParam("search", term.trim());
        }}
      >
        <Search className="text-muted-foreground pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2" />
        <Input
          value={term}
          onChange={(e) => setTerm(e.target.value)}
          placeholder={t("searchPlaceholder")}
          className="pl-8"
          aria-label={t("searchPlaceholder")}
        />
        {term ? (
          <button
            type="button"
            onClick={() => {
              setTerm("");
              setParam("search", "");
            }}
            aria-label={td("toolbar.clearSearch")}
            className="text-muted-foreground hover:text-foreground absolute top-1/2 right-2 -translate-y-1/2"
          >
            <X className="size-4" />
          </button>
        ) : null}
      </form>

      <div className="flex flex-wrap items-center gap-2">
        {pending ? <Loader2 className="text-muted-foreground size-4 animate-spin" /> : null}

        <Select value={kind || "all"} onValueChange={(v) => setParam("kind", v === "all" ? "" : v)}>
          <SelectTrigger size="sm" className="w-36">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">{t("allKinds")}</SelectItem>
            {KINDS.map((k) => (
              <SelectItem key={k} value={k}>
                {tNav(`kind.${k}`)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        <Select value={sort} onValueChange={(v) => setParam("sort", v)}>
          <SelectTrigger size="sm" className="w-36">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {SORT_KEYS.map((key) => (
              <SelectItem key={key} value={key}>
                {td(`sort.${key}`)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        <Button
          type="button"
          size="sm"
          variant={trashed ? "secondary" : "outline"}
          onClick={() => setParam("trashed", trashed ? "" : "1")}
        >
          <Trash2 className="size-4" /> {t("showTrash")}
        </Button>

        <div className="ring-foreground/10 flex items-center rounded-md ring-1">
          <Button
            type="button"
            size="icon-sm"
            variant="ghost"
            aria-pressed={view === "grid"}
            aria-label={td("toolbar.gridView")}
            className={cn("rounded-r-none", view === "grid" && "bg-muted")}
            onClick={() => setParam("view", "")}
          >
            <LayoutGrid className="size-4" />
          </Button>
          <Button
            type="button"
            size="icon-sm"
            variant="ghost"
            aria-pressed={view === "list"}
            aria-label={td("toolbar.listView")}
            className={cn("rounded-l-none", view === "list" && "bg-muted")}
            onClick={() => setParam("view", "list")}
          >
            <List className="size-4" />
          </Button>
        </div>
      </div>
    </div>
  );
}
