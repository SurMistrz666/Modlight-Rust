// Port of upstream app/s/[code]/unlock/page.tsx (password gate of a short link).
// @upstream app/s/[code]/unlock/page.tsx
//
// Server (Rust), as upstream: the short link by code, LEFT JOIN its owner ->
// notFound when there is no such link; redirect to /s/{code} when it has no
// password; otherwise the owner's skin and wallpaper. generateMetadata:
// { title: t("public.shortlink.metaTitle"), robots: { index: false, follow: false } }.
import { useTranslations } from "next-intl";
import { config } from "@/lib/config";
import type { BackgroundStyle } from "@/lib/background";
import { BrandMark } from "@/components/auth/brand-mark";
import { AuthCard } from "@/components/auth/auth-card";
import { ThemeApplier } from "@/components/theme-applier";
import { UnlockForm } from "@/app/s/[code]/unlock/unlock-form";

export interface ShortlinkUnlockPageProps {
  /** The stored code (for /s/{tag}/{name}/unlock: scopedShortCode(tag, name)) */
  code: string;
  /** users.theme of the link's owner (null when the owner row is missing) */
  ownerTheme: string | null;
  /** backgroundStyle(ownerUserCode, ownerBackgroundKey, ownerBackgroundDim) */
  background: BackgroundStyle | null;
}

/**
 * Public gate page for a password-protected short link. Renders a small branded
 * card with a password form; on success the form drops the unlock cookie and
 * navigates back to /s/{code}, which then passes the gate and redirects.
 *
 * Like every other public surface (file preview, album, user gallery) this page
 * wears the OWNER's skin rather than the visitor's: a shared link is the
 * owner's page, whoever opens it. The skin is joined in here so the gate is not
 * the one screen in the chain that flips back to the visitor's own theme.
 */
export default function ShortlinkUnlockPage({ props }: { props: ShortlinkUnlockPageProps }) {
  const t = useTranslations("public");
  const { code } = props;

  return (
    <div className="page-surface relative flex min-h-svh flex-col overflow-hidden">
      {/* First child on purpose: ThemeApplier emits a blocking inline script, so
          it has to run before the browser paints the wrong theme. */}
      <ThemeApplier theme={props.ownerTheme} background={props.background} />
      {/* Ambient background mirrors the auth shell, using token-driven accents. */}
      <div aria-hidden className="pointer-events-none absolute inset-0">
        <div className="bg-primary/10 absolute -bottom-40 left-1/2 size-[44rem] -translate-x-1/2 rounded-full blur-2xl" />
      </div>

      <header className="relative z-10 flex items-center justify-center px-6 pt-10 sm:justify-start sm:px-10">
        <BrandMark appName={config.appName} href="/" />
      </header>

      <main className="relative z-10 flex flex-1 items-center justify-center px-4 py-10 sm:py-12">
        <div className="w-full max-w-sm">
          <AuthCard
            title={t("shortlink.cardTitle")}
            description={t("shortlink.cardDescription")}
          >
            <UnlockForm code={code} />
          </AuthCard>
        </div>
      </main>

      <footer className="text-muted-foreground relative z-10 flex items-center justify-center px-6 pb-8 text-center text-xs">
        <p>
          &copy; {new Date().getFullYear()} {config.appName}
        </p>
      </footer>
    </div>
  );
}
