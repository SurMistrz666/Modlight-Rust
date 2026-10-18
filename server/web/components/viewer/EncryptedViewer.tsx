"use client";

import * as React from "react";
import Link from "next/link";
import { useLocale, useTranslations } from "next-intl";
import {
  AlertTriangle,
  Check,
  Clock,
  Copy,
  Download,
  Eye,
  HardDrive,
  KeyRound,
  Loader2,
  Lock,
  ShieldCheck,
} from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { humanFileSize, relativeTime } from "@/lib/format";
import { copyText } from "@/components/transfers/clipboard";
import { ModlightMark } from "@/components/brand/ModlightMark";
import { FloatingBar } from "@/components/viewer/FloatingBar";
import { ImageView } from "@/components/viewer/ImageView";
import { MediaPlayer } from "@/components/viewer/MediaPlayer";
import { TextView } from "@/components/viewer/TextView";
import {
  decryptedPresentation,
  extOfName,
  MAX_BLOB_SAVE_BYTES,
  SAVE_BLOB_TYPE,
  type DecryptedPresentation,
} from "@/components/viewer/decrypted-presentation";
import {
  assertNoKeyInUrl,
  decryptToBlob,
  decryptToSink,
  fetchContainerHead,
  openContainer,
  TransferError,
  type DecryptOptions,
} from "@/components/viewer/decrypt-transfer";
import { EnvelopeError, importKey, type EnvelopeParams } from "@/lib/crypto/envelope";
import { keyFromFragment, toBase64Url } from "@/lib/crypto/link";

export interface EncryptedViewerProps {
  /** Same-origin path of the raw container. Never carries the key. */
  rawPath: string;
  /** Size of the STORED container, which is all the server knows. */
  cipherSize: number;
  createdAt: string;
  views: number;
  downloads: number;
  appName: string;
}

type Phase = "no-key" | "opening" | "reading" | "ready" | "failed";

/**
 * Why something failed, as a key rather than a sentence.
 *
 * Deliberately not a translated string: a translator function is a new identity
 * on most renders, so an effect that closed over one would re-run - and this
 * component's effects FETCH. Re-running the opening effect would issue a second
 * `bytes=0-` request, which is the one request `/raw` counts, so every render
 * would spend another view and a burn-after-view link would die instantly.
 * Reasons go into state; the sentence is picked at render time.
 */
type FailureReason =
  | "badKey"
  | "notAContainer"
  | "truncated"
  | "version"
  | "malformed"
  | "gone"
  | "locked"
  | "network"
  | "cancelled";

/** Classify a failure. Pure, so it can live outside the component. */
function failureReason(err: unknown): FailureReason {
  if (err instanceof EnvelopeError) {
    switch (err.reason) {
      case "bad-key":
        return "badKey";
      case "not-a-container":
        return "notAContainer";
      case "truncated":
        return "truncated";
      case "unsupported-version":
        return "version";
      default:
        return "malformed";
    }
  }
  if (err instanceof TransferError) {
    if (err.status === 404) return "gone";
    if (err.status === 401) return "locked";
    return "network";
  }
  if (err instanceof DOMException && err.name === "AbortError") return "cancelled";
  return "network";
}

interface OpenFile {
  key: CryptoKey;
  params: EnvelopeParams;
  head: Uint8Array;
  complete: boolean;
  name: string;
  type: string;
  presentation: DecryptedPresentation;
}

/** File System Access, typed narrowly so no lib.dom version is required. */
interface SaveFilePicker {
  showSaveFilePicker?: (options?: {
    suggestedName?: string;
  }) => Promise<{ createWritable(): Promise<WritableFileStream> }>;
}
interface WritableFileStream {
  write(data: Uint8Array): Promise<void>;
  close(): Promise<void>;
  abort?(reason?: unknown): Promise<void>;
}

