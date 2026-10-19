//! Port of `lib/releases.ts`: the server-guarded view of the release channel.
//!
//! Everything lives in [`crate::releases_core`]; upstream keeps this module so
//! the app has the `server-only` guard (a release manifest reads settings and
//! the uploads table, and must never be pulled into a client bundle) while a
//! bundled script can import the core directly. Rust has no client bundle, so
//! this is a plain re-export kept for the one-module-per-upstream-file map.

pub use crate::releases_core::*;
