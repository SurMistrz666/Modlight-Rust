//! `lib/data/*`: database reads and writes behind the pages, actions and
//! route handlers.
//!
//! `user_prefs` is the accounts port's module (srv-auth branch), carried here
//! verbatim because the library code reads the same preferences.

pub mod admin_disk;
pub mod duplicates;
pub mod folders;
pub mod upload_query;
pub mod uploads;
pub mod user_prefs;
