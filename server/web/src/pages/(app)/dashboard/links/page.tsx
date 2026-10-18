// Port of upstream app/(app)/dashboard/links/page.tsx.
// @upstream app/(app)/dashboard/links/page.tsx
//
// Server (Rust), as upstream: requireUser(); listShortLinks(),
// getUserPrefs(user.id); rows mapped to the LinkManager shape (the password
// hash becomes `hasPassword`, never shipped); mayCreateRootShortLink(prefs).
// generateMetadata: { title: t("links.metaTitle") }.
import type { ComponentProps } from "react";
import { useTranslations } from "next-intl";
import { config } from "@/lib/config";
import { LinkManager } from "@/components/links/LinkManager";

export interface LinksPageProps {
  /** listShortLinks() rows, serialized as the upstream page maps them */
  links: ComponentProps<typeof LinkManager>["links"];
  /** prefs?.userCode ?? null */
  userCode: string | null;
  /** prefs ? mayCreateRootShortLink(prefs) : false */
  canUseRoot: boolean;
}

export default function LinksPage({ props }: { props: LinksPageProps }) {
  const t = useTranslations("links");
  const { links } = props;
  return (
    <div className="mx-auto w-full max-w-5xl space-y-8">
      <header className="space-y-1">
        <h1 className="font-heading text-2xl font-semibold tracking-tight">
          {t("pageTitle")}
        </h1>
        <p className="text-muted-foreground text-sm">
          {t("pageDescription", { appName: config.appName })}
        </p>
      </header>

      <LinkManager
        links={links}
        appUrl={config.appUrl}
        shortBase={config.shortUrl}
        userCode={props.userCode}
        canUseRoot={props.canUseRoot}
      />
    </div>
  );
}
