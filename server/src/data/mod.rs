//! `lib/data/*`: database reads and writes behind the pages, actions and
//! route handlers. Accounts (`sessions`, `user_prefs`, `admin`) came with the
//! srv-auth port; the library modules (folders, uploads, duplicates,
//! upload_query, admin_files) with srv-library, which reconciled the two into
//! one implementation per upstream function.

pub mod admin;
pub mod admin_disk;
pub mod admin_files;
pub mod duplicates;
pub mod folders;
pub mod sessions;
pub mod upload_query;
pub mod uploads;
pub mod user_prefs;
