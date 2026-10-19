// Client port of upstream components/landing/site-nav.tsx (an async Server Component there).
// @upstream components/landing/site-nav.tsx
// Deviation: `async` dropped and getTranslations (next-intl/server) replaced by
// useTranslations (next-intl); nothing else differs.
import Link from "next/link";
import { useTranslations } from "next-intl";
import { LayoutDashboard, Star } from "lucide-react";
import { BrandTile } from "@/components/auth/brand-mark";
import { Button } from "@/components/ui/button";
import { ThemeToggle } from "@/components/theme-toggle";

interface SiteNavProps {
  appName: string;
  isAuthed: boolean;
}

export function SiteNav({ appName, isAuthed }: SiteNavProps) {
  const t = useTranslations("landing");
  const links = [
    { href: "#features", label: t("nav.features") },
    { href: "#large-files", label: t("nav.largeFiles") },
    { href: "#sharex", label: t("nav.shareX") },
    {
      href: "https://github.com/Nimfek/Modlight/blob/main/docs/deployment.md",
      label: t("nav.docs"),
    },
  ];
  return (
    <header className="sticky top-0 z-50 w-full">
      <div className="border-border/60 bg-background/70 supports-[backdrop-filter]:bg-background/55 border-b backdrop-blur-xl">
        <nav className="mx-auto flex h-16 max-w-6xl items-center gap-4 px-6">
          <Link href="/" className="flex items-center gap-2.5">
            <BrandTile className="size-8" />
            <span className="text-lg font-semibold tracking-tight">{appName}</span>
          </Link>

          <div className="text-muted-foreground ml-4 hidden items-center gap-1 md:flex">
            {links.map((l) => (
              <Link
                key={l.href}
                href={l.href}
                className="hover:text-foreground rounded-md px-3 py-2 text-sm font-medium transition-colors"
              >
                {l.label}
              </Link>
            ))}
          </div>

          <div className="ml-auto flex items-center gap-1.5">
            <Button asChild variant="ghost" size="icon" aria-label={t("nav.starOnGithub")} className="hidden sm:inline-flex">
              <Link href="https://github.com/Nimfek/Modlight">
                <Star className="size-5" />
              </Link>
            </Button>
            <ThemeToggle />
            {isAuthed ? (
              <Button asChild size="sm" className="ml-1 h-9 px-4">
                <Link href="/dashboard">
                  <LayoutDashboard className="size-4" /> {t("nav.dashboard")}
                </Link>
              </Button>
            ) : (
              <>
                <Button asChild variant="ghost" size="sm" className="h-9 px-3">
                  <Link href="/login">{t("nav.signIn")}</Link>
                </Button>
                <Button asChild size="sm" className="ml-1 h-9 px-4">
                  <Link href="/register">{t("nav.getStarted")}</Link>
                </Button>
              </>
            )}
          </div>
        </nav>
      </div>
    </header>
  );
}
