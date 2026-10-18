"use client";

import {
  Ban,
  Calculator,
  FolderOpen,
  MailCheck,
  Send,
  ShieldCheck,
  ShieldOff,
  SlidersHorizontal,
  Trash2,
  Undo2,
} from "lucide-react";
import { useTranslations } from "next-intl";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import type { AdminUserRow } from "@/lib/admin-users";

/**
 * The actions available on one account, declared ONCE.
 *
 * They are reachable two ways: the three-dot button at the end of the row, and
 * a right-click anywhere on the row. Both render this list, because a context
 * menu that offers a different set from the button it mirrors is a second,
 * accidental product surface.
 */

export interface UserRowHandlers {
  onEdit: (u: AdminUserRow) => void;
  onRecalculate: (u: AdminUserRow) => void;
  onSetRole: (u: AdminUserRow, role: "admin" | "user") => void;
  onBanToggle: (u: AdminUserRow) => void;
  onRemove: (u: AdminUserRow) => void;
  onVerifyEmail: (u: AdminUserRow) => void;
  onResendInvite: (u: AdminUserRow) => void;
  onSendVerification: (u: AdminUserRow) => void;
}

export interface UserRowMenuProps extends UserRowHandlers {
  user: AdminUserRow;
  /** The signed-in administrator, who may not demote, ban or delete themselves. */
  isSelf: boolean;
  /** Whether outgoing mail is configured; the two "send" entries need it. */
  mailReady: boolean;
}

interface MenuItemProps {
  children: React.ReactNode;
  onSelect?: () => void;
  variant?: "default" | "destructive";
  disabled?: boolean;
  asChild?: boolean;
}

interface MenuParts {
  Item: React.ComponentType<MenuItemProps>;
  Separator: React.ComponentType;
  Label: React.ComponentType<{ children: React.ReactNode }>;
}

const DROPDOWN_PARTS: MenuParts = {
  Item: (p) => <DropdownMenuItem {...p} />,
  Separator: () => <DropdownMenuSeparator />,
  Label: ({ children }) => (
    <DropdownMenuLabel className="truncate font-normal">{children}</DropdownMenuLabel>
  ),
};

const CONTEXT_PARTS: MenuParts = {
  Item: (p) => <ContextMenuItem {...p} />,
  Separator: () => <ContextMenuSeparator />,
  Label: ({ children }) => <ContextMenuLabel className="truncate">{children}</ContextMenuLabel>,
};

/** Where an administrator reviews this account's library. */
export function userFilesHref(userId: string): string {
  return `/dashboard/admin/users/${userId}/files`;
}

function UserMenuItems({
  parts,
  user,
  isSelf,
  mailReady,
  onEdit,
  onRecalculate,
  onSetRole,
  onBanToggle,
  onRemove,
  onVerifyEmail,
  onResendInvite,
  onSendVerification,
}: UserRowMenuProps & { parts: MenuParts }) {
  const t = useTranslations("admin.users");
  const { Item, Separator, Label } = parts;
  const isAdmin = user.role === "admin";

  return (
    <>
      <Label>{user.email}</Label>
      <Separator />

      <Item onSelect={() => onEdit(user)}>
        <SlidersHorizontal /> {t("editAccount")}
      </Item>
      <Item asChild>
        {/* A new tab on purpose: reviewing somebody's files is a detour, and the
            administrator normally wants their place in the user list back. */}
        <a href={userFilesHref(user.id)} target="_blank" rel="noopener noreferrer">
          <FolderOpen /> {t("viewFiles")}
        </a>
      </Item>
      <Item onSelect={() => onRecalculate(user)}>
        <Calculator /> {t("recalc")}
      </Item>

      {!user.emailVerified || user.invitePending ? (
        <>
          <Separator />
          {!user.emailVerified ? (
            <Item onSelect={() => onVerifyEmail(user)}>
              <MailCheck /> {t("markVerified")}
            </Item>
          ) : null}
          {user.invitePending ? (
            <Item disabled={!mailReady} onSelect={() => onResendInvite(user)}>
              <Send /> {t("resendInvite")}
            </Item>
          ) : !user.emailVerified ? (
            <Item disabled={!mailReady} onSelect={() => onSendVerification(user)}>
              <Send /> {t("sendVerification")}
            </Item>
          ) : null}
        </>
      ) : null}

      <Separator />

      {isAdmin ? (
        <Item disabled={isSelf} onSelect={() => onSetRole(user, "user")}>
          <ShieldOff /> {t("demote")}
        </Item>
      ) : (
        <Item onSelect={() => onSetRole(user, "admin")}>
          <ShieldCheck /> {t("makeAdmin")}
        </Item>
      )}
      <Item disabled={isSelf} onSelect={() => onBanToggle(user)}>
        {user.banned ? <Undo2 /> : <Ban />}
        {user.banned ? t("unban") : t("ban")}
      </Item>

      <Separator />

      <Item variant="destructive" disabled={isSelf} onSelect={() => onRemove(user)}>
        <Trash2 /> {t("remove")}
      </Item>
    </>
  );
}

/** The three-dot button at the end of a row. */
export function UserRowDropdown({
  trigger,
  ...props
}: UserRowMenuProps & { trigger: React.ReactNode }) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>{trigger}</DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        <UserMenuItems parts={DROPDOWN_PARTS} {...props} />
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** The same list, opened by right-clicking the row itself. */
export function UserRowContextMenu({
  children,
  ...props
}: UserRowMenuProps & { children: React.ReactNode }) {
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>{children}</ContextMenuTrigger>
      <ContextMenuContent className="w-56">
        <UserMenuItems parts={CONTEXT_PARTS} {...props} />
      </ContextMenuContent>
    </ContextMenu>
  );
}
