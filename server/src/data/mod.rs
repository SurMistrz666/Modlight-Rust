//! `lib/data/*`: database reads and writes behind the pages and actions.
//!
//! The srv-auth port added only what the account pages and the `(app)` layout
//! need from modules that belong to the library port (folders, uploads,
//! duplicates): each such function names its upstream origin, so the two
//! ports can be reconciled.

pub mod duplicates;
pub mod folders;
pub mod uploads;
pub mod user_prefs;
