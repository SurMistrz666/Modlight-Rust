"use client";

import { useCallback, useMemo, useState } from "react";
import { useLocale, useTranslations } from "next-intl";
import { Check, Copy, ExternalLink, FolderOpen, Files } from "lucide-react";
import { humanFileSize } from "@/lib/format";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { copyText } from "@/components/transfers/clipboard";
import type { PreflightHit } from "@/components/upload/duplicate-preflight";

/** What the user may answer. Nothing here deletes anything, either way. */
export type DuplicateChoice = "skip" | "all";

/** One open question, owned by the transfers provider. */
export interface DuplicatePrompt {
  hits: PreflightHit[];
  /** Files in the batch, so "3 of 12" can be said rather than just "3". */
  total: number;
  /**
   * Files in this batch nothing could be said about: too large to hash, past
   * the batch budget, or answered `unknown` because the library holds files
   * with no stored checksum.
   *
   * Shown rather than dropped. "3 of 12 are copies" is a true sentence that a
   * reader will finish as "so the other 9 are new" - which is exactly the
   * inference this whole feature refuses to let the server make, and it would
   * be odd to refuse it there and invite it here.
   */
  uncompared: number;
  resolve: (choice: DuplicateChoice) => void;
}

/**
 * "You already have these" - shown BEFORE the first byte leaves.
 *
 * The whole point of the feature is here: the alternative the panel had was to
 * upload the file, take the space, mint a second link, and let the user find out
 * weeks later from the retroactive duplicate scan on the profile page.
 *
 * Two deliberate properties:
 *
 *  - Uploading anyway is always available and is never hidden behind a
 *    confirmation. A checksum match is a strong signal, not a licence to
 *    overrule someone: an identical file may genuinely be wanted twice (a
 *    second link with its own expiry, a copy in another folder), and a check
 *    that cannot be overridden is a check that gets worked around.
 *  - The existing copy's link is right here, with a copy button. Most of the
 *    time "you already have this" is not a warning at all, it is the answer:
 *    the user wanted a link to that file and was about to make a second one.
 *
 * The dialog only ever lists CONFIRMED matches. Files that could not be checked
 * - too large to hash, past the batch budget, or in a library that holds files
 * with no stored checksum - are never mentioned here, because there is nothing
 * to say about them and a list of maybes would train people to dismiss this.
 */
export function DuplicateDialog({ prompt }: { prompt: DuplicatePrompt | null }) {
  const t = useTranslations("upload.duplicates");
  const locale = useLocale();
  const [copied, setCopied] = useState<string | null>(null);

  const dateFormat = useMemo(
    () => new Intl.DateTimeFormat(locale, { dateStyle: "medium" }),
    [locale],
  );

  const copyOne = useCallback(async (id: string, url: string) => {
    if (!(await copyText(url))) return;
    setCopied(id);
    window.setTimeout(() => setCopied((cur) => (cur === id ? null : cur)), 1500);
  }, []);

  const hits = prompt?.hits ?? [];
  const count = hits.length;
  const total = prompt?.total ?? 0;
  const uncompared = prompt?.uncompared ?? 0;

  return (
    <Dialog
      open={Boolean(prompt)}
      // Closing by any route means "skip the copies", which is the recommended
      // answer and the one the primary button carries. It deliberately does NOT
      // mean "cancel everything": the user asked for an upload, and swallowing
      // that request because a warning was dismissed would be a worse surprise
      // than either explicit choice.
      onOpenChange={(open) => {
        if (!open) prompt?.resolve("skip");
      }}
    >
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <div className="bg-primary/10 text-primary mb-1 grid size-10 place-items-center rounded-lg">
            <Files className="size-5" />
          </div>
          <DialogTitle>{t("title", { count })}</DialogTitle>
          <DialogDescription>
            {total > count ? t("bodyPartial", { count, total }) : t("body", { count })}
          </DialogDescription>
        </DialogHeader>

        <ul className="ring-foreground/10 divide-foreground/10 max-h-72 divide-y overflow-y-auto rounded-lg ring-1">
          {hits.map((hit) => (
            <li key={hit.id} className="flex items-start gap-3 p-3">
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-medium">{hit.name}</p>
                <p className="text-muted-foreground mt-0.5 truncate text-xs">
                  {t("existing", {
                    size: humanFileSize(hit.existing.size),
                    date: dateFormat.format(new Date(hit.existing.createdAt)),
                  })}
                </p>
                <p className="text-muted-foreground/90 mt-1 flex items-center gap-1.5 text-xs">
                  <FolderOpen className="size-3.5 shrink-0" />
                  <span className="min-w-0 truncate">
                    {hit.existing.folderName
                      ? t("inFolder", { folder: hit.existing.folderName })
                      : t("inLibrary")}
                  </span>
                  {hit.copies > 1 ? (
                    <span className="shrink-0">{t("copies", { count: hit.copies })}</span>
                  ) : null}
                </p>
              </div>

              <div className="flex shrink-0 items-center gap-1">
                <Button
                  variant="ghost"
                  size="sm"
                  aria-label={t("copyExisting", { name: hit.existing.filename })}
                  onClick={() => void copyOne(hit.id, hit.existing.copyUrl)}
                >
                  {copied === hit.id ? (
                    <Check className="text-primary size-4" />
                  ) : (
                    <Copy className="size-4" />
                  )}
                </Button>
                <Button variant="ghost" size="sm" asChild>
                  <a
                    href={hit.existing.url}
                    target="_blank"
                    rel="noreferrer"
                    aria-label={t("openExisting", { name: hit.existing.filename })}
                  >
                    <ExternalLink className="size-4" />
                  </a>
                </Button>
              </div>
            </li>
          ))}
        </ul>

        {uncompared > 0 ? (
          <p className="text-muted-foreground text-xs">
            {t("uncompared", { count: uncompared })}
          </p>
        ) : null}

        <p className="text-muted-foreground text-xs">{t("hint")}</p>

        <DialogFooter>
          <Button variant="outline" onClick={() => prompt?.resolve("all")}>
            {t("uploadAnyway", { count })}
          </Button>
          <Button onClick={() => prompt?.resolve("skip")}>{t("skip", { count })}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
