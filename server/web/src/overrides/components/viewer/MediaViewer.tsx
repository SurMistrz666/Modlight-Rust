// Client port of upstream components/viewer/MediaViewer.tsx (an async Server Component there).
// @upstream components/viewer/MediaViewer.tsx
// Deviations: `async` dropped; getTranslations/getFormatter/getLocale ->
// useTranslations/useFormatter/useLocale. `config` and `sandboxShape` resolve to
// their browser versions (src/overrides/lib/config.ts, the render-ticket mirror).
import {
  FileText,
  FileAudio,
  FileVideo,
  FileImage,
  FileArchive,
  FileCode,
  File as FileIcon,
  Eye,
  Download,
  Clock,
  HardDrive,
  Ruler,
  ShieldCheck,
} from "lucide-react";
import Link from "next/link";
import { useFormatter, useLocale, useTranslations } from "next-intl";
import { config } from "@/lib/config";
import {
  mediaKind,
  mimeToIcon,
  isDisplayableImage,
  playableVideoType,
  playableAudioType,
} from "@/lib/media/mime";
import { buildLinkFormats } from "@/lib/media/link-formats";
import { humanFileSize, relativeTime } from "@/lib/format";
import { cn } from "@/lib/utils";
import { sandboxShape } from "@/lib/media/render-ticket";
import { codeLanguageLabel, showsSource, sourceReadings } from "@/lib/media/source-preview";
import { docxSupport } from "@/lib/media/docx-support";
import { ModlightMark } from "@/components/brand/ModlightMark";
import { ImageView } from "@/components/viewer/ImageView";
import { SourceView } from "@/components/viewer/SourceView";
import type { TextUnavailable } from "@/components/viewer/TextView";
import { DocumentView } from "@/components/viewer/DocumentView";
import { MediaActions } from "@/components/viewer/MediaActions";
import { MediaPlayer } from "@/components/viewer/MediaPlayer";
import { CopyButton } from "@/components/viewer/CopyButton";
import { FloatingBar } from "@/components/viewer/FloatingBar";
import { PlaybackNote, type PlaybackRepair } from "@/components/viewer/PlaybackNote";
import type { PlaybackDiagnosis } from "@/lib/media/playback-health";

export interface MediaViewerProps {
  filename: string;
  mimeType: string;
  ext: string;
  size: number;
  views: number;
  downloads: number;
  createdAt: Date;
  width?: number | null;
  height?: number | null;
  /** Frames in the file; decides what "Download as…" offers and how it labels it. */
  frames?: number | null;
  publicUrl: string;
  rawUrl: string;
  downloadUrl: string;
  /** Owner preference: prefer copying the raw URL. */
  copyRaw?: boolean;
  /**
   * The viewer owns this file (or is an admin), so the encoder-backed
   * conversions are on offer. False for a stranger holding a public link.
   */
  canEncode?: boolean;
  /**
   * Where to open this file on the app's own hostname, when this page is being
   * served somewhere the session cookie does not reach. Turns a silently short
   * "Download as…" menu into one link that explains it.
   */
  signInUrl?: string | null;
  /** Raw text content for text/paste files, read server-side. */
  textContent?: string | null;
  /** Set when the file is text but its contents could not be inlined, and why. */
  textUnavailable?: TextUnavailable;
  /**
   * Relative URL (ticket included) of the sandboxed render for an uploaded
   * HTML/SVG document. Null whenever rendering is not on offer.
   */
  renderPath?: string | null;
  /**
   * The two things the video player fetches beside the file itself: the
   * scrub-bar preview track and the still shown before playback. Null whenever
   * this file cannot have one.
   *
   * Paths on the page's own origin, never URLs on the owner's preferred
   * hostname - a text track is fetched under CORS rules, this deployment
   * answers on three names with no CORS headers between them, and the unlock
   * cookie of a protected file exists only on the one the visitor is on.
   */
  timelineTrack?: string | null;
  posterUrl?: string | null;
  /**
   * What is wrong with this video's SHAPE, when anything is.
   *
   * Computed on the server from the upload row alone
   * (lib/media/playback-health.ts) and set only for the file's owner: a
   * visitor cannot act on it, and a page that apologises for the recording
   * it is showing helps nobody.
   */
  playback?: PlaybackDiagnosis | null;
  /** What the repair button needs to open its dialog. Null for a visitor. */
  playbackRepair?: PlaybackRepair | null;
}

