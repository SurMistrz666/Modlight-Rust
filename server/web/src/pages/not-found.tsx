// Port of upstream app/not-found.tsx (an async Server Component there).
// @upstream app/not-found.tsx
// Deviation: `async` dropped, getTranslations -> useTranslations.
import Link from "next/link";
import { useTranslations } from "next-intl";
import { FileQuestion } from "lucide-react";
import { BrandTile } from "@/components/auth/brand-mark";
import { Button } from "@/components/ui/button";

/**
 * The most-seen page in the product that nobody designs: every expired link,
 * burned one-time upload, deleted file and mistyped code lands here — usually
 * on a stranger who has never heard of this server. So it explains the three
 * plausible causes instead of shrugging, and offers a way forward for both a
 * visitor and the owner.
 */
export default function NotFound() {
  const t = useTranslations("common");

  return (
    <main className="relative grid min-h-dvh w-full place-items-center overflow-hidden px-6 py-16">
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 -z-10"
        style={{
          backgroundImage:
            "radial-gradient(48rem 32rem at 50% -10%, color-mix(in oklch, var(--primary) 16%, transparent), transparent 70%)",
        }}
      />
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 -z-10 opacity-[0.05] [background-image:linear-gradient(to_right,currentColor_1px,transparent_1px),linear-gradient(to_bottom,currentColor_1px,transparent_1px)] [background-size:32px_32px] [mask-image:radial-gradient(ellipse_at_center,black,transparent_75%)]"
      />

      <div className="flex w-full max-w-lg flex-col items-center gap-6 text-center">
        <Link
          href="/"
          className="text-muted-foreground hover:text-foreground inline-flex items-center gap-2 text-sm font-semibold tracking-tight transition-colors"
        >
          <BrandTile className="size-6" />
          Modlight
        </Link>

        <div className="relative">
          <p
            aria-hidden
            className="from-foreground to-foreground/25 font-heading bg-gradient-to-b bg-clip-text text-[5.5rem] leading-none font-bold tracking-tighter text-transparent tabular-nums select-none sm:text-[7rem]"
          >
            404
          </p>
          <span className="bg-card text-muted-foreground ring-foreground/10 absolute -right-2 -bottom-1 grid size-10 place-items-center rounded-2xl ring-1">
            <FileQuestion className="size-5" />
          </span>
        </div>

        <div className="space-y-2">
          <h1 className="font-heading text-2xl font-semibold tracking-tight text-balance">
            {t("notFound.title")}
          </h1>
          <p className="text-muted-foreground text-sm text-pretty">{t("notFound.desc")}</p>
        </div>

        <ul className="text-muted-foreground/90 grid w-full max-w-sm gap-1.5 text-left text-xs">
          {["deleted", "expired", "typo"].map((reason) => (
            <li
              key={reason}
              className="bg-muted/50 ring-foreground/10 flex items-start gap-2 rounded-lg px-3 py-2 ring-1"
            >
              <span aria-hidden className="bg-muted-foreground/50 mt-1.5 size-1 shrink-0 rounded-full" />
              {t(`notFound.reasons.${reason}`)}
            </li>
          ))}
        </ul>

        <div className="flex flex-col items-center gap-2 sm:flex-row">
          <Button size="lg" asChild>
            <Link href="/">{t("notFound.home")}</Link>
          </Button>
          <Button size="lg" variant="ghost" asChild>
            <Link href="/dashboard">{t("notFound.dashboard")}</Link>
          </Button>
        </div>
      </div>
    </main>
  );
}
