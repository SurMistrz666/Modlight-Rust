"use client";

import Link from "next/link";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useState, useTransition } from "react";
import { useTranslations } from "next-intl";
import {
  AlertTriangle,
  Archive,
  ArrowRight,
  ChevronRight,
  ChevronsUpDown,
  Copy,
  CopyCheck,
  ExternalLink,
  FileText,
  Film,
  Folders,
  HardDrive,
  Image as ImageIcon,
  KeyRound,
  Layers,
  Link2,
  ListTree,
  Loader2,
  LogOut,
  Menu,
  Music,
  PanelLeftClose,
  PanelLeftOpen,
  ScrollText,
  ShieldCheck,
  Trash2,
  UploadCloud,
  UserRound,
} from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { humanFileSize } from "@/lib/format";
import { BrandTile } from "@/components/auth/brand-mark";
import { Button } from "@/components/ui/button";
import { Sheet, SheetContent, SheetTrigger, SheetTitle } from "@/components/ui/sheet";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import { ThemeToggle } from "@/components/theme-toggle";
import { signOut } from "@/lib/auth-client";
import { emptyTrash } from "@/app/actions/uploads";
import {
  FolderTree,
  RowMenu,
  buildDashboardHref,
  openRowMenuOnKey,
  type FolderTreeNode,
} from "@/components/dashboard/FolderTree";

export interface ShellUser {
  name?: string | null;
  email: string;
  /**
   * Resolved avatar URL, or null for the initials circle.
   *
   * Already a URL by the time it arrives: `user.image` holds a storage key, and
   * turning one into `/avatar/{tag}` needs the account's tag, which the layout
   * has and a client component should not have to fetch.
   */
  image?: string | null;
  role?: string | null;
  userCode?: string | null;
}

/**
 * Everything the sidebar needs to describe the library. Optional so the shell
 * still renders (as plain navigation) if a route can't afford the queries.
 */
export interface ShellSidebarData {
  folders: FolderTreeNode[];
  /** Per-kind file counts, keyed by the FileKind strings from lib/data/uploads. */
  counts: Record<string, number>;
  totalFiles: number;
  unsortedCount: number;
  storage: { used: number; max: number };
}

/* -------------------------------------------------------------------------- */
/* Navigation model                                                            */
/* -------------------------------------------------------------------------- */

type LinkItem = {
  href: string;
  labelKey: string;
  icon: typeof UploadCloud;
};

/**
 * Smart views over the whole library. These are URL filters, not routes — the
 * dashboard reads `?kind=` and the sidebar just links to it, so a view and a
 * filter chip are always the same thing.
 */
const KIND_VIEWS = [
  { kind: "image", icon: ImageIcon },
  { kind: "video", icon: Film },
  { kind: "audio", icon: Music },
  { kind: "document", icon: FileText },
  { kind: "archive", icon: Archive },
  { kind: "paste", icon: ScrollText },
] as const;

const CREATE_ITEMS: LinkItem[] = [
  { href: "/dashboard/upload", labelKey: "upload", icon: UploadCloud },
  { href: "/dashboard/paste", labelKey: "newPaste", icon: ScrollText },
  { href: "/dashboard/links", labelKey: "shortLinks", icon: Link2 },
];

const ACCOUNT_ITEMS: LinkItem[] = [
  { href: "/dashboard/tokens", labelKey: "sharexApi", icon: KeyRound },
  { href: "/dashboard/profile", labelKey: "profile", icon: UserRound },
];

/**
 * Route → nav label, for the mobile header. Longest prefix first: the phone
 * header is the only place that says where you are, since the rail is behind
 * a button there.
 */
const TITLE_ROUTES: { prefix: string; labelKey: string }[] = [
  { prefix: "/dashboard/folders", labelKey: "folders" },
  { prefix: "/dashboard/trash", labelKey: "trash" },
  { prefix: "/dashboard/duplicates", labelKey: "duplicates" },
  { prefix: "/dashboard/upload", labelKey: "upload" },
  { prefix: "/dashboard/paste", labelKey: "newPaste" },
  { prefix: "/dashboard/links", labelKey: "shortLinks" },
  { prefix: "/dashboard/tokens", labelKey: "sharexApi" },
  { prefix: "/dashboard/profile", labelKey: "profile" },
  { prefix: "/dashboard/admin", labelKey: "admin" },
  { prefix: "/dashboard", labelKey: "files" },
];