const ICONS: Record<string, React.ComponentType<{ className?: string }>> = {
  FileText,
  FileAudio,
  FileVideo,
  FileImage,
  FileArchive,
  FileCode,
  File: FileIcon,
};

/**
 * Premium, kind-aware file viewer (Imgur/Gyazo-grade). Server component.
 *
 * Layout: the media gets the whole screen, and the file's identity plus every
 * action live in a bar floating over the bottom edge. Nothing frames the media —
 * no card, no glow, no checkerboard — so a screenshot looks like the file and
 * not like a product page. `--viewer-bar-space` (published by FloatingBar) is
 * the bar's measured height; reserving it here is what keeps the bar from ever
 * covering the bottom of the picture.
 */
export function MediaViewer(props: MediaViewerProps) {
  const {
    filename,
    mimeType,
    ext,
    size,
    views,
    downloads,
    createdAt,
    width,
    height,
    frames,
    publicUrl,
    rawUrl,
    downloadUrl,
    copyRaw = false,
    canEncode = false,
    signInUrl = null,
    textContent,
    textUnavailable,
    renderPath = null,
    timelineTrack = null,
    posterUrl = null,
    playback = null,
    playbackRepair = null,
  } = props;

  const t = useTranslations("viewer");
  const format = useFormatter();
  // relativeTime formats through Intl, so it has to be told whose language
  // this is; without it a Polish reader gets English ages.
  const locale = useLocale();
  // The extension is passed everywhere the type is: rows stored as
  // application/octet-stream (ShareX on Windows, old imports) are
  // otherwise classified as "other" and land on the icon placeholder.
  const kind = mediaKind(mimeType, ext);
  const isImage = isDisplayableImage(mimeType, ext);
  const formats = buildLinkFormats({ url: publicUrl, rawUrl, filename, isImage });
  const typeLabel = prettyType(mimeType, ext);

  /**
   * One line, always. Each entry carries its own responsive visibility and its
   * separator travels with it, so hiding "dimensions" on a phone never leaves a
   * dangling dot. The counters render as icon + number with the spelled-out
   * value on `aria-label`/`title`: "12 views · 3 downloads" is what pushed this
   * row onto a second line on every screen narrower than a desktop.
   */
  const facts: { key: string; node: React.ReactNode; className?: string }[] = [
    { key: "size", node: <Meta icon={HardDrive} text={humanFileSize(size)} /> },
    { key: "type", node: <Meta icon={ShieldCheck} text={typeLabel} /> },
    ...(width && height
      ? [
          {
            key: "dimensions",
            node: <Meta icon={Ruler} text={`${width} × ${height}`} />,
            className: "hidden md:inline-flex",
          },
        ]
      : []),
    {
      key: "views",
      node: (
        <Count
          icon={Eye}
          value={format.number(views)}
          label={t("views", { count: views })}
        />
      ),
    },
    {
      key: "downloads",
      node: (
        <Count
          icon={Download}
          value={format.number(downloads)}
          label={t("downloads", { count: downloads })}
        />
      ),
    },
    {
      key: "created",
      node: <Meta icon={Clock} text={relativeTime(createdAt, locale)} />,
      className: "hidden sm:inline-flex",
    },
  ];

  const appName = config.appName;

  return (
    <div
      className="relative flex min-h-0 w-full flex-1 flex-col"
      style={{ paddingBottom: "calc(var(--viewer-bar-space, 7rem) + 0.75rem)" }}
    >
      {/* Way back to the panel.
          Without it a shared link is a
          dead end: a signed-in owner following their own link had no route
          onwards except the browser's back button. Floated over the stage
          rather than given a row of its own — the viewer's whole layout is
          "the file gets every pixel the bar does not need", and a header would
          take a slice of that from every file forever. */}
      <Link
        href="/dashboard"
        aria-label={t("backToPanel", { appName })}
        title={t("backToPanel", { appName })}
        className="bg-card/80 ring-foreground/10 text-muted-foreground hover:text-foreground absolute top-3 left-3 z-20 inline-flex items-center gap-2 rounded-full py-1.5 pr-3.5 pl-2 text-xs font-medium shadow-sm ring-1 backdrop-blur transition-colors"
      >
        <ModlightMark className="size-5 shrink-0" />
        <span className="hidden sm:inline">{appName}</span>
      </Link>

      {/* Stage — every pixel the floating bar doesn't need. */}
      <div className="relative flex min-h-0 w-full flex-1 items-center justify-center overflow-hidden">
        <Stage
          kind={kind}
          ext={ext}
          filename={filename}
          mimeType={mimeType}
          rawUrl={rawUrl}
          size={size}
          width={width}
          height={height}
          textContent={textContent}
          textUnavailable={textUnavailable}
          renderPath={renderPath}
          frames={frames}
          timelineTrack={timelineTrack}
          posterUrl={posterUrl}
          videoUnsupported={t("videoUnsupported")}
          audioUnsupported={t("audioUnsupported")}
          noPreviewLabel={t("noInlinePreview")}
          copyRawLabel={t("copyRawLink")}
        />
      </div>

      <FloatingBar>
        {/* The bar measures itself and publishes its height, so an extra
            row here cannot end up covering the bottom of the picture. */}
        {playback ? (
          <PlaybackNote
            diagnosis={playback}
            repair={playbackRepair}
            className="mb-2.5"
          />
        ) : null}
        <div className="flex flex-col gap-2.5 lg:flex-row lg:items-center lg:gap-4">
          <div className="flex min-w-0 flex-1 items-center gap-2.5">
            <KindBadge mimeType={mimeType} ext={ext} />
            <div className="min-w-0">
              <h1
                className="truncate text-sm font-semibold tracking-tight text-foreground"
                title={filename}
              >
                {filename}
              </h1>
              <div className="mt-0.5 flex min-w-0 items-center gap-x-2 overflow-hidden text-[11px] whitespace-nowrap text-muted-foreground">
                {facts.map((fact, i) => (
                  <span
                    key={fact.key}
                    className={cn("inline-flex shrink-0 items-center gap-2", fact.className)}
                  >
                    {i > 0 ? <Dot /> : null}
                    {fact.node}
                  </span>
                ))}
              </div>
            </div>
          </div>

          <div className="min-w-0 lg:shrink-0">
            <MediaActions
              publicUrl={publicUrl}
              rawUrl={rawUrl}
              downloadUrl={downloadUrl}
              formats={formats}
              copyRaw={copyRaw}
              mimeType={mimeType}
              ext={ext}
              frames={frames}
              canEncode={canEncode}
              size={size}
              width={width}
              height={height}
              signInUrl={signInUrl}
            />
          </div>
        </div>
      </FloatingBar>
    </div>
  );
}

