"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslations } from "next-intl";
import { FolderOpen, UploadCloud } from "lucide-react";
import { cn } from "@/lib/utils";
import { filesFromDrop, isExemptTarget, isExternalFileDrag } from "@/components/transfers/drag";
import { useTransfers } from "@/components/transfers/TransfersProvider";

/**
 * Drag a file off the desktop, drop it anywhere on the panel, and it lands in
 * the folder that is open — the same thing that would happen in a file manager.
 *
 * The listeners sit on `window` rather than on a wrapper element for two
 * reasons: the drop must work over the sidebar and the header as well as the
 * page body, and a wrapper would have to swallow events that the folder tree
 * still needs for its own (internal) drag-to-move.
 */
export function GlobalDropZone() {
  const t = useTranslations("upload");
  const { startFiles, target } = useTransfers();
  const [active, setActive] = useState(false);

  /**
   * dragenter/dragleave fire per element and bubble, so a plain boolean would
   * flicker every time the pointer crosses a child. Counting them does not.
   */
  const depth = useRef(0);
  /** Escape hides the overlay; the drop is still swallowed, never navigated to. */
  const dismissed = useRef(false);

  const reset = useCallback(() => {
    depth.current = 0;
    dismissed.current = false;
    setActive(false);
  }, []);

  useEffect(() => {
    function onDragEnter(e: DragEvent) {
      if (!isExternalFileDrag(e.dataTransfer)) return;
      depth.current += 1;
      if (!dismissed.current) setActive(true);
    }

    function onDragOver(e: DragEvent) {
      if (!isExternalFileDrag(e.dataTransfer)) return;
      if (isExemptTarget(e.target)) return;
      // Without this the browser navigates away to the dropped file, which is
      // the single most destructive default on the platform.
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
      if (!dismissed.current && depth.current === 0) {
        // Some browsers deliver dragover without a matching dragenter when the
        // pointer enters over an iframe or a scrollbar.
        depth.current = 1;
        setActive(true);
      }
    }

    function onDragLeave(e: DragEvent) {
      if (!isExternalFileDrag(e.dataTransfer)) return;
      // relatedTarget is null when the pointer left the window entirely.
      if (!e.relatedTarget) return reset();
      depth.current = Math.max(0, depth.current - 1);
      if (depth.current === 0) setActive(false);
    }

    function onDrop(e: DragEvent) {
      if (!isExternalFileDrag(e.dataTransfer)) return reset();
      if (isExemptTarget(e.target)) return reset();

      e.preventDefault();
      const wasDismissed = dismissed.current;
      const dt = e.dataTransfer;
      reset();
      if (wasDismissed || !dt) return;

      void filesFromDrop(dt).then((files) => {
        if (files.length > 0) void startFiles(files);
      });
    }

    function onKeyDown(e: KeyboardEvent) {
      if (e.key !== "Escape" || depth.current === 0) return;
      dismissed.current = true;
      setActive(false);
    }

    window.addEventListener("dragenter", onDragEnter);
    window.addEventListener("dragover", onDragOver);
    window.addEventListener("dragleave", onDragLeave);
    window.addEventListener("drop", onDrop);
    window.addEventListener("keydown", onKeyDown);
    // A drag that ends anywhere else (Escape at the OS level, a failed drop)
    // never sends us a drop event, so the overlay needs this way out too.
    window.addEventListener("dragend", reset);

    return () => {
      window.removeEventListener("dragenter", onDragEnter);
      window.removeEventListener("dragover", onDragOver);
      window.removeEventListener("dragleave", onDragLeave);
      window.removeEventListener("drop", onDrop);
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("dragend", reset);
    };
  }, [reset, startFiles]);

  if (!active) return null;

  const destination = target.isDefault
    ? target.folderName
      ? t("dropzone.toFolder", { folder: target.folderName })
      : t("dropzone.toLibrary")
    : target.folderId
      ? t("dropzone.toFolder", { folder: target.folderName ?? t("dropzone.thisFolder") })
      : t("dropzone.toUnsorted");

  return (
    <div
      // Purely an announcement: every drag event is handled on `window`, so the
      // overlay must never become a target itself.
      className={cn(
        "animate-in fade-in pointer-events-none fixed inset-0 z-[100] duration-150",
        "bg-background/80 grid place-items-center p-6 backdrop-blur-sm",
      )}
      role="status"
      aria-live="polite"
    >
      <div className="ring-primary/40 bg-card/90 animate-in zoom-in-95 flex w-full max-w-md flex-col items-center gap-4 rounded-2xl border-2 border-dashed border-transparent p-8 text-center shadow-lg ring-2 duration-150">
        <span className="bg-primary/10 text-primary ring-primary/20 grid size-14 place-items-center rounded-2xl ring-1">
          <UploadCloud className="size-7" />
        </span>
        <div className="space-y-1.5">
          <p className="text-lg font-semibold tracking-tight">{t("dropzone.title")}</p>
          <p className="text-muted-foreground flex items-center justify-center gap-1.5 text-sm">
            <FolderOpen className="size-4 shrink-0" />
            <span className="min-w-0 truncate">{destination}</span>
          </p>
        </div>
        <p className="text-muted-foreground/80 text-xs">{t("dropzone.hint")}</p>
      </div>
    </div>
  );
}
