// Port of upstream app/(app)/dashboard/tokens/page.tsx (Integrations).
// @upstream app/(app)/dashboard/tokens/page.tsx
//
// Server (Rust), as upstream: requireUser(); myTokens() mapped without the
// token hash; myUrlPrefs().baseUrl; the "friendly handle" for the .sxcu file
// name; plus currentRelease() for <AppCard>, which upstream's AppCard awaited
// itself (see the AppCard port). `metadata = { title: "Integrations" }`.
import type { ComponentProps } from "react";
import { useTranslations } from "next-intl";
import { config } from "@/lib/config";
import type { ReleaseManifest } from "@/lib/releases";
import { TokenManager } from "@/components/tokens/TokenManager";
import { SetupGuide } from "@/components/tokens/SetupGuide";
import { OneClickShareX } from "@/components/tokens/OneClickShareX";
import { AppCard } from "@/components/tokens/AppCard";

export interface TokensPageProps {
  /** myTokens() rows mapped as upstream maps them (no tokenHash) */
  tokens: ComponentProps<typeof TokenManager>["tokens"];
  /** myUrlPrefs().baseUrl */
  appUrl: string;
  /** session.user.name?.split(" ")[0] || email local part || userCode || "user" */
  username: string;
  /** session.user.userCode, name, email */
  user: { userCode: string | null; name: string | null; email: string };
  /** currentRelease() (read by <AppCard> upstream) */
  release: ReleaseManifest | null;
}

export default function TokensPage({ props }: { props: TokensPageProps }) {
  const t = useTranslations("tokens");
  const { tokens, appUrl, username } = props;
  const multiDomain = config.appDomains.length > 1;
  return (
    <div className="mx-auto w-full max-w-5xl space-y-8">
      <header className="space-y-1">
        <h1 className="font-heading text-2xl font-semibold tracking-tight">
          {t("pageTitle")}
        </h1>
        <p className="text-muted-foreground text-sm">
          {t("pageIntro", { appName: config.appName })}
        </p>
      </header>

      {/* Our own client first, ShareX second.
          ShareX stays fully supported — it is what these accounts use today and
          nothing about it breaks. But it is no longer the headline: somebody
          arriving here should install our app, sign in, and be done, and the
          order of the page is the clearest way to say that. */}
      <AppCard release={props.release} />

      <section className="space-y-3">
        <div className="space-y-1">
          <h2 className="font-heading text-lg font-semibold tracking-tight">
            {t("sections.sharex.title")}
          </h2>
          <p className="text-muted-foreground text-sm">{t("sections.sharex.desc")}</p>
        </div>
        <OneClickShareX
          appUrl={appUrl}
          appName={config.appName}
          username={username}
          showDomainNote={multiDomain}
        />
      </section>

      <div className="space-y-3">
        <div className="space-y-1">
          <h2 className="font-heading text-lg font-semibold tracking-tight">
            {t("advanced")}
          </h2>
          <p className="text-muted-foreground text-sm">
            {t("advancedDesc")}
          </p>
        </div>

        <TokenManager
          tokens={tokens}
          appUrl={appUrl}
          appName={config.appName}
          userCode={props.user.userCode}
          userName={props.user.name ?? null}
          userEmail={props.user.email}
        />
      </div>

      <SetupGuide appUrl={appUrl} appName={config.appName} />
    </div>
  );
}
