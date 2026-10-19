//! `lib/media/*`: what a stored file is (type sniffing, MIME rules), how its
//! links are spelled, and the native tooling (libvips, ffmpeg) that reads it.

pub mod actions;
pub mod av_meta;
pub mod derived_cache;
pub mod image;
pub mod mime;
pub mod motion_preview;
pub mod paste_type;
pub mod paths;
pub mod sniff;
pub mod urls;
