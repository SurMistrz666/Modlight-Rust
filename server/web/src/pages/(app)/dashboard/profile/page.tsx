// Port of upstream app/(app)/dashboard/profile/page.tsx.
// @upstream app/(app)/dashboard/profile/page.tsx
//
// Server (Rust), as upstream: requireUser(); userStats(), getUserPrefs(),
// folderTree(), myUrlPrefs(), listMySessions(). Three values the page derived
// from server-only inputs arrive ready: the flattened folder list
// (flattenFolders(tree)), the avatar URL (avatarUrl over `user.image`, a storage
// key the browser must not see) and the wallpaper URL (backgroundUrl over
// `backgroundKey`, which embeds the internal user id).
// `metadata = { title: "Profile" }`.
import { useTranslations } from "next-intl";
import { config } from "@/lib/config";
import type { SessionView } from "@/lib/data/sessions";
import { DEFAULT_THEME } from "@/lib/themes";
import type { UrlPrefs } from "@/app/actions/account";
import { ProfileForm } from "@/components/profile/ProfileForm";
import { SecuritySection } from "@/components/profile/SecuritySection";
import { SessionList } from "@/components/profile/SessionList";
import { QuotaCard } from "@/components/profile/QuotaCard";
import { AppearanceSection } from "@/components/profile/AppearanceSection";
import { DuplicatesSection } from "@/components/profile/DuplicatesSection";
import { BackgroundSection } from "@/components/profile/BackgroundSection";
import { AvatarSection } from "@/components/profile/AvatarSection";
import { DIM_DEFAULT } from "@/lib/background";
import { NamingSection } from "@/components/profile/NamingSection";
import { DomainSection } from "@/components/profile/DomainSection";
import {
  TagSection,
  DefaultFolderSection,
  type FolderOption,
} from "@/components/profile/TagSection";
import { EncryptionSettings } from "../../../../encryption/EncryptionSettings";
import type { VaultStatus } from "../../../../encryption/actions";

export interface ProfilePageProps {
  /** The session user's fields the page reads (session.user) */
  u: {
    name: string;
    email: string;
    userCode: string | null;
    hideUploads: boolean | null;
    copyRaw: boolean | null;
    usedQuota: number | null;
    maxQuota: number | null;
    role: string | null;
    theme: string | null;
    locale: string | null;
    twoFactorEnabled: boolean | null;
    emailVerified: boolean;
  };
  /** userStats(u.id) */
  stats: { count: number; totalSize: number; totalViews: number };
  /** getUserPrefs(u.id), the fields the page reads */
  prefs: { userCode: string | null; skipTagInUrl: boolean; defaultFolderId: string | null; backgroundDim: number } | null;
  /** myUrlPrefs() */
  urlPrefs: UrlPrefs;
  /** listMySessions() */
  sessions: SessionView[];
  /** flattenFolders(folderTree(u.id)) */
  folders: FolderOption[];
  /** avatarUrl(prefs?.userCode ?? u.userCode, u.image) */
  avatarUrl: string | null;
  /** backgroundUrl(prefs?.userCode, prefs?.backgroundKey) */
  backgroundUrl: string | null;
  /** Not upstream: the "File encryption" card (docs/ENCRYPTION.md). */
  encryption?: VaultStatus;
}
/**
 * One labelled band of settings.
 *
 * The cards inside sit in a two-column grid on wide viewports and collapse to
 * one below that. Grid items stretch by default, so two cards sharing a row end
 * up the same height — the reason this is a grid and not a column of cards is
 * exactly that: the page used to be a single tall stack with the whole right
 * half of the screen empty.
 */
function SettingsGroup({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="space-y-3">
      <div className="flex items-center gap-3">
        <h2 className="text-muted-foreground text-xs font-semibold tracking-wider uppercase">
          {title}
        </h2>
        <span aria-hidden className="bg-foreground/10 h-px flex-1" />
      </div>
      <div className="grid gap-5 xl:grid-cols-2">{children}</div>
    </section>
  );
}

export default function ProfilePage({ props }: { props: ProfilePageProps }) {
  const t = useTranslations("profile");
  const { u, stats, prefs, urlPrefs, sessions } = props;

  // Previews are built from the hostname this account actually publishes on, so
  // the tag/naming cards show the real link rather than the deployment default.
  const tag = prefs?.userCode ?? u.userCode ?? null;
  const linkTag = prefs?.skipTagInUrl ? null : tag;

  return (
    <div className="mx-auto w-full max-w-[90rem] space-y-8">
      <header className="space-y-1">
        <h1 className="font-heading text-2xl font-semibold tracking-tight">{t("pageTitle")}</h1>
        <p className="text-muted-foreground text-sm">{t("pageIntro")}</p>
      </header>

      {/* Who you are + what you are using. */}
      <SettingsGroup title={t("groups.account")}>
        <ProfileForm
          defaults={{
            name: u.name ?? "",
            email: u.email,
            userCode: u.userCode ?? null,
            hideUploads: Boolean(u.hideUploads),
            copyRaw: Boolean(u.copyRaw),
          }}
        />

        <QuotaCard
          usedQuota={u.usedQuota ?? 0}
          maxQuota={u.maxQuota ?? -1}
          totalSize={stats.totalSize}
          fileCount={stats.count}
          totalViews={stats.totalViews}
        />

        {/* Full width: it renders a list, not a form. Nothing runs until the
            owner presses Scan, so this costs the page no queries on load. */}
        <DuplicatesSection />
      </SettingsGroup>

      {/* Everything that shapes a share link, in the order it appears in one:
          host / tag / file name — plus where the file itself lands. */}
      <SettingsGroup title={t("groups.links")}>
        <TagSection
          appUrl={urlPrefs.baseUrl}
          tag={tag}
          isAdmin={u.role === "admin"}
          skipTagInUrl={prefs?.skipTagInUrl ?? false}
        />

        <NamingSection
          baseUrl={urlPrefs.baseUrl}
          tag={linkTag}
          scheme={urlPrefs.scheme}
          pattern={urlPrefs.pattern}
          wordCount={urlPrefs.wordCount}
          randomLength={urlPrefs.randomLength}
        />

        {/* Renders nothing when the deployment answers on a single hostname —
            the grid simply flows the next card into its place. */}
        <DomainSection
          domains={config.appDomains}
          defaultDomain={config.appUrl}
          preferredDomain={urlPrefs.preferredDomain}
          tag={linkTag}
        />

        <DefaultFolderSection
          folders={props.folders}
          defaultFolderId={prefs?.defaultFolderId ?? null}
        />
      </SettingsGroup>

      {/* Appearance is owned by the theming work; it keeps its slot here. */}
      <SettingsGroup title={t("groups.personal")}>
        <AvatarSection
          url={props.avatarUrl}
          initials={(u.name || u.email).slice(0, 2).toUpperCase()}
        />

        <AppearanceSection theme={u.theme ?? DEFAULT_THEME} locale={u.locale ?? null} />

        <BackgroundSection
          url={props.backgroundUrl}
          dim={prefs?.backgroundDim ?? DIM_DEFAULT}
        />

        <SecuritySection
          twoFactorEnabled={Boolean(u.twoFactorEnabled)}
          emailVerified={Boolean(u.emailVerified)}
        />

        {/* Not upstream: per-account file encryption. Full width: the list of
            browsers that remember the password carries long labels. */}
        <EncryptionSettings initial={props.encryption} />

        {/* Full width: one row per session, and the rows carry long strings
            (the raw User-Agent) that a half-width card would only truncate. */}
        <SessionList sessions={sessions} />
      </SettingsGroup>
    </div>
  );
}
