//! Server actions (`app/actions/*.ts`), served over RPC ([`crate::rpc`]).
//!
//! One Rust module per upstream file, same name in snake_case
//! (`app/actions/shortlink-unlock.ts` -> `shortlink_unlock.rs`), each exposing
//! `pub fn register(reg: &mut RpcRegistry)` that registers every exported
//! function under the upstream module and export names:
//! `reg.register("shortlink-unlock", "unlockShortlink", unlock_shortlink)`.

pub mod tokens;

use crate::rpc::RpcRegistry;

/// Every action module. ONE LINE PER MODULE.
pub fn register_all(reg: &mut RpcRegistry) {
    tokens::register(reg); // app/actions/tokens.ts
}
