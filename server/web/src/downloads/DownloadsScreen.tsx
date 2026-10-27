// Not upstream: the Downloads page (PL "Do pobrania"), the desktop app's builds
// for macOS and Windows (server/docs/SKELETON.md section 9, item 45).
//
// One card per platform: the newest build on top (version, build, dates, size,
// change note, a big Download button, the SHA-256 to copy, short install
// steps), older builds folded below, "no builds yet" when there are none.
// Administrators also delete builds and read how build machines publish; the
// server address in those instructions is this page's own origin.
import { useId, useState, useSyncExternalStore, useTransition } from "react";
import { useRouter } from "next/navigation";
import { useFormatter, useTranslations } from "next-intl";
import {
  AlertTriangle,
  Apple,
  Check,
  ChevronRight,
  CircleCheck,
  Copy,
  Download,
  Info,
  Loader2,
  PackageOpen,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { config } from "@/lib/config";
import { humanFileSize } from "@/lib/format";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import {
  CHUNK_SIZE,
  PLATFORMS,
  TOKEN_UPLOADER,
  deleteBuild,
  splitNewest,
  type Build,
  type DownloadsList,
  type Platform,
} from "./api";

/** This page's origin: the server the app signs in to, and the scripts talk to. */
const noopSubscribe = () => () => {};
function useOrigin(): string {
  return useSyncExternalStore(
    noopSubscribe,
    () => window.location.origin,
    () => "",
  );
}

async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

/** The Windows mark, four panes (lucide has no brand icons). */
function WindowsMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden className={className}>
      <path d="M3 5.5 10.5 4.5v7H3zM11.5 4.35 21 3v8.5h-9.5zM3 12.5h7.5v7L3 18.5zM11.5 12.5H21V21l-9.5-1.35z" />
    </svg>
  );
}

function PlatformIcon({ platform, className }: { platform: Platform; className?: string }) {
  return platform === "macos" ? <Apple className={className} aria-hidden /> : <WindowsMark className={className} />;
}

function Caps({ children }: { children: React.ReactNode }) {
  return (
    <p className="text-muted-foreground text-[0.6875rem] font-semibold tracking-[0.08em] uppercase">{children}</p>
  );
}

function useBuildText() {
  const t = useTranslations("downloads");
  const format = useFormatter();
  return {
    t,
    date: (iso: string) => format.dateTime(new Date(iso), { dateStyle: "medium", timeStyle: "short" }),
    platform: (p: Platform) => t(`platform.${p}`),
    arch: (b: Build) => t(`arch.${b.platform}.${b.arch}`),
    version: (b: Build) => (b.build ? `${b.version} · ${t("build", { build: b.build })}` : b.version),
    who: (b: Build) => (b.uploader === TOKEN_UPLOADER ? t("buildMachine") : b.uploader),
  };
}

/* -------------------------------------------------------------------------- */
/* Pieces of a build                                                           */
/* -------------------------------------------------------------------------- */

function ShaBox({ build }: { build: Build }) {
  const { t } = useBuildText();
  const [copied, setCopied] = useState(false);

  async function copy() {
    if (await copyText(build.sha256)) {
      setCopied(true);
      toast.success(t("shaCopied"));
      setTimeout(() => setCopied(false), 1500);
    } else {
      toast.error(t("copyFailed"));
    }
  }

  return (
    <div className="space-y-1.5">
      <Caps>{t("sha256")}</Caps>
      <div className="bg-muted/50 ring-foreground/10 flex items-start gap-2 rounded-lg py-1.5 pr-1.5 pl-3 ring-1">
        <code className="min-w-0 flex-1 py-1 font-mono text-xs leading-relaxed break-all" data-sha256="">
          {build.sha256}
        </code>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button variant="ghost" size="icon-sm" onClick={copy} aria-label={t("copySha")}>
              {copied ? <Check className="text-primary" /> : <Copy />}
            </Button>
          </TooltipTrigger>
          <TooltipContent>{t("copySha")}</TooltipContent>
        </Tooltip>
      </div>
    </div>
  );
}