/**
 * The viewer for an end-to-end encrypted upload.
 *
 * Everything interesting happens in the visitor's browser. The server handed
 * this component a path and a byte count; it has never seen the filename, the
 * type or a single byte of content, and it cannot, because the key exists only
 * in the fragment of the URL - the one part a browser never transmits.
 *
 * Three things about the shape of this file are load-bearing:
 *
 *  - the key is read from `location.hash` and imported as a non-extractable
 *    `CryptoKey`. It is never put in state that could be serialised, never
 *    logged, and never passed to anything that builds a URL;
 *  - what gets rendered is decided by `decryptedPresentation`, which delegates
 *    to the same allowlist `/raw` uses. A decrypted HTML file is drawn as
 *    escaped text, exactly as an uploaded one is, and never as a document -
 *    a `blob:` URL would carry OUR origin;
 *  - reading is two ranged requests and no more, because the first one is what
 *    spends this file's view. See components/viewer/decrypt-transfer.ts.
 */
export function EncryptedViewer(props: EncryptedViewerProps) {
  const t = useTranslations("viewer.encrypted");
  const tv = useTranslations("viewer");
  const locale = useLocale();

  const [phase, setPhase] = React.useState<Phase>("opening");
  const [reason, setReason] = React.useState<FailureReason | null>(null);
  const [file, setFile] = React.useState<OpenFile | null>(null);
  const [progress, setProgress] = React.useState(0);
  const [objectUrl, setObjectUrl] = React.useState<string | null>(null);
  const [text, setText] = React.useState<string | null>(null);
  const [saving, setSaving] = React.useState(false);
  const [copied, setCopied] = React.useState(false);

  // Held in a ref, not in state: state is snapshotted by React DevTools and by
  // every error reporter that walks the fibre tree.
  const encodedKey = React.useRef<string>("");

  /* ---------------------------------------------------------------------- */
  /* Open the container: one ranged read, then the metadata frame.           */
  /* ---------------------------------------------------------------------- */
  React.useEffect(() => {
    const controller = new AbortController();
    let cancelled = false;

    void (async () => {
      // Read inside the async body, not in the effect body: `location` does not
      // exist during the server render, so this cannot be derived state, and a
      // synchronous setState here would be a cascading render.
      const raw = keyFromFragment(window.location.hash);
      if (!raw) {
        if (!cancelled) setPhase("no-key");
        return;
      }
      encodedKey.current = toBase64Url(raw);

      try {
        const key = await importKey(raw);
        raw.fill(0);
        assertNoKeyInUrl(props.rawPath, encodedKey.current);
        const { head, complete } = await fetchContainerHead(props.rawPath, controller.signal);
        const { params, metadata } = await openContainer(head, key);
        if (cancelled) return;
        setFile({
          key,
          params,
          head,
          complete,
          name: metadata.name,
          type: metadata.type,
          presentation: decryptedPresentation(
            metadata.type,
            extOfName(metadata.name),
            params.plainTotalBytes,
          ),
        });
        setPhase("reading");
      } catch (err) {
        if (cancelled) return;
        setReason(failureReason(err));
        setPhase("failed");
      }
    })();

    return () => {
      cancelled = true;
      controller.abort();
    };
  }, [props.rawPath]);

  /* ---------------------------------------------------------------------- */
  /* Read the body, when this file is one we can show.                       */
  /* ---------------------------------------------------------------------- */
  React.useEffect(() => {
    if (!file || phase !== "reading") return;
    // A file we will not preview needs no body read at all. The phase is left
    // where it is on purpose: the stage draws the download card from the
    // presentation, not from the phase, so there is nothing to advance.
    if (file.presentation.mode === "download") return;

    const controller = new AbortController();
    let cancelled = false;

    void (async () => {
      const options: DecryptOptions = {
        url: props.rawPath,
        key: file.key,
        params: file.params,
        head: file.head,
        complete: file.complete,
        signal: controller.signal,
        onProgress: (done, total) => {
          if (!cancelled && total > 0) setProgress(Math.round((done / total) * 100));
        },
      };

      try {
        if (file.presentation.mode === "text") {
          const blob = await decryptToBlob(options, "text/plain");
          if (cancelled) return;
          setText(await blob.text());
        } else {
          const blob = await decryptToBlob(
            options,
            file.presentation.renderType ?? SAVE_BLOB_TYPE,
          );
          if (cancelled) return;
          const href = URL.createObjectURL(blob);
          // The only place a URL is revoked eagerly: the read finished after we
          // were told to stop, so nothing will ever hold this one.
          if (cancelled) {
            URL.revokeObjectURL(href);
            return;
          }
          setObjectUrl(href);
        }
        if (!cancelled) setPhase("ready");
      } catch (err) {
        if (cancelled) return;
        setReason(failureReason(err));
        setPhase("failed");
      }
    })();

    // Deliberately NOT revoking the object URL here. This effect re-runs the
    // moment `phase` becomes "ready", so a revoke in its cleanup would tear
    // down the very URL the <img>/<video> is about to render from. Revocation
    // belongs to the URL's own lifetime, below.
    return () => {
      cancelled = true;
      controller.abort();
    };
  }, [file, phase, props.rawPath]);

  /** Free an object URL when it is replaced, and when the page goes away. */
  React.useEffect(() => {
    if (!objectUrl) return;
    return () => URL.revokeObjectURL(objectUrl);
  }, [objectUrl]);

  /* ---------------------------------------------------------------------- */
  /* Actions                                                                 */
  /* ---------------------------------------------------------------------- */

  const copyLink = React.useCallback(async () => {
    // The address bar already holds the whole thing, fragment included. Reading
    // it rather than rebuilding it means the copied link is exactly the one that
    // worked, and there is no second place that has to remember the format.
    const ok = await copyText(window.location.href);
    if (ok) {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } else {
      toast.error(t("copyFailed"));
    }
  }, [t]);

  const save = React.useCallback(async () => {
    if (!file || saving) return;
    setSaving(true);

    const picker = window as unknown as SaveFilePicker;
    // Asked for BEFORE any await, while the click still counts as a user
    // gesture: the picker is refused without one.
    let sink: WritableFileStream | null = null;
    if (typeof picker.showSaveFilePicker === "function") {
      try {
        const handle = await picker.showSaveFilePicker({ suggestedName: file.name });
        sink = await handle.createWritable();
      } catch {
        setSaving(false);
        return; // the visitor cancelled the dialog
      }
    }

    const options: DecryptOptions = {
      url: props.rawPath,
      key: file.key,
      params: file.params,
      head: file.head,
      complete: file.complete,
      onProgress: (done, total) => {
        if (total > 0) setProgress(Math.round((done / total) * 100));
      },
    };

    try {
      if (sink) {
        await decryptToSink(options, sink);
        toast.success(t("saved"));
        return;
      }
      if (file.params.plainTotalBytes > MAX_BLOB_SAVE_BYTES) {
        toast.error(t("tooBigToSave", { size: humanFileSize(MAX_BLOB_SAVE_BYTES) }));
        return;
      }
      const blob = await decryptToBlob(options, SAVE_BLOB_TYPE);
      const href = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      anchor.href = href;
      anchor.download = file.name;
      anchor.rel = "noopener";
      anchor.click();
      // Revoked late: revoking before the browser has picked the blob up
      // cancels the download in Safari.
      window.setTimeout(() => URL.revokeObjectURL(href), 60_000);
    } catch (err) {
      toast.error(t(`errors.${failureReason(err)}`));
    } finally {
      setSaving(false);
    }
  }, [file, saving, props.rawPath, t]);

  /* ---------------------------------------------------------------------- */
  /* Render                                                                  */
  /* ---------------------------------------------------------------------- */

  const displayName = file?.name ?? t("unknownName");
  const displaySize = file ? file.params.plainTotalBytes : props.cipherSize;

  return (
    <div
      className="relative flex min-h-0 w-full flex-1 flex-col"
      style={{ paddingBottom: "calc(var(--viewer-bar-space, 7rem) + 0.75rem)" }}
    >
      <Link
        href="/dashboard"
        aria-label={tv("backToPanel", { appName: props.appName })}
        title={tv("backToPanel", { appName: props.appName })}
        className="bg-card/80 ring-foreground/10 text-muted-foreground hover:text-foreground absolute top-3 left-3 z-20 inline-flex items-center gap-2 rounded-full py-1.5 pr-3.5 pl-2 text-xs font-medium shadow-sm ring-1 backdrop-blur transition-colors"
      >
        <ModlightMark className="size-5 shrink-0" />
        <span className="hidden sm:inline">{props.appName}</span>
      </Link>

      <div className="relative flex min-h-0 w-full flex-1 items-center justify-center overflow-hidden">
        <Stage
          phase={phase}
          reason={reason}
          file={file}
          objectUrl={objectUrl}
          text={text}
          progress={progress}
          onSave={save}
          saving={saving}
        />
      </div>

      <FloatingBar>
        <div className="flex flex-col gap-2.5 lg:flex-row lg:items-center lg:gap-4">
          <div className="flex min-w-0 flex-1 items-center gap-2.5">
            <span className="bg-primary/10 text-primary ring-primary/20 grid size-9 shrink-0 place-items-center rounded-lg ring-1">
              <Lock className="size-4" />
            </span>
            <div className="min-w-0">
              <h1
                className="text-foreground truncate text-sm font-semibold tracking-tight"
                title={displayName}
              >
                {displayName}
              </h1>
              <div className="text-muted-foreground mt-0.5 flex min-w-0 items-center gap-x-2 overflow-hidden text-[11px] whitespace-nowrap">
                <Fact icon={ShieldCheck} text={t("badge")} />
                <Dot />
                <Fact icon={HardDrive} text={humanFileSize(displaySize)} />
                <Dot />
                <Fact icon={Eye} text={String(props.views)} />
                <span className="hidden sm:inline-flex sm:items-center sm:gap-2">
                  <Dot />
                  <Fact icon={Download} text={String(props.downloads)} />
                </span>
                <span className="hidden md:inline-flex md:items-center md:gap-2">
                  <Dot />
                  <Fact icon={Clock} text={relativeTime(new Date(props.createdAt), locale)} />
                </span>
              </div>
            </div>
          </div>

          <div className="flex shrink-0 items-center gap-2">
            <button
              type="button"
              onClick={() => void copyLink()}
              className="ring-foreground/10 bg-card hover:bg-accent hover:text-accent-foreground inline-flex items-center gap-1.5 rounded-lg px-3 py-2 text-xs font-medium ring-1 transition-colors"
            >
              {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
              {copied ? t("copied") : t("copyLink")}
            </button>
            <button
              type="button"
              onClick={() => void save()}
              disabled={!file || saving}
              className="bg-primary text-primary-foreground hover:bg-primary/90 inline-flex items-center gap-1.5 rounded-lg px-3 py-2 text-xs font-medium transition-colors disabled:opacity-50"
            >
              {saving ? (
                <Loader2 className="size-3.5 animate-spin" />
              ) : (
                <Download className="size-3.5" />
              )}
              {t("save")}
            </button>
          </div>
        </div>
      </FloatingBar>
    </div>
  );
}

function Dot() {
  return <span className="bg-muted-foreground/40 size-0.5 shrink-0 rounded-full" aria-hidden />;
}

function Fact({
  icon: Icon,
  text,
}: {
  icon: React.ComponentType<{ className?: string }>;
  text: string;
}) {
  return (
    <span className="inline-flex shrink-0 items-center gap-1">
      <Icon className="size-3 opacity-70" aria-hidden />
      {text}
    </span>
  );
}

function Stage({
  phase,
  reason,
  file,
  objectUrl,
  text,
  progress,
  onSave,
  saving,
}: {
  phase: Phase;
  reason: FailureReason | null;
  file: OpenFile | null;
  objectUrl: string | null;
  text: string | null;
  progress: number;
  onSave: () => void;
  saving: boolean;
}) {
  const t = useTranslations("viewer.encrypted");

  if (phase === "no-key") {
    return (
      <Card icon={KeyRound} tone="warning" title={t("noKey.title")}>
        <p>{t("noKey.body")}</p>
      </Card>
    );
  }

  if (phase === "failed") {
    return (
      <Card icon={AlertTriangle} tone="danger" title={t("failed.title")}>
        <p>{t(`errors.${reason ?? "network"}`)}</p>
      </Card>
    );
  }

  if (!file || phase === "opening") {
    return (
      <Card icon={Lock} tone="muted" title={t("opening.title")}>
        <p>{t("opening.body")}</p>
      </Card>
    );
  }

  if (file.presentation.mode === "download") {
    const explanation =
      file.presentation.reason === "too-large"
        ? t("noPreview.tooLarge", { size: humanFileSize(file.params.plainTotalBytes) })
        : t("noPreview.type");
    return (
      <Card icon={Lock} tone="muted" title={file.name}>
        <p>{explanation}</p>
        <button
          type="button"
          onClick={onSave}
          disabled={saving}
          className="bg-primary text-primary-foreground hover:bg-primary/90 mt-4 inline-flex items-center gap-2 rounded-lg px-4 py-2 text-sm font-medium transition-colors disabled:opacity-50"
        >
          {saving ? <Loader2 className="size-4 animate-spin" /> : <Download className="size-4" />}
          {saving && progress > 0 ? t("savingPercent", { percent: progress }) : t("save")}
        </button>
      </Card>
    );
  }

  if (phase === "reading") {
    return (
      <Card icon={Lock} tone="muted" title={t("decrypting.title")}>
        <p>{t("decrypting.body", { percent: progress })}</p>
        <div className="bg-muted mt-4 h-1 w-full overflow-hidden rounded-full">
          <div
            className="bg-primary h-full transition-all"
            style={{ width: `${Math.max(2, progress)}%` }}
          />
        </div>
      </Card>
    );
  }

  if (file.presentation.mode === "text") {
    return (
      <div className="h-full w-full px-3 py-3 sm:px-6">
        {/* Escaped by React as text children, exactly like an uploaded paste.
            There is no dangerouslySetInnerHTML anywhere on this path. */}
        <TextView content={text ?? ""} size={file.params.plainTotalBytes} />
      </div>
    );
  }

  if (!objectUrl) return null;

  if (file.presentation.mode === "image") {
    return <ImageView rawUrl={objectUrl} filename={file.name} />;
  }

  return (
    <div className="flex h-full w-full items-center justify-center px-3 py-3 sm:px-6">
      <div className="w-full max-w-5xl">
        <MediaPlayer
          src={objectUrl}
          type={file.presentation.renderType ?? "video/mp4"}
          variant={file.presentation.mode === "audio" ? "audio" : "video"}
          unsupported={t("playerUnsupported")}
        />
      </div>
    </div>
  );
}

function Card({
  icon: Icon,
  tone,
  title,
  children,
}: {
  icon: React.ComponentType<{ className?: string }>;
  tone: "muted" | "warning" | "danger";
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="ring-foreground/10 bg-card/70 mx-4 w-full max-w-md rounded-2xl p-6 text-center shadow-sm ring-1 backdrop-blur">
      <span
        className={cn(
          "mx-auto mb-4 grid size-11 place-items-center rounded-xl ring-1",
          tone === "danger" && "bg-destructive/10 text-destructive ring-destructive/20",
          tone === "warning" && "bg-amber-500/10 text-amber-600 ring-amber-500/20",
          tone === "muted" && "bg-primary/10 text-primary ring-primary/20",
        )}
      >
        <Icon className="size-5" />
      </span>
      <h2 className="text-foreground text-sm font-semibold tracking-tight">{title}</h2>
      <div className="text-muted-foreground mt-2 text-sm leading-relaxed">{children}</div>
    </div>
  );
}

export default EncryptedViewer;
