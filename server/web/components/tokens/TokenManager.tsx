"use client";

import { useState, useTransition } from "react";
import {
  KeyRound,
  Plus,
  Trash2,
  Copy,
  Check,
  Download,
  TriangleAlert,
  ShieldCheck,
  ShieldMinus,
  Terminal,
  Sparkles,
  Clock3,
} from "lucide-react";
import { toast } from "sonner";
import { useLocale, useTranslations } from "next-intl";
import { relativeTime } from "@/lib/format";
import { narrowerScopes, type TokenScope } from "@/lib/api-scopes";
import { createUploadToken, narrowTokenScope, revokeUploadToken } from "@/app/actions/tokens";
import { ScopeBadge, ScopePicker } from "@/components/tokens/ScopePicker";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

interface TokenView {
  id: string;
  name: string;
  /**
   * What this token may do. Typed as a plain string because it comes off a
   * database column, and a row written by an older build (or by hand) can hold
   * anything - the list below shows an unrecognised value as-is rather than
   * crashing the page, and authentication refuses it anyway.
   */
  scope: string;
  preview: string;
  createdAt: string;
  lastUsedAt: string | null;
  revokedAt: string | null;
}

/** Narrow a stored string to a scope, or null if this build does not know it. */
function asScope(value: string): TokenScope | null {
  return (["upload", "read", "write", "full"] as const).includes(value as TokenScope)
    ? (value as TokenScope)
    : null;
}

interface TokenManagerProps {
  tokens: TokenView[];
  appUrl: string;
  appName: string;
  userCode?: string | null;
  userName?: string | null;
  userEmail: string;
}

/** Build the ShareX custom uploader (.sxcu) JSON from an app URL + token. */
function buildSxcu(appUrl: string, name: string, token: string): string {
  return JSON.stringify(
    {
      Version: "16.1.0",
      Name: name,
      DestinationType: "ImageUploader, TextUploader, FileUploader",
      RequestMethod: "POST",
      RequestURL: `${appUrl}/upload`,
      Headers: { "x-api-key": token },
      Body: "MultipartFormData",
      FileFormName: "upload",
      URL: "{json:url}",
      ThumbnailURL: "{json:raw_url}",
      DeletionURL: "{json:deletionUrl}",
    },
    null,
    2,
  );
}

/** Build a portable bash uploader that pipes the URL to the clipboard. */
function buildBashUploader(appUrl: string, token: string): string {
  return [
    "#!/usr/bin/env bash",
    "# Usage: ./upload.sh <file>",
    `URL=$(curl -s -H "x-api-key: ${token}" \\`,
    `  -F "upload=@$1" "${appUrl}/upload" | grep -o '"url":"[^"]*"' | cut -d'"' -f4)`,
    'echo "$URL"',
    "# copy to clipboard (xclip on Linux, pbcopy on macOS)",
    'command -v pbcopy >/dev/null && printf "%s" "$URL" | pbcopy',
    'command -v xclip >/dev/null && printf "%s" "$URL" | xclip -selection clipboard',
  ].join("\n");
}

function CopyButton({
  value,
  label,
  size = "sm",
  variant = "outline",
  className,
}: {
  value: string;
  label?: string;
  size?: "sm" | "default" | "xs" | "icon-sm";
  variant?: "outline" | "default" | "ghost" | "secondary";
  className?: string;
}) {
  const t = useTranslations("tokens");
  const tc = useTranslations("common");
  const [copied, setCopied] = useState(false);
  const isIcon = size === "icon-sm";
  const resolvedLabel = label ?? tc("copy");

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      toast.success(t("copiedToClipboard"));
      setTimeout(() => setCopied(false), 1600);
    } catch {
      toast.error(t("clipboardError"));
    }
  }

  return (
    <Button type="button" variant={variant} size={size} onClick={copy} className={className}>
      {copied ? <Check className="text-emerald-500" /> : <Copy />}
      {!isIcon && (copied ? tc("copied") : resolvedLabel)}
    </Button>
  );
}

