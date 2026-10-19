"use client";

import * as React from "react";
import Link from "next/link";
import nextDynamic from "next/dynamic";
import { useTheme } from "next-themes";
import { useTranslations } from "next-intl";
import Uppy, { type Meta, type UppyEventMap } from "@uppy/core";
import Tus, { type TusBody } from "@uppy/tus";
import Dashboard from "@uppy/react/dashboard";
import { AlertTriangle, ArrowRight, Check, Copy, KeyRound, Lock, UploadCloud } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { copyText } from "@/components/transfers/clipboard";
import { MEDIA_URL_HEADER } from "@/components/upload/protocol";
import {
  encryptingFileReader,
  planEncryption,
  serverVisibleMeta,
  ENCRYPTED_TUS_CHUNK_BYTES,
  type EncryptionPlan,
} from "@/components/upload/encrypted-source";
import { encryptedLink } from "@/lib/crypto/link";
import { RETRY_DELAYS } from "@/components/upload/uppy-client";

import "@uppy/core/css/style.min.css";
import "@uppy/dashboard/css/style.min.css";

/**
 * End-to-end encrypted uploads.
 *
 * ## Why this is a separate engine from the panel's ordinary uploader
 *
 * The app-wide uploader lives in the shell (components/transfers) so a transfer
 * survives navigation, and a file dropped on any page joins one queue. An
 * encrypted transfer cannot have that property and must not pretend to: the key
 * exists only in this tab's memory, deliberately never written to disk, so a
 * reload has nothing to resume WITH. Sharing the shell's queue would mean the
 * dock offering a "retry" that could only ever produce a file nobody can open.
 *
 * So this component owns its own Uppy instance, its own tus plugin and its own
 * lifetime, and the copy says plainly that the page has to stay open. Everything
 * else about the transfer is identical: same endpoint, same chunked PATCHes,
 * same resume-from-the-server's-offset behaviour, same retry schedule.
 *
 * ## Where the encryption is spliced in
 *
 * Nowhere near the bytes, as far as Uppy is concerned. tus-js-client asks a
 * `fileReader` for the bytes of the upload at `[start, end)`, so we hand it a
 * reader that answers with CONTAINER bytes instead of file bytes
 * (components/upload/encrypted-source.ts). Progress, pause, resume, retries and
 * the concurrency limit all keep working because none of them ever look at what
 * the bytes mean.
 *
 * The plan (key + header) is built in a pre-processor, which Uppy awaits before
 * the uploader plugin runs. That is also where the metadata the SERVER sees is
 * swapped for an opaque name and an opaque type, which is why the file list
 * changes the name it shows the moment an upload starts: that is exactly what
 * the server is about to be told, and seeing it is the point.
 */

/** Uppy metadata for this uploader. Only the two fields tus forwards. */
type EncryptedMeta = Meta;

type Events = UppyEventMap<EncryptedMeta, TusBody>;

/** One finished upload, as this page needs to remember it. */
interface FinishedUpload {
  id: string;
  /** The real filename, which only this tab ever knew. */
  name: string;
  /** The page URL plus the key fragment. The only copy of the key there is. */
  link: string;
}

