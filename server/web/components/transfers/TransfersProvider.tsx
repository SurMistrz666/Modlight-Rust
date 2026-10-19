"use client";

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { useTranslations } from "next-intl";
import { toast } from "sonner";
import type { UppyEventMap, UppyFile } from "@uppy/core";
import type { TusBody } from "@uppy/tus";
import { ROOT_FOLDER_META, FOLDER_META_KEY, MEDIA_URL_HEADER } from "@/components/upload/protocol";
import { copyText } from "@/components/transfers/clipboard";
import { preflightDuplicates } from "@/components/upload/duplicate-preflight";
import {
  DuplicateDialog,
  type DuplicateChoice,
  type DuplicatePrompt,
} from "@/components/upload/DuplicateDialog";
import { RateMeter } from "@/lib/transfer-rate";
import type { ModlightMeta, ModlightUppy } from "@/components/upload/uppy-client";

/** Uppy's own event signatures, narrowed to the two generics we use. */
type Events = UppyEventMap<ModlightMeta, TusBody>;

/* -------------------------------------------------------------------------- */
/* Types                                                                       */
/* -------------------------------------------------------------------------- */

export type TransferStatus = "waiting" | "uploading" | "paused" | "done" | "error";

export interface TransferItem {
  id: string;
  name: string;
  /** Total bytes, or 0 when the browser could not tell us. */
  size: number;
  uploaded: number;
  percent: number;
  status: TransferStatus;
  error?: string;
  /** Public URL, available once the server has finalized the upload. */
  url?: string;
}

export interface TransferStats {
  total: number;
  done: number;
  failed: number;
  active: number;
  totalBytes: number;
  uploadedBytes: number;
  percent: number;
  /** Smoothed transfer rate in bytes per second; 0 while nothing moves. */
  speed: number;
  /**
   * Mean rate since the batch started, in bytes per second.
   *
   * Kept alongside the live reading because it cannot be distorted by gaps in
   * sampling (a backgrounded tab), which makes it the number to trust when the
   * two disagree.
   */
  average: number;
  /** Seconds remaining, or null when it cannot be estimated yet. */
  eta: number | null;
  /** A batch is in progress (files may still be individually paused). */
  running: boolean;
  /** Everything still in flight is paused. */
  paused: boolean;
}

/** Where a new upload lands, resolved from the panel the user is looking at. */
export interface UploadTarget {
  /** Folder id, or null for "no folder". Meaningless while `isDefault`. */
  folderId: string | null;
  /** Display name, or null for the loose pile / an unknown id. */
  folderName: string | null;
  /** Nothing in the URL pins a folder, so the account default applies. */
  isDefault: boolean;
}

export interface TransferFolder {
  id: string;
  name: string;
}

interface TransfersContextValue {
  /** Null until the engine has been lazily loaded. */
  uppy: ModlightUppy | null;
  /** Load (once) and return the shared engine. */
  ensure: () => Promise<ModlightUppy>;
  /** Add files and start uploading them into the currently open folder. */
  startFiles: (files: File[]) => Promise<void>;
  target: UploadTarget;
  items: TransferItem[];
  stats: TransferStats;
  minimized: boolean;
  setMinimized: (v: boolean) => void;
  /**
   * Hide the dock while another surface already shows the same queue (the Uppy
   * Dashboard on /dashboard/upload). Returns nothing; call with `false` on unmount.
   */
  setDockHidden: (v: boolean) => void;
  dockHidden: boolean;
  pauseAll: () => void;
  resumeAll: () => void;
  toggleFile: (id: string) => void;
  cancelFile: (id: string) => void;
  retryFile: (id: string) => void;
  retryAll: () => void;
  clearFinished: () => void;
  cancelAll: () => void;
}

const TransfersContext = createContext<TransfersContextValue | null>(null);

export function useTransfers(): TransfersContextValue {
  const ctx = useContext(TransfersContext);
  if (!ctx) throw new Error("useTransfers must be used inside <TransfersProvider>");
  return ctx;
}

