// Port of upstream app/(auth)/authorize/page.tsx (desktop client permission prompt).
// @upstream app/(auth)/authorize/page.tsx
//
// Server (Rust), as upstream: parseAuthorizeRequest(port, state, name) FIRST;
// an invalid request renders the "invalid" card with no session lookup; a valid
// one without a session redirects to /login?next=<authorizePath(request)>.
// The page has no side effect. generateMetadata:
// { title: t("authorize.title"), robots: { index: false, follow: false } }.
// This path has its own CSP (connect-src adds http://127.0.0.1:*).
// Not upstream: `parsed.request.challenge` (the app's pickup) is passed on.
import Link from "next/link";
import { Unplug } from "lucide-react";
import { useTranslations } from "next-intl";
import { AuthCard } from "@/components/auth/auth-card";
import { AuthorizePrompt } from "@/components/authorize/authorize-prompt";
import type { ParsedAuthorizeRequest } from "@/lib/authorize/loopback";
import { config } from "@/lib/config";

export interface AuthorizePageProps {
  /** parseAuthorizeRequest({ port, state, name } from the query) */
  parsed: ParsedAuthorizeRequest;
  /** The signed-in account when `parsed.ok` (otherwise the server redirected), else null */
  session: { user: { email: string } } | null;
}

export default function AuthorizePage({ props }: { props: AuthorizePageProps }) {
  const t = useTranslations("authorize");
  const { parsed } = props;

  if (!parsed.ok) {
    return (
      <AuthCard title={t("invalid.title")} description={t("invalid.description")}>
        <div className="space-y-5 text-center">
          <div className="bg-muted text-muted-foreground mx-auto grid size-12 place-items-center rounded-2xl ring-1 ring-foreground/10">
            <Unplug className="size-6" />
          </div>
          <p className="text-muted-foreground text-sm leading-relaxed">{t("invalid.body")}</p>
          <Link
            href="/dashboard/tokens"
            className="text-foreground inline-block text-sm font-medium underline-offset-4 hover:underline"
          >
            {t("invalid.integrations")}
          </Link>
        </div>
      </AuthCard>
    );
  }

  const session = props.session;
  // Upstream: if (!session) redirect(`/login?next=...`) - the server already did.
  if (!session) return null;

  return (
    <AuthCard
      title={t("title")}
      description={t("description", { appName: config.appName })}
    >
      <AuthorizePrompt
        appName={config.appName}
        account={{ email: session.user.email }}
        request={{
          // The raw values the action re-validates. `port` crosses back as the
          // string it will be parsed from, so there is exactly one parser for
          // it and no chance of the page and the action disagreeing.
          port: String(parsed.request.port),
          state: parsed.request.state,
          clientName: parsed.request.clientName,
          callback: parsed.request.callback,
          // Not upstream: the desktop app's pickup challenge (SKELETON.md 9).
          challenge: (parsed.request as { challenge?: string }).challenge,
        }}
      />
    </AuthCard>
  );
}
