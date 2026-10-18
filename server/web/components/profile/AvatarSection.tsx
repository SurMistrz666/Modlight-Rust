"use client";

import { useCallback, useEffect, useRef, useState, useTransition } from "react";
import { useRouter } from "next/navigation";
import {
  ChevronLeft,
  ChevronRight,
  EyeOff,
  ImageIcon,
  Loader2,
  Search,
  Trash2,
  Upload,
  UserRound,
} from "lucide-react";
import { toast } from "sonner";
import { useTranslations } from "next-intl";
import { cn } from "@/lib/utils";
import { humanFileSize } from "@/lib/format";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { AVATAR_ACCEPT, AVATAR_MAX_BYTES } from "@/lib/avatar";
import { listAvatarPicks, setAvatarFromUpload, type AvatarPick } from "@/app/actions/avatar";

interface Props {
  /** Current avatar URL, or null when the account still shows initials. */
  url: string | null;
  /** Fallback text: the same two letters every other avatar in the panel draws. */
  initials: string;
}

/** Degradation notes the server can report, mapped to a message key. */
const DEGRADED_KEYS = {
  frames: "degradedFrames",
  pixels: "degradedPixels",
  "output-size": "degradedSize",
} as const;

export function AvatarSection({ url: initialUrl, initials }: Props) {
  const router = useRouter();
  const t = useTranslations("profile.avatar");
  const fileRef = useRef<HTMLInputElement>(null);

  const [url, setUrl] = useState<string | null>(initialUrl);
  const [busy, setBusy] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);

  /** Announce a successful save, plus the reason motion was dropped if it was. */
  const announce = useCallback(
    (degraded: keyof typeof DEGRADED_KEYS | null) => {
      toast.success(t("saved"));
      // Never a silent freeze: a GIF that arrives still without explanation
      // reads as a bug in the panel, which is the mistake Round 12 closed on the
      // conversion menu.
      if (degraded) toast.warning(t(DEGRADED_KEYS[degraded]));
    },
    [t],
  );

  async function onPickFile(file: File) {
    if (file.size > AVATAR_MAX_BYTES) {
      toast.error(t("tooLarge"));
      return;
    }
    setBusy(true);
    try {
      // The raw file as the body, not multipart: there is exactly one field, and
      // Server Actions cap bodies at 1 MB, which no photograph respects.
      const res = await fetch("/api/avatar", {
        method: "POST",
        headers: { "Content-Type": file.type },
        body: file,
      });
      const body = (await res.json().catch(() => null)) as
        | { url?: string | null; degraded?: keyof typeof DEGRADED_KEYS | null }
        | null;

      if (!res.ok) {
        toast.error(t(errorKey(res.status)));
        return;
      }
      setUrl(body?.url ?? null);
      announce(body?.degraded ?? null);
      router.refresh();
    } catch {
      toast.error(t("uploadError"));
    } finally {
      setBusy(false);
      if (fileRef.current) fileRef.current.value = "";
    }
  }

  async function onChoose(id: string) {
    setBusy(true);
    try {
      const result = await setAvatarFromUpload(id);
      if (!result.ok) {
        toast.error(t(pickErrorKey(result.error)));
        return;
      }
      setUrl(result.url);
      setPickerOpen(false);
      announce(result.degraded);
      router.refresh();
    } catch {
      toast.error(t("uploadError"));
    } finally {
      setBusy(false);
    }
  }

  async function onRemove() {
    setBusy(true);
    try {
      const res = await fetch("/api/avatar", { method: "DELETE" });
      if (!res.ok) {
        toast.error(t("removeError"));
        return;
      }
      setUrl(null);
      toast.success(t("removed"));
      router.refresh();
    } catch {
      toast.error(t("removeError"));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card>
      <CardHeader className="border-b">
        <CardTitle className="flex items-center gap-2">
          <UserRound className="text-primary size-4" />
          {t("title")}
        </CardTitle>
        <CardDescription>{t("desc")}</CardDescription>
      </CardHeader>

      <CardContent className="space-y-5">
        <div className="flex items-center gap-4">
          {/* The real component at a real size, so what you see here is exactly
              what the rail and the admin list will draw. */}
          <Avatar className="size-20">
            {url ? <AvatarImage src={url} alt="" /> : null}
            <AvatarFallback className="text-xl font-semibold">{initials}</AvatarFallback>
          </Avatar>

          <div className="flex min-w-0 flex-col gap-1">
            <div className="flex items-center gap-2">
              <Avatar className="size-8">
                {url ? <AvatarImage src={url} alt="" /> : null}
                <AvatarFallback className="text-xs font-semibold">{initials}</AvatarFallback>
              </Avatar>
              <Avatar className="size-7">
                {url ? <AvatarImage src={url} alt="" /> : null}
                <AvatarFallback className="text-xs font-semibold">{initials}</AvatarFallback>
              </Avatar>
            </div>
            <p className="text-muted-foreground text-xs leading-relaxed">{t("sizesNote")}</p>
          </div>
        </div>

        <input
          ref={fileRef}
          type="file"
          accept={AVATAR_ACCEPT}
          className="hidden"
          onChange={(e) => {
            const file = e.target.files?.[0];
            if (file) void onPickFile(file);
          }}
        />

        <div className="flex flex-wrap gap-2">
          <Button type="button" size="sm" disabled={busy} onClick={() => fileRef.current?.click()}>
            {busy ? <Loader2 className="size-4 animate-spin" /> : <Upload className="size-4" />}
            {url ? t("replace") : t("choose")}
          </Button>

          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() => setPickerOpen(true)}
          >
            <ImageIcon className="size-4" />
            {t("fromFiles")}
          </Button>

          {url && (
            <Button type="button" size="sm" variant="outline" disabled={busy} onClick={onRemove}>
              <Trash2 className="size-4" />
              {t("remove")}
            </Button>
          )}
        </div>

        <p className="text-muted-foreground text-xs leading-relaxed">{t("note")}</p>
        <p className="text-muted-foreground text-xs leading-relaxed">{t("visibility")}</p>
      </CardContent>

      {/* Mounted only while open, which is also how its state is reset:
          reopening the picker on page 4 of a search somebody typed twenty
          minutes ago is never what was wanted. */}
      {pickerOpen && <AvatarPicker busy={busy} onOpenChange={setPickerOpen} onChoose={onChoose} />}
    </Card>
  );
}

