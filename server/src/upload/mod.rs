//! `lib/upload/*`: the raw upload paths of server.ts (tus, ShareX) and their
//! helpers. NOT PORTED YET apart from the tus sweep hook; the uploads agent
//! adds `sharex.rs`, `disk.rs`, `finalize.rs`, ... here and replaces the
//! placeholder in [`crate::http::raw_upload_router`].

pub mod tus;