/* -------------------------------------------------------------------------- */
/* Snapshot                                                                    */
/* -------------------------------------------------------------------------- */

const EMPTY_STATS: TransferStats = {
  total: 0,
  done: 0,
  failed: 0,
  active: 0,
  totalBytes: 0,
  uploadedBytes: 0,
  percent: 0,
  speed: 0,
  average: 0,
  eta: null,
  running: false,
  paused: false,
};

/** Routes where `?folder=` names the folder the user is looking at. */
const FOLDER_SCOPED = ["/dashboard/folders", "/dashboard/upload"];

/**
 * Bytes a file has actually put on the wire.
 *
 * A finished file occasionally reports slightly fewer bytes than it has (the
 * last chunk lands together with the response), so a completed file counts as
 * its full size. Shared by the item list and the byte totals, which must never
 * disagree: one drives the progress bar, the other the speed readout.
 */
function sentBytes(size: number, uploaded: number, complete: boolean): number {
  return complete ? size : Math.min(uploaded, size || uploaded);
}

/**
 * Byte totals for the whole queue.
 *
 * Deliberately cheap and synchronous: this runs on every Uppy state update,
 * OUTSIDE the animation frame that batches rendering, because rAF is suspended
 * in a hidden tab and the speed meter must keep counting while the user is
 * looking at something else.
 */
function readBytes(uppy: ModlightUppy): { uploaded: number; total: number } {
  let uploaded = 0;
  let total = 0;
  for (const file of uppy.getFiles()) {
    const progress = file.progress;
    const size = progress.bytesTotal ?? file.size ?? 0;
    const sent = typeof progress.bytesUploaded === "number" ? progress.bytesUploaded : 0;
    total += size;
    uploaded += sentBytes(size, sent, progress.uploadComplete === true);
  }
  return { uploaded, total };
}

function readItems(uppy: ModlightUppy, urls: Map<string, string>): TransferItem[] {
  return uppy.getFiles().map((file) => {
    const progress = file.progress;
    const size = progress.bytesTotal ?? file.size ?? 0;
    const uploaded = typeof progress.bytesUploaded === "number" ? progress.bytesUploaded : 0;
    const complete = progress.uploadComplete === true;

    const status: TransferStatus = file.error
      ? "error"
      : complete
        ? "done"
        : file.isPaused
          ? "paused"
          : progress.uploadStarted
            ? "uploading"
            : "waiting";

    const percent =
      status === "done"
        ? 100
        : typeof progress.percentage === "number"
          ? Math.min(100, Math.max(0, progress.percentage))
          : size > 0
            ? Math.min(100, (uploaded / size) * 100)
            : 0;

    return {
      id: file.id,
      name: file.name || file.meta?.name || "file",
      size,
      uploaded: sentBytes(size, uploaded, status === "done"),
      percent,
      status,
      error: file.error ?? undefined,
      url: urls.get(file.id),
    };
  });
}

function summarize(items: TransferItem[], running: boolean): TransferStats {
  let totalBytes = 0;
  let uploadedBytes = 0;
  let done = 0;
  let failed = 0;
  let active = 0;
  let pending = 0;
  let pausedPending = 0;

  for (const item of items) {
    totalBytes += item.size;
    uploadedBytes += item.uploaded;
    if (item.status === "done") done++;
    else if (item.status === "error") failed++;
    else {
      pending++;
      if (item.status === "uploading") active++;
      if (item.status === "paused") pausedPending++;
    }
  }

  return {
    total: items.length,
    done,
    failed,
    active,
    totalBytes,
    uploadedBytes,
    percent: totalBytes > 0 ? Math.min(100, (uploadedBytes / totalBytes) * 100) : 0,
    speed: 0,
    average: 0,
    eta: null,
    // `pending` counts too: Uppy registers a batch a tick after the files are
    // added, and without this the dock would flash "finished" between the drop
    // and the first PATCH.
    running: running || pending > 0,
    paused: pending > 0 && pausedPending === pending,
  };
}