function InstallSteps({ platform }: { platform: Platform }) {
  const { t } = useBuildText();
  const origin = useOrigin();
  const steps = ["step1", "step2", "step3"] as const;
  return (
    <div className="space-y-2.5">
      <Caps>{t("install.title")}</Caps>
      <ol className="space-y-2">
        {steps.map((step, i) => (
          <li key={step} className="flex gap-2.5 text-sm">
            <span className="bg-primary/10 text-primary grid size-5 shrink-0 place-items-center rounded-full text-[0.6875rem] font-semibold tabular-nums">
              {i + 1}
            </span>
            <span className="text-muted-foreground min-w-0 break-words">
              {t(`install.${platform}.${step}`, { origin })}
            </span>
          </li>
        ))}
      </ol>
      <p className="text-muted-foreground flex gap-2 text-xs leading-relaxed">
        <Info className="mt-0.5 size-3.5 shrink-0" aria-hidden />
        <span>{t(`install.${platform}.hint`)}</span>
      </p>
    </div>
  );
}

function DeleteBuild({ build, compact }: { build: Build; compact?: boolean }) {
  const { t, platform, version } = useBuildText();
  const tc = useTranslations("common");
  const router = useRouter();
  const [open, setOpen] = useState(false);
  const [pending, startTransition] = useTransition();
  const label = t("deleteLabel", { filename: build.filename });

  function remove() {
    startTransition(async () => {
      try {
        await deleteBuild(build.id);
        setOpen(false);
        toast.success(t("deleted"));
      } catch (err) {
        toast.error(t("deleteFailed"), { description: err instanceof Error ? err.message : undefined });
      }
      router.refresh();
    });
  }

  return (
    <>
      {compact ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={label}
              onClick={() => setOpen(true)}
              className="text-muted-foreground hover:text-destructive"
            >
              <Trash2 />
            </Button>
          </TooltipTrigger>
          <TooltipContent>{label}</TooltipContent>
        </Tooltip>
      ) : (
        <Button variant="destructive" size="sm" onClick={() => setOpen(true)} aria-label={label}>
          <Trash2 /> {t("delete")}
        </Button>
      )}
      <Dialog open={open} onOpenChange={(next) => !pending && setOpen(next)}>
        <DialogContent>
          <DialogHeader>
            <div className="bg-destructive/10 text-destructive mb-1 grid size-10 place-items-center rounded-lg">
              <AlertTriangle className="size-5" />
            </div>
            <DialogTitle>{t("deleteTitle")}</DialogTitle>
            <DialogDescription>
              {t("deleteDesc", {
                platform: platform(build.platform),
                version: version(build),
                filename: build.filename,
                size: humanFileSize(build.size),
              })}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <DialogClose asChild>
              <Button variant="outline" disabled={pending}>
                {tc("cancel")}
              </Button>
            </DialogClose>
            <Button variant="destructive" onClick={remove} disabled={pending}>
              {pending ? <Loader2 className="animate-spin" /> : <Trash2 />}
              {t("deleteConfirm")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

/** The newest build of a platform, in full. */
function NewestBuild({ build, others, admin }: { build: Build; others: Build[]; admin: boolean }) {
  const { t, date, platform, arch, version, who } = useBuildText();
  const size = humanFileSize(build.size);
  return (
    <div className="space-y-5 px-5 py-5">
      <div className="space-y-1">
        <p className="font-heading text-2xl leading-tight font-semibold tracking-tight break-words tabular-nums">
          {build.version}
          {build.build ? (
            <span className="text-muted-foreground ml-2 text-sm font-medium">{t("build", { build: build.build })}</span>
          ) : null}
        </p>
        <p className="text-muted-foreground text-xs tabular-nums">
          {t("built", { date: date(build.built) })} · {size}
        </p>
      </div>

      {build.note ? (
        <div className="bg-muted/40 ring-foreground/10 space-y-1 rounded-lg px-3 py-2.5 ring-1">
          <Caps>{t("whatsNew")}</Caps>
          <p className="text-sm leading-relaxed break-words whitespace-pre-line">{build.note}</p>
        </div>
      ) : null}

      <div className="space-y-2">
        <Button asChild size="lg" className="h-11 w-full gap-2 px-6 text-sm sm:w-auto">
          <a
            href={build.url}
            download={build.filename}
            aria-label={t("downloadLabel", { filename: build.filename, size })}
            data-download={build.id}
          >
            <Download className="size-4" />
            {t("download", { platform: platform(build.platform) })}
          </a>
        </Button>
        <p className="text-muted-foreground font-mono text-xs break-all">{build.filename}</p>
      </div>

      {others.length > 0 ? (
        <ul className="space-y-2">
          {others.map((other) => (
            <li
              key={other.id}
              className="ring-foreground/10 flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-lg px-3 py-2 ring-1"
            >
              <span className="min-w-0 flex-1 text-sm">
                {t("alsoFor", { arch: arch(other) })}
                <span className="text-muted-foreground ml-2 text-xs tabular-nums">
                  {version(other)} · {humanFileSize(other.size)}
                </span>
              </span>
              <Button asChild variant="outline" size="sm">
                <a
                  href={other.url}
                  download={other.filename}
                  aria-label={t("downloadLabel", { filename: other.filename, size: humanFileSize(other.size) })}
                >
                  <Download /> {t("downloadShort")}
                </a>
              </Button>
              {admin ? <DeleteBuild build={other} compact /> : null}
            </li>
          ))}
        </ul>
      ) : null}

      <ShaBox build={build} />
      <InstallSteps platform={build.platform} />

      <div className="border-foreground/10 flex flex-wrap items-center justify-between gap-3 border-t pt-4">
        <p className="text-muted-foreground min-w-0 text-xs break-words">
          {t("published", { date: date(build.published), who: who(build) })}
        </p>
        {admin ? <DeleteBuild build={build} /> : null}
      </div>
    </div>
  );
}

function OlderBuilds({ builds, admin }: { builds: Build[]; admin: boolean }) {
  const { t, date, arch, version } = useBuildText();
  const [open, setOpen] = useState(false);
  const id = useId();
  return (
    <div className="border-foreground/10 border-t">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        aria-controls={id}
        className="text-muted-foreground hover:text-foreground focus-visible:ring-ring flex w-full items-center gap-2 px-5 py-3 text-left text-sm font-medium transition-colors outline-none focus-visible:ring-2"
      >
        <ChevronRight aria-hidden className={cn("size-4 shrink-0 transition-transform", open && "rotate-90")} />
        {t("older", { count: builds.length })}
      </button>
      <ul id={id} className={cn("divide-foreground/10 border-foreground/10 divide-y border-t", !open && "hidden")}>
        {builds.map((b) => (
          <li key={b.id} className="flex items-start gap-3 px-5 py-3" data-build={b.id}>
            <div className="min-w-0 flex-1 space-y-0.5">
              <p className="text-sm font-medium break-words tabular-nums">
                {version(b)}
                <span className="text-muted-foreground ml-2 text-xs font-normal">{arch(b)}</span>
              </p>
              <p className="text-muted-foreground text-xs tabular-nums">
                {date(b.built)} · {humanFileSize(b.size)}
              </p>
              {b.note ? (
                <p className="text-muted-foreground line-clamp-3 text-xs break-words whitespace-pre-line">{b.note}</p>
              ) : null}
            </div>
            <div className="flex shrink-0 items-center gap-1">
              <Button asChild variant="outline" size="sm">
                <a
                  href={b.url}
                  download={b.filename}
                  aria-label={t("downloadLabel", { filename: b.filename, size: humanFileSize(b.size) })}
                >
                  <Download /> <span className="max-sm:sr-only">{t("downloadShort")}</span>
                </a>
              </Button>
              <CopyShaIcon build={b} />
              {admin ? <DeleteBuild build={b} compact /> : null}
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}

function CopyShaIcon({ build }: { build: Build }) {
  const { t } = useBuildText();
  async function copy() {
    if (await copyText(build.sha256)) toast.success(t("shaCopied"));
    else toast.error(t("copyFailed"));
  }
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button variant="ghost" size="icon-sm" onClick={copy} aria-label={t("copySha")}>
          <Copy />
        </Button>
      </TooltipTrigger>
      <TooltipContent className="max-w-[min(90vw,28rem)] font-mono break-all">{build.sha256}</TooltipContent>
    </Tooltip>
  );
}

function PlatformCard({ platform, builds, admin }: { platform: Platform; builds: Build[]; admin: boolean }) {
  const { t, arch } = useBuildText();
  const { newest, older } = splitNewest(builds);
  const [main, ...others] = newest;
  return (
    <Card className="min-w-0 gap-0 py-0" data-platform={platform}>
      <div className="border-foreground/10 flex items-center gap-3 border-b px-5 py-4">
        <span className="bg-muted ring-foreground/10 grid size-10 shrink-0 place-items-center rounded-lg ring-1">
          <PlatformIcon platform={platform} className="size-5" />
        </span>
        <div className="min-w-0 flex-1">
          <h2 className="font-heading text-base font-semibold tracking-tight">{t(`platform.${platform}`)}</h2>
          {main ? <p className="text-muted-foreground truncate text-xs">{arch(main)}</p> : null}
        </div>
        {main ? (
          <Badge variant="secondary" className="shrink-0">
            {t("newest")}
          </Badge>
        ) : null}
      </div>
      {main ? (
        <NewestBuild build={main} others={others} admin={admin} />
      ) : (
        <div className="flex flex-col items-center gap-2 px-5 py-12 text-center">
          <div className="bg-muted text-muted-foreground mb-1 grid size-11 place-items-center rounded-full">
            <PackageOpen className="size-5" aria-hidden />
          </div>
          <p className="text-sm font-medium">{t("empty")}</p>
          <p className="text-muted-foreground max-w-xs text-xs leading-relaxed">
            {admin ? t("emptyHintAdmin") : t("emptyHint", { platform: t(`platform.${platform}`) })}
          </p>
        </div>
      )}
      {older.length > 0 ? <OlderBuilds builds={older} admin={admin} /> : null}
    </Card>
  );
}

/* -------------------------------------------------------------------------- */
/* Publishing (administrators)                                                 */
/* -------------------------------------------------------------------------- */

function CodeBlock({ code, label }: { code: string; label: string }) {
  const t = useTranslations("downloads");
  const [copied, setCopied] = useState(false);
  async function copy() {
    if (await copyText(code)) {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } else {
      toast.error(t("copyFailed"));
    }
  }
  return (
    <div className="relative min-w-0">
      <pre
        aria-label={label}
        className="bg-muted/50 ring-foreground/10 max-w-full overflow-x-auto rounded-lg py-3 pr-20 pl-3 font-mono text-xs leading-relaxed ring-1"
      >
        <code>{code}</code>
      </pre>
      <Button variant="outline" size="xs" onClick={copy} className="absolute top-2 right-2">
        {copied ? <Check /> : <Copy />}
        {copied ? t("publish.copied") : t("publish.copy")}
      </Button>
    </div>
  );
}

function PublishGuide({ publishToken, keep }: { publishToken: boolean; keep: number }) {
  const t = useTranslations("downloads");
  const origin = useOrigin();
  const server = origin || "https://files.example.com";
  const example = "Modlight-1.2.0-45-macos-arm64.zip";

  const start = [
    `POST ${server}/api/downloads/upload`,
    "Content-Type: application/json",
    "",
    "{",
    '  "platform": "macos", "arch": "arm64",',
    '  "version": "1.2.0", "build": "45", "built": "2026-10-27T12:00:00Z",',
    `  "filename": "${example}",`,
    '  "size": 73400320, "sha256": "<64 hex digits>",',
    '  "note": "What changed"',
    "}",
    "",
    `-> {"upload": "<upload>", "chunkSize": ${CHUNK_SIZE}}`,
  ].join("\n");
  const chunks = [
    `PUT ${server}/api/downloads/upload/<upload>/0     bytes 0 .. ${CHUNK_SIZE - 1}`,
    `PUT ${server}/api/downloads/upload/<upload>/1     the next ${CHUNK_SIZE} bytes`,
    "...",
  ].join("\n");
  const finish = [
    `POST ${server}/api/downloads/upload/<upload>/finish`,
    "",
    `-> {"id": "...", "version": "1.2.0", "url": "/downloads/<id>/${example}", ...}`,
  ].join("\n");
  const script = [
    "#!/bin/sh",
    "# publish.sh FILE PLATFORM ARCH VERSION BUILD [NOTE]",
    "# e.g. publish.sh Modlight-1.2.0-45-macos-arm64.zip macos arm64 1.2.0 45 \"What changed\"",
    "set -eu",
    `SERVER="${server}"`,
    'TOKEN="$RELEASE_PUBLISH_TOKEN"',
    'FILE="$1"; NAME=$(basename "$FILE")',
    'SIZE=$(wc -c < "$FILE" | tr -d " ")',
    'SHA=$( (shasum -a 256 "$FILE" 2>/dev/null || sha256sum "$FILE") | cut -d" " -f1)',
    'UPLOAD=$(curl -fsS -X POST "$SERVER/api/downloads/upload" \\',
    '  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \\',
    '  --data "{\\"platform\\":\\"$2\\",\\"arch\\":\\"$3\\",\\"version\\":\\"$4\\",\\"build\\":\\"$5\\",\\"size\\":$SIZE,\\"sha256\\":\\"$SHA\\",\\"filename\\":\\"$NAME\\",\\"note\\":\\"${6:-}\\"}" \\',
    "  | sed -n 's/.*\"upload\":\"\\([0-9a-f]*\\)\".*/\\1/p')",
    `CHUNK=${CHUNK_SIZE}; N=0`,
    'while [ $((N * CHUNK)) -lt "$SIZE" ]; do',
    '  dd if="$FILE" bs=$CHUNK skip=$N count=1 2>/dev/null |',
    '    curl -fsS -X PUT "$SERVER/api/downloads/upload/$UPLOAD/$N" \\',
    '      -H "Authorization: Bearer $TOKEN" --data-binary @- > /dev/null',
    "  N=$((N + 1))",
    "done",
    'curl -fsS -X POST "$SERVER/api/downloads/upload/$UPLOAD/finish" -H "Authorization: Bearer $TOKEN"',
  ].join("\n");

  const steps = [
    { key: "start", code: start },
    { key: "chunks", code: chunks },
    { key: "finish", code: finish },
  ] as const;

  return (
    <section className="space-y-4" data-publish-guide="">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 space-y-1">
          <h2 className="font-heading text-lg font-semibold tracking-tight">{t("publish.title")}</h2>
          <p className="text-muted-foreground max-w-2xl text-sm">{t("publish.intro", { keep })}</p>
        </div>
        <Badge variant={publishToken ? "secondary" : "destructive"} className="shrink-0">
          {publishToken ? <CircleCheck /> : <AlertTriangle />}
          {publishToken ? t("publish.tokenSet") : t("publish.tokenMissing")}
        </Badge>
      </div>

      {publishToken ? null : (
        <div className="bg-destructive/5 text-foreground ring-destructive/20 flex gap-3 rounded-lg px-4 py-3 text-sm ring-1">
          <AlertTriangle className="text-destructive mt-0.5 size-4 shrink-0" aria-hidden />
          <p className="min-w-0 leading-relaxed break-words">{t("publish.tokenMissingHint")}</p>
        </div>
      )}

      <Card className="min-w-0 gap-0 py-0">
        <div className="border-foreground/10 space-y-2 border-b px-5 py-4">
          <p className="text-sm">{t("publish.auth")}</p>
          <code className="bg-muted/50 ring-foreground/10 inline-block max-w-full rounded-md px-2 py-1 font-mono text-xs break-all ring-1">
            Authorization: Bearer $RELEASE_PUBLISH_TOKEN
          </code>
        </div>
        <ol className="divide-foreground/10 divide-y">
          {steps.map(({ key, code }, i) => (
            <li key={key} className="flex gap-3 px-5 py-4">
              <span className="bg-primary/10 text-primary grid size-6 shrink-0 place-items-center rounded-full text-xs font-semibold tabular-nums">
                {i + 1}
              </span>
              <div className="min-w-0 flex-1 space-y-2.5">
                <p className="text-sm leading-relaxed">{t(`publish.${key}`)}</p>
                <CodeBlock code={code} label={t(`publish.${key}`)} />
              </div>
            </li>
          ))}
        </ol>
        <div className="border-foreground/10 space-y-2 border-t px-5 py-4">
          <p className="text-sm">{t("publish.names")}</p>
          <ul className="flex flex-col gap-1.5">
            {["macos-arm64", "windows-x64"].map((tail) => (
              <li key={tail}>
                <code className="bg-muted/50 ring-foreground/10 inline-block max-w-full rounded-md px-2 py-1 font-mono text-xs break-all ring-1">
                  {`Modlight-<version>-<build>-${tail}.zip`}
                </code>
              </li>
            ))}
          </ul>
        </div>
        <div className="border-foreground/10 space-y-2.5 border-t px-5 py-4">
          <p className="text-sm">{t("publish.script")}</p>
          <CodeBlock code={script} label={t("publish.script")} />
        </div>
      </Card>
    </section>
  );
}

/* -------------------------------------------------------------------------- */
/* The page                                                                    */
/* -------------------------------------------------------------------------- */

export function DownloadsScreen({ list }: { list: DownloadsList }) {
  const t = useTranslations("downloads");
  const byPlatform = new Map(list.platforms.map((p) => [p.platform, p.builds] as const));
  return (
    <div className="mx-auto w-full max-w-5xl space-y-8">
      <header className="space-y-1">
        <h1 className="font-heading text-2xl font-semibold tracking-tight">{t("title")}</h1>
        <p className="text-muted-foreground text-sm">{t("intro", { appName: config.appName, keep: list.keep })}</p>
      </header>

      <div className="grid items-start gap-6 lg:grid-cols-2">
        {PLATFORMS.map((platform) => (
          <PlatformCard
            key={platform}
            platform={platform}
            builds={byPlatform.get(platform) ?? []}
            admin={list.admin}
          />
        ))}
      </div>

      {list.admin ? <PublishGuide publishToken={list.publishToken === true} keep={list.keep} /> : null}
    </div>
  );
}