/* -------------------------------------------------------------------------- */
/* Picker                                                                      */
/* -------------------------------------------------------------------------- */

function AvatarPicker({
  busy,
  onOpenChange,
  onChoose,
}: {
  busy: boolean;
  onOpenChange: (open: boolean) => void;
  onChoose: (id: string) => void;
}) {
  const t = useTranslations("profile.avatar");
  const [items, setItems] = useState<AvatarPick[]>([]);
  const [total, setTotal] = useState(0);
  const [page, setPage] = useState(1);
  const [perPage, setPerPage] = useState(24);
  const [search, setSearch] = useState("");
  const [loading, startLoading] = useTransition();

  const load = useCallback(
    (nextPage: number, query: string) => {
      startLoading(async () => {
        const result = await listAvatarPicks({ page: nextPage, search: query });
        setItems(result.items);
        setTotal(result.total);
        setPage(result.page);
        setPerPage(result.perPage);
      });
    },
    [],
  );

  // One effect for both the first load and every search after it. The first run
  // is immediate - a dialog that spends 300 ms empty reads as broken - and the
  // rest are debounced, so typing does not fire a query per keystroke.
  const firstRun = useRef(true);
  useEffect(() => {
    if (firstRun.current) {
      firstRun.current = false;
      load(1, "");
      return;
    }
    const id = setTimeout(() => load(1, search), 300);
    return () => clearTimeout(id);
  }, [search, load]);

  const pages = Math.max(1, Math.ceil(total / Math.max(1, perPage)));

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t("pickerTitle")}</DialogTitle>
          <DialogDescription>{t("pickerDesc")}</DialogDescription>
        </DialogHeader>

        <div className="relative">
          <Search className="text-muted-foreground pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2" />
          <Input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t("pickerSearch")}
            className="pl-8"
          />
        </div>

        <div className="min-h-[16rem]">
          {loading ? (
            <div className="text-muted-foreground grid h-64 place-items-center text-sm">
              <Loader2 className="size-5 animate-spin" />
            </div>
          ) : items.length === 0 ? (
            <div className="text-muted-foreground grid h-64 place-items-center gap-2 text-center text-sm">
              <ImageIcon className="mx-auto size-6 opacity-60" />
              {t("pickerEmpty")}
            </div>
          ) : (
            <div className="grid max-h-[22rem] grid-cols-3 gap-2 overflow-y-auto sm:grid-cols-4 md:grid-cols-6">
              {items.map((item) => (
                <button
                  key={item.id}
                  type="button"
                  disabled={busy || item.tooLarge}
                  title={item.tooLarge ? t("pickerTooLarge") : item.filename}
                  onClick={() => onChoose(item.id)}
                  className={cn(
                    "ring-border focus-visible:ring-ring relative aspect-square overflow-hidden rounded-lg ring-1 transition",
                    item.tooLarge
                      ? "cursor-not-allowed opacity-40"
                      : "hover:ring-primary hover:ring-2",
                  )}
                >
                  {/* eslint-disable-next-line @next/next/no-img-element */}
                  <img
                    src={item.thumbUrl}
                    alt={item.filename}
                    loading="lazy"
                    className="size-full object-cover"
                  />
                  {!item.published && (
                    <span
                      title={t("pickerPrivate")}
                      className="bg-background/80 text-muted-foreground absolute top-1 right-1 grid size-5 place-items-center rounded-full"
                    >
                      <EyeOff className="size-3" />
                    </span>
                  )}
                  {item.tooLarge && (
                    <span className="bg-background/80 text-foreground absolute inset-x-1 bottom-1 truncate rounded px-1 text-[0.6rem]">
                      {humanFileSize(item.size)}
                    </span>
                  )}
                </button>
              ))}
            </div>
          )}
        </div>

        <div className="flex items-center justify-between gap-2">
          <p className="text-muted-foreground text-xs">{t("pickerPrivateNote")}</p>
          {pages > 1 && (
            <div className="flex shrink-0 items-center gap-1">
              <Button
                type="button"
                size="icon"
                variant="outline"
                className="size-8"
                disabled={page <= 1 || loading}
                aria-label={t("pickerPrev")}
                onClick={() => load(page - 1, search)}
              >
                <ChevronLeft className="size-4" />
              </Button>
              <span className="text-muted-foreground px-1 text-xs tabular-nums">
                {page} / {pages}
              </span>
              <Button
                type="button"
                size="icon"
                variant="outline"
                className="size-8"
                disabled={page >= pages || loading}
                aria-label={t("pickerNext")}
                onClick={() => load(page + 1, search)}
              >
                <ChevronRight className="size-4" />
              </Button>
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

/* -------------------------------------------------------------------------- */

function errorKey(status: number): string {
  switch (status) {
    case 413:
      return "tooLarge";
    case 415:
      return "badType";
    case 503:
      return "busy";
    default:
      return "uploadError";
  }
}

function pickErrorKey(error: string): string {
  switch (error) {
    case "not-found":
      return "pickMissing";
    case "too-large":
      return "tooLarge";
    case "unsupported":
      return "badType";
    case "too-many-pixels":
      return "tooManyPixels";
    case "busy":
      return "busy";
    default:
      return "uploadError";
  }
}
