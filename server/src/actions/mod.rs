//! Server actions (`app/actions/*.ts`), served over RPC ([`crate::rpc`]).
//!
//! One Rust module per upstream file, same name in snake_case
//! (`app/actions/shortlink-unlock.ts` -> `shortlink_unlock.rs`), each exposing
//! `pub fn register(reg: &mut RpcRegistry)` that registers every exported
//! function under the upstream module and export names:
//! `reg.register("shortlink-unlock", "unlockShortlink", unlock_shortlink)`.

pub mod authorize;
pub mod mail;
pub mod registration;
pub mod releases;
pub mod tokens;

use crate::rpc::RpcRegistry;

/// Every action module. ONE LINE PER MODULE.
pub fn register_all(reg: &mut RpcRegistry) {
    releases::register(reg); // app/actions/releases.ts
    authorize::register(reg); // app/actions/authorize.ts
    mail::register(reg); // app/actions/mail.ts
    registration::register(reg); // app/actions/registration.ts
    tokens::register(reg); // app/actions/tokens.ts
}
