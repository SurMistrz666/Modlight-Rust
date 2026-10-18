// Port of upstream app/(app)/layout.tsx (the signed-in panel shell).
// @upstream app/(app)/layout.tsx
//
// Upstream this is a Server Component: requireUser() (redirect to /login when
// signed out), then folderTree / kindCounts / userStats / getUserPrefs and the
// `ml_sidebar` cookie, shaped into the props below by toTreeNodes(),
// flattenFolders(), backgroundStyle() and avatarUrl(). The server now does all
// of that and sends the shaped props (contract/layouts.ts AppLayoutData); the
// JSX is the same, element for element.
import { AppShell } from "@/components/app-shell";
import { ThemeApplier } from "@/components/theme-applier";
import { TransfersProvider } from "@/components/transfers/TransfersProvider";
import { GlobalDropZone } from "@/components/transfers/GlobalDropZone";
import { TransferDock } from "@/components/transfers/TransferDock";
import { DuplicateReminderSlot } from "@/components/duplicates/DuplicateReminderSlot";
import type { AppLayoutData } from "../../contract/layouts";

export default function AppLayout({ data, children }: { data: AppLayoutData; children: React.ReactNode }) {
  return (
    <>
      <ThemeApplier theme={data.theme} background={data.background} />
      {/*
        The transfers provider wraps the whole panel so an upload outlives the
        page it started on: the queue, its progress and the floating dock all
        hang off one Uppy instance mounted here, not on /dashboard/upload.
      */}
      <TransfersProvider folders={data.transferFolders} defaultFolderId={data.defaultFolderId}>
        <AppShell user={data.user} sidebar={data.sidebar} defaultCollapsed={data.defaultCollapsed}>
          {/*
            Mounted once, here, because the slot renders nothing on almost every
            load: a snooze cookie or a count under both thresholds answers it
            without a query, and the session it needs is already in hand. Per
            page it would be a second session lookup on every route.
          */}
          <DuplicateReminderSlot state={data.duplicateReminder} />
          {children}
        </AppShell>
        <GlobalDropZone />
        <TransferDock />
      </TransfersProvider>
    </>
  );
}