function Stage({
  kind,
  filename,
  mimeType,
  ext,
  rawUrl,
  size,
  width,
  height,
  textContent,
  textUnavailable,
  renderPath,
  frames,
  timelineTrack,
  posterUrl,
  videoUnsupported,
  audioUnsupported,
  noPreviewLabel,
  copyRawLabel,
}: {
  kind: ReturnType<typeof mediaKind>;
  filename: string;
  mimeType: string;
  /** Fallback evidence when the stored MIME is application/octet-stream. */
  ext: string;
  rawUrl: string;
  size: number;
  width?: number | null;
  height?: number | null;
  textContent?: string | null;
  textUnavailable?: TextUnavailable;
  renderPath?: string | null;
  /**
   * Video only, all three. `frames` names a global on `window`, so leaving it
   * out of this list does not fail to compile - it silently resolves to
   * `window.frames` and hands the player a Window object.
   */
  frames?: number | null;
  timelineTrack?: string | null;
  posterUrl?: string | null;
  videoUnsupported: string;
  audioUnsupported: string;
  noPreviewLabel: string;
  copyRawLabel: string;
}) {
  // An uploaded page (.html/.svg) is shown as source, with the option to run it
  // in a sandbox. SVG is otherwise classified as "other" and would land on the
  // icon placeholder, which tells the viewer nothing about what is in the file.
  const shape = sandboxShape(mimeType, ext);

  // Documents come first, because a .docx also answers to `shape` and would
  // otherwise fall into the Source/Page toggle below - which for a zip archive
  // offers a "source" view that cannot exist.
  const document = docxSupport(mimeType, ext);
  if (document) {
    return (
      <div className="h-full w-full px-3 py-3 sm:px-4">
        <DocumentView
          renderPath={document === "docx" ? (renderPath ?? null) : null}
          unavailable={document === "legacy-doc" ? "legacy-doc" : "too-large"}
          filename={filename}
          rawUrl={rawUrl}
          size={size}
        />
      </div>
    );
  }

  if (kind === "image") {
    return <ImageView rawUrl={rawUrl} filename={filename} width={width} height={height} />;
  }

  if (kind === "video") {
    // A container is not a codec. .mov/.m4v/.3gp are ISO base media just like
    // .mp4 and decode fine - browsers only refuse the LABEL "video/quicktime" -
    // so those are advertised as video/mp4 and play. .mkv/.avi/.wmv/.flv have no
    // demuxer in any browser, so they fall through to the download card instead
    // of rendering a player that can only ever show its error text.
    const playType = playableVideoType(mimeType, ext);
    if (!playType) {
      return (
        <UnknownStage
          filename={filename}
          mimeType={mimeType}
          rawUrl={rawUrl}
          size={size}
          noPreviewLabel={noPreviewLabel}
          copyRawLabel={copyRawLabel}
        />
      );
    }

    /**
     * The player fills the stage and letterboxes itself with `object-contain`,
     * so the clip is centered on the page background instead of inside a black
     * slab. Filling the box also keeps the old collapse bug dead: a <video> has
     * no intrinsic size until its metadata lands, and left to size itself it
     * shrank to the height of the control bar until playback started.
     */
    return (
      <div className="flex h-full w-full items-center justify-center px-2 py-2 sm:px-4">
        <MediaPlayer
          src={rawUrl}
          type={playType}
          unsupported={videoUnsupported}
          timelineTrack={timelineTrack}
          poster={posterUrl}
          frames={frames}
        />
      </div>
    );
  }

  if (kind === "audio") {
    return (
      <div className="flex h-full w-full flex-col items-center justify-center gap-6 px-6">
        <div className="flex size-24 items-center justify-center rounded-2xl border border-border/70 bg-gradient-to-br from-primary/15 to-primary/5">
          <FileAudio className="size-10 text-primary" />
        </div>
        {/* Same controls as the video player, so a shared audio link is skinned
            like everything else instead of showing the browser's own bar. */}
        <div className="w-full max-w-md">
          <MediaPlayer
            src={rawUrl}
            type={playableAudioType(mimeType, ext) ?? mimeType}
            variant="audio"
            unsupported={audioUnsupported}
          />
        </div>
      </div>
    );
  }

  // Anything /raw hands over as text reads as source here. `showsSource` is the
  // ONE definition of that, shared with the preview page that decided whether to
  // read the bytes at all - which is exactly what the old condition was not.
  // `kind === "text" || shape` missed .json, .sh, .sql, .xml and a ShareX .sxcu:
  // `mediaKind` calls all of them "other" and none of them is renderable
  // markup, so the page read half a megabyte off disk and then drew a download
  // card for a file the very next click showed as text.
  if (showsSource({ mimeType, ext })) {
    return (
      <div className="h-full w-full px-3 py-3 sm:px-4">
        <SourceView
          content={textContent}
          size={size}
          unavailable={textUnavailable}
          filename={filename}
          rawUrl={rawUrl}
          readings={sourceReadings({ rawUrl, renderPath })}
          language={codeLanguageLabel({ mimeType, ext })}
          // Narrowed rather than widened: SourceView is a client component, and
          // giving it the full SandboxShape would mean importing the sandbox
          // rules (and node:crypto behind them) into the browser bundle. The
          // document shape has already returned above.
          shape={shape === "html" || shape === "svg" ? shape : null}
        />
      </div>
    );
  }

  if (kind === "pdf") {
    return (
      <div className="h-full w-full px-3 py-3 sm:px-4">
        <iframe
          src={rawUrl}
          title={filename}
          className="mx-auto h-full w-full max-w-5xl rounded-xl border border-border/60 bg-card"
        />
      </div>
    );
  }

  // archive / other → big icon placeholder
  return (
    <UnknownStage
      filename={filename}
      mimeType={mimeType}
      rawUrl={rawUrl}
      size={size}
      noPreviewLabel={noPreviewLabel}
      copyRawLabel={copyRawLabel}
    />
  );
}

