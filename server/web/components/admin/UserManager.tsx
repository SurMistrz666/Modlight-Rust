"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import {
  UserPlus,
  Search,
  MoreHorizontal,
  ShieldCheck,
  Ban,
  Trash2,
  RefreshCw,
  Loader2,
  Send,
  MailWarning,
  Users as UsersIcon,
  Link2,
  Ghost,
  FolderOpen,
  KeyRound,
  Files,
  HardDrive,
  TriangleAlert,
  AtSign,
  Globe,
  Hash,
  IdCard,
  ShieldAlert,
} from "lucide-react";
import { toast } from "sonner";
import { useLocale, useTranslations } from "next-intl";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import { Separator } from "@/components/ui/separator";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogClose,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { humanFileSize, relativeTime } from "@/lib/format";
import { formatQuotaInput, isUnlimited, parseQuota, quotaRatio } from "@/lib/quota";
import type { AdminUserDetail, AdminUserRow } from "@/lib/admin-users";
import type { UserDeletionImpact } from "@/lib/data/admin";
import { listAdminUsers, recalculateUserQuota } from "@/app/actions/account";
import {
  createAccount,
  deleteUserAccount,
  loadDeletionImpact,
  loadUserDetail,
  resendInvite,
  sendVerificationLink,
  setUserBanned,
  setUserEmailVerified,
  setUserRole,
  updateUserAccount,
  type FileDisposition,
  type UpdateUserPatch,
  type UpdateUserResult,
  type UserFieldIssue,
} from "@/app/actions/admin";
import { UserRowContextMenu, UserRowDropdown } from "./UserRowMenu";

function initialsOf(u: AdminUserRow) {
  const src = u.name?.trim() || u.email;
  return src.slice(0, 2).toUpperCase();
}

function errMessage(error: unknown, fallback: string): string {
  if (error && typeof error === "object" && "message" in error) {
    const m = (error as { message?: unknown }).message;
    if (typeof m === "string" && m) return m;
  }
  return fallback;
}

/**
 * Root short links come from the role OR the column — `mayCreateRootShortLink()`
 * in lib/data/user-prefs.ts grants them to every admin regardless of the flag.
 *
 * The switch reflects that, so an admin who can demonstrably mint `/{code}`
 * links never sees the permission sitting at "off".
 *
 * Deliberately NOT solved by writing `true` into the column when someone is
 * promoted: the flag would then survive a demotion and quietly leave an ex-admin
 * with a permission nobody granted them. The draft therefore holds the raw
 * column value and only the *display* folds the role in — otherwise demoting
 * somebody in this very dialog would persist the grant on the way out.
 */
function grantedByRole(role: string | null | undefined): boolean {
  return role === "admin";
}

/** Draft state of the edit dialog, seeded from the freshly loaded account. */
interface EditDraft {
  name: string;
  username: string;
  email: string;
  tag: string;
  password: string;
  unlimited: boolean;
  quota: string;
  role: "admin" | "user";
  banned: boolean;
  rootLinks: boolean;
  publicProfile: boolean;
  skipTagInUrl: boolean;
}

function draftFrom(u: AdminUserDetail): EditDraft {
  return {
    name: u.name ?? "",
    username: u.username ?? "",
    email: u.email,
    tag: u.tag ?? "",
    password: "",
    unlimited: isUnlimited(u.maxQuota),
    quota: formatQuotaInput(u.maxQuota),
    role: u.role === "admin" ? "admin" : "user",
    banned: u.banned,
    rootLinks: u.allowRootShortLinks,
    publicProfile: u.publicProfile,
    skipTagInUrl: u.skipTagInUrl,
  };
}

/**
 * Used / limit, with a hairline meter. Unlimited accounts show the used figure
 * against ∞ rather than a bar — a progress bar with no ceiling is a lie.
 */
function StorageCell({ used, max }: { used: number; max: number }) {
  const ratio = quotaRatio(used, max);
  const tone =
    ratio == null
      ? "bg-primary"
      : ratio >= 1
        ? "bg-rose-500"
        : ratio >= 0.85
          ? "bg-amber-500"
          : "bg-primary";

  return (
    <div className="flex min-w-28 flex-col gap-1.5">
      <span className="text-xs tabular-nums">
        <span className="font-medium">{humanFileSize(used)}</span>
        <span className="text-muted-foreground"> / {isUnlimited(max) ? "∞" : humanFileSize(max)}</span>
      </span>
      {ratio != null && (
        <span className="bg-foreground/10 block h-1 w-full overflow-hidden rounded-full">
          <span
            className={cn("block h-full rounded-full transition-[width]", tone)}
            style={{ width: `${Math.max(2, ratio * 100)}%` }}
          />
        </span>
      )}
    </div>
  );
}

