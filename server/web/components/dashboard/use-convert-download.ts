"use client";

import { useCallback } from "react";
import { useTranslations } from "next-intl";
import { toast } from "sonner";
import { noteKey, REFUSAL_KEYS } from "@/lib/media/convert-notes";

/**
 * "Download as…", with a mouth.
 *
 * The entries in that menu used to be plain `<a download>`. When the server
 * refused - no audio track to extract, codecs that will not go into the chosen
 * container, a file over the size ceiling, the transform queue full - it said so
 * in an `X-Convert-Refused` header on a response the browser had already
 * committed to downloading. Nothing read the header, so every one of those
 * refusals reached the user as the word "Failed" in the downloads shelf.
 *
 * That single missing reader is why "mp3" looked broken on 93 of 304 videos: the
 * offer was honest, the refusal was correct, and the explanation was thrown
 * away.
 *
 * So the click asks first (`?check=1`, which runs every gate and no work) and
 * only then starts the download. The download itself stays an anchor click
 * rather than a fetch-to-blob: a fetched 2 GB video would be assembled in the
 * tab's memory before the user saw a single byte, and the browser's own download
 * manager already does this well.
 */

export function useConvertDownload() {
  const t = useTranslations("files");

  const start = useCallback(
    async (href: string, label: string) => {
      // The menu closes the instant the entry is chosen, so there is nowhere on
      // screen left to put a spinner - and the check can genuinely take a while
      // when the transform queue is busy, because it waits for a slot before it
      // can probe. Silence there reads as a dead click.
      //
      // Delayed on purpose: a check that answers from the row or from cache is
      // done in around a tenth of a second, and a toast that appears and
      // vanishes in that time is worse than no toast. This only ever appears for
      // the slow case it exists to explain.
      let pending: string | number | undefined;
      const announce = setTimeout(() => {
        pending = toast.loading(t("convert.checking", { format: label }));
      }, 400);

      const settle = () => {
        clearTimeout(announce);
        if (pending !== undefined) toast.dismiss(pending);
      };

      try {
        const res = await fetch(`${href}${href.includes("?") ? "&" : "?"}check=1`, {
          // The owner-only encoders are recognised by session cookie; without
          // this every check on a private file would come back 403.
          credentials: "same-origin",
        });

        settle();

        if (!res.ok) {
          const reason = res.headers.get("X-Convert-Refused") ?? "";
          toast.error(t("convert.refusedTitle", { format: label }), {
            description: REFUSAL_KEYS.has(reason)
              ? t(`convert.refused.${reason}`)
              : t("convert.refused.unknown"),
          });
          return;
        }

        // A yes that comes with a caveat: the format will be produced, but the
        // sound could not come along, or this is a clip of something longer.
        // Told BEFORE the download rather than discovered after it.
        // `audio-reencoded` is deliberately left out: the file keeps its sound
        // and its full length, only the encoding changed. A toast on every
        // container copy is the noise that teaches people to stop reading
        // toasts - including the two below, which they need.
        const note = noteKey(res.headers.get("X-Convert-Note"));
        if (note && note !== "audioReencoded") {
          toast.info(t("convert.startedTitle", { format: label }), {
            description: t(`convert.note.${note}`),
          });
        }

        // Same-origin anchor click: the browser streams it to disk and names it
        // from Content-Disposition. `rel=noopener` costs nothing and keeps this
        // from ever handing a window reference away.
        const a = document.createElement("a");
        a.href = href;
        a.download = "";
        a.rel = "noopener";
        document.body.appendChild(a);
        a.click();
        a.remove();
      } catch {
        settle();
        // fetch only rejects when the request never got an answer: offline, DNS,
        // a connection dropped mid-flight. Saying "the server refused" here
        // would be a guess, and the wrong one.
        toast.error(t("convert.refusedTitle", { format: label }), {
          description: t("convert.refused.offline"),
        });
      } finally {
        // Belt and braces: settle() is idempotent, and a path that ever forgot
        // it would leave a spinner on screen forever.
        settle();
      }
    },
    [t],
  );

  return { start };
}
