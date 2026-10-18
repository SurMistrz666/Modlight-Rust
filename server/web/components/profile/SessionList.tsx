"use client";

import { useState, useTransition } from "react";
import { useRouter } from "next/navigation";
import { useFormatter, useLocale, useTranslations } from "next-intl";
import {
  Clock3,
  Globe,
  Info,
  Laptop,
  LogOut,
  MonitorSmartphone,
  Smartphone,
  Tablet,
  Terminal,
} from "lucide-react";
import { toast } from "sonner";
import { relativeTime } from "@/lib/format";
import { revokeMyOtherSessions, revokeMySession } from "@/app/actions/account";
import type { DeviceKind, SessionView } from "@/lib/data/sessions";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

const ICONS: Record<DeviceKind, typeof Laptop> = {
  desktop: Laptop,
  mobile: Smartphone,
  tablet: Tablet,
  client: Terminal,
  unknown: Globe,
};

/**
 * The account's live sessions, with revocation.
 *
 * Everything shown here is either a timestamp we wrote ourselves or a string the
 * client sent us. The client-sent parts - the User-Agent and the address - are
 * rendered as TEXT, never as markup and never into an href: this component has
 * no `dangerouslySetInnerHTML` and must not grow one, because a User-Agent is
 * free text chosen by whoever opened the session.
 *
 * The current session carries a badge and no button. It is the one mistake this
 * screen invites: every row looks the same, and signing yourself out of the page
 * you are reading looks like a bug rather than like the thing you just clicked.
 */
