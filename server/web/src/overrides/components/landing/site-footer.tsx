// Client port of upstream components/landing/site-footer.tsx (an async Server Component there).
// @upstream components/landing/site-footer.tsx
// Deviation: `async` dropped and getTranslations (next-intl/server) replaced by
// useTranslations (next-intl); nothing else differs.
import Link from "next/link";
import { useTranslations } from "next-intl";
import { GitFork } from "lucide-react";
import { BrandTile } from "@/components/auth/brand-mark";

interface SiteFooterProps {
  appName: string;
}

export function SiteFooter({ appName }: SiteFooterProps) {
  const t = useTranslations("landing");

  const columns: { heading: string; links: { href: string; label: string }[] }[] = [
    {
      heading: t("footer.colProduct"),
      links: [
        { href: "#features", label: t("footer.linkFeatures") },
        { href: "#large-files", label: t("footer.linkLargeFiles") },
        { href: "#sharex", label: t("footer.linkShareX") },
        { href: "/register", label: t("footer.linkGetStarted") },
      ],
    },
    {
      heading: t("footer.colDocs"),
      links: [
        { href: "https://github.com/Nimfek/Modlight/blob/main/docs/deployment.md", label: t("footer.linkDeployment") },
        { href: "https://github.com/Nimfek/Modlight/blob/main/docs/sharex-setup.md", label: t("footer.linkShareXSetup") },
      ],
    },
    {
      heading: t("footer.colProject"),
      links: [
        { href: "https://github.com/Nimfek/Modlight", label: t("footer.linkGithub") },
        { href: "https://github.com/Nimfek/Modlight/blob/main/LICENSE", label: t("footer.linkLicense") },
        { href: "https://github.com/Nimfek/Modlight/issues", label: t("footer.linkIssues") },
      ],
    },
  ];

  return (
    <footer className="border-border/60 border-t">
      <div className="mx-auto max-w-6xl px-6 py-14">
        <div className="grid gap-10 sm:grid-cols-2 lg:grid-cols-[1.5fr_repeat(3,1fr)]">
          <div>
            <Link href="/" className="flex items-center gap-2.5">
              <BrandTile className="size-8" />
              <span className="text-lg font-semibold tracking-tight">{appName}</span>
            </Link>
            <p className="text-muted-foreground mt-4 max-w-xs text-sm leading-relaxed">
              {t("footer.tagline")}
            </p>
          </div>

          {columns.map((col) => (
            <div key={col.heading}>
              <h3 className="text-foreground text-sm font-semibold">{col.heading}</h3>
              <ul className="mt-4 space-y-3">
                {col.links.map((l) => (
                  <li key={l.label}>
                    <Link
                      href={l.href}
                      className="text-muted-foreground hover:text-foreground text-sm transition-colors"
                    >
                      {l.label}
                    </Link>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>

        <div className="border-border/60 mt-12 flex flex-col items-center justify-between gap-4 border-t pt-6 sm:flex-row">
          <p className="text-muted-foreground text-xs">
            {t("footer.copyright", { year: new Date().getFullYear(), appName })}
          </p>
          <Link
            href="https://github.com/Nimfek/Modlight"
            className="text-muted-foreground hover:text-foreground inline-flex items-center gap-1.5 text-xs transition-colors"
          >
            <GitFork className="size-4" /> {t("footer.sourceCode")}
          </Link>
        </div>
      </div>
    </footer>
  );
}