function UnknownStage({
  filename,
  mimeType,
  rawUrl,
  size,
  noPreviewLabel,
  copyRawLabel,
}: {
  filename: string;
  mimeType: string;
  rawUrl: string;
  size: number;
  noPreviewLabel: string;
  copyRawLabel: string;
}) {
  const Icon = ICONS[mimeToIcon(mimeType)] ?? FileIcon;
  return (
    <div className="flex h-full w-full flex-col items-center justify-center gap-5 px-6 text-center">
      <div className="flex size-28 items-center justify-center rounded-3xl border border-border/70 bg-gradient-to-br from-primary/15 via-primary/5 to-transparent">
        <Icon className="size-12 text-primary" />
      </div>
      <div className="space-y-1">
        <p className="max-w-md truncate font-medium text-foreground" title={filename}>
          {filename}
        </p>
        <p className="text-sm text-muted-foreground">
          {humanFileSize(size)} · {noPreviewLabel}
        </p>
      </div>
      <CopyButton value={rawUrl} label={copyRawLabel} variant="outline" />
    </div>
  );
}

function KindBadge({ mimeType, ext }: { mimeType: string; ext?: string }) {
  const Icon = ICONS[mimeToIcon(mimeType, ext)] ?? FileIcon;
  return (
    <div className="flex size-9 shrink-0 items-center justify-center rounded-xl border border-border/70 bg-gradient-to-br from-primary/15 to-primary/5">
      <Icon className="size-5 text-primary" />
    </div>
  );
}

