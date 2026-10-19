//! Server actions (`app/actions/*.ts`), served over RPC ([`crate::rpc`]).
//!
//! One Rust module per upstream file, same name in snake_case
//! (`app/actions/shortlink-unlock.ts` -> `shortlink_unlock.rs`), each exposing
//! `pub fn register(reg: &mut RpcRegistry)` that registers every exported
//! function under the upstream module and export names:
//! `reg.register("shortlink-unlock", "unlockShortlink", unlock_shortlink)`.

pub mod account;
pub mod admin;
pub mod admin_library;
pub mod authorize;
pub mod avatar;
pub mod duplicates;
pub mod folders;
pub mod invite;
pub mod login_identifier;
pub mod mail;
pub mod pastes;
pub mod registration;
pub mod releases;
pub mod shortlink_unlock;
pub mod shortlinks;
pub mod tokens;
pub mod unlock;
pub mod uploads;

use crate::rpc::RpcRegistry;

/// Every action module. ONE LINE PER MODULE.
pub fn register_all(reg: &mut RpcRegistry) {
    releases::register(reg); // app/actions/releases.ts
    authorize::register(reg); // app/actions/authorize.ts
    mail::register(reg); // app/actions/mail.ts
    registration::register(reg); // app/actions/registration.ts
    account::register(reg); // app/actions/account.ts
    admin::register(reg); // app/actions/admin.ts (account administration)
    invite::register(reg); // app/actions/invite.ts
    login_identifier::register(reg); // app/actions/login-identifier.ts
    avatar::register(reg); // app/actions/avatar.ts
    tokens::register(reg); // app/actions/tokens.ts
    uploads::register(reg); // app/actions/uploads.ts
    folders::register(reg); // app/actions/folders.ts
    duplicates::register(reg); // app/actions/duplicates.ts
    shortlinks::register(reg); // app/actions/shortlinks.ts
    shortlink_unlock::register(reg); // app/actions/shortlink-unlock.ts
    pastes::register(reg); // app/actions/pastes.ts
    admin_library::register(reg); // app/actions/admin.ts (orphans, account deletion, maintenance)
    unlock::register(reg); // app/actions/unlock.ts
}
