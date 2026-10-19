"use client";

import { useEffect, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import { ImageIcon, Loader2, Trash2, TriangleAlert, Upload } from "lucide-react";
import { toast } from "sonner";
import { useTranslations } from "next-intl";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  CardDescription,
} from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { myBackground, updateBackgroundDim } from "@/app/actions/account";
import {
  BACKGROUND_ACCEPT,
  BACKGROUND_MAX_BYTES,
  DIM_DEFAULT,
  DIM_MAX,
  DIM_MIN,
  DIM_SAFE,
} from "@/lib/background";

interface Props {
  /** Current wallpaper URL, or null. */
  url: string | null;
  /** Current veil strength, 0-100. */
  dim: number;
}

/**
 * Paint the wallpaper variables straight onto <html>.
 *
 * The whole point of this card is that you can see what you are choosing, and
 * the wallpaper is a property of the document rather than of any element this
 * component renders - so adjusting the veil has to reach out to the root. The
 * server round-trip then persists what the eye already agreed to.
 */
function preview(url: string | null, dim: number): void {
  const root = document.documentElement;
  if (url) {
    root.style.setProperty("--ml-bg-image", `url("${url}")`);
    root.style.setProperty("--ml-bg-dim", String(dim / 100));
    root.setAttribute("data-bg", "on");
  } else {
    root.style.removeProperty("--ml-bg-image");
    root.style.removeProperty("--ml-bg-dim");
    root.removeAttribute("data-bg");
  }
}

export function BackgroundSection({ url: initialUrl, dim: initialDim }: Props) {
  const router = useRouter();
  const t = useTranslations("profile.background");
  const fileRef = useRef<HTMLInputElement>(null);
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const [url, setUrl] = useState<string | null>(initialUrl);
  const [dim, setDim] = useState<number>(initialDim ?? DIM_DEFAULT);
  const [busy, setBusy] = useState(false);

  // A pending debounce must not outlive the card: leaving the page mid-drag
  // would otherwise fire a write against a component that no longer exists.
  useEffect(() => {
    return () => {
      if (saveTimer.current) clearTimeout(saveTimer.current);
    };
  }, []);

  function onDim(next: number) {
    setDim(next);
    preview(url, next);
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(() => {
      updateBackgroundDim(next).catch(() => toast.error(t("dimError")));
    }, 400);
  }

  async function onPick(file: File) {
    if (file.size > BACKGROUND_MAX_BYTES) {
      toast.error(t("tooLarge"));
      return;
    }
    setBusy(true);
    try {
      // The raw file as the body, not multipart: there is exactly one field, and
      // Server Actions cap bodies at 1 MB, which no wallpaper respects.
      const res = await fetch("/api/background", {
        method: "POST",
        headers: { "Content-Type": file.type },
        body: file,
      });
      if (!res.ok) {
        const code = res.status === 413 ? "tooLarge" : res.status === 415 ? "badType" : "uploadError";
        toast.error(t(code));
        return;
      }
      // Read the URL back rather than guessing it: the storage key carries a
      // fresh random token, which is what busts the cache on a replacement.
      const state = await myBackground();
      setUrl(state.url);
      setDim(state.dim);
      preview(state.url, state.dim);
      toast.success(t("saved"));
      router.refresh();
    } catch {
      toast.error(t("uploadError"));
    } finally {
      setBusy(false);
      if (fileRef.current) fileRef.current.value = "";
    }
  }

  async function onRemove() {
    setBusy(true);
    try {
      const res = await fetch("/api/background", { method: "DELETE" });
      if (!res.ok) {
        toast.error(t("removeError"));
        return;
      }
      setUrl(null);
      preview(null, dim);
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
          <ImageIcon className="text-primary size-4" />
          {t("title")}
        </CardTitle>
        <CardDescription>{t("desc")}</CardDescription>
      </CardHeader>

      <CardContent className="space-y-5">
        <div
          className="ring-foreground/10 relative grid h-40 place-items-center overflow-hidden rounded-lg ring-1"
          style={
            url
              ? {
                  backgroundImage: `url("${url}")`,
                  backgroundSize: "cover",
                  backgroundPosition: "center",
                }
              : undefined
          }
        >
          {url ? (
            // Mirrors the real veil, so the strip reads as a true sample of the
            // page rather than as an unrelated thumbnail.
            <span
              aria-hidden
              className="absolute inset-0"
              style={{
                backgroundColor: `color-mix(in oklab, var(--background) ${dim}%, transparent)`,
              }}
            />
          ) : (
            <div className="text-muted-foreground flex flex-col items-center gap-2 text-sm">
              <ImageIcon className="size-6 opacity-60" />
              {t("empty")}
            </div>
          )}

          {url && (
            <span className="text-foreground bg-card/85 relative rounded-md px-3 py-1.5 text-sm font-medium">
              {t("sample")}
            </span>
          )}
        </div>

        <input
          ref={fileRef}
          type="file"
          accept={BACKGROUND_ACCEPT}
          className="hidden"
          onChange={(e) => {
            const file = e.target.files?.[0];
            if (file) void onPick(file);
          }}
        />

        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            size="sm"
            disabled={busy}
            onClick={() => fileRef.current?.click()}
          >
            {busy ? <Loader2 className="size-4 animate-spin" /> : <Upload className="size-4" />}
            {url ? t("replace") : t("choose")}
          </Button>

          {url && (
            <Button type="button" size="sm" variant="outline" disabled={busy} onClick={onRemove}>
              <Trash2 className="size-4" />
              {t("remove")}
            </Button>
          )}
        </div>

        {url && (
          <div className="space-y-2">
            <div className="flex items-center justify-between gap-3">
              <Label htmlFor="bg-dim">{t("dim")}</Label>
              <span className="text-muted-foreground tabular-nums text-sm">{dim}%</span>
            </div>
            <input
              id="bg-dim"
              type="range"
              min={DIM_MIN}
              max={DIM_MAX}
              step={5}
              value={dim}
              onChange={(e) => onDim(Number(e.target.value))}
              className="accent-primary h-2 w-full cursor-pointer"
            />
            {/* Not a block - it is your own panel and a vivid wallpaper is a
                fair choice. But below this the worst case stops clearing 4.5:1
                on some skins, and that is worth knowing before you notice it. */}
            {dim < DIM_SAFE ? (
              <p className="flex items-start gap-1.5 text-xs leading-relaxed text-amber-600 dark:text-amber-500">
                <TriangleAlert className="mt-0.5 size-3.5 shrink-0" />
                {t("dimWarning")}
              </p>
            ) : (
              <p className="text-muted-foreground text-xs leading-relaxed">{t("dimHelp")}</p>
            )}
          </div>
        )}

        <p className="text-muted-foreground text-xs leading-relaxed">{t("note")}</p>
      </CardContent>
    </Card>
  );
}