function Meta({
  icon: Icon,
  text,
}: {
  icon: React.ComponentType<{ className?: string }>;
  text: string;
}) {
  return (
    <span className="inline-flex items-center gap-1 whitespace-nowrap">
      <Icon className="size-3.5 shrink-0 opacity-70" aria-hidden />
      {text}
    </span>
  );
}

/**
 * A counter shown as icon + number. The full phrase ("12 views") stays on the
 * accessible name and the tooltip, which is what lets six facts share one line
 * on a phone.
 */
function Count({
  icon: Icon,
  value,
  label,
}: {
  icon: React.ComponentType<{ className?: string }>;
  value: string;
  label: string;
}) {
  return (
    <span
      className="inline-flex items-center gap-1 whitespace-nowrap tabular-nums"
      title={label}
      aria-label={label}
      role="img"
    >
      <Icon className="size-3.5 shrink-0 opacity-70" aria-hidden />
      <span aria-hidden>{value}</span>
    </span>
  );
}

function Dot() {
  return (
    <span aria-hidden className={cn("text-muted-foreground/40")}>
      ·
    </span>
  );
}

function prettyType(mime: string, ext: string): string {
  if (ext) return ext.toUpperCase();
  const sub = mime.split("/")[1];
  return (sub ?? mime).toUpperCase();
}
