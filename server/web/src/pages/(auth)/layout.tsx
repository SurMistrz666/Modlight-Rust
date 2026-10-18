// Port of upstream app/(auth)/layout.tsx (an async Server Component there).
// @upstream app/(auth)/layout.tsx
// Deviations: `async` dropped, getTranslations -> useTranslations; `config` is
// the browser config (appName from the root layout payload); the layout data
// prop is empty (contract/layouts.ts AuthLayoutData).
import Link from "next/link";
import { useTranslations } from "next-intl";
import { BrandMark } from "@/components/auth/brand-mark";
import { config } from "@/lib/config";
import type { AuthLayoutData } from "../../contract/layouts";

/**
 * Centered, branded auth shell. Server component — reads server-only `config`
 * for branding and renders an ambient violet field behind a single column that
 * holds the wordmark, the active auth card (children), and a quiet footer.
 */
export default function AuthLayout({
  children,
}: {
  data: AuthLayoutData;
  children: React.ReactNode;
}) {
  const t = useTranslations("auth");
  const year = new Date().getFullYear();

  return (
    <div className="page-surface relative flex min-h-svh flex-col overflow-hidden">
      {/* Ambient background: layered radial glows + faint grid, dark-first. */}
      <div aria-hidden className="pointer-events-none absolute inset-0">
        <div className="absolute inset-0 bg-[radial-gradient(60%_50%_at_50%_-10%,oklch(0.546_0.215_277/0.22),transparent_70%)]" />
        <div className="absolute -bottom-40 left-1/2 size-[44rem] -translate-x-1/2 rounded-full bg-[radial-gradient(closest-side,oklch(0.546_0.215_277/0.12),transparent)] blur-2xl" />
        <div className="absolute inset-0 [mask-image:radial-gradient(70%_60%_at_50%_0%,black,transparent)] bg-[linear-gradient(to_right,oklch(0.5_0_0/0.06)_1px,transparent_1px),linear-gradient(to_bottom,oklch(0.5_0_0/0.06)_1px,transparent_1px)] bg-[size:36px_36px]" />
      </div>

      <header className="relative z-10 flex items-center justify-center px-6 pt-10 sm:justify-start sm:px-10">
        <BrandMark appName={config.appName} href="/" />
      </header>

      <main className="relative z-10 flex flex-1 items-center justify-center px-4 py-10 sm:py-12">
        <div className="w-full max-w-sm">{children}</div>
      </main>

      <footer className="text-muted-foreground relative z-10 flex flex-col items-center gap-1 px-6 pb-8 text-center text-xs sm:flex-row sm:justify-between sm:px-10">
        <p>
          &copy; {year} {config.appName}
        </p>
        <p>
          {t("layout.needHelp")}{" "}
          <Link
            href="/"
            className="text-foreground/80 underline-offset-4 hover:underline"
          >
            {t("layout.backToHome")}
          </Link>
        </p>
      </footer>
    </div>
  );
}
