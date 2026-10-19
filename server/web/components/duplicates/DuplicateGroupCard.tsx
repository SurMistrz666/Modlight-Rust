"use client";

import { useRef } from "react";
import {
  BadgeCheck,
  Eye,
  ExternalLink,
  Folder,
  Play,
  RotateCcw,
  ShieldOff,
  TriangleAlert,
} from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Thumbnail, useInView } from "@/components/dashboard/Thumbnail";
import {
  KindIcon,
  buildOpenUrl,
  buildRawUrl,
  extLabel,
  isImage,
  isVideo,
  kindFromMime,
  kindTint,
} from "@/components/dashboard/media";
import { humanFileSize } from "@/lib/format";
import { cn } from "@/lib/utils";
import { keptId } from "./selection";
import type { DuplicateGroup } from "@/lib/data/duplicates";

/**
 * One set of identical files: what it is, how many there are, and which copy
 * survives.
 *
 * The preview is drawn ONCE for the whole set rather than once per row. Every
 * file in a group hashes to the same sha256, so they are the same bytes and the
 * same picture; six thumbnails of one screenshot would be six requests spent
 * saying the same thing, and would make a set of copies look like a set of
 * different files.
 *
 * Every link and every preview is RELATIVE, so it resolves on the host the
 * panel is being read on. That is not cosmetic: the session cookie is host-only,
 * and an account whose files carry the default password (Round 6 stamps one on
 * every new upload) would be refused by its own thumbnails on any other
 * configured hostname.
 */
export function DuplicateGroupCard({
  group,
  selected,
  userCode,
  disabled,
  onToggle,
  onKeepOnly,
  onKeepSet,
  onCountAgain,
}: {
  group: DuplicateGroup;
  selected: ReadonlySet<string>;
  userCode: string;
  disabled: boolean;
  onToggle: (id: string) => void;
  onKeepOnly: (id: string) => void;
  /** "These copies are meant to exist": stop counting this set for good. */
  onKeepSet: () => void;
  /** Undo of the above. */
  onCountAgain: () => void;
}) {
  const t = useTranslations("profile.duplicates");
  const format = useFormatter();

  const first = group.files[0];
  const sole = keptId(group, selected);
  // A trimmed set always keeps its unlisted copies, so it can never be emptied
  // from this screen; an untrimmed one can, and then it says so.
  const trimmed = group.copies > group.files.length;
  const emptied = !trimmed && group.files.every((file) => selected.has(file.id));

  return (
    <section
      className={cn(
        "bg-card ring-foreground/10 overflow-hidden rounded-xl ring-1",
        group.dismissed && "opacity-80",
      )}
    >
      <header className="flex items-start gap-3 p-3">
        <GroupPreview group={group} userCode={userCode} />

        <div className="min-w-0 flex-1">
          <p className="truncate font-medium">{first.filename}</p>
          <p className="text-muted-foreground mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs">
            <span>{t("copies", { count: group.copies })}</span>
            <span aria-hidden>&middot;</span>
            <span className="tabular-nums">{t("each", { size: humanFileSize(group.size) })}</span>
            {group.dismissed && (
              <Badge variant="secondary" className="text-[10px]">
                {t("keptBadge")}
              </Badge>
            )}
          </p>
        </div>

        <div className="shrink-0 text-right">
          <div className="text-sm font-semibold tabular-nums">
            {t("saves", { size: humanFileSize(group.reclaimable) })}
          </div>
          {group.dismissed ? (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="mt-1 h-7"
              disabled={disabled}
              onClick={onCountAgain}
            >
              <RotateCcw className="size-3.5" />
              {t("countAgain")}
            </Button>
          ) : (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="mt-1 h-7"
              disabled={disabled}
              onClick={onKeepSet}
            >
              <ShieldOff className="size-3.5" />
              {t("keepSet")}
            </Button>
          )}
        </div>
      </header>

      <ul className="divide-foreground/5 border-foreground/5 divide-y border-t">
        {group.files.map((file, index) => {
          const checked = selected.has(file.id);
          const keeping = sole === file.id;

          return (
            <li
              key={file.id}
              className={cn(
                "flex items-center gap-3 px-3 py-2 text-sm transition-colors",
                checked && "bg-destructive/5",
              )}
            >
              <Checkbox
                id={`dup-${file.id}`}
                checked={checked}
                disabled={disabled}
                onCheckedChange={() => onToggle(file.id)}
                aria-label={t("removeAria", { filename: file.filename })}
              />

              <label htmlFor={`dup-${file.id}`} className="min-w-0 flex-1 cursor-pointer">
                <span className="flex flex-wrap items-center gap-2">
                  <span className={cn("truncate font-medium", checked && "line-through opacity-70")}>
                    {file.filename}
                  </span>
                  <Badge variant="outline" className="font-mono text-[10px]">
                    {extLabel(file)}
                  </Badge>
                  {index === 0 && (
                    <Badge variant="secondary" className="text-[10px]">
                      {t("oldest")}
                    </Badge>
                  )}
                </span>

                <span className="text-muted-foreground mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs">
                  <span className="tabular-nums">
                    {format.dateTime(new Date(file.createdAt), { dateStyle: "medium" })}
                  </span>
                  <span className="tabular-nums">{humanFileSize(file.size)}</span>
                  {file.folderName && (
                    <span className="inline-flex items-center gap-1">
                      <Folder className="size-3" />
                      {file.folderName}
                    </span>
                  )}
                  <span className="inline-flex items-center gap-1 tabular-nums">
                    <Eye className="size-3" />
                    {file.views.toLocaleString()}
                  </span>
                </span>
              </label>

              {keeping ? (
                <span className="text-primary inline-flex shrink-0 items-center gap-1 text-xs font-medium">
                  <BadgeCheck className="size-3.5" />
                  {t("keeping")}
                </span>
              ) : (
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="h-7 shrink-0"
                  disabled={disabled}
                  onClick={() => onKeepOnly(file.id)}
                  aria-label={t("keepOnlyAria", { filename: file.filename })}
                >
                  {t("keepOnly")}
                </Button>
              )}

              <Button asChild variant="ghost" size="icon" className="size-8 shrink-0">
                <a
                  href={buildOpenUrl(userCode, file.code, file.ext)}
                  target="_blank"
                  rel="noopener noreferrer"
                  aria-label={t("openAria", { filename: file.filename })}
                >
                  <ExternalLink className="size-4" />
                </a>
              </Button>
            </li>
          );
        })}
      </ul>

      {trimmed && (
        <p className="text-muted-foreground border-foreground/5 border-t px-3 py-2 text-xs">
          {t("moreCopies", { shown: group.files.length, total: group.copies })}
        </p>
      )}

      {emptied && (
        <p className="text-destructive bg-destructive/5 flex items-start gap-1.5 px-3 py-2 text-xs">
          <TriangleAlert className="mt-0.5 size-3.5 shrink-0" />
          {t("noCopyKept")}
        </p>
      )}
    </section>
  );
}