export function UserManager({
  currentUserId,
  mailReady,
}: {
  currentUserId: string;
  /** Outgoing mail is configured, so invitations and confirmations can be sent. */
  mailReady: boolean;
}) {
  const t = useTranslations("admin.users");
  // relativeTime formats through Intl, so it has to be told whose language
  // this is; without it a Polish reader gets English ages.
  const locale = useLocale();
  const tc = useTranslations("common");
  // Lives under `profile` rather than `admin` only because this file's namespace
  // file is owned elsewhere; the string itself is admin-facing.
  const tPerm = useTranslations("profile.permissions");
  const [users, setUsers] = useState<AdminUserRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [query, setQuery] = useState("");
  const [busyId, setBusyId] = useState<string | null>(null);

  // Create dialog state. `mode` decides whether the administrator hands over a
  // password out of band or the new user sets their own from a mailed link.
  const [createOpen, setCreateOpen] = useState(false);
  const [creating, setCreating] = useState(false);
  const [form, setForm] = useState<{
    name: string;
    email: string;
    password: string;
    role: string;
    mode: "invite" | "password";
  }>({
    name: "",
    email: "",
    password: "",
    role: "user",
    // Invitations are the better default where they work: nothing guessable
    // exists between creating the account and the person using it.
    mode: mailReady ? "invite" : "password",
  });

  // Edit dialog state. `editTarget` is the row that was clicked (so the dialog
  // can title itself immediately); `detail` is the account read back fresh.
  const [editTarget, setEditTarget] = useState<AdminUserRow | null>(null);
  const [detail, setDetail] = useState<AdminUserDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [draft, setDraft] = useState<EditDraft | null>(null);
  const [saving, setSaving] = useState(false);
  // Set once the operator has been shown what the sensitive changes will do.
  const [confirmSensitive, setConfirmSensitive] = useState(false);

  // Remove confirm state. The dialog is a decision, not a yes/no: the operator
  // has to say what happens to the account's files before it can be submitted.
  const [removeTarget, setRemoveTarget] = useState<AdminUserRow | null>(null);
  const [removing, setRemoving] = useState(false);
  const [impact, setImpact] = useState<UserDeletionImpact | null>(null);
  const [impactLoading, setImpactLoading] = useState(false);
  const [disposition, setDisposition] = useState<FileDisposition>("delete");

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setUsers(await listAdminUsers());
    } catch (error) {
      toast.error(t("loadError"), { description: errMessage(error, t("tryRefreshing")) });
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    void load();
  }, [load]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return users;
    return users.filter(
      (u) =>
        u.email.toLowerCase().includes(q) ||
        u.name.toLowerCase().includes(q) ||
        (u.tag ?? "").toLowerCase().includes(q),
    );
  }, [users, query]);

  /** Translated copy for one rejected field — server errors travel as codes. */
  const issueText = useCallback(
    (issue: UserFieldIssue) =>
      issue.field === "tag"
        ? t(`tagIssue.${issue.reason}`)
        : t(`issue.${issue.field}.${issue.reason}`),
    [t],
  );

  /** Report a rejected update. Every issue is shown: they are all fixable. */
  const reportIssues = useCallback(
    (res: Extract<UpdateUserResult, { ok: false }>) => {
      if (res.reason === "not-found") {
        toast.error(t("updateErrorTitle"), { description: t("removeReason.not-found") });
        return;
      }
      for (const issue of res.issues) {
        toast.error(t("updateErrorTitle"), { description: issueText(issue) });
      }
    },
    [t, issueText],
  );

  /**
   * Create an account, whatever self-service registration is set to.
   *
   * Goes through our own action rather than the Better Auth admin client: the
   * invitation branch has to strip the throwaway credential and mail a link,
   * and the gate that used to reject this call when sign-up was disabled lived
   * on a shared code path (see the comment in app/actions/admin.ts).
   */
  async function handleCreate() {
    const email = form.email.trim();
    if (!email) {
      toast.error(t("emailRequired"));
      return;
    }
    if (form.mode === "password" && !form.password) {
      toast.error(t("passwordRequired"));
      return;
    }

    setCreating(true);
    try {
      const res = await createAccount({
        email,
        name: form.name.trim() || undefined,
        role: form.role === "admin" ? "admin" : "user",
        mode: form.mode,
        password: form.mode === "password" ? form.password : undefined,
      });
      if (!res.ok) {
        toast.error(t("createErrorTitle"), { description: t(`createReason.${res.reason}`) });
        return;
      }

      if (res.invited && !res.mailSent) {
        // The account exists but the link did not go out; saying "created" and
        // nothing else would leave an account nobody can get into.
        toast.warning(t("createdTitle"), { description: t("createdInviteFailed", { email }) });
      } else {
        toast.success(t("createdTitle"), {
          description: res.invited ? t("createdInvited", { email }) : email,
        });
      }

      setForm({
        name: "",
        email: "",
        password: "",
        role: "user",
        mode: mailReady ? "invite" : "password",
      });
      setCreateOpen(false);
      void load();
    } catch (error) {
      toast.error(t("createErrorTitle"), { description: errMessage(error, t("tryAgain")) });
    } finally {
      setCreating(false);
    }
  }

  /** Confirm an address by hand: the way out of a broken or abandoned mailbox. */
  async function handleVerifyEmail(u: AdminUserRow) {
    setBusyId(u.id);
    try {
      const res = await setUserEmailVerified(u.id, true);
      if (!res.ok) {
        toast.error(t("verifyErrorTitle"), { description: t("removeReason.not-found") });
        return;
      }
      toast.success(t("verifiedToast", { email: u.email }));
      setUsers((prev) =>
        prev.map((x) => (x.id === u.id ? { ...x, emailVerified: true } : x)),
      );
    } catch (error) {
      toast.error(t("verifyErrorTitle"), { description: errMessage(error, t("tryAgain")) });
    } finally {
      setBusyId(null);
    }
  }

  async function handleResendInvite(u: AdminUserRow) {
    setBusyId(u.id);
    try {
      const res = await resendInvite(u.id);
      if (!res.ok) {
        toast.error(t("inviteErrorTitle"), { description: t(`mailReason.${res.reason}`) });
        return;
      }
      toast.success(t("inviteSent", { email: u.email }));
    } catch (error) {
      toast.error(t("inviteErrorTitle"), { description: errMessage(error, t("tryAgain")) });
    } finally {
      setBusyId(null);
    }
  }

  async function handleSendVerification(u: AdminUserRow) {
    setBusyId(u.id);
    try {
      const res = await sendVerificationLink(u.id);
      if (!res.ok) {
        toast.error(t("verificationErrorTitle"), { description: t(`mailReason.${res.reason}`) });
        return;
      }
      toast.success(t("verificationSent", { email: u.email }));
    } catch (error) {
      toast.error(t("verificationErrorTitle"), {
        description: errMessage(error, t("tryAgain")),
      });
    } finally {
      setBusyId(null);
    }
  }

  /**
   * Role and ban go through our own Server Actions rather than the Better Auth
   * client: the self-demotion guard lives there, and it has to sit on the same
   * side of the wire as the write.
   */
  async function handleSetRole(u: AdminUserRow, role: "admin" | "user") {
    setBusyId(u.id);
    try {
      const res = await setUserRole(u.id, role);
      if (!res.ok) {
        reportIssues(res);
        return;
      }
      toast.success(
        role === "admin"
          ? t("nowAdmin", { email: u.email })
          : t("nowUser", { email: u.email }),
      );
      setUsers((prev) => prev.map((x) => (x.id === u.id ? { ...x, role } : x)));
    } catch (error) {
      toast.error(t("roleErrorTitle"), { description: errMessage(error, t("tryAgain")) });
    } finally {
      setBusyId(null);
    }
  }

  async function handleBanToggle(u: AdminUserRow) {
    const next = !u.banned;
    setBusyId(u.id);
    try {
      const res = await setUserBanned(u.id, next);
      if (!res.ok) {
        reportIssues(res);
        return;
      }
      toast.success(
        next ? t("bannedToast", { email: u.email }) : t("reinstatedToast", { email: u.email }),
      );
      setUsers((prev) => prev.map((x) => (x.id === u.id ? { ...x, banned: next } : x)));
    } catch (error) {
      toast.error(next ? t("banErrorTitle") : t("unbanErrorTitle"), {
        description: errMessage(error, t("tryAgain")),
      });
    } finally {
      setBusyId(null);
    }
  }

  async function handleRecalculate(u: AdminUserRow) {
    setBusyId(u.id);
    try {
      const { used } = await recalculateUserQuota(u.id);
      setUsers((prev) => prev.map((x) => (x.id === u.id ? { ...x, usedQuota: used } : x)));
      toast.success(t("recalcDone"), { description: humanFileSize(used) });
    } catch (error) {
      toast.error(t("recalcErrorTitle"), { description: errMessage(error, t("tryAgain")) });
    } finally {
      setBusyId(null);
    }
  }

  /**
   * Open the edit dialog and read the account back from the server. The table
   * row was loaded when the tab opened; editing a stale copy of an email address
   * or a quota is how two admins silently overwrite each other.
   */
  function openEdit(u: AdminUserRow) {
    setEditTarget(u);
    setDetail(null);
    setDraft(null);
    setConfirmSensitive(false);
    setDetailLoading(true);
    void loadUserDetail(u.id)
      .then((data) => {
        setDetail(data);
        if (data) setDraft(draftFrom(data));
      })
      .catch(() => setDetail(null))
      .finally(() => setDetailLoading(false));
  }

  function closeEdit() {
    setEditTarget(null);
    setDetail(null);
    setDraft(null);
    setConfirmSensitive(false);
  }

  // What the form would send. Computed rather than tracked so "did anything
  // change" and "what changed" can never disagree.
  const pendingPatch = useMemo<UpdateUserPatch | null>(() => {
    if (!detail || !draft) return null;
    const patch: UpdateUserPatch = {};

    if (draft.name.trim() !== (detail.name ?? "")) patch.name = draft.name.trim();
    if (draft.username.trim() !== (detail.username ?? "")) patch.username = draft.username.trim();
    if (draft.email.trim().toLowerCase() !== detail.email.toLowerCase()) patch.email = draft.email.trim();
    if (draft.tag.trim() !== (detail.tag ?? "")) patch.tag = draft.tag.trim();
    if (draft.password) patch.newPassword = draft.password;
    if (draft.role !== (detail.role === "admin" ? "admin" : "user")) patch.role = draft.role;
    if (draft.banned !== detail.banned) patch.banned = draft.banned;
    if (draft.rootLinks !== detail.allowRootShortLinks) patch.allowRootShortLinks = draft.rootLinks;
    if (draft.publicProfile !== detail.publicProfile) patch.publicProfile = draft.publicProfile;
    if (draft.skipTagInUrl !== detail.skipTagInUrl) patch.skipTagInUrl = draft.skipTagInUrl;

    const quotaChanged =
      draft.unlimited !== isUnlimited(detail.maxQuota) ||
      (!draft.unlimited && draft.quota.trim() !== formatQuotaInput(detail.maxQuota));
    if (quotaChanged) {
      // -1 is the stored sentinel for "no limit"; NaN is caught on submit so the
      // operator gets a message instead of a silent no-op.
      patch.maxQuota = draft.unlimited ? -1 : (parseQuota(draft.quota) ?? Number.NaN);
    }

    return Object.keys(patch).length > 0 ? patch : null;
  }, [detail, draft]);

  const sensitive = !!pendingPatch && (!!pendingPatch.newPassword || pendingPatch.email !== undefined);
  // The confirmation only stands while there is still something to confirm: if
  // the operator clears the password field after being asked, the button must
  // stop threatening to reset it.
  const awaitingConfirm = confirmSensitive && sensitive;

  async function saveEdits() {
    if (!detail || !draft) return;
    if (!pendingPatch) {
      closeEdit();
      return;
    }
    if (pendingPatch.maxQuota !== undefined && Number.isNaN(pendingPatch.maxQuota)) {
      toast.error(t("quotaInvalidTitle"), { description: t("quotaInvalidDesc") });
      return;
    }
    // Changing somebody's address or password takes their account away from
    // them. One extra, named confirmation before that happens.
    if (sensitive && !awaitingConfirm) {
      setConfirmSensitive(true);
      return;
    }

    setSaving(true);
    try {
      const res = await updateUserAccount(detail.id, pendingPatch);
      if (!res.ok) {
        reportIssues(res);
        setConfirmSensitive(false);
        return;
      }

      const notes: string[] = [];
      if (res.changed.includes("email")) notes.push(t("noteEmail", { email: res.email }));
      if (res.changed.includes("password")) notes.push(t("notePassword"));
      toast.success(t("updatedToast", { name: res.name || res.email }), {
        description: notes.length ? notes.join(" ") : undefined,
      });

      closeEdit();
      void load();
    } catch (error) {
      toast.error(t("updateErrorTitle"), { description: errMessage(error, t("tryAgain")) });
    } finally {
      setSaving(false);
    }
  }

  /**
   * Open the delete dialog and go fetch what is actually at stake. The numbers
   * are read fresh rather than taken from the table row: the row was loaded
   * when the tab opened and may be minutes old, and "delete 3 files" printed
   * over an account that now holds 3000 is not a confirmation.
   */
  function openRemove(u: AdminUserRow) {
    setRemoveTarget(u);
    setDisposition("delete");
    setImpact(null);
    setImpactLoading(true);
    void loadDeletionImpact(u.id)
      .then((data) => setImpact(data))
      .catch(() => setImpact(null))
      .finally(() => setImpactLoading(false));
  }

  async function confirmRemove() {
    if (!removeTarget) return;
    const target = removeTarget;
    setRemoving(true);
    try {
      const res = await deleteUserAccount(target.id, disposition);
      if (!res.ok) {
        toast.error(t("removeErrorTitle"), { description: t(`removeReason.${res.reason}`) });
        // The files really are gone in this case — the listing must not keep
        // pretending the account still owns them.
        if (res.reason === "files-removed") void load();
        return;
      }
      toast.success(t("removedToast", { email: target.email }), {
        description:
          disposition === "keep"
            ? t("removedKept", { count: res.orphaned })
            : t("removedPurged", { count: res.deleted, size: humanFileSize(res.bytes) }),
      });
      if (res.failed > 0) {
        toast.warning(t("removeFailedTitle"), {
          description: t("removeFailedDesc", { count: res.failed }),
        });
      }
      setUsers((prev) => prev.filter((x) => x.id !== target.id));
      setRemoveTarget(null);
    } catch (error) {
      toast.error(t("removeErrorTitle"), { description: errMessage(error, t("tryAgain")) });
    } finally {
      setRemoving(false);
    }
  }

  const editingSelf = !!detail && detail.id === currentUserId;
  // Admins hold the root-short-link permission by virtue of the role, so the
  // switch shows the truth and refuses to pretend it can be turned off. It reads
  // the DRAFT role, so promoting somebody updates it before the save.
  const rootLocked = !!draft && grantedByRole(draft.role);

  return (
    <div className="flex flex-col gap-4">
      {/* Toolbar */}
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="relative w-full sm:max-w-xs">
          <Search className="text-muted-foreground pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2" />
          <Input
            placeholder={t("searchPlaceholder")}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            className="pl-8"
          />
        </div>
        <div className="flex items-center gap-2">
          <Button variant="outline" onClick={() => void load()} disabled={loading}>
            <RefreshCw className={cn("size-4", loading && "animate-spin")} /> {t("refresh")}
          </Button>
          <Button onClick={() => setCreateOpen(true)}>
            <UserPlus className="size-4" /> {t("newUser")}
          </Button>
        </div>
      </div>

      {/* Table */}
      <div className="ring-foreground/10 overflow-x-auto rounded-xl ring-1">
        <Table>
          <TableHeader>
            <TableRow className="bg-muted/40 hover:bg-muted/40">
              <TableHead className="pl-4">{t("colUser")}</TableHead>
              <TableHead className="hidden md:table-cell">{t("colTag")}</TableHead>
              <TableHead className="hidden text-right lg:table-cell">{t("colFiles")}</TableHead>
              <TableHead>{t("colStorage")}</TableHead>
              <TableHead>{t("colRole")}</TableHead>
              <TableHead className="hidden sm:table-cell">{t("colStatus")}</TableHead>
              <TableHead className="hidden xl:table-cell">{t("colJoined")}</TableHead>
              <TableHead className="pr-4 text-right">{t("colActions")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading ? (
              Array.from({ length: 5 }).map((_, i) => (
                <TableRow key={i} className="hover:bg-transparent">
                  <TableCell className="pl-4">
                    <div className="flex items-center gap-3">
                      <Skeleton className="size-8 rounded-full" />
                      <div className="flex flex-col gap-1.5">
                        <Skeleton className="h-3 w-28" />
                        <Skeleton className="h-2.5 w-40" />
                      </div>
                    </div>
                  </TableCell>
                  <TableCell className="hidden md:table-cell">
                    <Skeleton className="h-5 w-16 rounded-md" />
                  </TableCell>
                  <TableCell className="hidden lg:table-cell">
                    <Skeleton className="ml-auto h-3 w-8" />
                  </TableCell>
                  <TableCell><Skeleton className="h-3 w-24" /></TableCell>
                  <TableCell><Skeleton className="h-5 w-14 rounded-full" /></TableCell>
                  <TableCell className="hidden sm:table-cell">
                    <Skeleton className="h-5 w-16 rounded-full" />
                  </TableCell>
                  <TableCell className="hidden xl:table-cell">
                    <Skeleton className="h-3 w-16" />
                  </TableCell>
                  <TableCell className="pr-4 text-right">
                    <Skeleton className="ml-auto size-7 rounded-md" />
                  </TableCell>
                </TableRow>
              ))
            ) : filtered.length === 0 ? (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={8} className="py-14 text-center whitespace-normal">
                  <div className="text-muted-foreground mx-auto flex max-w-xs flex-col items-center gap-2">
                    <span className="bg-muted grid size-11 place-items-center rounded-full">
                      <UsersIcon className="size-5" />
                    </span>
                    <p className="text-foreground text-sm font-medium">
                      {query ? t("noMatching") : t("noUsers")}
                    </p>
                    <p className="text-xs">
                      {query ? t("tryDifferent") : t("createFirst")}
                    </p>
                  </div>
                </TableCell>
              </TableRow>
            ) : (
              filtered.map((u) => {
                const isSelf = u.id === currentUserId;
                const isAdmin = u.role === "admin";
                const menu = {
                  user: u,
                  isSelf,
                  mailReady,
                  onEdit: openEdit,
                  onRecalculate: handleRecalculate,
                  onSetRole: handleSetRole,
                  onBanToggle: handleBanToggle,
                  onRemove: openRemove,
                  onVerifyEmail: handleVerifyEmail,
                  onResendInvite: handleResendInvite,
                  onSendVerification: handleSendVerification,
                };
                return (
                  // Right-click anywhere on the row opens the same menu as the
                  // three-dot button, matching the files view.
                  <UserRowContextMenu key={u.id} {...menu}>
                    <TableRow>
                      <TableCell className="pl-4">
                        <div className="flex items-center gap-3">
                          <Avatar className="size-8">
                            {u.avatar ? <AvatarImage src={u.avatar} alt="" /> : null}
                            <AvatarFallback className="text-xs">{initialsOf(u)}</AvatarFallback>
                          </Avatar>
                          <div className="min-w-0">
                            <div className="flex items-center gap-1.5">
                              <span className="truncate text-sm font-medium">
                                {u.name || t("unnamed")}
                              </span>
                              {isSelf && (
                                <Badge variant="secondary" className="h-4 px-1.5 text-[0.65rem]">
                                  {t("you")}
                                </Badge>
                              )}
                            </div>
                            <span className="text-muted-foreground block truncate text-xs">
                              {u.email}
                            </span>
                          </div>
                        </div>
                      </TableCell>
                      <TableCell className="hidden md:table-cell">
                        {u.tag ? (
                          <code className="bg-muted text-foreground/80 rounded-md px-1.5 py-0.5 font-mono text-xs">
                            /{u.tag}
                          </code>
                        ) : (
                          <span className="text-muted-foreground text-xs">-</span>
                        )}
                      </TableCell>
                      <TableCell className="hidden text-right text-sm tabular-nums lg:table-cell">
                        {u.files.toLocaleString()}
                      </TableCell>
                      <TableCell>
                        <StorageCell used={u.usedQuota} max={u.maxQuota} />
                      </TableCell>
                      <TableCell>
                        {isAdmin ? (
                          <Badge className="gap-1 bg-violet-500/15 text-violet-500">
                            <ShieldCheck className="size-3" /> {t("roleAdmin")}
                          </Badge>
                        ) : (
                          <Badge variant="outline" className="text-muted-foreground">
                            {t("roleUser")}
                          </Badge>
                        )}
                      </TableCell>
                      <TableCell className="hidden sm:table-cell">
                        <div className="flex flex-wrap items-center gap-1.5">
                          {u.banned ? (
                            <Badge variant="destructive" className="gap-1">
                              <Ban className="size-3" /> {t("banned")}
                            </Badge>
                          ) : (
                            <span className="inline-flex items-center gap-1.5 text-xs text-emerald-500">
                              <span className="size-1.5 rounded-full bg-emerald-500" /> {t("active")}
                            </span>
                          )}
                          {/* Who cannot sign in yet, and why. An invitation that is
                              still outstanding is a different state from an address
                              that was never confirmed, and the fix differs too. */}
                          {u.invitePending ? (
                            <Badge
                              variant="outline"
                              className="gap-1 border-amber-500/30 text-amber-600 dark:text-amber-400"
                            >
                              <Send className="size-3" /> {t("invited")}
                            </Badge>
                          ) : !u.emailVerified ? (
                            <Badge variant="outline" className="text-muted-foreground gap-1">
                              <MailWarning className="size-3" /> {t("unconfirmed")}
                            </Badge>
                          ) : null}
                        </div>
                      </TableCell>
                      <TableCell className="text-muted-foreground hidden text-xs xl:table-cell">
                        {u.createdAt ? relativeTime(new Date(u.createdAt), locale) : "-"}
                      </TableCell>
                      <TableCell className="pr-4 text-right">
                        <UserRowDropdown
                          {...menu}
                          trigger={
                            <Button variant="ghost" size="icon-sm" disabled={busyId === u.id}>
                              {busyId === u.id ? (
                                <Loader2 className="size-4 animate-spin" />
                              ) : (
                                <MoreHorizontal className="size-4" />
                              )}
                              <span className="sr-only">{t("actionsFor", { email: u.email })}</span>
                            </Button>
                          }
                        />
                      </TableCell>
                    </TableRow>
                  </UserRowContextMenu>
                );
              })
            )}
          </TableBody>
        </Table>
      </div>

      {!loading && filtered.length > 0 && (
        <p className="text-muted-foreground text-xs">
          {t("showing", { filtered: filtered.length, total: users.length })}
        </p>
      )}

      {/* Create user dialog */}
      <Dialog open={createOpen} onOpenChange={(o) => !creating && setCreateOpen(o)}>
        {/* Scrolls on a phone: the two sign-in options make this dialog taller
            than the viewport on a small screen. */}
        <DialogContent className="max-h-[88svh] w-[calc(100vw-2rem)] min-w-0 overflow-y-auto sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{t("createTitle")}</DialogTitle>
            <DialogDescription>
              {t("createDesc")}
            </DialogDescription>
          </DialogHeader>
          <div className="flex flex-col gap-4 py-1">
            <div className="flex flex-col gap-2">
              <Label htmlFor="cu-name">{t("name")}</Label>
              <Input
                id="cu-name"
                placeholder={t("namePlaceholder")}
                value={form.name}
                onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
              />
            </div>
            <div className="flex flex-col gap-2">
              <Label htmlFor="cu-email">{t("email")}</Label>
              <Input
                id="cu-email"
                type="email"
                placeholder={t("emailPlaceholder")}
                value={form.email}
                onChange={(e) => setForm((f) => ({ ...f, email: e.target.value }))}
              />
            </div>
            {/* How the person gets in. Two answers, both legitimate: an install
                without mail has to hand a password over by other means. */}
            <div className="flex flex-col gap-2">
              <Label className="text-xs">{t("createModeQuestion")}</Label>
              <div className="grid gap-2 sm:grid-cols-2">
                <DispositionCard
                  selected={form.mode === "invite"}
                  onSelect={() => setForm((f) => ({ ...f, mode: "invite" }))}
                  icon={<Send className="size-4" />}
                  title={t("modeInviteTitle")}
                  description={mailReady ? t("modeInviteDesc") : t("modeInviteUnavailable")}
                  disabled={creating || !mailReady}
                />
                <DispositionCard
                  selected={form.mode === "password"}
                  onSelect={() => setForm((f) => ({ ...f, mode: "password" }))}
                  icon={<KeyRound className="size-4" />}
                  title={t("modePasswordTitle")}
                  description={t("modePasswordDesc")}
                  disabled={creating}
                />
              </div>
            </div>
            {form.mode === "password" ? (
              <div className="flex flex-col gap-2">
                <Label htmlFor="cu-password">{t("password")}</Label>
                <Input
                  id="cu-password"
                  type="password"
                  autoComplete="new-password"
                  placeholder={t("passwordPlaceholder")}
                  value={form.password}
                  onChange={(e) => setForm((f) => ({ ...f, password: e.target.value }))}
                />
                <p className="text-muted-foreground text-xs leading-relaxed">
                  {t("modePasswordHint")}
                </p>
              </div>
            ) : null}
            <div className="flex flex-col gap-2">
              <Label htmlFor="cu-role">{t("role")}</Label>
              <Select
                value={form.role}
                onValueChange={(v) => setForm((f) => ({ ...f, role: v }))}
              >
                <SelectTrigger id="cu-role" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="user">{t("roleUser")}</SelectItem>
                  <SelectItem value="admin">{t("roleAdmin")}</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>
          <DialogFooter>
            <DialogClose asChild>
              <Button variant="outline" disabled={creating}>
                {tc("cancel")}
              </Button>
            </DialogClose>
            <Button onClick={handleCreate} disabled={creating}>
              {creating ? <Loader2 className="size-4 animate-spin" /> : <UserPlus className="size-4" />}
              {creating ? t("creating") : t("createUser")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Edit account dialog — identity, address, security, storage, permissions */}
      <Dialog
        open={!!editTarget}
        onOpenChange={(o) => {
          if (saving || o) return;
          closeEdit();
        }}
      >
        <DialogContent className="max-h-[88svh] w-[calc(100vw-2rem)] min-w-0 gap-0 overflow-y-auto sm:max-w-2xl">
          <DialogHeader className="min-w-0">
            <DialogTitle>{t("editTitle")}</DialogTitle>
            <DialogDescription className="min-w-0 truncate">
              {editTarget ? t("editDesc", { email: editTarget.email }) : null}
            </DialogDescription>
          </DialogHeader>

          {detailLoading || !draft || !detail ? (
            <div className="flex flex-col gap-4 py-4">
              {Array.from({ length: 5 }).map((_, i) => (
                <div key={i} className="flex flex-col gap-2">
                  <Skeleton className="h-3 w-24" />
                  <Skeleton className="h-9 w-full rounded-md" />
                </div>
              ))}
              {!detailLoading && !detail && (
                <p className="text-muted-foreground text-sm">{t("removeReason.not-found")}</p>
              )}
            </div>
          ) : (
            <div className="flex flex-col gap-6 py-4">
              {/* ── Identity ─────────────────────────────────────────────── */}
              <Section icon={<IdCard className="size-4" />} title={t("sectionIdentity")}>
                <div className="grid gap-4 sm:grid-cols-2">
                  <Field htmlFor="eu-name" label={t("name")}>
                    <Input
                      id="eu-name"
                      value={draft.name}
                      autoComplete="off"
                      onChange={(e) => setDraft((d) => (d ? { ...d, name: e.target.value } : d))}
                    />
                  </Field>
                  <Field
                    htmlFor="eu-username"
                    label={t("usernameLabel")}
                    help={t("usernameHelp")}
                  >
                    <Input
                      id="eu-username"
                      value={draft.username}
                      spellCheck={false}
                      autoComplete="off"
                      className="font-mono"
                      placeholder={t("usernamePlaceholder")}
                      onChange={(e) => setDraft((d) => (d ? { ...d, username: e.target.value } : d))}
                    />
                  </Field>
                </div>

                <Field
                  htmlFor="eu-email"
                  label={t("email")}
                  help={detail.emailVerified ? t("emailHelp") : t("emailHelpUnverified")}
                >
                  <div className="relative">
                    <AtSign className="text-muted-foreground pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2" />
                    <Input
                      id="eu-email"
                      type="email"
                      className="pl-8"
                      value={draft.email}
                      spellCheck={false}
                      autoComplete="off"
                      onChange={(e) => setDraft((d) => (d ? { ...d, email: e.target.value } : d))}
                    />
                  </div>
                </Field>
              </Section>

              <Separator />

              {/* ── Public address ───────────────────────────────────────── */}
              <Section icon={<Hash className="size-4" />} title={t("sectionAddress")}>
                <Field htmlFor="eu-tag" label={t("tagLabel")} help={t("tagHelp")}>
                  <div className="flex items-center gap-2">
                    <span className="text-muted-foreground shrink-0 font-mono text-sm">/</span>
                    <Input
                      id="eu-tag"
                      value={draft.tag}
                      spellCheck={false}
                      autoComplete="off"
                      className="font-mono"
                      onChange={(e) => setDraft((d) => (d ? { ...d, tag: e.target.value } : d))}
                    />
                  </div>
                </Field>
              </Section>

              <Separator />

              {/* ── Security ─────────────────────────────────────────────── */}
              <Section icon={<KeyRound className="size-4" />} title={t("sectionSecurity")}>
                <Field
                  htmlFor="eu-password"
                  label={t("newPasswordLabel")}
                  help={detail.hasPassword ? t("newPasswordHelp") : t("newPasswordHelpNone")}
                >
                  <Input
                    id="eu-password"
                    type="password"
                    autoComplete="new-password"
                    placeholder={t("newPasswordPlaceholder")}
                    value={draft.password}
                    onChange={(e) => setDraft((d) => (d ? { ...d, password: e.target.value } : d))}
                  />
                </Field>

                <div className="grid gap-4 sm:grid-cols-2">
                  <Field htmlFor="eu-role" label={t("role")}>
                    <Select
                      value={draft.role}
                      disabled={editingSelf}
                      onValueChange={(v) =>
                        setDraft((d) => (d ? { ...d, role: v === "admin" ? "admin" : "user" } : d))
                      }
                    >
                      <SelectTrigger id="eu-role" className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="user">{t("roleUser")}</SelectItem>
                        <SelectItem value="admin">{t("roleAdmin")}</SelectItem>
                      </SelectContent>
                    </Select>
                  </Field>

                  <div className="flex flex-col gap-2">
                    <Label className="text-xs">{t("colStatus")}</Label>
                    <div className="ring-foreground/10 flex h-9 items-center justify-between gap-3 rounded-md px-3 ring-1">
                      <span
                        className={cn(
                          "flex items-center gap-1.5 text-sm",
                          draft.banned ? "text-destructive" : "text-emerald-500",
                        )}
                      >
                        {draft.banned ? <Ban className="size-3.5" /> : <span className="size-1.5 rounded-full bg-emerald-500" />}
                        {draft.banned ? t("banned") : t("active")}
                      </span>
                      <Switch
                        id="eu-banned"
                        aria-label={t("blockedLabel")}
                        checked={draft.banned}
                        disabled={editingSelf}
                        onCheckedChange={(c) => setDraft((d) => (d ? { ...d, banned: c } : d))}
                      />
                    </div>
                  </div>
                </div>

                {editingSelf && (
                  <p className="text-muted-foreground flex items-start gap-1.5 text-xs leading-relaxed">
                    <ShieldAlert className="mt-0.5 size-3.5 shrink-0 text-amber-500" />
                    {t("selfLockoutHint")}
                  </p>
                )}
              </Section>

              <Separator />

              {/* ── Storage ──────────────────────────────────────────────── */}
              <Section icon={<HardDrive className="size-4" />} title={t("sectionStorage")}>
                <div className="flex flex-col gap-2">
                  <div className="flex items-center justify-between gap-3">
                    <Label htmlFor="eu-quota" className="text-xs">{t("quotaLabel")}</Label>
                    <div className="flex items-center gap-2">
                      <Label
                        htmlFor="eu-unlimited"
                        className="text-muted-foreground text-xs font-normal"
                      >
                        {t("quotaUnlimited")}
                      </Label>
                      <Switch
                        id="eu-unlimited"
                        checked={draft.unlimited}
                        onCheckedChange={(c) =>
                          setDraft((d) => (d ? { ...d, unlimited: c } : d))
                        }
                      />
                    </div>
                  </div>
                  <Input
                    id="eu-quota"
                    value={draft.unlimited ? "" : draft.quota}
                    placeholder={draft.unlimited ? "∞" : t("quotaPlaceholder")}
                    disabled={draft.unlimited}
                    spellCheck={false}
                    autoComplete="off"
                    className={cn(draft.unlimited && "opacity-60")}
                    onChange={(e) => setDraft((d) => (d ? { ...d, quota: e.target.value } : d))}
                  />
                  <p className="text-muted-foreground text-xs leading-relaxed">
                    {t("quotaHelp", { used: humanFileSize(detail.usedQuota) })}
                  </p>
                </div>
              </Section>

              <Separator />

              {/* ── Permissions ──────────────────────────────────────────── */}
              <Section icon={<ShieldCheck className="size-4" />} title={t("sectionPermissions")}>
                <Toggle
                  id="eu-rootlinks"
                  icon={<Link2 className="size-4.5" />}
                  label={t("rootLinksLabel")}
                  description={t("rootLinksDesc")}
                  checked={rootLocked || draft.rootLinks}
                  disabled={rootLocked}
                  onCheckedChange={(c) => setDraft((d) => (d ? { ...d, rootLinks: c } : d))}
                  note={rootLocked ? tPerm("rootLinksAlwaysAdmin") : undefined}
                />
                <Toggle
                  id="eu-publicprofile"
                  icon={<Globe className="size-4.5" />}
                  label={t("publicProfileLabel")}
                  description={t("publicProfileDesc", { tag: draft.tag || detail.tag || "tag" })}
                  checked={draft.publicProfile}
                  onCheckedChange={(c) => setDraft((d) => (d ? { ...d, publicProfile: c } : d))}
                />
                <Toggle
                  id="eu-skiptag"
                  icon={<Hash className="size-4.5" />}
                  label={t("skipTagLabel")}
                  description={t("skipTagDesc")}
                  checked={draft.skipTagInUrl}
                  onCheckedChange={(c) => setDraft((d) => (d ? { ...d, skipTagInUrl: c } : d))}
                />
              </Section>

              {/* The named confirmation for the two changes that hand over
                  control of somebody's account. */}
              {awaitingConfirm && pendingPatch && (
                <div className="border-destructive/30 bg-destructive/5 flex items-start gap-2.5 rounded-lg border p-3">
                  <TriangleAlert className="text-destructive mt-0.5 size-4 shrink-0" />
                  <div className="min-w-0 text-xs leading-relaxed">
                    <p className="text-foreground font-medium">
                      {t("confirmSensitiveTitle", { name: detail.name || detail.email })}
                    </p>
                    <ul className="text-muted-foreground mt-1 list-disc space-y-0.5 pl-4">
                      {pendingPatch.email !== undefined && (
                        <li>{t("confirmEmail", { from: detail.email, to: pendingPatch.email })}</li>
                      )}
                      {pendingPatch.newPassword && (
                        <li>{editingSelf ? t("confirmPasswordSelf") : t("confirmPassword")}</li>
                      )}
                    </ul>
                  </div>
                </div>
              )}
            </div>
          )}

          <DialogFooter>
            <DialogClose asChild>
              <Button variant="outline" disabled={saving}>
                {tc("cancel")}
              </Button>
            </DialogClose>
            <Button
              onClick={saveEdits}
              disabled={saving || detailLoading || !detail || !pendingPatch}
              variant={awaitingConfirm ? "destructive" : "default"}
            >
              {saving ? <Loader2 className="size-4 animate-spin" /> : null}
              {saving
                ? t("savingChanges")
                : awaitingConfirm
                  ? t("confirmSensitiveCta")
                  : tc("save")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Remove account dialog — states the damage, then asks what happens to
          the files. Deleting an account used to silently take every byte the
          person had ever uploaded with it; now that is one of two explicit
          answers. */}
      <Dialog open={!!removeTarget} onOpenChange={(o) => !removing && !o && setRemoveTarget(null)}>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{t("removeTitle")}</DialogTitle>
            <DialogDescription>
              {t.rich("removeConfirm", {
                email: removeTarget?.email ?? "",
                b: (chunks) => <span className="text-foreground font-medium">{chunks}</span>,
              })}
            </DialogDescription>
          </DialogHeader>

          <div className="flex flex-col gap-4 py-1">
            {/* What the account holds */}
            <div className="ring-foreground/10 grid grid-cols-2 gap-x-4 gap-y-2 rounded-lg p-3 ring-1 sm:grid-cols-4">
              {impactLoading || !impact ? (
                Array.from({ length: 4 }).map((_, i) => (
                  <div key={i} className="flex flex-col gap-1.5">
                    <Skeleton className="h-2.5 w-14" />
                    <Skeleton className="h-4 w-12" />
                  </div>
                ))
              ) : (
                <>
                  <ImpactStat
                    icon={<Files className="size-3.5" />}
                    label={t("impactFiles")}
                    value={impact.files.toLocaleString()}
                  />
                  <ImpactStat
                    icon={<HardDrive className="size-3.5" />}
                    label={t("impactStorage")}
                    value={humanFileSize(impact.bytes)}
                  />
                  <ImpactStat
                    icon={<FolderOpen className="size-3.5" />}
                    label={t("impactFolders")}
                    value={impact.folders.toLocaleString()}
                  />
                  <ImpactStat
                    icon={<KeyRound className="size-3.5" />}
                    label={t("impactTokens")}
                    value={impact.tokens.toLocaleString()}
                  />
                </>
              )}
            </div>

            {/* The decision */}
            <div className="flex flex-col gap-2">
              <Label className="text-xs font-medium">{t("dispositionQuestion")}</Label>
              <div className="grid gap-2 sm:grid-cols-2">
                <DispositionCard
                  selected={disposition === "delete"}
                  onSelect={() => setDisposition("delete")}
                  icon={<Trash2 className="size-4" />}
                  title={t("dispDeleteTitle")}
                  description={t("dispDeleteDesc")}
                  disabled={removing}
                />
                <DispositionCard
                  selected={disposition === "keep"}
                  onSelect={() => setDisposition("keep")}
                  icon={<Ghost className="size-4" />}
                  title={t("dispKeepTitle")}
                  description={t("dispKeepDesc")}
                  disabled={removing}
                />
              </div>
            </div>

            <div className="border-destructive/30 bg-destructive/5 flex items-start gap-2.5 rounded-lg border p-3">
              <TriangleAlert className="text-destructive mt-0.5 size-4 shrink-0" />
              <p className="text-muted-foreground text-xs leading-relaxed">
                {disposition === "delete" ? t("dispDeleteWarning") : t("dispKeepWarning")}
              </p>
            </div>
          </div>

          <DialogFooter>
            <DialogClose asChild>
              <Button variant="outline" disabled={removing}>
                {tc("cancel")}
              </Button>
            </DialogClose>
            <Button variant="destructive" onClick={confirmRemove} disabled={removing}>
              {removing ? <Loader2 className="size-4 animate-spin" /> : <Trash2 className="size-4" />}
              {removing
                ? t("removing")
                : disposition === "delete"
                  ? t("removeWithFiles")
                  : t("removeKeepFiles")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

/** A titled group of fields in the edit dialog. */
function Section({
  icon,
  title,
  children,
}: {
  icon: React.ReactNode;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="flex flex-col gap-4">
      <h3 className="text-muted-foreground flex items-center gap-2 text-xs font-medium tracking-wide uppercase">
        {icon}
        {title}
      </h3>
      {children}
    </section>
  );
}

/** Label + control + optional help text, so every field is spaced identically. */
function Field({
  htmlFor,
  label,
  help,
  children,
}: {
  htmlFor: string;
  label: string;
  help?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex min-w-0 flex-col gap-2">
      <Label htmlFor={htmlFor} className="text-xs">
        {label}
      </Label>
      {children}
      {help && <p className="text-muted-foreground text-xs leading-relaxed">{help}</p>}
    </div>
  );
}

/** One permission row: icon, explanation, switch. */
function Toggle({
  id,
  icon,
  label,
  description,
  checked,
  disabled,
  onCheckedChange,
  note,
}: {
  id: string;
  icon: React.ReactNode;
  label: string;
  description: string;
  checked: boolean;
  disabled?: boolean;
  onCheckedChange: (checked: boolean) => void;
  note?: string;
}) {
  return (
    <div className="ring-foreground/10 flex flex-col gap-2 rounded-lg p-3 ring-1">
      <div className="flex items-center gap-3">
        <span className="bg-muted text-muted-foreground grid size-9 shrink-0 place-items-center rounded-lg">
          {icon}
        </span>
        <div className="min-w-0 flex-1">
          <Label htmlFor={id} className="font-medium">
            {label}
          </Label>
          <p className="text-muted-foreground mt-0.5 text-xs leading-relaxed">{description}</p>
        </div>
        <Switch
          id={id}
          checked={checked}
          disabled={disabled}
          aria-readonly={disabled}
          onCheckedChange={onCheckedChange}
        />
      </div>
      {note && (
        <p className="text-muted-foreground flex items-start gap-1.5 pl-12 text-xs leading-relaxed">
          <ShieldCheck className="mt-0.5 size-3.5 shrink-0 text-violet-500" />
          {note}
        </p>
      )}
    </div>
  );
}

/** One number in the "what this account holds" strip. */
function ImpactStat({
  icon,
  label,
  value,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
}) {
  return (
    <div className="min-w-0">
      <span className="text-muted-foreground flex items-center gap-1 text-[0.7rem]">
        {icon}
        <span className="truncate">{label}</span>
      </span>
      <span className="mt-0.5 block text-sm font-medium tabular-nums">{value}</span>
    </div>
  );
}

/**
 * One of the two answers to "what happens to the files". A pair of large
 * targets rather than a select: the choice is the point of the dialog, and it
 * has to be impossible to submit without having looked at it.
 */
function DispositionCard({
  selected,
  onSelect,
  icon,
  title,
  description,
  disabled,
}: {
  selected: boolean;
  onSelect: () => void;
  icon: React.ReactNode;
  title: string;
  description: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      disabled={disabled}
      onClick={onSelect}
      className={cn(
        "flex flex-col gap-1 rounded-lg border p-3 text-left transition-colors disabled:opacity-60",
        selected
          ? "border-primary bg-primary/5"
          : "border-foreground/10 hover:bg-muted/50",
      )}
    >
      <span
        className={cn(
          "flex items-center gap-2 text-sm font-medium",
          selected ? "text-primary" : "text-foreground",
        )}
      >
        {icon}
        {title}
      </span>
      <span className="text-muted-foreground text-xs leading-relaxed">{description}</span>
    </button>
  );
}
