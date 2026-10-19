"use client";

import { useRouter } from "next/navigation";
import { useEffect, useMemo, useState, useTransition } from "react";
import { useFormatter, useLocale, useTranslations } from "next-intl";
import { toast } from "sonner";
import {
  ChevronLeft,
  ChevronRight,
  Copy,
  Check,
  Download,
  ExternalLink,
  Eye,
  Flame,
  Folder,
  FolderInput,
  Globe,
  Inbox,
  KeyRound,
  Clock,
  Loader2,
  Lock,
  Settings2,
  Trash2,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { humanFileSize, relativeTime } from "@/lib/format";
import { buildLinkFormats, type LinkFormats } from "@/lib/media/link-formats";
import { diagnosePlayback } from "@/lib/media/playback-health";
import { PlaybackNote } from "@/components/viewer/PlaybackNote";
import { togglePublish, trashUpload } from "@/app/actions/uploads";
import { MoveToFolderDialog } from "./FileMenu";
import { LinkSettingsDialog } from "./LinkSettingsDialog";
import {
  buildDownloadUrl,
  buildOpenUrl,
  buildPublicUrl,
  buildRawUrl,
  extLabel,
  isAudio,
  isImage,
  isVideo,
  KindIcon,
  kindTint,
} from "./media";
import type { FileItem, FileViewMode } from "./types";

const LINK_KEYS: (keyof LinkFormats)[] = ["direct", "raw", "markdown", "bbcode", "html"];

/**
 * Right-hand detail sheet.
 *
 * Clicking a tile used to fling the user onto the public page in a new tab,
 * which is the wrong destination when the intent is "what is this and what's
 * its link?". Everything that used to require that round-trip lives here:
 * preview, metadata, every link format, folder, stats and the link gates.
 */
export function DetailsDrawer({
  items,
  index,
  onIndexChange,
  onClose,
  userCode,
  appUrl,
  copyRaw,
  currentFolderId,
  mode = "owner",
  onTrashed,
  onMoved,
}: {
  items: FileItem[];
  /** Index into `items`, or null when closed. */
  index: number | null;
  onIndexChange: (i: number) => void;
  onClose: () => void;
  userCode: string;
  appUrl: string;
  copyRaw?: boolean;
  currentFolderId?: string | null;
  /** Owner's own library, or an administrator reviewing it. See types.ts. */
  mode?: FileViewMode;
  onTrashed?: (id: string) => void;
  onMoved?: (ids: string[], folderId: string | null) => void;
}) {
  const t = useTranslations("files");
  const format = useFormatter();
  // relativeTime formats through Intl, so it has to be told whose language
  // this is; without it a Polish reader gets English ages.
  const locale = useLocale();
  const router = useRouter();
  const [pending, startTransition] = useTransition();
  const [moveOpen, setMoveOpen] = useState(false);
  const [linkSettingsOpen, setLinkSettingsOpen] = useState(false);

  const item = index != null ? items[index] : undefined;

  // The list can shrink underneath us (trash, move out of a folder).
  useEffect(() => {
    if (index != null && !items[index]) onClose();
  }, [index, items, onClose]);

  const urls = useMemo(() => {
    if (!item) return null;
    const publicUrl = buildPublicUrl(appUrl, userCode, item.code, item.ext);
    const rawUrl = buildRawUrl(appUrl, userCode, item.code, item.ext);
    return {
      publicUrl,
      rawUrl,
      // Opening keeps the owner on the host they are signed in on; the copied
      // links keep their chosen hostname. See buildOpenUrl.
      openUrl: buildOpenUrl(userCode, item.code, item.ext),
      downloadUrl: buildDownloadUrl(appUrl, userCode, item.code),
      formats: buildLinkFormats({
        url: publicUrl,
        rawUrl,
        filename: item.filename,
        isImage: isImage(item.mimeType, item.ext),
      }),
    };
  }, [item, appUrl, userCode]);

  if (!item || !urls) {
    return null;
  }

  const hasPrev = index != null && index > 0;
  const hasNext = index != null && index < items.length - 1;

  /**
   * Is this video outside the shape a browser plays well?
   *
   * Owner only, deliberately. An administrator looking through somebody else's
   * library cannot change how that person records, and the repair is a rewrite
   * of a file that is not theirs - the same reason "Move to folder" is absent in
   * that mode. Null for every other file, which is almost all of them.
   */
  const playback = mode === "owner" ? diagnosePlayback(item) : null;

  function go(delta: number) {
    if (index == null) return;
    const next = index + delta;
    if (next < 0 || next >= items.length) return;
    onIndexChange(next);
  }

  function onTogglePublish() {
    startTransition(async () => {
      try {
        const res = await togglePublish(item!.id);
        toast.success(res.published ? t("toast.nowPublic") : t("toast.nowPrivate"));
        router.refresh();
      } catch {
        toast.error(t("toast.couldntChangeVisibility"));
      }
    });
  }

  function onTrash() {
    const id = item!.id;
    onClose();
    onTrashed?.(id);
    startTransition(async () => {
      try {
        await trashUpload(id);
        toast.success(t("toast.movedToTrash", { count: 1 }));
      } catch {
        toast.error(t("toast.couldntMoveToTrash", { count: 1 }));
      }
      router.refresh();
    });
  }

  const created = new Date(item.createdAt);

  return (
    <>
      <Sheet open onOpenChange={(open) => !open && onClose()}>
        <SheetContent
          side="right"
          className="w-full gap-0 p-0 data-[side=right]:sm:max-w-md"
          onKeyDown={(e) => {
            const el = e.target as HTMLElement | null;
            const typing =
              !!el && (el.isContentEditable || /^(input|textarea)$/i.test(el.tagName));
            if (typing) return;
            if (e.key === "ArrowLeft") {
              e.preventDefault();
              go(-1);
            } else if (e.key === "ArrowRight") {
              e.preventDefault();
              go(1);
            }
          }}
        >
          {/* Header: paging + identity */}
          <SheetHeader className="gap-2 border-b border-foreground/10 p-4 pr-14">
            <div className="flex items-center gap-1">
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={() => go(-1)}
                disabled={!hasPrev}
                aria-label={t("details.previous")}
              >
                <ChevronLeft className="size-4" />
              </Button>
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={() => go(1)}
                disabled={!hasNext}
                aria-label={t("details.next")}
              >
                <ChevronRight className="size-4" />
              </Button>
              <span className="text-muted-foreground ml-1 text-xs tabular-nums">
                {t("details.position", { index: (index ?? 0) + 1, total: items.length })}
              </span>
            </div>
            <SheetTitle className="truncate" title={item.filename}>
              {item.filename}
            </SheetTitle>
            <SheetDescription className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs">
              <span
                className={cn(
                  "rounded px-1.5 py-0.5 font-medium tracking-wide",
                  kindTint(item.kind),
                )}
              >
                {extLabel(item)}
              </span>
              <span className="tabular-nums">{humanFileSize(item.size)}</span>
              {item.width && item.height ? (
                <span className="tabular-nums">
                  {item.width} × {item.height}
                </span>
              ) : null}
            </SheetDescription>
          </SheetHeader>

          <div className="flex-1 overflow-y-auto">
            {/* Preview */}
            <div className="bg-muted/40 ring-foreground/10 relative m-4 grid aspect-video place-items-center overflow-hidden rounded-lg ring-1">
              {isImage(item.mimeType, item.ext) ? (
                // eslint-disable-next-line @next/next/no-img-element
                <img
                  src={`${urls.rawUrl}?w=1024&h=1024`}
                  alt={item.filename}
                  className="size-full object-contain"
                />
              ) : isVideo(item.mimeType, item.ext) ? (
                <video
                  src={urls.rawUrl}
                  controls
                  preload="metadata"
                  playsInline
                  className="size-full object-contain"
                />
              ) : isAudio(item.mimeType, item.ext) ? (
                <div className="flex w-full flex-col items-center gap-3 px-4">
                  <div className={cn("grid size-14 place-items-center rounded-2xl", kindTint(item.kind))}>
                    <KindIcon kind={item.kind} mime={item.mimeType} className="size-6" />
                  </div>
                  <audio src={urls.rawUrl} controls className="w-full" />
                </div>
              ) : (
                <div className="flex flex-col items-center gap-2">
                  <div className={cn("grid size-14 place-items-center rounded-2xl", kindTint(item.kind))}>
                    <KindIcon kind={item.kind} mime={item.mimeType} className="size-6" />
                  </div>
                  <span className="text-muted-foreground text-xs tracking-wider uppercase">
                    {extLabel(item)}
                  </span>
                </div>
              )}
            </div>

            {/* The one thing the owner would otherwise only learn from a
                complaint. Above the link buttons on purpose: it has to be read
                before the link is copied, not after. */}
            {playback ? (
              <div className="px-4 pb-4">
                <PlaybackNote
                  diagnosis={playback}
                  repair={{ item, userCode, appUrl }}
                />
              </div>
            ) : null}

            {/* Primary actions */}
            <div className="flex items-center gap-2 px-4">
              <CopyButton
                value={urls.formats[copyRaw ? "raw" : "direct"]}
                label={t("details.copyLink")}
                className="flex-1"
              />
              <Button variant="outline" size="icon" asChild aria-label={t("menu.open")}>
                <a href={urls.openUrl} target="_blank" rel="noopener noreferrer">
                  <ExternalLink className="size-4" />
                </a>
              </Button>
              <Button variant="outline" size="icon" asChild aria-label={t("menu.download")}>
                <a href={urls.downloadUrl}>
                  <Download className="size-4" />
                </a>
              </Button>
            </div>

            {/* Facts */}
            <Section title={t("details.about")}>
              <Fact label={t("details.type")} value={item.mimeType} mono />
              <Fact label={t("details.size")} value={humanFileSize(item.size)} />
              {item.width && item.height ? (
                <Fact
                  label={t("details.dimensions")}
                  value={`${item.width} × ${item.height} px`}
                />
              ) : null}
              <Fact
                label={t("details.uploaded")}
                value={format.dateTime(created, {
                  dateStyle: "medium",
                  timeStyle: "short",
                })}
                hint={relativeTime(item.createdAt, locale)}
              />
              <Fact label={t("details.code")} value={item.code} mono />
            </Section>

            <Separator />

            {/* Placement + visibility */}
            <Section title={t("details.placement")}>
              <Row
                icon={item.folderName ? <Folder className="size-4" /> : <Inbox className="size-4" />}
                label={t("details.folder")}
                value={item.folderName ?? t("details.noFolder")}
                action={
                  // Folders are the owner's, so an admin gets the fact without
                  // a button that would move the file into their own tree.
                  mode === "owner" ? (
                    <Button variant="outline" size="sm" onClick={() => setMoveOpen(true)}>
                      <FolderInput className="size-3.5" /> {t("details.change")}
                    </Button>
                  ) : undefined
                }
              />
              <Row
                icon={item.published ? <Globe className="size-4" /> : <Lock className="size-4" />}
                label={t("details.visibility")}
                value={item.published ? t("card.public") : t("card.private")}
                action={
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={onTogglePublish}
                    disabled={pending}
                  >
                    {pending ? (
                      <Loader2 className="size-3.5 animate-spin" />
                    ) : item.published ? (
                      <Lock className="size-3.5" />
                    ) : (
                      <Globe className="size-3.5" />
                    )}
                    {item.published ? t("menu.makePrivate") : t("menu.makePublic")}
                  </Button>
                }
              />
            </Section>

            <Separator />

            {/* Every link format */}
            <Section title={t("details.links")}>
              <div className="flex flex-col gap-1.5">
                {LINK_KEYS.map((k) => (
                  <LinkRow
                    key={k}
                    label={t(`linkFormats.${k}`)}
                    value={urls.formats[k]}
                    copiedLabel={t("toast.labelCopied", { label: t(`linkFormats.${k}`) })}
                    failLabel={t("toast.couldntCopy")}
                  />
                ))}
              </div>
            </Section>

            <Separator />

            {/* Stats */}
            <Section title={t("details.stats")}>
              <div className="grid grid-cols-2 gap-2">
                <Stat
                  icon={<Eye className="size-4" />}
                  label={t("details.views")}
                  value={item.views.toLocaleString()}
                />
                <Stat
                  icon={<Download className="size-4" />}
                  label={t("details.downloads")}
                  value={item.downloads.toLocaleString()}
                />
              </div>
            </Section>

            <Separator />

            {/* Link gates */}
            <Section title={t("details.linkSettings")}>
              <div className="flex flex-wrap items-center gap-1.5">
                {item.expiresAt ? (
                  <Gate icon={<Clock className="size-3" />}>
                    {t("details.expiresOn", {
                      date: format.dateTime(new Date(item.expiresAt), {
                        dateStyle: "medium",
                        timeStyle: "short",
                      }),
                    })}
                  </Gate>
                ) : null}
                {item.maxViews != null ? (
                  <Gate icon={<Flame className="size-3" />}>
                    {t("card.limitedToViews", { count: item.maxViews })}
                  </Gate>
                ) : null}
                {item.hasPassword ? (
                  <Gate icon={<KeyRound className="size-3" />}>
                    {t("card.passwordProtected")}
                  </Gate>
                ) : null}
                {!item.expiresAt && item.maxViews == null && !item.hasPassword ? (
                  <span className="text-muted-foreground text-xs">
                    {t("details.noGates")}
                  </span>
                ) : null}
              </div>
              <Button
                variant="outline"
                size="sm"
                className="w-fit"
                onClick={() => setLinkSettingsOpen(true)}
              >
                <Settings2 className="size-3.5" /> {t("details.editLinkSettings")}
              </Button>
            </Section>

            <Separator />

            {/* Danger zone */}
            <Section title={t("details.danger")}>
              <div className="ring-destructive/20 flex items-center justify-between gap-3 rounded-lg p-3 ring-1">
                <div className="min-w-0">
                  <p className="text-sm font-medium">{t("details.trashTitle")}</p>
                  <p className="text-muted-foreground text-xs">{t("details.trashDesc")}</p>
                </div>
                <Button
                  variant="destructive"
                  size="sm"
                  onClick={onTrash}
                  disabled={pending}
                  className="shrink-0"
                >
                  <Trash2 className="size-3.5" /> {t("details.trash")}
                </Button>
              </div>
            </Section>
          </div>
        </SheetContent>
      </Sheet>

      {mode === "owner" ? (
        <MoveToFolderDialog
          open={moveOpen}
          onOpenChange={setMoveOpen}
          ids={[item.id]}
          currentFolderId={item.folderId}
          label={item.filename}
          onMoved={(ids, folderId) => {
            onMoved?.(ids, folderId);
            if (currentFolderId !== undefined && folderId !== currentFolderId) onClose();
          }}
        />
      ) : null}
      <LinkSettingsDialog
        open={linkSettingsOpen}
        onOpenChange={setLinkSettingsOpen}
        item={item}
      />
    </>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="flex flex-col gap-2 p-4">
      <h3 className="text-muted-foreground text-[11px] font-semibold tracking-wider uppercase">
        {title}
      </h3>
      {children}
    </section>
  );
}

function Fact({
  label,
  value,
  hint,
  mono,
}: {
  label: string;
  value: string;
  hint?: string;
  mono?: boolean;
}) {
  return (
    <div className="flex items-baseline justify-between gap-3 py-0.5 text-sm">
      <span className="text-muted-foreground shrink-0 text-xs">{label}</span>
      <span className={cn("min-w-0 truncate text-right", mono && "font-mono text-xs")}>
        {value}
        {hint ? <span className="text-muted-foreground ml-1.5 text-xs">({hint})</span> : null}
      </span>
    </div>
  );
}

function Row({
  icon,
  label,
  value,
  action,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
  action: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-3 py-1">
      <div className="flex min-w-0 items-center gap-2">
        <span className="text-muted-foreground shrink-0">{icon}</span>
        <div className="min-w-0">
          <p className="text-muted-foreground text-[11px]">{label}</p>
          <p className="truncate text-sm">{value}</p>
        </div>
      </div>
      <div className="shrink-0">{action}</div>
    </div>
  );
}

function Stat({
  icon,
  label,
  value,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
}) {
  return (
    <div className="bg-muted/40 ring-foreground/10 flex items-center gap-3 rounded-lg px-3 py-2 ring-1">
      <span className="text-muted-foreground">{icon}</span>
      <div className="min-w-0">
        <p className="text-muted-foreground text-[11px]">{label}</p>
        <p className="text-sm font-medium tabular-nums">{value}</p>
      </div>
    </div>
  );
}

function Gate({ icon, children }: { icon: React.ReactNode; children: React.ReactNode }) {
  return (
    <span className="bg-muted text-foreground ring-foreground/10 inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-xs ring-1">
      {icon}
      {children}
    </span>
  );
}

function LinkRow({
  label,
  value,
  copiedLabel,
  failLabel,
}: {
  label: string;
  value: string;
  copiedLabel: string;
  failLabel: string;
}) {
  const [copied, setCopied] = useState(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      toast.success(copiedLabel);
      setTimeout(() => setCopied(false), 1400);
    } catch {
      toast.error(failLabel);
    }
  }

  return (
    <button
      type="button"
      onClick={copy}
      title={value}
      className="group/link bg-muted/40 hover:bg-muted ring-foreground/10 flex w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left ring-1 transition-colors"
    >
      <span className="text-muted-foreground w-16 shrink-0 text-[11px] font-medium">
        {label}
      </span>
      <span className="min-w-0 flex-1 truncate font-mono text-xs">{value}</span>
      <span className="text-muted-foreground group-hover/link:text-foreground shrink-0 transition-colors">
        {copied ? (
          <Check className="text-primary size-3.5" />
        ) : (
          <Copy className="size-3.5" />
        )}
      </span>
    </button>
  );
}

function CopyButton({
  value,
  label,
  className,
}: {
  value: string;
  label: string;
  className?: string;
}) {
  const t = useTranslations("files");
  const [copied, setCopied] = useState(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      toast.success(t("toast.linkCopied"));
      setTimeout(() => setCopied(false), 1400);
    } catch {
      toast.error(t("toast.couldntCopy"));
    }
  }

  return (
    <Button onClick={copy} className={className}>
      {copied ? <Check className="size-4" /> : <Copy className="size-4" />}
      {label}
    </Button>
  );
}