export function TokenManager({
  tokens,
  appUrl,
  appName,
  userCode,
  userName,
  userEmail,
}: TokenManagerProps) {
  const t = useTranslations("tokens");
  const tc = useTranslations("common");
  const [createOpen, setCreateOpen] = useState(false);
  // relativeTime formats through Intl, so it has to be told whose language
  // this is; without it a Polish reader gets English ages.
  const locale = useLocale();
  const [name, setName] = useState("");
  // `upload` is the default because it is what a ShareX config needs and what
  // most tokens should be. A dialog that opens on the widest option teaches the
  // widest option.
  const [scope, setScope] = useState<TokenScope>("upload");
  const [pending, startTransition] = useTransition();
  const [created, setCreated] = useState<{ name: string; token: string; scope: TokenScope } | null>(
    null,
  );
  const [revoking, setRevoking] = useState<string | null>(null);
  // The token whose scope is being narrowed, and the choice being considered.
  const [narrowing, setNarrowing] = useState<{ id: string; from: TokenScope } | null>(null);
  const [narrowTo, setNarrowTo] = useState<TokenScope | null>(null);

  const active = tokens.filter((t) => !t.revokedAt);
  const suggestedName =
    (userName?.split(" ")[0] || userEmail.split("@")[0] || "ShareX") +
    (userCode ? `@${userCode}` : "");

  function handleCreate() {
    const trimmed = name.trim() || "ShareX";
    startTransition(async () => {
      try {
        // The scope the SERVER settled on, not the one this form sent: they are
        // the same in every normal case, and showing the server's answer is what
        // keeps the success dialog honest if they ever are not.
        const result = await createUploadToken(trimmed, scope);
        setCreated({ name: trimmed, token: result.token, scope: result.scope });
        setName("");
      } catch {
        toast.error(t("manager.createError"));
      }
    });
  }

  function handleNarrow() {
    if (!narrowing || !narrowTo) return;
    const target = narrowing;
    startTransition(async () => {
      const result = await narrowTokenScope(target.id, narrowTo);
      if (result.ok) {
        toast.success(t("manager.narrowed"));
        setNarrowing(null);
        setNarrowTo(null);
      } else {
        toast.error(t("manager.narrowError"));
      }
    });
  }

  function handleRevoke(id: string) {
    setRevoking(id);
    startTransition(async () => {
      try {
        await revokeUploadToken(id);
        toast.success(t("manager.revoked"));
      } catch {
        toast.error(t("manager.revokeError"));
      } finally {
        setRevoking(null);
      }
    });
  }

  function downloadSxcu(tokenName: string, token: string) {
    const safe = (tokenName || appName).replace(/[^a-z0-9-_]+/gi, "-").replace(/^-+|-+$/g, "") || appName;
    const content = buildSxcu(appUrl, `${appName} — ${tokenName}`, token);
    const blob = new Blob([content], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${safe}.sxcu`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
    toast.success(t("manager.downloadSuccess"));
  }

  function closeCreate() {
    setCreateOpen(false);
    // Clear the one-time secret on close so it can't be re-read.
    setTimeout(() => setCreated(null), 200);
  }

  return (
    <>
      <Card>
        <CardHeader className="border-b">
          <div className="flex items-start justify-between gap-4">
            <div className="space-y-1">
              <CardTitle className="flex items-center gap-2">
                <KeyRound className="text-primary size-4" />
                {t("manager.title")}
              </CardTitle>
              <CardDescription>
                {t("manager.desc")}
              </CardDescription>
            </div>
            <Button
              onClick={() => {
                setName(suggestedName);
                setScope("upload");
                setCreateOpen(true);
              }}
              size="sm"
            >
              <Plus />
              {t("manager.createToken")}
            </Button>
          </div>
        </CardHeader>

        <CardContent className="px-0 pb-0">
          {active.length === 0 ? (
            <div className="flex flex-col items-center justify-center gap-3 px-6 py-14 text-center">
              <div className="bg-primary/10 text-primary grid size-12 place-items-center rounded-2xl">
                <KeyRound className="size-6" />
              </div>
              <div className="space-y-1">
                <p className="text-sm font-medium">{t("manager.emptyTitle")}</p>
                <p className="text-muted-foreground mx-auto max-w-xs text-sm">
                  {t("manager.emptyDesc")}
                </p>
              </div>
              <Button
                onClick={() => {
                  setName(suggestedName);
                  setScope("upload");
                  setCreateOpen(true);
                }}
                size="sm"
                className="mt-1"
              >
                <Sparkles />
                {t("manager.createFirst")}
              </Button>
            </div>
          ) : (
            <ul className="divide-border divide-y">
              {active.map((token) => (
                <li
                  key={token.id}
                  className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3.5 sm:px-6"
                >
                  <div className="bg-muted text-muted-foreground grid size-9 shrink-0 place-items-center rounded-lg">
                    <KeyRound className="size-4" />
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="truncate text-sm font-medium">{token.name}</span>
                      {asScope(token.scope) ? (
                        <ScopeBadge scope={asScope(token.scope)!} />
                      ) : (
                        <Badge variant="destructive" className="gap-1">
                          <TriangleAlert className="size-3" />
                          {t("manager.unknownScope")}
                        </Badge>
                      )}
                      <code className="text-muted-foreground bg-muted rounded px-1.5 py-0.5 font-mono text-[11px]">
                        ····{token.preview}
                      </code>
                    </div>
                    <div className="text-muted-foreground mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs">
                      <span>{t("manager.created", { time: relativeTime(token.createdAt, locale) })}</span>
                      <span aria-hidden className="opacity-40">
                        •
                      </span>
                      <span className="inline-flex items-center gap-1">
                        <Clock3 className="size-3" />
                        {token.lastUsedAt
                          ? t("manager.used", { time: relativeTime(token.lastUsedAt, locale) })
                          : t("manager.neverUsed")}
                      </span>
                    </div>
                  </div>
                  <div className="flex items-center gap-2">
                    {/* Offered only when there is somewhere narrower to go. A
                        button that opens a dialog with nothing in it is worse
                        than no button. Widening is never offered anywhere: it
                        would promote a leaked token without a new secret, so it
                        means creating a new token, and the server refuses it
                        whatever this UI does. */}
                    {(() => {
                      const current = asScope(token.scope);
                      if (!current || narrowerScopes(current).length === 0) return null;
                      return (
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => {
                            setNarrowing({ id: token.id, from: current });
                            setNarrowTo(narrowerScopes(current)[0] ?? null);
                          }}
                          disabled={pending}
                        >
                          <ShieldMinus />
                          {t("manager.narrow")}
                        </Button>
                      );
                    })()}
                    <Button
                      variant="destructive"
                      size="sm"
                      onClick={() => handleRevoke(token.id)}
                      disabled={pending && revoking === token.id}
                    >
                      <Trash2 />
                      {pending && revoking === token.id
                        ? t("manager.revoking")
                        : t("manager.revoke")}
                    </Button>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <Dialog open={createOpen} onOpenChange={(o) => (o ? setCreateOpen(true) : closeCreate())}>
        <DialogContent className="sm:max-w-md">
          {!created ? (
            <>
              <DialogHeader>
                <DialogTitle className="flex items-center gap-2">
                  <Plus className="text-primary size-4" />
                  {t("dialog.createTitle")}
                </DialogTitle>
                <DialogDescription>
                  {t("dialog.createDesc")}
                </DialogDescription>
              </DialogHeader>

              <div className="space-y-2">
                <Label htmlFor="token-name">{t("dialog.nameLabel")}</Label>
                <Input
                  id="token-name"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder={t("dialog.namePlaceholder")}
                  autoFocus
                  maxLength={120}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && !pending) handleCreate();
                  }}
                />
              </div>

              <div className="space-y-2">
                <Label>{t("dialog.scopeLabel")}</Label>
                <ScopePicker value={scope} onChange={setScope} disabled={pending} />
                <p className="text-muted-foreground text-xs">{t("dialog.scopeHint")}</p>
              </div>

              <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
                <Button variant="ghost" onClick={closeCreate} disabled={pending}>
                  {tc("cancel")}
                </Button>
                <Button onClick={handleCreate} disabled={pending}>
                  <KeyRound />
                  {pending ? t("dialog.generating") : t("dialog.generate")}
                </Button>
              </div>
            </>
          ) : (
            <>
              <DialogHeader>
                <DialogTitle className="flex items-center gap-2">
                  <ShieldCheck className="size-4 text-emerald-500" />
                  {t("dialog.createdTitle")}
                </DialogTitle>
                <DialogDescription>
                  {t("dialog.createdDesc")}
                </DialogDescription>
              </DialogHeader>

              <div className="border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-400 flex items-start gap-2.5 rounded-lg border p-3 text-xs">
                <TriangleAlert className="mt-0.5 size-4 shrink-0" />
                <p className="leading-relaxed">
                  {t.rich("dialog.warning", {
                    b: (chunks) => <span className="font-semibold">{chunks}</span>,
                  })}
                </p>
              </div>

              <div className="flex items-center gap-2 text-xs">
                <span className="text-muted-foreground">{t("dialog.scopeIs")}</span>
                <ScopeBadge scope={created.scope} />
              </div>

              <div className="space-y-1.5">
                <Label className="text-xs">{t("dialog.yourToken")}</Label>
                <div className="flex items-center gap-2">
                  <code className="bg-muted ring-foreground/10 min-w-0 flex-1 truncate rounded-lg px-3 py-2 font-mono text-xs ring-1">
                    {created.token}
                  </code>
                  <CopyButton value={created.token} variant="default" size="icon-sm" />
                </div>
              </div>

              <Separator />

              <div className="grid gap-2 sm:grid-cols-2">
                <Button
                  onClick={() => downloadSxcu(created.name, created.token)}
                  className="w-full"
                >
                  <Download />
                  {t("dialog.sxcuButton")}
                </Button>
                <CopyButton
                  value={buildBashUploader(appUrl, created.token)}
                  label={t("dialog.copyBash")}
                  variant="outline"
                  className="w-full"
                />
              </div>

              <div className="bg-muted/40 ring-foreground/10 rounded-lg p-3 ring-1">
                <div className="text-muted-foreground mb-1.5 flex items-center gap-1.5 text-[11px] font-medium tracking-wide uppercase">
                  <Terminal className="size-3" />
                  {t("dialog.bashUploader")}
                </div>
                <pre className="overflow-x-auto font-mono text-[11px] leading-relaxed whitespace-pre">
                  {buildBashUploader(appUrl, created.token)}
                </pre>
              </div>

              <div className="flex justify-end">
                <Button variant="ghost" onClick={closeCreate}>
                  {t("dialog.done")}
                </Button>
              </div>
            </>
          )}
        </DialogContent>
      </Dialog>

      <Dialog
        open={narrowing !== null}
        onOpenChange={(o) => {
          if (!o) {
            setNarrowing(null);
            setNarrowTo(null);
          }
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <ShieldMinus className="text-primary size-4" />
              {t("dialog.narrowTitle")}
            </DialogTitle>
            <DialogDescription>{t("dialog.narrowDesc")}</DialogDescription>
          </DialogHeader>

          {narrowing && (
            <>
              <div className="flex items-center gap-2 text-xs">
                <span className="text-muted-foreground">{t("dialog.narrowFrom")}</span>
                <ScopeBadge scope={narrowing.from} />
              </div>
              <ScopePicker
                value={narrowTo ?? narrowerScopes(narrowing.from)[0]}
                onChange={setNarrowTo}
                allowed={narrowerScopes(narrowing.from)}
                disabled={pending}
              />
              <p className="text-muted-foreground text-xs">{t("dialog.narrowWarning")}</p>
            </>
          )}

          <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
            <Button
              variant="ghost"
              onClick={() => {
                setNarrowing(null);
                setNarrowTo(null);
              }}
              disabled={pending}
            >
              {tc("cancel")}
            </Button>
            <Button onClick={handleNarrow} disabled={pending || !narrowTo}>
              <ShieldMinus />
              {pending ? t("dialog.narrowing") : t("dialog.narrowConfirm")}
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