export function SessionList({ sessions }: { sessions: SessionView[] }) {
  const t = useTranslations("profile.sessions");
  const tc = useTranslations("common");
  const format = useFormatter();
  const locale = useLocale();
  const router = useRouter();

  const [pending, startTransition] = useTransition();
  const [busyId, setBusyId] = useState<string | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);

  const others = sessions.filter((s) => !s.current).length;

  /**
   * Absolute time for the tooltip behind every relative one.
   *
   * Both of these are safe to render during SSR even though they depend on the
   * environment: the provider in the root layout is a Server Component, so
   * next-intl hands the client the SAME locale and time zone the server
   * formatted with. Reading either from the browser instead would make the
   * hydrated markup differ from the sent markup.
   */
  function stamp(value: string): string {
    return format.dateTime(new Date(value), { dateStyle: "medium", timeStyle: "short" });
  }

  /** "2 hr. ago" / "2 godz. temu" - relative time in the reader's language. */
  function ago(value: string): string {
    return relativeTime(value, locale);
  }

  function signOut(session: SessionView) {
    setBusyId(session.id);
    startTransition(async () => {
      try {
        const { outcome } = await revokeMySession(session.id);
        if (outcome === "ok") toast.success(t("revoked"));
        else if (outcome === "current") toast.error(t("currentRefused"));
        else if (outcome === "gone") toast.message(t("gone"));
        else toast.error(t("revokeError"));
        router.refresh();
      } catch {
        toast.error(t("revokeError"));
      } finally {
        setBusyId(null);
      }
    });
  }

  function signOutOthers() {
    startTransition(async () => {
      try {
        const { ok, revoked } = await revokeMyOtherSessions();
        if (ok) toast.success(t("revokedOthers", { count: revoked }));
        else toast.error(t("revokeError"));
        setConfirmOpen(false);
        router.refresh();
      } catch {
        toast.error(t("revokeError"));
      }
    });
  }

  return (
    <>
      <Card className="xl:col-span-2">
        <CardHeader className="border-b">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="space-y-1">
              <CardTitle className="flex items-center gap-2">
                <MonitorSmartphone className="text-primary size-4" />
                {t("title")}
              </CardTitle>
              <CardDescription>{t("desc")}</CardDescription>
            </div>
            {others > 0 ? (
              <Button
                variant="outline"
                size="sm"
                onClick={() => setConfirmOpen(true)}
                disabled={pending}
              >
                <LogOut />
                {t("signOutOthers")}
              </Button>
            ) : null}
          </div>
        </CardHeader>

        <CardContent className="px-0">
          {sessions.length === 0 ? (
            <p className="text-muted-foreground px-6 py-8 text-center text-sm">{t("empty")}</p>
          ) : (
            <ul className="divide-border divide-y">
              {sessions.map((s) => {
                const Icon = ICONS[s.device.kind];
                const name = s.device.browser ?? s.device.os ?? t("unknownDevice");
                const busy = pending && busyId === s.id;
                return (
                  <li
                    key={s.id}
                    className="flex flex-wrap items-start gap-x-4 gap-y-3 px-4 py-3.5 sm:px-6"
                  >
                    <div
                      className={
                        s.current
                          ? "bg-primary/10 text-primary grid size-9 shrink-0 place-items-center rounded-lg"
                          : "bg-muted text-muted-foreground grid size-9 shrink-0 place-items-center rounded-lg"
                      }
                    >
                      <Icon className="size-4" />
                    </div>

                    <div className="min-w-0 flex-1 space-y-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="text-sm font-medium">{name}</span>
                        {s.device.browser && s.device.os ? (
                          <span className="text-muted-foreground text-sm">{s.device.os}</span>
                        ) : null}
                        {s.current ? <Badge>{t("thisDevice")}</Badge> : null}
                        {s.impersonated ? (
                          <Badge variant="destructive">{t("impersonated")}</Badge>
                        ) : null}
                      </div>

                      <div className="text-muted-foreground flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs">
                        <span title={stamp(s.createdAt)}>
                          {t("signedIn", { time: ago(s.createdAt) })}
                        </span>
                        <span aria-hidden className="opacity-40">
                          •
                        </span>
                        <span className="inline-flex items-center gap-1" title={stamp(s.renewedAt)}>
                          <Clock3 className="size-3" />
                          {t("renewed", { time: ago(s.renewedAt) })}
                        </span>
                        <span aria-hidden className="opacity-40">
                          •
                        </span>
                        <span title={stamp(s.expiresAt)}>
                          {t("expires", { time: ago(s.expiresAt) })}
                        </span>
                      </div>

                      <div className="text-muted-foreground flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs">
                        {/* Plain text. The address is never turned into a link
                            or a lookup: we have no location data and will not
                            imply that we do. */}
                        <span className="font-mono">
                          {s.ip ? t("ip", { ip: s.ip }) : t("ipUnknown")}
                        </span>
                      </div>

                      {s.userAgent ? (
                        <p
                          className="text-muted-foreground/80 truncate font-mono text-[11px]"
                          title={s.userAgent}
                        >
                          <span className="sr-only">{t("uaLabel")}: </span>
                          {s.userAgent}
                        </p>
                      ) : null}

                      {s.current ? (
                        <p className="text-muted-foreground text-xs">{t("currentHint")}</p>
                      ) : null}
                    </div>

                    {s.current ? null : (
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => signOut(s)}
                        disabled={pending}
                        aria-label={t("signOutAria", { device: name })}
                      >
                        <LogOut />
                        {busy ? t("signingOut") : t("signOut")}
                      </Button>
                    )}
                  </li>
                );
              })}
            </ul>
          )}
        </CardContent>

        <CardFooter className="border-t">
          <Alert>
            <Info />
            <AlertTitle>{t("notesTitle")}</AlertTitle>
            <AlertDescription>
              <p className="mb-2!">{t("renewNote")}</p>
              <p>{t("ipNote")}</p>
            </AlertDescription>
          </Alert>
        </CardFooter>
      </Card>

      <Dialog open={confirmOpen} onOpenChange={(o) => (pending ? null : setConfirmOpen(o))}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t("confirmTitle")}</DialogTitle>
            <DialogDescription>{t("confirmBody", { count: others })}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setConfirmOpen(false)} disabled={pending}>
              {tc("cancel")}
            </Button>
            <Button variant="destructive" onClick={signOutOthers} disabled={pending}>
              <LogOut />
              {t("confirmCta", { count: others })}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
