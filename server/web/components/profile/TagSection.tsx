"use client";

import { useEffect, useRef, useState, useTransition } from "react";
import { useRouter } from "next/navigation";
import {
  AtSign,
  Check,
  FolderTree,
  Info,
  Link2,
  Loader2,
  ShieldCheck,
  TriangleAlert,
  X,
} from "lucide-react";
import { toast } from "sonner";
import { useTranslations } from "next-intl";
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
import { Switch } from "@/components/ui/switch";
import { Separator } from "@/components/ui/separator";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  checkTagAvailability,
  updateDefaultFolder,
  updateSkipTagInUrl,
  updateTag,
  type TagIssue,
} from "@/app/actions/account";
import { cn } from "@/lib/utils";

/**
 * Same shape as `checkTag()` in lib/naming.ts. Duplicated on purpose: that
 * module reaches node:crypto through lib/ids and cannot be imported into a
 * client bundle. It only short-circuits obviously-bad input — the server action
 * remains the authority, including on the reserved-word list.
 */
const TAG_SHAPE = /^[a-zA-Z0-9][a-zA-Z0-9_-]*$/;
const TAG_MIN = 2;
const TAG_MAX = 32;

type Availability =
  | { state: "idle" }
  | { state: "checking" }
  | { state: "ok" }
  | { state: "current" }
  | { state: "unavailable"; reason: TagIssue };

interface TagSectionProps {
  /** Base URL used in the link preview. */
  appUrl: string;
  /** Current tag (user.userCode). */
  tag: string | null;
  isAdmin: boolean;
  skipTagInUrl: boolean;
}

export function TagSection({ appUrl, tag, isAdmin, skipTagInUrl }: TagSectionProps) {
  const t = useTranslations("profile.tag");
  const router = useRouter();

  const current = tag ?? "";
  const [value, setValue] = useState(current);
  const [availability, setAvailability] = useState<Availability>({ state: "idle" });
  const [saving, startSave] = useTransition();
  const [skip, setSkip] = useState(skipTagInUrl);
  const [skipPending, startSkip] = useTransition();

  const trimmed = value.trim();
  const changed = trimmed !== current;

  // Debounced availability check. `seq` discards answers that arrive out of
  // order — typing fast otherwise leaves the verdict of an older prefix on
  // screen next to the newer input.
  const seq = useRef(0);
  useEffect(() => {
    if (!changed) {
      setAvailability({ state: "idle" });
      return;
    }
    const local = ++seq.current;

    const shapeIssue = localIssue(trimmed);
    if (shapeIssue) {
      setAvailability({ state: "unavailable", reason: shapeIssue });
      return;
    }

    setAvailability({ state: "checking" });
    const timer = setTimeout(async () => {
      try {
        const result = await checkTagAvailability(trimmed);
        if (seq.current !== local) return;
        setAvailability(
          result.status === "unavailable"
            ? { state: "unavailable", reason: result.reason }
            : { state: result.status === "current" ? "current" : "ok" },
        );
      } catch {
        if (seq.current === local) setAvailability({ state: "idle" });
      }
    }, 400);

    return () => clearTimeout(timer);
  }, [trimmed, changed]);

  function save() {
    if (!changed || saving) return;
    startSave(async () => {
      const result = await updateTag(trimmed);
      if (!result.ok) {
        setAvailability({ state: "unavailable", reason: result.reason });
        toast.error(t(`issue.${result.reason}`));
        return;
      }
      toast.success(t("saved"));
      setAvailability({ state: "idle" });
      router.refresh();
    });
  }

  function toggleSkip(next: boolean) {
    const previous = skip;
    setSkip(next);
    startSkip(async () => {
      try {
        await updateSkipTagInUrl(next);
        toast.success(next ? t("skipEnabled") : t("skipDisabled"));
        router.refresh();
      } catch {
        setSkip(previous);
        toast.error(t("skipError"));
      }
    });
  }

  // Preview of the resulting link. Built here rather than through
  // lib/media/urls.ts, which is server-side.
  const previewTag = trimmed || current || t("previewFallback");
  const preview = skip ? `${appUrl}/screenshot.png` : `${appUrl}/${previewTag}/screenshot.png`;

  const blocked = availability.state === "unavailable";
  const canSave = changed && !saving && (availability.state === "ok" || availability.state === "idle");

  return (
    <Card>
      <CardHeader className="border-b">
        <CardTitle className="flex items-center gap-2">
          <AtSign className="text-primary size-4" />
          {t("title")}
        </CardTitle>
        <CardDescription>{t("desc")}</CardDescription>
      </CardHeader>

      {/* flex-1 keeps the footer pinned to the bottom when this card is stretched
          to match a taller neighbour in the same grid row. */}
      <CardContent className="flex-1 space-y-5">
        <div className="space-y-2">
          <Label htmlFor="tag">{t("label")}</Label>
          <div className="flex items-stretch">
            <span className="border-input bg-muted text-muted-foreground inline-flex items-center rounded-l-lg border border-r-0 px-2.5 font-mono text-xs">
              /
            </span>
            <Input
              id="tag"
              value={value}
              onChange={(e) => setValue(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && canSave) save();
              }}
              maxLength={TAG_MAX}
              autoComplete="off"
              spellCheck={false}
              aria-invalid={blocked}
              aria-describedby="tag-status"
              disabled={saving}
              className={cn("rounded-l-none font-mono", blocked && "border-destructive")}
            />
          </div>

          <p id="tag-status" className="flex min-h-5 items-center gap-1.5 text-xs" aria-live="polite">
            {availability.state === "checking" && (
              <span className="text-muted-foreground inline-flex items-center gap-1.5">
                <Loader2 className="size-3.5 animate-spin" />
                {t("checking")}
              </span>
            )}
            {availability.state === "ok" && (
              <span className="inline-flex items-center gap-1.5 text-emerald-600 dark:text-emerald-400">
                <Check className="size-3.5" />
                {t("available", { tag: trimmed })}
              </span>
            )}
            {availability.state === "current" && (
              <span className="text-muted-foreground inline-flex items-center gap-1.5">
                <Info className="size-3.5" />
                {t("unchanged")}
              </span>
            )}
            {availability.state === "unavailable" && (
              <span className="text-destructive inline-flex items-center gap-1.5">
                <X className="size-3.5" />
                {t(`issue.${availability.reason}`)}
              </span>
            )}
          </p>
        </div>

        {/* What the link will look like. */}
        <div className="bg-muted/40 ring-foreground/10 space-y-1.5 rounded-lg px-3 py-2.5 ring-1">
          <p className="text-muted-foreground flex items-center gap-1.5 text-xs font-medium">
            <Link2 className="size-3.5" />
            {t("previewLabel")}
          </p>
          <p className="text-foreground font-mono text-xs break-all">{preview}</p>
        </div>

        {/* The whole reason changing a tag is safe. */}
        <div className="border-primary/25 bg-primary/5 flex items-start gap-2.5 rounded-lg border px-3 py-2.5">
          <ShieldCheck className="text-primary mt-0.5 size-4 shrink-0" />
          <div className="space-y-0.5">
            <p className="text-sm font-medium">{t("aliasTitle")}</p>
            <p className="text-muted-foreground text-xs leading-relaxed">{t("aliasBody")}</p>
          </div>
        </div>

        {isAdmin && (
          <>
            <Separator />
            <div className="space-y-3">
              <div className="flex items-start justify-between gap-4">
                <div className="space-y-0.5">
                  <Label htmlFor="skipTag" className="cursor-pointer">
                    {t("skipLabel")}
                  </Label>
                  <p className="text-muted-foreground text-sm leading-snug">{t("skipHelp")}</p>
                </div>
                <Switch
                  id="skipTag"
                  checked={skip}
                  onCheckedChange={toggleSkip}
                  disabled={skipPending}
                />
              </div>
              <p className="text-muted-foreground flex items-start gap-1.5 text-xs leading-relaxed">
                <TriangleAlert className="mt-0.5 size-3.5 shrink-0" />
                {t("skipWarning")}
              </p>
            </div>
          </>
        )}
      </CardContent>

      <CardFooter className="justify-end gap-2">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          onClick={() => setValue(current)}
          disabled={!changed || saving}
        >
          {t("discard")}
        </Button>
        <Button type="button" size="sm" onClick={save} disabled={!canSave}>
          {saving ? <Loader2 className="size-4 animate-spin" /> : <Check className="size-4" />}
          {t("save")}
        </Button>
      </CardFooter>
    </Card>
  );
}