function EncryptedPanel() {
  const t = useTranslations("upload.encrypted");
  const { resolvedTheme } = useTheme();

  const [links, setLinks] = React.useState<FinishedUpload[]>([]);
  const [copiedId, setCopiedId] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState(false);

  /**
   * The engine and its two plan indexes, built once and together.
   *
   * Together because the tus file reader closes over one of them: the reader is
   * handed the raw `File` and looks a plan up by object identity, while the
   * success handler only has an Uppy file id. Both must find the SAME plan,
   * because a retry that re-planned would mint a new key and a new header for a
   * container the server is already holding half of, producing a file that
   * decrypts to nothing with either key.
   *
   * One `useState` initializer rather than a pair of refs: a ref's `.current`
   * must not be read during render, and the reader has to be wired in at
   * construction time.
   */
  const [engine] = React.useState(() => {
    const byBlob = new WeakMap<Blob, EncryptionPlan>();
    const byId = new Map<string, EncryptionPlan>();
    const uppy = new Uppy<EncryptedMeta, TusBody>({
      autoProceed: false,
      restrictions: { maxFileSize: null },
    }).use(Tus, {
      endpoint: "/api/files",
      chunkSize: ENCRYPTED_TUS_CHUNK_BYTES,
      // Two at a time rather than three: every chunk costs an AES-GCM pass over
      // 8 MiB before it can be sent, and three of those on a phone starves the
      // main thread of the very frames that draw the progress bar.
      limit: 2,
      retryDelays: RETRY_DELAYS,
      withCredentials: true,
      removeFingerprintOnSuccess: true,
      // The seam. See components/upload/encrypted-source.ts.
      fileReader: encryptingFileReader(byBlob),
    });
    return { uppy, byBlob, byId };
  });
  const { uppy, byBlob, byId } = engine;

  /* -------------------------------------------------------------------- */
  /* Plan every file before the uploader plugin sees it.                   */
  /* -------------------------------------------------------------------- */
  React.useEffect(() => {
    const preprocess = async (fileIDs: string[]) => {
      for (const id of fileIDs) {
        const file = uppy.getFile(id);
        // Uppy also models remote files, whose `data` is a size and nothing
        // else. There is no local plaintext to encrypt for one of those, and
        // sending it unencrypted under an `.mlenc` name would be a lie, so it
        // is skipped and the file reader refuses it by name below.
        if (!file || !(file.data instanceof Blob)) continue;
        if (byId.has(id)) continue; // a retry: keep the original key
        const data = file.data;

        const plan = await planEncryption({
          name: (file.meta.name as string) || file.name || "file",
          type: file.type || "application/octet-stream",
          size: file.size ?? 0,
        });
        byBlob.set(data, plan);
        byId.set(id, plan);

        // What the server is about to be told. Swapped here rather than at
        // selection time so the visitor sees the real name while choosing and
        // the opaque one the instant it becomes the truth.
        const visible = serverVisibleMeta(plan);
        uppy.setFileMeta(id, { name: visible.name, type: visible.type });
      }
    };

    uppy.addPreProcessor(preprocess);
    return () => {
      uppy.removePreProcessor(preprocess);
    };
  }, [uppy, byBlob, byId]);

  /* -------------------------------------------------------------------- */
  /* Collect links, with the key fragment attached in the browser.         */
  /* -------------------------------------------------------------------- */
  React.useEffect(() => {
    const onSuccess: Events["upload-success"] = (file, response) => {
      if (!file) return;
      const plan = byId.get(file.id);
      if (!plan) return;
      let url: string | null = null;
      try {
        // tus answers with no body, so finalize hands the page URL back as a
        // response header on the last PATCH.
        url = response?.body?.xhr?.getResponseHeader(MEDIA_URL_HEADER) ?? null;
      } catch {
        url = null;
      }
      if (!url) return;
      setLinks((current) => [
        ...current,
        { id: file.id, name: plan.plainName, link: encryptedLink(url, plan.encodedKey) },
      ]);
    };

    const onUploadStart = () => setBusy(true);
    const onComplete = () => setBusy(false);

    uppy.on("upload-success", onSuccess);
    uppy.on("upload", onUploadStart);
    uppy.on("complete", onComplete);
    return () => {
      uppy.off("upload-success", onSuccess);
      uppy.off("upload", onUploadStart);
      uppy.off("complete", onComplete);
    };
  }, [uppy, byId]);

  /* -------------------------------------------------------------------- */
  /* Closing the tab mid-transfer destroys the key. Say so.                */
  /* -------------------------------------------------------------------- */
  React.useEffect(() => {
    if (!busy) return;
    const warn = (event: BeforeUnloadEvent) => event.preventDefault();
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, [busy]);

  const copyOne = React.useCallback(
    async (id: string, link: string, name: string) => {
      if (await copyText(link)) {
        setCopiedId(id);
        window.setTimeout(() => setCopiedId((cur) => (cur === id ? null : cur)), 1500);
      } else {
        toast.error(t("copyFailed"), { description: name });
      }
    },
    [t],
  );

  const copyAll = React.useCallback(async () => {
    const text = links.map((l) => l.link).join("\n");
    if (await copyText(text)) toast.success(t("copiedAll", { count: links.length }));
    else toast.error(t("copyFailed"));
  }, [links, t]);

  const dashboardTheme =
    resolvedTheme === "light" ? "light" : resolvedTheme === "dark" ? "dark" : "auto";

  return (
    <div className="space-y-5" data-no-global-drop>
      <div className="ring-primary/25 bg-primary/5 rounded-xl p-4 ring-1">
        <div className="flex items-start gap-3">
          <span className="bg-primary/10 text-primary ring-primary/20 grid size-9 shrink-0 place-items-center rounded-lg ring-1">
            <KeyRound className="size-4" />
          </span>
          <div className="min-w-0 space-y-1.5 text-sm">
            <p className="font-medium">{t("deal.title")}</p>
            <ul className="text-muted-foreground list-disc space-y-1 pl-4 text-xs leading-relaxed">
              <li>{t("deal.link")}</li>
              <li>{t("deal.lost")}</li>
              <li>{t("deal.leaks")}</li>
              <li>{t("deal.noPreview")}</li>
              <li>{t("deal.tab")}</li>
            </ul>
          </div>
        </div>
      </div>

      <div className="uppy-dashboard-shell ring-foreground/10 bg-card overflow-hidden rounded-xl shadow-sm ring-1">
        <Dashboard
          uppy={uppy}
          theme={dashboardTheme}
          width="100%"
          height={420}
          proudlyDisplayPoweredByUppy={false}
          note={t("dashboardNote")}
        />
      </div>

      {links.length > 0 ? (
        <div className="ring-primary/20 bg-primary/5 rounded-xl ring-1">
          <div className="border-primary/15 flex flex-wrap items-center justify-between gap-3 border-b px-4 py-3">
            <p className="text-sm font-medium">{t("results.title", { count: links.length })}</p>
            <div className="flex items-center gap-2">
              {links.length > 1 ? (
                <Button variant="outline" size="sm" onClick={() => void copyAll()}>
                  <Copy className="size-4" />
                  {t("results.copyAll")}
                </Button>
              ) : null}
              <Button asChild size="sm">
                <Link href="/dashboard">
                  {t("results.viewFiles")}
                  <ArrowRight className="size-4" />
                </Link>
              </Button>
            </div>
          </div>

          <div className="border-primary/15 flex items-start gap-2 border-b px-4 py-2.5">
            <AlertTriangle className="mt-0.5 size-4 shrink-0 text-amber-500" aria-hidden />
            <p className="text-muted-foreground text-xs leading-relaxed">{t("results.warning")}</p>
          </div>

          <ul className="divide-primary/10 divide-y">
            {links.map((item) => (
              <li key={item.id} className="flex items-center gap-3 px-4 py-2.5">
                <Lock className="text-primary size-4 shrink-0" aria-hidden />
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm">{item.name}</p>
                  <p className="text-muted-foreground truncate font-mono text-xs">{item.link}</p>
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => void copyOne(item.id, item.link, item.name)}
                  aria-label={t("results.copyOne", { name: item.name })}
                >
                  {copiedId === item.id ? (
                    <Check className="text-primary size-4" />
                  ) : (
                    <Copy className="size-4" />
                  )}
                </Button>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  );
}

function PanelSkeleton() {
  const t = useTranslations("upload");
  return (
    <div className="bg-card/40 ring-foreground/5 grid h-[420px] place-items-center rounded-xl border border-dashed ring-1">
      <div className="text-muted-foreground flex flex-col items-center gap-3">
        <UploadCloud className={cn("size-8 animate-pulse")} />
        <p className="text-sm">{t("preparing")}</p>
      </div>
    </div>
  );
}

// ssr:false keeps Uppy and the WebCrypto plumbing out of the prerender. Legal
// here because the surrounding module is already a Client Component.
const LazyEncryptedPanel = nextDynamic(() => Promise.resolve(EncryptedPanel), {
  ssr: false,
  loading: () => <PanelSkeleton />,
});

export function EncryptedUploader() {
  return <LazyEncryptedPanel />;
}

export default EncryptedUploader;