/* -------------------------------------------------------------------------- */
/* Provider                                                                    */
/* -------------------------------------------------------------------------- */

export function TransfersProvider({
  folders,
  defaultFolderId,
  chunkSize,
  children,
}: {
  /** Flat folder list, only used to name the drop target in the UI. */
  folders: TransferFolder[];
  /** Account default, applied whenever the URL does not pin a folder. */
  defaultFolderId: string | null;
  chunkSize?: number;
  children: React.ReactNode;
}) {
  const t = useTranslations("upload");
  const router = useRouter();
  const pathname = usePathname();
  const params = useSearchParams();

  const [uppy, setUppy] = useState<ModlightUppy | null>(null);
  const [items, setItems] = useState<TransferItem[]>([]);
  const [running, setRunning] = useState(false);
  const [rate, setRate] = useState<{ speed: number; average: number; eta: number | null }>({
    speed: 0,
    average: 0,
    eta: null,
  });
  const [minimized, setMinimized] = useState(false);
  const [dockHidden, setDockHidden] = useState(false);

  const instance = useRef<ModlightUppy | null>(null);
  const loading = useRef<Promise<ModlightUppy> | null>(null);
  const urls = useRef(new Map<string, string>());
  const frame = useRef<number | null>(null);
  /** Latest byte totals, sampled by the rate ticker without re-rendering. */
  const bytes = useRef({ uploaded: 0, total: 0 });

  /* ---- engine ----------------------------------------------------------- */

  const ensure = useCallback(async (): Promise<ModlightUppy> => {
    if (instance.current) return instance.current;
    if (!loading.current) {
      // Dynamic so Uppy and tus-js-client stay out of the bundle of every panel
      // page that never uploads anything. The instance is NOT destroyed on
      // unmount: transfers are meant to outlive any single view, and tearing it
      // down would also break React's double-invoked effects in development.
      loading.current = import("@/components/upload/uppy-client").then((mod) => {
        const created = mod.createUppy(chunkSize);
        instance.current = created;
        setUppy(created);
        return created;
      });
    }
    return loading.current;
  }, [chunkSize]);

  /* ---- state mirroring -------------------------------------------------- */

  useEffect(() => {
    if (!uppy) return;

    const sync = () => {
      // Byte totals are updated here, synchronously, and NOT inside the frame
      // below. Browsers suspend requestAnimationFrame in a hidden tab, so doing
      // the accounting there froze the counter the moment the user switched away
      // and produced a fake burst of speed when they came back. The transfer
      // never stopped; only the measurement did.
      bytes.current = readBytes(uppy);

      if (frame.current != null) return;
      // tus reports progress many times a second per chunk; coalescing the
      // RENDER into one frame keeps a 20-file queue from re-rendering the dock
      // into the ground. Skipping frames while hidden is fine: nobody is looking.
      frame.current = requestAnimationFrame(() => {
        frame.current = null;
        setItems(readItems(uppy, urls.current));
        setRunning(Object.keys(uppy.getState().currentUploads).length > 0);
      });
    };

    const onSuccess: Events["upload-success"] = (file, response) => {
      if (!file) return;
      let url: string | null = null;
      try {
        // tus has no response body, so finalize hands the link back as a header
        // on the final PATCH and we read it off the raw XHR.
        url = response?.body?.xhr?.getResponseHeader(MEDIA_URL_HEADER) ?? null;
      } catch {
        url = null;
      }
      if (url) urls.current.set(file.id, url);
      sync();
    };

    const onComplete: Events["complete"] = (result) => {
      sync();
      // The library the user is looking at is now stale: a file dropped into
      // the open folder should appear in it without a manual reload.
      router.refresh();

      const ok = result.successful ?? [];
      const bad = result.failed ?? [];

      if (ok.length === 1) {
        const url = urls.current.get(ok[0].id);
        if (url) {
          void copyText(url).then((copied) => {
            toast.success(t("toast.one"), { description: copied ? t("toast.copied") : url });
          });
        } else {
          toast.success(t("toast.one"));
        }
      } else if (ok.length > 1) {
        toast.success(t("toast.many", { count: ok.length }));
      }

      if (bad.length > 0) {
        toast.error(t("toast.failed", { count: bad.length }), {
          description: t("toast.failedHint"),
        });
      }
    };

    uppy.on("state-update", sync);
    uppy.on("upload-success", onSuccess);
    uppy.on("complete", onComplete);
    sync();

    return () => {
      uppy.off("state-update", sync);
      uppy.off("upload-success", onSuccess);
      uppy.off("complete", onComplete);
      if (frame.current != null) {
        cancelAnimationFrame(frame.current);
        frame.current = null;
      }
    };
    // `t` is memoized per locale by next-intl, so this subscription is stable
    // in practice; listing it keeps the toasts honest if that ever changes.
  }, [uppy, router, t]);

  /* ---- duplicate preflight ---------------------------------------------- */

  /**
   * "You already have this file" - asked before the first byte leaves.
   *
   * Hooked in as an Uppy PRE-PROCESSOR rather than into `startFiles`, and that
   * is the only placement that works: files reach the queue from two directions
   * (the app-wide drop zone, which calls `startFiles`, and the Dashboard's own
   * picker on /dashboard/upload, which calls `uppy.upload()` itself), and a
   * check wired into one of them would silently not exist in the other. A
   * pre-processor sits on the single path both must pass through, runs before
   * the uploader step, and - because `#runUpload` re-reads the batch after every
   * step - may remove files from the batch it was handed.
   *
   * Uppy's `onBeforeUpload` was the obvious alternative and cannot be used: it
   * is synchronous, and this needs a hash, a request and a human.
   */
  const [prompt, setPrompt] = useState<DuplicatePrompt | null>(null);
  /** File ids already checked, so a retry does not re-hash or re-ask. */
  const checkedIds = useRef(new Set<string>());
  /** Resolver for the question on screen, so nothing can be left waiting. */
  const awaitingAnswer = useRef<((choice: DuplicateChoice) => void) | null>(null);

  const answer = useCallback((choice: DuplicateChoice) => {
    const resolve = awaitingAnswer.current;
    awaitingAnswer.current = null;
    setPrompt(null);
    resolve?.(choice);
  }, []);

  useEffect(() => {
    // An unanswerable question must never leave an upload waiting on a promise
    // that can no longer be resolved. Unmounting means the panel is gone, and
    // the user did ask for an upload, so that is what the batch becomes.
    return () => {
      const resolve = awaitingAnswer.current;
      awaitingAnswer.current = null;
      resolve?.("all");
    };
  }, []);

  useEffect(() => {
    if (!uppy) return;

    // Forgetting a removed file means re-adding it asks again, which is what
    // somebody who just took it out of the queue would expect.
    const forget = (file?: { id: string }) => {
      if (file) checkedIds.current.delete(file.id);
    };
    uppy.on("file-removed", forget);

    const preprocess = async (fileIDs: string[]) => {
      const fresh = uppy
        .getFilesByIds(fileIDs)
        .filter((file): file is UppyFile<ModlightMeta, TusBody> =>
          Boolean(file) && !checkedIds.current.has(file.id),
        );
      if (fresh.length === 0) return;
      for (const file of fresh) checkedIds.current.add(file.id);

      // Uppy's own UI shows a pre-processing state from these. Emitted in a
      // try/finally pair because a file left "pre-processing" forever is a
      // frozen row in the Dashboard, which looks exactly like a hung upload.
      for (const file of fresh) {
        uppy.emit("preprocess-progress", file, {
          mode: "indeterminate",
          message: t("duplicates.checking"),
        });
      }

      let outcome: Awaited<ReturnType<typeof preflightDuplicates>> = {
        hits: [],
        checked: 0,
        unchecked: 0,
      };
      try {
        outcome = await preflightDuplicates(
          fresh.flatMap((file) =>
            // `UppyFile.data` is only a Blob for a local file; a Companion-style
            // remote one carries a descriptor with no bytes in it. We install no
            // such plugin, but a file whose bytes are not in the browser cannot
            // be hashed in the browser, and flatMap is how that stays a fact
            // rather than a cast.
            file.data instanceof Blob
              ? [
                  {
                    id: file.id,
                    name: file.name || file.meta?.name || "file",
                    size: file.size ?? 0,
                    data: file.data,
                  },
                ]
              : [],
          ),
        );
      } finally {
        for (const file of fresh) uppy.emit("preprocess-complete", file);
      }

      if (outcome.hits.length === 0) return;

      const choice = await new Promise<DuplicateChoice>((resolve) => {
        awaitingAnswer.current = resolve;
        setPrompt({
          hits: outcome.hits,
          total: fresh.length,
          uncompared: outcome.unchecked,
          resolve: answer,
        });
      });
      if (choice !== "skip") return;

      for (const hit of outcome.hits) {
        uppy.removeFile(hit.id);
        // `file-removed` fires and clears the memo, which is right: a file taken
        // out here was never uploaded, so a later re-drop deserves the question
        // again rather than a silent second copy.
      }
      toast.success(t("duplicates.skipped", { count: outcome.hits.length }));
    };

    uppy.addPreProcessor(preprocess);

    return () => {
      uppy.off("file-removed", forget);
      uppy.removePreProcessor(preprocess);
    };
  }, [uppy, answer, t]);

  /* ---- speed / ETA ------------------------------------------------------ */

  useEffect(() => {
    // Nothing to sample while idle. The reading is zeroed where it is consumed
    // rather than reset here, so this effect never writes state on its way out.
    if (!running) return;

    // performance.now() rather than Date.now(): it is monotonic, so an NTP step
    // or a laptop waking from sleep cannot hand the meter a negative interval.
    const meter = new RateMeter();
    const tick = () => {
      meter.sample(bytes.current.uploaded, performance.now());
      const left = Math.max(0, bytes.current.total - bytes.current.uploaded);
      setRate({ speed: meter.current, average: meter.average, eta: meter.eta(left) });
    };
    tick();

    const id = window.setInterval(tick, 1000);
    // A hidden tab throttles setInterval to roughly one tick a minute. The
    // window meter handles that on its own (it divides by the time that really
    // passed), and sampling once more the instant the tab comes back gives it a
    // fresh pair to work from rather than a minute-old one.
    const onVisibility = () => {
      if (!document.hidden) tick();
    };
    document.addEventListener("visibilitychange", onVisibility);

    return () => {
      window.clearInterval(id);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [running]);

  /* ---- auto-collapse ---------------------------------------------------- */

  const wasRunning = useRef(false);
  useEffect(() => {
    if (running && !wasRunning.current) setMinimized(false);
    // "Finished" folds the dock down to its pill, which is the point: the queue
    // is done, the links stay one click away.
    if (!running && wasRunning.current) setMinimized(true);
    wasRunning.current = running;
  }, [running]);

  /* ---- leaving mid-transfer -------------------------------------------- */

  useEffect(() => {
    if (!running) return;
    const warn = (e: BeforeUnloadEvent) => e.preventDefault();
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, [running]);

  /* ---- drop target ------------------------------------------------------ */

  const folderNames = useMemo(
    () => new Map(folders.map((f) => [f.id, f.name] as const)),
    [folders],
  );

  const target = useMemo<UploadTarget>(() => {
    const scoped = pathname === "/dashboard" || FOLDER_SCOPED.some((p) => pathname.startsWith(p));
    const raw = scoped ? params.get("folder") : null;

    if (raw === null || raw === "") {
      return {
        folderId: defaultFolderId,
        folderName: defaultFolderId ? (folderNames.get(defaultFolderId) ?? null) : null,
        isDefault: true,
      };
    }
    // "none" is the Unsorted view: what you see there is the loose pile, so a
    // drop belongs in the loose pile too, not in the account default.
    if (raw === "none") return { folderId: null, folderName: null, isDefault: false };
    return { folderId: raw, folderName: folderNames.get(raw) ?? null, isDefault: false };
  }, [pathname, params, defaultFolderId, folderNames]);

  /* ---- actions ---------------------------------------------------------- */

  const startFiles = useCallback(
    async (files: File[]) => {
      if (files.length === 0) return;
      const engine = await ensure();

      // Absent metadata means "use the account default", which is exactly what
      // the server does for ShareX. Only an explicit choice is transmitted.
      const folderMeta = target.isDefault ? undefined : (target.folderId ?? ROOT_FOLDER_META);

      let added = 0;
      let rejected = 0;
      for (const file of files) {
        try {
          engine.addFile({
            name: file.name,
            type: file.type,
            data: file,
            meta: folderMeta ? { [FOLDER_META_KEY]: folderMeta } : {},
          });
          added++;
        } catch {
          // Duplicates and restriction failures both land here; Uppy has
          // already surfaced its own reason, so just keep count.
          rejected++;
        }
      }

      if (added === 0) {
        if (rejected > 0) toast.error(t("dock.noneAdded"));
        return;
      }

      setMinimized(false);
      void engine.upload();
    },
    [ensure, t, target],
  );

  // Every control below reads the engine from state rather than the ref: by the
  // time a button exists there is an instance, and state keeps the callbacks
  // honest about when that became true.
  const pauseAll = useCallback(() => uppy?.pauseAll(), [uppy]);
  const resumeAll = useCallback(() => uppy?.resumeAll(), [uppy]);
  const retryAll = useCallback(() => void uppy?.retryAll(), [uppy]);
  const cancelAll = useCallback(() => {
    uppy?.cancelAll();
    urls.current.clear();
  }, [uppy]);

  const toggleFile = useCallback((id: string) => uppy?.pauseResume(id), [uppy]);
  const cancelFile = useCallback(
    (id: string) => {
      uppy?.removeFile(id);
      urls.current.delete(id);
    },
    [uppy],
  );
  const retryFile = useCallback((id: string) => void uppy?.retryUpload(id), [uppy]);

  const clearFinished = useCallback(() => {
    if (!uppy) return;
    const settled = readItems(uppy, urls.current).filter(
      (item) => item.status === "done" || item.status === "error",
    );
    for (const item of settled) {
      uppy.removeFile(item.id);
      urls.current.delete(item.id);
    }
  }, [uppy]);

  /* ---- value ------------------------------------------------------------ */

  const stats = useMemo<TransferStats>(() => {
    if (items.length === 0) return EMPTY_STATS;
    const base = summarize(items, running);
    // The sampler stops ticking when a batch ends, so the last reading is
    // dropped here instead of being written back into state from an effect.
    return running ? { ...base, speed: rate.speed, average: rate.average, eta: rate.eta } : base;
  }, [items, running, rate]);

  const value = useMemo<TransfersContextValue>(
    () => ({
      uppy,
      ensure,
      startFiles,
      target,
      items,
      stats,
      minimized,
      setMinimized,
      dockHidden,
      setDockHidden,
      pauseAll,
      resumeAll,
      toggleFile,
      cancelFile,
      retryFile,
      retryAll,
      clearFinished,
      cancelAll,
    }),
    [
      uppy,
      ensure,
      startFiles,
      target,
      items,
      stats,
      minimized,
      dockHidden,
      pauseAll,
      resumeAll,
      toggleFile,
      cancelFile,
      retryFile,
      retryAll,
      clearFinished,
      cancelAll,
    ],
  );

  return (
    <TransfersContext.Provider value={value}>
      {children}
      <DuplicateDialog prompt={prompt} />
    </TransfersContext.Provider>
  );
}
