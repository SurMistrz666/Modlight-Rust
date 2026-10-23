"use client";
// Client port of upstream components/authorize/authorize-prompt.tsx.
// @upstream components/authorize/authorize-prompt.tsx
// Deviations (not upstream; server/docs/SKELETON.md section 9, "Desktop sign-in
// pickup"): the request may carry the app's pickup `challenge`. Then approve
// and deny go through src/rpc/authorize-pickup.ts with it, the server keeps the
// outcome for the app to collect, and after approving the page says the app
// picks the key up by itself. The loopback POST is still tried (best effort:
// some browsers never send it) and the key stays available to copy as the
// last resort. Without a challenge everything is exactly upstream's.
// Security review 2026-10-24: P2, the prompt shows the app's confirmation code
// (`confirmCode`, XXX-XXX, derived from the challenge on the server) next to
// Approve and says the key goes to the app showing that code, wherever it runs;
// P1, the server only stores the outcome for a state this account's own page
// render registered, and answers `pickup: false` otherwise: the page then
// falls back to upstream's loopback POST and manual key.

import { useState, useTransition } from "react";
import {
  ArrowRight,
  Check,
  Copy,
  FolderOpen,
  Laptop,
  Link2,
  Loader2,
  ShieldAlert,
  ShieldCheck,
  Trash2,
  TriangleAlert,
  Upload,
} from "lucide-react";
import { toast } from "sonner";
import { useTranslations } from "next-intl";
import type { AuthorizeRefusal } from "@/app/actions/authorize";
import { approveDesktopClient, denyDesktopClient } from "../../../rpc/authorize-pickup";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { cn } from "@/lib/utils";

interface AuthorizePromptProps {
  appName: string;
  /**
   * Whose account is on the table. The address, not the display name: a person
   * with two accounts open needs to know which one is about to hand out a key,
   * and only one of those two values is unambiguous.
   */
  account: { email: string };
  request: {
    /** Raw values, re-validated by the action. */
    port: string;
    state: string;
    clientName: string;
    /** Server-built `http://127.0.0.1:<port>/`. Never assembled here. */
    callback: string;
    /** Not upstream: the app's pickup challenge, when it sent one. */
    challenge?: string;
    /** Not upstream: the code the app shows (XXX-XXX), with a challenge. */
    confirmCode?: string;
  };
}

/** How long to wait for the local listener before offering the manual route. */
const HANDOFF_TIMEOUT_MS = 5000;

type Phase =
  | { name: "idle" }
  | { name: "working" }
  | { name: "delivered" }
  /** The token exists but the local app never received it. */
  | { name: "manual"; token: string }
  /** Not upstream: the server holds the token for the app's pickup. */
  | { name: "pickup"; token: string }
  | { name: "denied" }
  | { name: "refused"; reason: AuthorizeRefusal };

/**
 * Deliver a payload to the app's loopback listener.
 *
 * `no-cors` with a `text/plain` body keeps this a CORS-simple request: no
 * preflight is sent, so the listener inside the desktop app can be twenty lines
 * that read a body and answer 204, with no CORS headers to get wrong. The reply
 * is opaque in return, which means success here reads "the bytes were
 * delivered", not "the app was happy with them" — the difference is why the
 * caller keeps a manual fallback.
 *
 * `credentials: "omit"` because a local process has no use for this site's
 * cookies, and the abort keeps a port that accepts connections and then says
 * nothing from leaving the prompt spinning forever.
 */
async function postToLoopback(callback: string, payload: unknown): Promise<boolean> {
  const abort = new AbortController();
  const timer = setTimeout(() => abort.abort(), HANDOFF_TIMEOUT_MS);
  try {
    await fetch(callback, {
      method: "POST",
      mode: "no-cors",
      cache: "no-store",
      credentials: "omit",
      referrerPolicy: "no-referrer",
      headers: { "content-type": "text/plain;charset=UTF-8" },
      body: JSON.stringify(payload),
      signal: abort.signal,
    });
    return true;
  } catch {
    return false;
  } finally {
    clearTimeout(timer);
  }
}

