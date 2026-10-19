"use client";

import { useState } from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { CopyCheck } from "lucide-react";
import { useTranslations } from "next-intl";
import { Button } from "@/components/ui/button";
import { humanFileSize } from "@/lib/format";
import { snoozeCookie } from "./snooze";

/**
 * The reminder: one line, two buttons, no dialog.
 *
 * Deliberately not a modal and deliberately not a badge that sits there
 * forever. It appears when an account has passed one of the two thresholds
 * (`REMINDER_MIN_REDUNDANT` / `REMINDER_MIN_BYTES`), it can be put away for a
 * week, and the page it links to can silence it for good for the sets it is
 * talking about. A reminder that cannot be answered is a nag, and a nag gets
 * scrolled past forever - which costs more than never having shown it.
 *
 * The numbers are the ones the reminder query produced, with every set the
 * owner has waved off already subtracted. What the sentence quotes and what
 * decided to show it are therefore the same number.
 */
export function DuplicateReminder({
  redundant,
  reclaimable,
}: {
  /** Copies beyond the one kept in each set: what cleaning up would remove. */
  redundant: number;
  /** Bytes cleaning up would free. */
  reclaimable: number;
}) {
  const t = useTranslations("profile.duplicates.reminder");
  const pathname = usePathname();
  const [hidden, setHidden] = useState(false);

  // Never speak on the page that answers it. The slot is rendered by the panel
  // shell, which has no idea which route is under it.
  if (hidden || pathname?.startsWith("/dashboard/duplicates")) return null;

  function snooze() {
    // Written from the browser rather than through an action: this is a
    // preference of one browser about one week, and it should not cost a round
    // trip on a click whose entire job is to get out of the way.
    document.cookie = snoozeCookie(Date.now());
    setHidden(true);
  }

  return (
    <div className="border-primary/25 bg-primary/5 mb-5 flex flex-wrap items-center gap-x-3 gap-y-2 rounded-xl border px-3.5 py-2.5">
      <CopyCheck className="text-primary size-4 shrink-0" />
      <p className="text-sm">
        {t("body", { count: redundant, size: humanFileSize(reclaimable) })}
      </p>

      <div className="ml-auto flex items-center gap-1.5">
        <Button asChild size="sm" variant="outline">
          <Link href="/dashboard/duplicates">{t("review")}</Link>
        </Button>
        <Button type="button" size="sm" variant="ghost" onClick={snooze}>
          {t("later")}
        </Button>
      </div>
    </div>
  );
}