/** Instant, offline verdict for input that cannot possibly be valid. */
function localIssue(tag: string): TagIssue | null {
  if (tag.length < TAG_MIN) return "too-short";
  if (tag.length > TAG_MAX) return "too-long";
  if (!TAG_SHAPE.test(tag)) return "charset";
  return null;
}

// ---------------------------------------------------------------------------
// Default upload folder
// ---------------------------------------------------------------------------

export interface FolderOption {
  id: string;
  name: string;
  /** Nesting level, used to indent the option. */
  depth: number;
}

const NO_FOLDER = "__none__";

/**
 * Folder that new uploads land in. Applies to every ingest path (web upload and
 * ShareX), so files stop piling up unsorted.
 */
export function DefaultFolderSection({
  folders,
  defaultFolderId,
}: {
  folders: FolderOption[];
  defaultFolderId: string | null;
}) {
  const t = useTranslations("profile.uploadFolder");
  const router = useRouter();
  const [value, setValue] = useState(defaultFolderId ?? NO_FOLDER);
  const [pending, startTransition] = useTransition();

  const selected = folders.find((f) => f.id === value);

  function pick(next: string) {
    if (next === value || pending) return;
    const previous = value;
    setValue(next);
    startTransition(async () => {
      try {
        const { ok } = await updateDefaultFolder(next === NO_FOLDER ? null : next);
        if (!ok) {
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
          <FolderTree className="text-primary size-4" />
          {t("title")}
        </CardTitle>
        <CardDescription>{t("desc")}</CardDescription>
      </CardHeader>

      <CardContent className="flex-1 space-y-2">
        <Label htmlFor="defaultFolder">{t("label")}</Label>
        <Select value={value} onValueChange={pick} disabled={pending || folders.length === 0}>
          <SelectTrigger id="defaultFolder" className="w-full sm:w-72">
            <SelectValue>{selected ? selected.name : t("none")}</SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={NO_FOLDER}>{t("none")}</SelectItem>
            {folders.map((f) => (
              <SelectItem key={f.id} value={f.id}>
                <span style={{ paddingLeft: `${f.depth * 12}px` }}>{f.name}</span>
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <p className="text-muted-foreground text-xs">
          {folders.length === 0 ? t("emptyHelp") : t("help")}
        </p>
      </CardContent>
    </Card>
  );
}
