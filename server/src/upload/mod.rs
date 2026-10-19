//! `lib/upload/*`: the raw upload paths of server.ts (tus, ShareX) and their
//! helpers (free space, finalize, folder targets, duplicate checks).

pub mod busboy;
pub mod disk;
pub mod duplicate_check;
pub mod duplicate_lookup;
pub mod finalize;
pub mod folder_target;
pub mod sharex;
pub mod tus;
pub mod tus_server;
