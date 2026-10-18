/**
 * Data of the three upstream layouts, as carried in `layouts` of every page
 * payload (docs/CONTRACT.md "Layouts").
 */
import type { BackgroundStyle } from "@/lib/background";
import type { Mode } from "@/lib/theme-cookies";
import type { ThemeId } from "@/lib/themes";
import type { ShellSidebarData, ShellUser } from "@/components/app-shell";
import type { TransferFolder } from "@/components/transfers/TransfersProvider";

/**
 * The public part of upstream `config` (lib/config.ts). Everything else in it is
 * server-only and never leaves the server.
 */
export interface PublicConfig {
  /** APP_NAME, default "Modlight". */
  appName: string;
  /** EMBED_BRAND, or the brand inside APP_URL (brandFromUrl). */
  embedBrand: string;
  /** APP_URL without a trailing slash. */
  appUrl: string;
  /** APP_URL first, then APP_DOMAINS, deduplicated. */
  appDomains: string[];
  /** SHORTLINK_URL, falling back to APP_URL. */
  shortUrl: string;
}

/** app/layout.tsx */
export interface RootLayoutData {
  /** i18n/request.ts: cookie `ml_locale`, else Accept-Language (pl if it names pl, else en). */
  locale: "en" | "pl";
  /**
   * The server process's IANA time zone. next-intl hands it to the client
   * provider (NextIntlClientProvider inherits `timeZone` from the request config,
   * which defaults to Intl.DateTimeFormat().resolvedOptions().timeZone - "UTC"
   * in the upstream Docker image).
   */
  timeZone: string;
  /** resolveMode(cookie ml_mode): the ThemeProvider's defaultTheme and <html data-mode>. */
  mode: Mode;
  /** resolveTheme(session?.user.theme ?? cookie ml_skin): <html data-theme>. */
  skin: ThemeId;
  config: PublicConfig;
}

/** app/(app)/layout.tsx */
export interface AppLayoutData {
  /** session.user.theme, unresolved (ThemeApplier resolves it). */
  theme: string | null;
  /** backgroundStyle(prefs.userCode, prefs.backgroundKey, prefs.backgroundDim). */
  background: BackgroundStyle | null;
  /** flattenFolders(folderTree(userId)): every folder as { id, name }, depth first. */
  transferFolders: TransferFolder[];
  /** getUserPrefs(userId)?.defaultFolderId ?? null. */
  defaultFolderId: string | null;
  /** The AppShell `user` prop (image already resolved through avatarUrl). */
  user: ShellUser;
  /** The AppShell `sidebar` prop. */
  sidebar: ShellSidebarData;
  /** cookie ml_sidebar === "collapsed". */
  defaultCollapsed: boolean;
  /**
   * What DuplicateReminderSlot decided: null when the snooze cookie holds, the
   * reminder is not due, or the count failed; otherwise the two numbers it shows.
   */
  duplicateReminder: { redundant: number; reclaimable: number } | null;
}

/** app/(auth)/layout.tsx reads only config.appName (root) and translations. */
export type AuthLayoutData = Record<string, never>;