const SIDEBAR_COOKIE = "ml_sidebar";
const SECTION_STORAGE_PREFIX = "ml_nav_section_";

/* -------------------------------------------------------------------------- */
/* Section collapse                                                            */
/* -------------------------------------------------------------------------- */

export interface SectionState {
  open: boolean;
  toggle: () => void;
}

/**
 * One rail section's expanded/collapsed state, remembered per section.
 *
 * Starts expanded so the server markup and the first client paint agree; the
 * stored preference is applied in an effect right after hydration. Lifted into
 * the shell so the desktop rail and the mobile drawer never disagree.
 */
function useSectionCollapse(id: string): SectionState {
  const [open, setOpen] = useState(true);

  useEffect(() => {
    try {
      if (window.localStorage.getItem(SECTION_STORAGE_PREFIX + id) === "collapsed") {
        setOpen(false);
      }
    } catch {
      // Private mode — the section just stays expanded.
    }
  }, [id]);

  const toggle = useCallback(() => {
    setOpen((prev) => {
      const next = !prev;
      try {
        window.localStorage.setItem(SECTION_STORAGE_PREFIX + id, next ? "expanded" : "collapsed");
      } catch {
        // Ignore — collapsing still works for this session.
      }
      return next;
    });
  }, [id]);

  return { open, toggle };
}

/* -------------------------------------------------------------------------- */
/* Primitives                                                                  */
/* -------------------------------------------------------------------------- */

function SectionLabel({ children, collapsed }: { children: React.ReactNode; collapsed: boolean }) {
  if (collapsed) {
    return <div aria-hidden className="bg-foreground/10 mx-auto my-2 h-px w-6" />;
  }
  return (
    <p className="text-muted-foreground/70 px-3 pb-1 text-[0.6875rem] font-semibold tracking-[0.08em] uppercase">
      {children}
    </p>
  );
}

/**
 * A collapsible rail section.
 *
 * The header *is* the control — the same gesture the folder tree already uses,
 * so all four sections behave identically. `<button>` gives Enter/Space and
 * focus for free; `aria-expanded` + `aria-controls` describe it to a reader.
 */
