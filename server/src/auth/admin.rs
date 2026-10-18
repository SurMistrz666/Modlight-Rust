//! The admin plugin (`admin()`): roles, bans, sessions of other users. NOT
//! IMPLEMENTED YET: the auth agent owns this.
//!
//! Upstream: better-auth 1.6.13 `plugins/admin/` (`admin.mjs`, `routes.mjs`).
//! Server-side calls Modlight makes (app/actions/admin.ts, lib/admin-users.ts):
//! `createUser`, `removeUser`, `setRole`, `banUser`, `unbanUser`,
//! `listSessions`, `revokeSession`, `revokeUserSessions`,
//! `revokeOtherSessions`. Each needs the same database writes as the plugin.
//!
//! Already ported: the session-creation ban hook,
//! [`super::session::check_ban_before_session_create`], and the default
//! `bannedUserMessage` ([`super::session::BANNED_USER_MESSAGE`]).
//! The plugin's `/list-sessions` after-hook hides impersonation sessions.
