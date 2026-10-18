"use client";

import * as React from "react";
import { useTranslations } from "next-intl";
import { Lock, UploadCloud } from "lucide-react";
import { cn } from "@/lib/utils";
import { Uploader } from "@/components/upload/Uploader";
import { EncryptedUploader } from "@/components/upload/EncryptedUploader";

type Mode = "standard" | "encrypted";

/**
 * The opt-in control for end-to-end encryption, and the reason it is a pair of
 * modes rather than a checkbox on the dropzone.
 *
 * The two uploaders are genuinely different machines, not one machine with a
 * flag: the standard one belongs to the app shell so a transfer survives
 * navigation and the floating dock can show it, while an encrypted transfer has
 * to live and die with this page because its key is only ever in this tab's
 * memory. A checkbox would promise a single queue whose halves behave
 * differently, and the difference would only surface at the moment it hurt -
 * navigating away and finding half the files gone.
 *
 * So the choice is made BEFORE any file is picked, the consequences are on
 * screen next to it, and switching resets nothing that is already in flight
 * because the two never share a queue.
 */
export function UploadModes() {
  const t = useTranslations("upload.modes");
  const [mode, setMode] = React.useState<Mode>("standard");

  return (
    <div className="space-y-5">
      <div
        role="tablist"
        aria-label={t("label")}
        className="ring-foreground/10 bg-card grid grid-cols-1 gap-1 rounded-xl p-1 ring-1 sm:grid-cols-2"
      >
        <ModeTab
          selected={mode === "standard"}
          onSelect={() => setMode("standard")}
          icon={UploadCloud}
          title={t("standard.title")}
          hint={t("standard.hint")}
        />
        <ModeTab
          selected={mode === "encrypted"}
          onSelect={() => setMode("encrypted")}
          icon={Lock}
          title={t("encrypted.title")}
          hint={t("encrypted.hint")}
        />
      </div>

      {/* Unmounted rather than hidden. A hidden Uppy Dashboard is still a drop
          target and still holds a queue, so both would compete for the same
          drop and the invisible one could win. */}
      {mode === "standard" ? <Uploader /> : <EncryptedUploader />}
    </div>
  );
}

function ModeTab({
  selected,
  onSelect,
  icon: Icon,
  title,
  hint,
}: {
  selected: boolean;
  onSelect: () => void;
  icon: React.ComponentType<{ className?: string }>;
  title: string;
  hint: string;
}) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={selected}
      onClick={onSelect}
      className={cn(
        "flex items-start gap-3 rounded-lg px-3 py-2.5 text-left transition-colors",
        selected
          ? "bg-primary/10 ring-primary/25 ring-1"
          : "hover:bg-accent/60 text-muted-foreground",
      )}
    >
      <span
        className={cn(
          "mt-0.5 grid size-8 shrink-0 place-items-center rounded-lg",
          selected ? "bg-primary/15 text-primary" : "bg-muted",
        )}
      >
        <Icon className="size-4" />
      </span>
      <span className="min-w-0">
        <span
          className={cn("block text-sm font-medium", selected ? "text-foreground" : undefined)}
        >
          {title}
        </span>
        <span className="text-muted-foreground mt-0.5 block text-xs leading-relaxed">{hint}</span>
      </span>
    </button>
  );
}

export default UploadModes;