/** The set's picture: one request for bytes every copy shares. */
function GroupPreview({ group, userCode }: { group: DuplicateGroup; userCode: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const near = useInView(ref, { rootMargin: "300px", once: true });

  const file = group.files[0];
  const kind = kindFromMime(file.mimeType, file.ext);
  // Relative on purpose - see the component comment.
  const raw = buildRawUrl("", userCode, file.code, file.ext);

  return (
    <div
      ref={ref}
      className="bg-muted/40 relative size-20 shrink-0 overflow-hidden rounded-lg sm:size-24"
    >
      {isImage(file.mimeType, file.ext) ? (
        <Thumbnail
          src={`${raw}?w=240&h=240`}
          alt={file.filename}
          blurhash={file.blurhash}
          className="size-full"
        />
      ) : isVideo(file.mimeType, file.ext) ? (
        <>
          {near && (
            // A metadata-only <video> is the only way to a real first frame:
            // the image endpoint cannot rasterize video. Mounted once the card
            // is near the viewport, so a hundred sets do not open a hundred
            // media connections at once.
            <video
              src={raw}
              muted
              playsInline
              preload="metadata"
              className="absolute inset-0 size-full object-cover"
            />
          )}
          <span className="bg-background/70 text-foreground ring-foreground/10 absolute top-1/2 left-1/2 grid size-8 -translate-x-1/2 -translate-y-1/2 place-items-center rounded-full ring-1 backdrop-blur">
            <Play className="size-3.5 translate-x-px fill-current" />
          </span>
        </>
      ) : (
        <div className="grid size-full place-items-center">
          <div className={cn("grid size-11 place-items-center rounded-xl", kindTint(kind))}>
            <KindIcon kind={kind} mime={file.mimeType} className="size-5" />
          </div>
        </div>
      )}
    </div>
  );
}