export function AuthorizePrompt({ appName, account, request }: AuthorizePromptProps) {
  const t = useTranslations("authorize");
  const tp = useTranslations("authorizePickup");
  const pickup = Boolean(request.challenge);
  const tc = useTranslations("common");
  const [phase, setPhase] = useState<Phase>({ name: "idle" });
  const [pending, startTransition] = useTransition();
  const [copied, setCopied] = useState(false);

  // The client chose this string; when it chose nothing, we say what we know
  // rather than render an empty space where a name should be.
  const clientLabel = request.clientName || t("unknownClient");
  const host = `127.0.0.1:${request.port}`;

  function approve() {
    startTransition(async () => {
      setPhase({ name: "working" });
      const result = await approveDesktopClient({
        port: request.port,
        state: request.state,
        name: request.clientName,
        ...(request.challenge ? { challenge: request.challenge } : {}),
      });

      if (!result.ok) {
        setPhase({ name: "refused", reason: result.reason });
        return;
      }

      // `pickup: false`: the server did not keep it (the state was not
      // registered by this account's page render); upstream's flow then.
      if (pickup && (result as { pickup?: boolean }).pickup !== false) {
        // The app collects the key from the server whatever happens next; the
        // loopback POST is only a head start, and its result changes nothing.
        setPhase({ name: "pickup", token: result.token });
        void postToLoopback(result.callback, { state: result.state, token: result.token });
        return;
      }

      const delivered = await postToLoopback(result.callback, {
        state: result.state,
        token: result.token,
      });
      setPhase(delivered ? { name: "delivered" } : { name: "manual", token: result.token });
    });
  }

  function deny() {
    startTransition(async () => {
      setPhase({ name: "working" });
      // Best effort, and no token in it: telling the app "no" is the difference
      // between it closing its listener now and hanging until its own timeout.
      // With a challenge the server keeps the refusal for the app's pickup too.
      if (pickup) {
        await denyDesktopClient({
          port: request.port,
          state: request.state,
          name: request.clientName,
          challenge: request.challenge,
        }).catch(() => undefined);
      }
      await postToLoopback(request.callback, { state: request.state, error: "access_denied" });
      setPhase({ name: "denied" });
    });
  }

  function retry(token: string) {
    startTransition(async () => {
      const delivered = await postToLoopback(request.callback, { state: request.state, token });
      // Re-sending the SAME token, never minting a second one: a failed handoff
      // is a listener that went away, not a credential that went missing.
      if (delivered) setPhase({ name: "delivered" });
      else toast.error(t("manual.stillUnreachable", { host }));
    });
  }

  async function copyToken(token: string) {
    try {
      await navigator.clipboard.writeText(token);
      setCopied(true);
      toast.success(t("manual.copied"));
      setTimeout(() => setCopied(false), 1600);
    } catch {
      toast.error(t("manual.copyFailed"));
    }
  }

  if (phase.name === "delivered") {
    return (
      <Outcome
        tone="ok"
        icon={<ShieldCheck className="size-6" />}
        title={t("done.title", { client: clientLabel })}
        body={t("done.body", { appName })}
        note={t("done.close")}
      />
    );
  }

  if (phase.name === "pickup") {
    return (
      <div className="space-y-5">
        <Outcome
          tone="ok"
          icon={<ShieldCheck className="size-6" />}
          title={tp("waiting.title", { client: clientLabel })}
          body={tp("waiting.body", { appName })}
          note={tp("waiting.close")}
        />
        <details className="text-left">
          <summary className="text-muted-foreground cursor-pointer text-xs">{tp("fallback.summary")}</summary>
          <div className="mt-3 space-y-2">
            <p className="text-muted-foreground text-xs leading-relaxed">{tp("fallback.body")}</p>
            <div className="bg-muted/50 flex items-center gap-2 rounded-lg p-2 ring-1 ring-foreground/10">
              <code className="min-w-0 flex-1 truncate px-1 font-mono text-xs">{phase.token}</code>
              <Button
                size="icon-sm"
                variant="outline"
                onClick={() => copyToken(phase.token)}
                aria-label={tc("copy")}
              >
                {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
              </Button>
            </div>
            <p className="text-muted-foreground text-xs leading-relaxed">{t("manual.hint")}</p>
          </div>
        </details>
      </div>
    );
  }

  if (phase.name === "denied") {
    return (
      <Outcome
        tone="muted"
        icon={<ShieldAlert className="size-6" />}
        title={t("denied.title")}
        body={t("denied.body")}
        note={t("done.close")}
      />
    );
  }

  if (phase.name === "refused") {
    return (
      <div className="space-y-5">
        <Outcome
          tone="warn"
          icon={<TriangleAlert className="size-6" />}
          title={t("refused.title")}
          body={t(`refused.${phase.reason}`)}
        />
        {phase.reason === "signed-out" ? null : (
          <Button variant="outline" className="w-full" onClick={() => setPhase({ name: "idle" })}>
            {t("refused.back")}
          </Button>
        )}
      </div>
    );
  }

  if (phase.name === "manual") {
    return (
      <div className="space-y-5">
        <Outcome
          tone="warn"
          icon={<TriangleAlert className="size-6" />}
          title={t("manual.title", { host })}
          body={t("manual.body", { client: clientLabel })}
        />

        <div className="space-y-2">
          <div className="bg-muted/50 flex items-center gap-2 rounded-lg p-2 ring-1 ring-foreground/10">
            <code className="min-w-0 flex-1 truncate px-1 font-mono text-xs">{phase.token}</code>
            <Button
              size="icon-sm"
              variant="outline"
              onClick={() => copyToken(phase.token)}
              aria-label={tc("copy")}
            >
              {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
            </Button>
          </div>
          <p className="text-muted-foreground text-xs leading-relaxed">{t("manual.hint")}</p>
        </div>

        <Button
          variant="outline"
          className="w-full"
          onClick={() => retry(phase.token)}
          disabled={pending}
        >
          {pending ? <Loader2 className="size-4 animate-spin" /> : null}
          {t("manual.retry")}
        </Button>
      </div>
    );
  }

  const busy = pending || phase.name === "working";

  return (
    <div className="space-y-6">
      {/* Who is asking. The name is the client's own claim, so it is set in a
          quoted, monospaced frame rather than woven into our sentences — a
          prompt should never look like it is vouching for the app. */}
      <div className="bg-muted/40 flex items-start gap-3 rounded-xl p-3.5 ring-1 ring-foreground/10">
        <div className="bg-background text-primary grid size-9 shrink-0 place-items-center rounded-lg ring-1 ring-foreground/10">
          <Laptop className="size-4.5" />
        </div>
        <div className="min-w-0 space-y-0.5">
          <p className="truncate text-sm font-medium">{clientLabel}</p>
          <p className="text-muted-foreground font-mono text-xs">{host}</p>
        </div>
      </div>

      <div className="space-y-3">
        <p className="text-sm font-medium">{t("permissions.heading")}</p>
        <ul className="space-y-2.5">
          <Permission icon={<Upload className="size-4" />} text={t("permissions.upload")} />
          <Permission icon={<FolderOpen className="size-4" />} text={t("permissions.read")} />
          <Permission icon={<Trash2 className="size-4" />} text={t("permissions.delete")} />
          {/* Added on review: the token also opens POST /api/shortlinks and PUT
              /api/client-settings, which landed alongside this page from other
              hands. A consent prompt that lists three of five capabilities is
              not consent to the other two - and minting redirects under
              somebody's own tag is the one with consequences off this site. */}
          <Permission icon={<Link2 className="size-4" />} text={t("permissions.links")} />
        </ul>
        <p className="text-muted-foreground text-xs leading-relaxed">
          {t("permissions.notIncluded")}
        </p>
      </div>

      <Separator />

      <div className="space-y-1.5">
        <p className="text-muted-foreground text-xs leading-relaxed">
          {t("account", { email: account.email })}
        </p>
        <p className="text-muted-foreground text-xs leading-relaxed">
          {pickup ? tp("local", { code: request.confirmCode ?? "" }) : t("local", { host })}
        </p>
      </div>

      {pickup && request.confirmCode ? (
        // Review 2026-10-24, P2: a phishing link carries somebody else's
        // challenge, so its code is one no app of this person shows.
        <div
          className="bg-muted/40 space-y-2 rounded-xl p-3.5 text-center ring-1 ring-foreground/10"
          data-testid="confirm-code"
        >
          <p className="text-muted-foreground text-xs font-medium">{tp("confirm.label")}</p>
          <p className="font-mono text-2xl font-semibold tracking-[0.2em]">{request.confirmCode}</p>
          <p className="text-sm font-medium leading-relaxed">{tp("confirm.match")}</p>
          <p className="text-muted-foreground text-xs leading-relaxed">{tp("confirm.mismatch")}</p>
        </div>
      ) : null}

      <div className="flex flex-col gap-2">
        <Button size="lg" className="h-10 w-full" onClick={approve} disabled={busy}>
          {busy ? <Loader2 className="size-4 animate-spin" /> : <ShieldCheck className="size-4" />}
          {busy ? t("approving") : t("approve")}
        </Button>
        <Button variant="ghost" className="h-10 w-full" onClick={deny} disabled={busy}>
          {t("deny")}
        </Button>
      </div>

      {/* The last line, and the one that matters if this page was opened by a
          link somebody sent: nothing has been issued yet. */}
      <p className="text-muted-foreground border-border/60 border-t pt-4 text-xs leading-relaxed">
        {t("unexpected")}
      </p>
    </div>
  );
}

function Permission({ icon, text }: { icon: React.ReactNode; text: string }) {
  return (
    <li className="flex items-start gap-2.5 text-sm leading-relaxed">
      <span className="text-muted-foreground mt-0.5 shrink-0">{icon}</span>
      <span>{text}</span>
    </li>
  );
}

function Outcome({
  tone,
  icon,
  title,
  body,
  note,
}: {
  tone: "ok" | "warn" | "muted";
  icon: React.ReactNode;
  title: string;
  body: string;
  note?: string;
}) {
  return (
    <div className="space-y-4 text-center">
      <div
        className={cn(
          "mx-auto grid size-12 place-items-center rounded-2xl ring-1 ring-foreground/10",
          tone === "ok" && "bg-primary/10 text-primary",
          tone === "warn" && "bg-destructive/10 text-destructive",
          tone === "muted" && "bg-muted text-muted-foreground",
        )}
      >
        {icon}
      </div>
      <div className="space-y-1.5">
        <p className="font-heading text-base font-semibold tracking-tight">{title}</p>
        <p className="text-muted-foreground text-sm leading-relaxed">{body}</p>
      </div>
      {note ? (
        <p className="text-muted-foreground inline-flex items-center gap-1.5 text-xs">
          <ArrowRight className="size-3" />
          {note}
        </p>
      ) : null}
    </div>
  );
}
