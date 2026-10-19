// Client port of upstream components/landing/features.tsx (an async Server Component there).
// @upstream components/landing/features.tsx
// Deviation: `async` dropped and getTranslations (next-intl/server) replaced by
// useTranslations (next-intl); nothing else differs.
import {
  UploadCloud,
  MousePointerClick,
  MessageSquare,
  FolderTree,
  Timer,
  Users,
  Moon,
  Cloud,
  GitFork,
  type LucideIcon,
} from "lucide-react";
import { useTranslations } from "next-intl";
import { cn } from "@/lib/utils";

interface Feature {
  icon: LucideIcon;
  title: string;
  description: string;
  span?: boolean;
}

export function Features() {
  const t = useTranslations("landing");

  const features: Feature[] = [
    {
      icon: UploadCloud,
      title: t("features.resumableTitle"),
      description: t("features.resumableDesc"),
      span: true,
    },
    {
      icon: MousePointerClick,
      title: t("features.shareXTitle"),
      description: t("features.shareXDesc"),
    },
    {
      icon: MessageSquare,
      title: t("features.embedsTitle"),
      description: t("features.embedsDesc"),
    },
    {
      icon: FolderTree,
      title: t("features.albumsTitle"),
      description: t("features.albumsDesc"),
    },
    {
      icon: Timer,
      title: t("features.expiringTitle"),
      description: t("features.expiringDesc"),
    },
    {
      icon: Users,
      title: t("features.quotasTitle"),
      description: t("features.quotasDesc"),
    },
    {
      icon: Cloud,
      title: t("features.storageTitle"),
      description: t("features.storageDesc"),
    },
    {
      icon: Moon,
      title: t("features.darkModeTitle"),
      description: t("features.darkModeDesc"),
    },
    {
      icon: GitFork,
      title: t("features.openSourceTitle"),
      description: t("features.openSourceDesc"),
    },
  ];

  return (
    <section id="features" className="mx-auto max-w-6xl px-6 py-20 sm:py-28">
      <div className="mx-auto max-w-2xl text-center">
        <p className="text-primary text-sm font-semibold tracking-wide uppercase">
          {t("features.eyebrow")}
        </p>
        <h2 className="mt-3 text-3xl font-semibold tracking-tight text-balance sm:text-4xl">
          {t("features.heading")}
        </h2>
        <p className="text-muted-foreground mt-4 text-lg text-balance">
          {t("features.subhead")}
        </p>
      </div>

      <div className="mt-14 grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {features.map((f) => (
          <article
            key={f.title}
            className={cn(
              "group border-border/70 bg-card/60 hover:border-primary/40 relative overflow-hidden rounded-xl border p-6 transition-colors",
              f.span && "sm:col-span-2 lg:col-span-1",
            )}
          >
            <div
              aria-hidden
              className="from-primary/10 pointer-events-none absolute inset-0 bg-gradient-to-br to-transparent opacity-0 transition-opacity group-hover:opacity-100"
            />
            <div className="bg-primary/10 text-primary ring-primary/20 relative grid size-11 place-items-center rounded-lg ring-1">
              <f.icon className="size-5" />
            </div>
            <h3 className="relative mt-5 text-base font-semibold tracking-tight">
              {f.title}
            </h3>
            <p className="text-muted-foreground relative mt-2 text-sm leading-relaxed">
              {f.description}
            </p>
          </article>
        ))}
      </div>
    </section>
  );
}
