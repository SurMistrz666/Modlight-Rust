//! Server actions (`app/actions/*.ts`), served over RPC ([`crate::rpc`]).
//!
//! One Rust module per upstream file, same name in snake_case
//! (`app/actions/shortlink-unlock.ts` -> `shortlink_unlock.rs`), each exposing
//! `pub fn register(reg: &mut RpcRegistry)` that registers every exported
//! function under the upstream module and export names:
//! `reg.register("shortlink-unlock", "unlockShortlink", unlock_shortlink)`.

pub mod admin_storage;
pub mod duplicates;
pub mod folders;
pub mod js_value;
pub mod pastes;
pub mod shortlink_unlock;
pub mod shortlinks;
pub mod tokens;
pub mod uploads;

use crate::rpc::RpcRegistry;

/// Every action module. ONE LINE PER MODULE.
pub fn register_all(reg: &mut RpcRegistry) {
    tokens::register(reg); // app/actions/tokens.ts
    uploads::register(reg); // app/actions/uploads.ts
    folders::register(reg); // app/actions/folders.ts
    duplicates::register(reg); // app/actions/duplicates.ts
    shortlinks::register(reg); // app/actions/shortlinks.ts
    shortlink_unlock::register(reg); // app/actions/shortlink-unlock.ts
    pastes::register(reg); // app/actions/pastes.ts
    admin_storage::register(reg); // app/actions/admin.ts (orphans, account deletion, maintenance)
}