function NavSection({
  id,
  title,
  count,
  collapsed,
  state,
  children,
}: {
  /** Unique DOM id for the panel — scoped by caller so rail ≠ drawer. */
  id: string;
  title: string;
  /** Shown next to the title while folded, so a fold never hides the numbers. */
  count?: number;
  collapsed: boolean;
  state: SectionState;
  children: React.ReactNode;
}) {
  const t = useTranslations("nav");

  // Icon rail: there are no labels to fold, so the divider stands in and every
  // item stays reachable in one click.
  if (collapsed) {
    return (
      <div className="flex flex-col gap-0.5">
        <SectionLabel collapsed>{title}</SectionLabel>
        {children}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-0.5">
      <div className="flex items-center gap-1 pr-2 pl-1">
        <button
          type="button"
          onClick={state.toggle}
          aria-expanded={state.open}
          aria-controls={id}
          title={
            state.open ? t("collapseSection", { name: title }) : t("expandSection", { name: title })
          }
          className="text-muted-foreground/70 hover:text-foreground focus-visible:ring-ring flex min-w-0 flex-1 items-center gap-1 rounded px-2 py-1 text-left transition-colors outline-none focus-visible:ring-2 max-lg:min-h-10"
        >
          <ChevronRight
            aria-hidden
            className={cn(
              "size-3.5 shrink-0 transition-transform duration-200",
              state.open && "rotate-90",
            )}
          />
          <span className="truncate text-[0.6875rem] font-semibold tracking-[0.08em] uppercase">
            {title}
          </span>
          {!state.open && typeof count === "number" && count > 0 ? (
            <span className="text-muted-foreground/60 shrink-0 text-[0.6875rem] font-medium tabular-nums">
              {count.toLocaleString()}
            </span>
          ) : null}
        </button>
      </div>

      <div
        id={id}
        // `hidden` (the utility, not the attribute) so Tailwind's display class
        // wins the cascade against the `flex` on the same element.
        className={cn("flex flex-col gap-0.5", !state.open && "hidden")}
      >
        {children}
      </div>
    </div>
  );
}

/**
 * Dialogs opened from a menu item must outlive the menu (which unmounts on
 * select), and opening them a tick later avoids Radix's close animation
 * leaving `pointer-events: none` behind on the body.
 */
function defer(fn: () => void) {
  setTimeout(fn, 0);
}

/**
 * What a rail row offers on right-click.
 *
 * Every row here is a link, so replacing the browser's own menu is only fair
 * if ours carries the same staples: open, open in a new tab, copy the address.
 * Rows with something extra to offer (the trash) append it after these.
 */
function NavRowMenuItems({ href, onNavigate }: { href: string; onNavigate?: () => void }) {
  const t = useTranslations("nav");

  async function copyLink() {
    try {
      await navigator.clipboard.writeText(new URL(href, window.location.origin).toString());
      toast.success(t("rowMenu.linkCopied"));
    } catch {
      toast.error(t("rowMenu.copyFailed"));
    }
  }

  return (
    <>
      <RowMenu.Item asChild>
        <Link href={href} onClick={onNavigate}>
          <ArrowRight /> {t("rowMenu.open")}
        </Link>
      </RowMenu.Item>
      <RowMenu.Item asChild>
        <a href={href} target="_blank" rel="noopener noreferrer">
          <ExternalLink /> {t("rowMenu.openNewTab")}
        </a>
      </RowMenu.Item>
      <RowMenu.Item onSelect={copyLink}>
        <Copy /> {t("rowMenu.copyLink")}
      </RowMenu.Item>
    </>
  );
}

function NavRow({
  href,
  label,
  icon: Icon,
  active,
  count,
  collapsed,
  onNavigate,
  menuExtra,
}: {
  href: string;
  label: string;
  icon: typeof UploadCloud;
  active: boolean;
  count?: number;
  collapsed: boolean;
  onNavigate?: () => void;
  /** Row-specific items appended below the open / copy staples. */
  menuExtra?: React.ReactNode;
}) {
  const row = (
    <Link
      href={href}
      onClick={onNavigate}
      onKeyDown={openRowMenuOnKey}
      aria-current={active ? "page" : undefined}
      title={collapsed ? undefined : label}
      className={cn(
        "group/nav relative flex items-center rounded-lg text-sm font-medium transition-colors",
        // Comfortable thumb target on a phone; unchanged on the desktop rail.
        collapsed ? "size-9 justify-center" : "gap-3 px-3 py-2 max-lg:min-h-11",
        active
          ? "bg-accent text-foreground"
          : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
      )}
    >
      <span
        aria-hidden
        className={cn(
          "bg-primary absolute top-1/2 left-0 h-4 w-0.5 -translate-y-1/2 rounded-full transition-opacity",
          active && !collapsed ? "opacity-100" : "opacity-0",
        )}
      />
      <Icon
        className={cn(
          "size-4 shrink-0 transition-colors",
          active ? "text-primary" : "text-muted-foreground group-hover/nav:text-foreground",
        )}
      />
      {collapsed ? null : (
        <>
          <span className="min-w-0 flex-1 truncate">{label}</span>
          {typeof count === "number" && count > 0 ? (
            <span className="text-muted-foreground/70 text-[0.6875rem] font-medium tabular-nums">
              {count.toLocaleString()}
            </span>
          ) : null}
        </>
      )}
    </Link>
  );

  // The tooltip has to sit between the menu trigger and the link: Radix's
  // `asChild` needs a real element underneath it, and only the link is one.
  const trigger = <RowMenu.Trigger asChild>{row}</RowMenu.Trigger>;

  return (
    <RowMenu.Root>
      {collapsed ? (
        <Tooltip>
          <TooltipTrigger asChild>{trigger}</TooltipTrigger>
          <TooltipContent side="right" className="flex items-center gap-2">
            {label}
            {typeof count === "number" && count > 0 ? (
              <span className="opacity-70 tabular-nums">{count.toLocaleString()}</span>
            ) : null}
          </TooltipContent>
        </Tooltip>
      ) : (
        trigger
      )}
      <RowMenu.Content className="w-56">
        <NavRowMenuItems href={href} onNavigate={onNavigate} />
        {menuExtra}
      </RowMenu.Content>
    </RowMenu.Root>
  );
}

/**
 * The trash row, which carries the one destructive action in the rail.
 *
 * "Empty trash" deletes bytes for good, so it is a two-step: the menu opens a
 * confirmation, never the deletion itself. The item count lives on the trash
 * page, not in the shell's sidebar payload, so the dialog states the effect in
 * words instead of paying for a count query on every page render.
 */
function TrashNavRow({
  active,
  collapsed,
  onNavigate,
}: {
  active: boolean;
  collapsed: boolean;
  onNavigate?: () => void;
}) {
  const t = useTranslations("nav");
  const tTrash = useTranslations("dashboard.trash");
  const router = useRouter();
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [pending, startTransition] = useTransition();

  function empty() {
    setConfirmOpen(false);
    startTransition(async () => {
      try {
        const res = await emptyTrash();
        toast.success(
          res.deleted > 0 ? tTrash("emptied", { count: res.deleted }) : tTrash("alreadyEmpty"),
        );
        router.refresh();
      } catch {
        toast.error(tTrash("couldntEmpty"));
        router.refresh();
      }
    });
  }

  return (
    <>
      <NavRow
        href="/dashboard/trash"
        label={t("trash")}
        icon={Trash2}
        active={active}
        collapsed={collapsed}
        onNavigate={onNavigate}
        menuExtra={
          <>
            <RowMenu.Separator />
            <RowMenu.Item
              variant="destructive"
              disabled={pending}
              onSelect={() => defer(() => setConfirmOpen(true))}
            >
              <Trash2 /> {tTrash("emptyTrash")}
            </RowMenu.Item>
          </>
        }
      />

      <Dialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <DialogContent>
          <DialogHeader>
            <div className="bg-destructive/10 text-destructive mb-1 grid size-10 place-items-center rounded-lg">
              <AlertTriangle className="size-5" />
            </div>
            <DialogTitle>{tTrash("emptyTrashTitle")}</DialogTitle>
            <DialogDescription>{tTrash("emptyTrashDescAll")}</DialogDescription>
          </DialogHeader>
          <DialogFooter showCloseButton>
            <Button variant="destructive" onClick={empty} disabled={pending}>
              {pending ? (
                <Loader2 className="size-4 animate-spin" />
              ) : (
                <Trash2 className="size-4" />
              )}
              {tTrash("emptyTrash")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

/* -------------------------------------------------------------------------- */
/* Storage meter                                                               */
/* -------------------------------------------------------------------------- */

function StorageMeter({
  storage,
  collapsed,
}: {
  storage: { used: number; max: number };
  collapsed: boolean;
}) {
  const t = useTranslations("nav");
  const unlimited = storage.max < 0;
  const pct = unlimited ? 0 : Math.min(100, Math.round((storage.used / Math.max(1, storage.max)) * 100));
  const critical = !unlimited && pct >= 90;

  if (collapsed) {
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          <div className="text-muted-foreground mx-auto grid size-9 place-items-center rounded-lg">
            <HardDrive className={cn("size-4", critical && "text-destructive")} />
          </div>
        </TooltipTrigger>
        <TooltipContent side="right">
          {unlimited
            ? t("storage.used", { used: humanFileSize(storage.used) })
            : t("storage.ofQuota", {
                used: humanFileSize(storage.used),
                total: humanFileSize(storage.max),
              })}
        </TooltipContent>
      </Tooltip>
    );
  }

  return (
    <div className="px-4 pb-2">
      <div className="mb-1.5 flex items-baseline justify-between gap-2">
        <span className="text-muted-foreground/70 text-[0.6875rem] font-semibold tracking-[0.08em] uppercase">
          {t("storage.label")}
        </span>
        <span
          className={cn(
            "text-[0.6875rem] font-medium tabular-nums",
            critical ? "text-destructive" : "text-muted-foreground",
          )}
        >
          {unlimited ? humanFileSize(storage.used) : `${pct}%`}
        </span>
      </div>
      {unlimited ? (
        <p className="text-muted-foreground/70 text-[0.6875rem]">{t("storage.unlimited")}</p>
      ) : (
        <>
          <div className="bg-muted h-1 w-full overflow-hidden rounded-full">
            <div
              className={cn(
                "h-full rounded-full transition-[width] duration-500",
                critical ? "bg-destructive" : "bg-primary",
              )}
              style={{ width: `${Math.max(pct, storage.used > 0 ? 2 : 0)}%` }}
            />
          </div>
          <p className="text-muted-foreground/70 mt-1.5 text-[0.6875rem] tabular-nums">
            {t("storage.ofQuota", {
              used: humanFileSize(storage.used),
              total: humanFileSize(storage.max),
            })}
          </p>
        </>
      )}
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Sidebar body                                                                */
/* -------------------------------------------------------------------------- */

function SidebarNav({
  scope,
  sidebar,
  sections,
  isAdmin,
  userCode,
  collapsed,
  onNavigate,
  onExpand,
}: {
  /** Distinguishes the rail's DOM ids from the drawer's — both can be mounted. */
  scope: string;
  sidebar?: ShellSidebarData;
  sections: { library: SectionState; create: SectionState; account: SectionState };
  isAdmin: boolean;
  userCode?: string | null;
  collapsed: boolean;
  onNavigate?: () => void;
  onExpand?: () => void;
}) {
  const t = useTranslations("nav");
  const pathname = usePathname();
  const params = useSearchParams();

  const onDashboard = pathname === "/dashboard";
  const onFolderBrowser = pathname.startsWith("/dashboard/folders");
  const activeKind = onDashboard ? params.get("kind") : null;
  const activeFolder = onDashboard ? params.get("folder") : null;
  const allActive = onDashboard && !activeKind && !activeFolder;

  const accountItems = isAdmin
    ? [...ACCOUNT_ITEMS, { href: "/dashboard/admin", labelKey: "admin", icon: ShieldCheck }]
    : ACCOUNT_ITEMS;

  return (
    <nav className={cn("flex flex-col gap-5", collapsed ? "px-3" : "px-2")}>
      {/* Library */}
      <NavSection
        id={`nav-${scope}-library`}
        title={t("library")}
        count={sidebar?.totalFiles}
        collapsed={collapsed}
        state={sections.library}
      >
        {/* The explorer comes first: it is where /dashboard lands anyone who
            uses folders, so the rail should read in the same order. */}
        <NavRow
          href="/dashboard/folders"
          label={t("folders")}
          icon={Folders}
          active={onFolderBrowser}
          collapsed={collapsed}
          onNavigate={onNavigate}
        />
        <NavRow
          // `all=1` is what keeps this link on the flat list: a bare /dashboard
          // redirects to the explorer for accounts that have folders.
          href={buildDashboardHref(params, { all: "1" })}
          label={t("files")}
          icon={Layers}
          active={allActive}
          count={sidebar?.totalFiles}
          collapsed={collapsed}
          onNavigate={onNavigate}
        />
        {KIND_VIEWS.map(({ kind, icon }) => {
          const count = sidebar?.counts?.[kind];
          // Hide a smart view the user has nothing for — a library of
          // screenshots shouldn't carry five permanently empty rows.
          if (sidebar && !count) return null;
          return (
            <NavRow
              key={kind}
              // `all=1` rides along so that clearing the filters from here
              // lands back on the flat list instead of the folder explorer.
              href={buildDashboardHref(params, { all: "1", kind })}
              label={t(`kind.${kind}`)}
              icon={icon}
              active={activeKind === kind}
              count={count}
              collapsed={collapsed}
              onNavigate={onNavigate}
            />
          );
        })}
        {/* Duplicates sits beside the trash because it answers the same
            question: what the library holds that it does not need. The
            reminder banner links here as well, and a screen reachable only
            from a banner that can be put away for a week is a screen nobody
            can find again. */}
        <NavRow
          href="/dashboard/duplicates"
          label={t("duplicates")}
          icon={CopyCheck}
          active={pathname.startsWith("/dashboard/duplicates")}
          collapsed={collapsed}
          onNavigate={onNavigate}
        />
        <TrashNavRow
          active={pathname.startsWith("/dashboard/trash")}
          collapsed={collapsed}
          onNavigate={onNavigate}
        />
      </NavSection>

      {/* Folder tree. Collapsed, the rail has no room for it — the button
          reopens the sidebar rather than pretending to be a tree. */}
      {collapsed ? (
        <div className="flex flex-col gap-0.5">
          <SectionLabel collapsed>{t("foldersSection")}</SectionLabel>
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                onClick={onExpand}
                aria-label={t("foldersSection")}
                className={cn(
                  "text-muted-foreground hover:bg-accent/60 hover:text-foreground grid size-9 place-items-center rounded-lg transition-colors",
                  activeFolder && "bg-accent text-foreground",
                )}
              >
                <ListTree className={cn("size-4", activeFolder && "text-primary")} />
              </button>
            </TooltipTrigger>
            <TooltipContent side="right">{t("foldersSection")}</TooltipContent>
          </Tooltip>
        </div>
      ) : sidebar ? (
        <FolderTree
          scope={scope}
          folders={sidebar.folders}
          activeFolderId={activeFolder}
          unsortedCount={sidebar.unsortedCount}
          userCode={userCode}
          onNavigate={onNavigate}
        />
      ) : null}

      {/* Create */}
      <NavSection
        id={`nav-${scope}-create`}
        title={t("create")}
        collapsed={collapsed}
        state={sections.create}
      >
        {CREATE_ITEMS.map((item) => (
          <NavRow
            key={item.href}
            href={item.href}
            label={t(item.labelKey)}
            icon={item.icon}
            active={pathname === item.href || pathname.startsWith(`${item.href}/`)}
            collapsed={collapsed}
            onNavigate={onNavigate}
          />
        ))}
      </NavSection>

      {/* Account */}
      <NavSection
        id={`nav-${scope}-account`}
        title={t("account")}
        collapsed={collapsed}
        state={sections.account}
      >
        {accountItems.map((item) => (
          <NavRow
            key={item.href}
            href={item.href}
            label={t(item.labelKey)}
            icon={item.icon}
            active={pathname === item.href || pathname.startsWith(`${item.href}/`)}
            collapsed={collapsed}
            onNavigate={onNavigate}
          />
        ))}
      </NavSection>
    </nav>
  );
}

function Brand({ collapsed }: { collapsed?: boolean }) {
  const t = useTranslations("nav");
  return (
    <Link
      href="/dashboard"
      className={cn(
        "group/brand flex items-center py-5",
        collapsed ? "justify-center px-2" : "gap-2.5 px-5",
      )}
    >
      <BrandTile className="size-9 transition-transform group-hover/brand:scale-105" />
      {collapsed ? null : (
        <div className="flex min-w-0 flex-col leading-none">
          <span className="truncate text-base font-semibold tracking-tight">Modlight</span>
          <span className="text-muted-foreground truncate text-[0.6875rem] font-medium">
            {t("mediaPanel")}
          </span>
        </div>
      )}
    </Link>
  );
}

/* -------------------------------------------------------------------------- */
/* Shell                                                                       */
/* -------------------------------------------------------------------------- */

export function AppShell({
  user,
  sidebar,
  defaultCollapsed = false,
  children,
}: {
  user: ShellUser;
  sidebar?: ShellSidebarData;
  defaultCollapsed?: boolean;
  children: React.ReactNode;
}) {
  const t = useTranslations("nav");
  const router = useRouter();
  const pathname = usePathname();
  const [mobileOpen, setMobileOpen] = useState(false);
  const [collapsed, setCollapsed] = useState(defaultCollapsed);
  const isAdmin = user.role === "admin";
  const initials = (user.name || user.email).slice(0, 2).toUpperCase();

  // Lifted so the rail and the mobile drawer always show the same folds.
  const sections = {
    library: useSectionCollapse("library"),
    create: useSectionCollapse("create"),
    account: useSectionCollapse("account"),
  };

  const titleKey = TITLE_ROUTES.find((r) => pathname.startsWith(r.prefix))?.labelKey;

  const setCollapsedPersisted = useCallback((next: boolean) => {
    setCollapsed(next);
    // A cookie (not localStorage) so the server renders the right width on the
    // very first paint — no sidebar snapping about after hydration.
    document.cookie = `${SIDEBAR_COOKIE}=${next ? "collapsed" : "expanded"};path=/;max-age=31536000;samesite=lax`;
  }, []);

  async function handleSignOut() {
    await signOut();
    router.push("/login");
    router.refresh();
  }

  const accountMenu = (
    <>
      <DropdownMenuLabel className="truncate font-normal">{user.email}</DropdownMenuLabel>
      <DropdownMenuSeparator />
      <DropdownMenuItem asChild>
        <Link href="/dashboard/profile">
          <UserRound className="size-4" /> {t("profile")}
        </Link>
      </DropdownMenuItem>
      {isAdmin && (
        <DropdownMenuItem asChild>
          <Link href="/dashboard/admin">
            <ShieldCheck className="size-4" /> {t("admin")}
          </Link>
        </DropdownMenuItem>
      )}
      <DropdownMenuSeparator />
      <DropdownMenuItem onClick={handleSignOut} variant="destructive">
        <LogOut className="size-4" /> {t("signOut")}
      </DropdownMenuItem>
    </>
  );

  /** The account card that closes the rail (and the mobile drawer). */
  const accountCard = (railCollapsed: boolean) => (
    <div className="p-3">
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label={user.name || user.email}
            className={cn(
              "bg-background/60 ring-foreground/10 hover:bg-accent hover:ring-foreground/20 flex w-full items-center rounded-lg text-left ring-1 transition-colors",
              railCollapsed ? "justify-center p-1.5" : "gap-2.5 p-2",
            )}
          >
            <Avatar className="size-8">
              {/* Radix renders the fallback until the image reports "loaded", so
                  an avatar whose bytes have gone (or an account that never set
                  one) shows initials rather than a broken picture. */}
              {user.image ? <AvatarImage src={user.image} alt="" /> : null}
              <AvatarFallback className="text-xs font-semibold">{initials}</AvatarFallback>
            </Avatar>
            {railCollapsed ? null : (
              <>
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-medium">{user.name || user.email}</p>
                  <p className="text-muted-foreground truncate text-xs">
                    {user.userCode ? `/${user.userCode}` : user.email}
                  </p>
                </div>
                <ChevronsUpDown className="text-muted-foreground size-4 shrink-0" />
              </>
            )}
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent side="top" align="start" className="w-[15.5rem]">
          {accountMenu}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );

  return (
    <div className="page-surface flex min-h-dvh">
      {/* Desktop sidebar — pinned to the viewport with its own scroll, so the
          storage meter and the account card never drift below a long page. */}
      <aside
        data-collapsed={collapsed || undefined}
        className={cn(
          "bg-card/60 sticky top-0 hidden h-dvh shrink-0 flex-col border-r backdrop-blur transition-[width] duration-200 lg:flex",
          collapsed ? "w-[4.25rem]" : "w-[17rem]",
        )}
      >
        <div className="relative shrink-0">
          <Brand collapsed={collapsed} />
          {collapsed ? null : (
            <button
              type="button"
              onClick={() => setCollapsedPersisted(true)}
              aria-label={t("collapseSidebar")}
              title={t("collapseSidebar")}
              className="text-muted-foreground hover:bg-accent hover:text-foreground absolute top-1/2 right-3 grid size-7 -translate-y-1/2 place-items-center rounded-lg transition-colors"
            >
              <PanelLeftClose className="size-4" />
            </button>
          )}
        </div>

        {/* Only the navigation scrolls. */}
        <div className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto pt-1 pb-4">
          <SidebarNav
            scope="rail"
            sidebar={sidebar}
            sections={sections}
            isAdmin={isAdmin}
            userCode={user.userCode}
            collapsed={collapsed}
            onExpand={() => setCollapsedPersisted(false)}
          />
        </div>

        <div className="border-foreground/10 shrink-0 border-t pt-2">
          {collapsed ? (
            <div className="flex flex-col items-center gap-1 pb-2">
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    type="button"
                    onClick={() => setCollapsedPersisted(false)}
                    aria-label={t("expandSidebar")}
                    className="text-muted-foreground hover:bg-accent hover:text-foreground grid size-9 place-items-center rounded-lg transition-colors"
                  >
                    <PanelLeftOpen className="size-4" />
                  </button>
                </TooltipTrigger>
                <TooltipContent side="right">{t("expandSidebar")}</TooltipContent>
              </Tooltip>
            </div>
          ) : null}

          {sidebar ? <StorageMeter storage={sidebar.storage} collapsed={collapsed} /> : null}
          {accountCard(collapsed)}
        </div>
      </aside>

      <div className="flex min-w-0 flex-1 flex-col">
        {/* Top bar */}
        <header className="bg-background/70 sticky top-0 z-30 flex h-14 items-center gap-1 border-b px-2 backdrop-blur-md sm:gap-2 sm:px-4">
          <Sheet open={mobileOpen} onOpenChange={setMobileOpen}>
            <SheetTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="size-11 lg:hidden"
                aria-label={t("menu")}
              >
                <Menu className="size-5" />
              </Button>
            </SheetTrigger>
            {/* Same rail, in a drawer: navigation scrolls, the footer stays. */}
            <SheetContent side="left" className="bg-card flex flex-col gap-0 p-0">
              <SheetTitle className="sr-only">{t("navigation")}</SheetTitle>
              <div className="shrink-0">
                <Brand />
              </div>
              <div className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto pb-4">
                <SidebarNav
                  scope="sheet"
                  sidebar={sidebar}
                  sections={sections}
                  isAdmin={isAdmin}
                  userCode={user.userCode}
                  collapsed={false}
                  onNavigate={() => setMobileOpen(false)}
                />
              </div>
              <div className="border-foreground/10 shrink-0 border-t pt-2">
                {sidebar ? <StorageMeter storage={sidebar.storage} collapsed={false} /> : null}
                {accountCard(false)}
              </div>
            </SheetContent>
          </Sheet>

          {/* Where you are — the phone has no rail to read it from. */}
          <span className="min-w-0 flex-1 truncate text-sm font-semibold tracking-tight lg:hidden">
            {titleKey ? t(titleKey) : "Modlight"}
          </span>

          <div className="hidden flex-1 lg:block" />

          <div className="[&_button]:size-10 lg:[&_button]:size-8">
            <ThemeToggle />
          </div>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" className="h-11 gap-2 px-2 lg:h-8">
                <Avatar className="size-7">
                  {user.image ? <AvatarImage src={user.image} alt="" /> : null}
                  <AvatarFallback className="text-xs font-semibold">{initials}</AvatarFallback>
                </Avatar>
                <span className="hidden text-sm font-medium sm:inline">
                  {user.name || user.email}
                </span>
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-56">
              {accountMenu}
            </DropdownMenuContent>
          </DropdownMenu>
        </header>

        <main className="min-w-0 flex-1 p-4 sm:p-6 lg:p-8">{children}</main>
      </div>
    </div>
  );
}
