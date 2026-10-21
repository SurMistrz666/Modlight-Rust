// Port of upstream app/(app)/dashboard/admin/page.tsx.
// @upstream app/(app)/dashboard/admin/page.tsx
//
// Server (Rust), as upstream: requireAdmin() (signed out -> /login, not an
// admin -> /dashboard); isRegistrationEnabled(), registrationKeyState(),
// toMailStatus(loadMailSettings(true)), loadGlobalStats() (the two aggregate
// queries in the upstream file), serverStorageUsage(); plus diskInsights(),
// which upstream's <ServerDisk> awaited itself (see the ServerDisk port).
// `metadata = { title: "Admin" }`. Not upstream: `turnstile`, the state of
// the optional Cloudflare Turnstile card under Settings (src/turnstile/).
import {
  ShieldCheck,
  SlidersHorizontal,
  Users,
  BarChart3,
  Ghost,
  Wrench,
  Mail,
} from "lucide-react";
import { useTranslations } from "next-intl";
import type { RegistrationKeyState } from "@/lib/registration";
import type { MailStatus } from "@/lib/mail-settings";
import type { ServerStorageUsage } from "@/lib/data/admin";
import type { DiskInsights } from "@/lib/data/admin-disk";
import { Badge } from "@/components/ui/badge";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { SettingsForm } from "@/components/admin/SettingsForm";
import { MailSettings } from "@/components/admin/MailSettings";
import { UserManager } from "@/components/admin/UserManager";
import { StatsCards, type GlobalStats } from "@/components/admin/StatsCards";
import { ReleaseChannel } from "@/components/admin/ReleaseChannel";
import { ServerDisk } from "@/components/admin/ServerDisk";
import { OrphanFiles } from "@/components/admin/OrphanFiles";
import { Maintenance } from "@/components/admin/Maintenance";
import { TurnstileSettings } from "../../../../turnstile/TurnstileSettings";
import type { TurnstileStatus } from "../../../../turnstile/actions";

export interface AdminPageProps {
  /** session.user.id and session.user.email of the signed-in administrator */
  session: { user: { id: string; email: string } };
  /** isRegistrationEnabled() */
  regEnabled: boolean;
  /** registrationKeyState() */
  keyState: RegistrationKeyState;
  /** toMailStatus(await loadMailSettings(true)) */
  mail: MailStatus;
  /** loadGlobalStats() (upstream page-local function) */
  stats: GlobalStats;
  /** serverStorageUsage() */
  disk: ServerStorageUsage;
  /** diskInsights() (awaited inside <ServerDisk> upstream) */
  diskInsights: DiskInsights;
  /** isUnlimited(config.defaultUserQuota) ? null : humanFileSize(config.defaultUserQuota) */
  defaultQuota: string | null;
  /**
   * Not upstream: the optional Cloudflare Turnstile card's state
   * (server/src/turnstile.rs). Absent from an older server: no card.
   */
  turnstile?: TurnstileStatus;
}

export default function AdminPage({ props }: { props: AdminPageProps }) {
  const t = useTranslations("admin");
  const { session, regEnabled, keyState, mail, stats, disk, defaultQuota } = props;
  return (
    <div className="mx-auto flex w-full max-w-5xl flex-col gap-6">
      {/* Header */}
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-start gap-3">
          <span className="bg-primary/10 text-primary grid size-10 shrink-0 place-items-center rounded-xl">
            <ShieldCheck className="size-5" />
          </span>
          <div>
            <h1 className="font-heading text-xl font-semibold tracking-tight">
              {t("title")}
            </h1>
            <p className="text-muted-foreground text-sm">
              {t("intro")}
            </p>
          </div>
        </div>
        <Badge variant="outline" className="text-muted-foreground hidden sm:inline-flex">
          {t("signedInAs", { email: session.user.email })}
        </Badge>
      </div>

      {/* Five sections, in the order an operator works through them: look at the
          install, then the people on it, then the mess, then the knobs, then the
          jobs. Each tab's content mounts on activation, so the heavy ones
          (orphan listing, maintenance) cost nothing until asked for. */}
      <Tabs defaultValue="overview" className="gap-6">
        <div className="-mx-1 overflow-x-auto px-1">
          <TabsList className="bg-muted/60 h-9 w-max justify-start">
            <TabsTrigger value="overview" className="gap-1.5 px-3">
              <BarChart3 className="size-4" /> {t("tabs.overview")}
            </TabsTrigger>
            <TabsTrigger value="users" className="gap-1.5 px-3">
              <Users className="size-4" /> {t("tabs.users")}
            </TabsTrigger>
            <TabsTrigger value="orphans" className="gap-1.5 px-3">
              <Ghost className="size-4" /> {t("tabs.orphans")}
              {stats.orphanFiles > 0 ? (
                <span className="bg-amber-500/15 text-amber-600 dark:text-amber-400 ml-0.5 rounded-full px-1.5 py-px text-[0.65rem] font-medium tabular-nums">
                  {stats.orphanFiles}
                </span>
              ) : null}
            </TabsTrigger>
            <TabsTrigger value="settings" className="gap-1.5 px-3">
              <SlidersHorizontal className="size-4" /> {t("tabs.settings")}
            </TabsTrigger>
            <TabsTrigger value="mail" className="gap-1.5 px-3">
              <Mail className="size-4" /> {t("tabs.mail")}
              {/* An install with no outgoing mail cannot reset a password or
                  confirm an address, and the operator should hear that from the
                  application rather than from a stuck user. */}
              {!mail.ready ? (
                <span className="ml-0.5 rounded-full bg-amber-500/15 px-1.5 py-px text-[0.65rem] font-medium text-amber-600 dark:text-amber-400">
                  {t("mail.offBadge")}
                </span>
              ) : null}
            </TabsTrigger>
            <TabsTrigger value="maintenance" className="gap-1.5 px-3">
              <Wrench className="size-4" /> {t("tabs.maintenance")}
            </TabsTrigger>
          </TabsList>
        </div>

        {/* The machine comes first, above the per-account tiles: an account over
            its quota is that account's problem, a full disk is everyone's. */}
        <TabsContent value="overview" className="flex flex-col gap-4">
          <ServerDisk usage={disk} insights={props.diskInsights} />
          <StatsCards stats={stats} />
        </TabsContent>

        <TabsContent value="users">
          <UserManager currentUserId={session.user.id} mailReady={mail.ready} />
        </TabsContent>

        <TabsContent value="orphans">
          <OrphanFiles />
        </TabsContent>

        <TabsContent value="settings" className="flex flex-col gap-6">
          <SettingsForm
            registration={{ enabled: regEnabled, keyState }}
            defaultQuota={defaultQuota}
          />
          {/* Not upstream: the optional Cloudflare Turnstile check. */}
          {props.turnstile ? <TurnstileSettings initial={props.turnstile} /> : null}
        </TabsContent>

        <TabsContent value="mail">
          <MailSettings initial={mail} adminEmail={session.user.email} />
        </TabsContent>

        <TabsContent value="maintenance" className="flex flex-col gap-4">
          <ReleaseChannel />
          <Maintenance />
        </TabsContent>
      </Tabs>
    </div>
  );
}
