//! `lib/media/*`: what a stored file is (type sniffing, MIME rules), how its
//! links are spelled, and the native tooling (libvips, ffmpeg) that reads it.

pub mod abort;
pub mod actions;
pub mod av_cost;
pub mod av_meta;
pub mod av_probe;
pub mod av_targets;
pub mod av_workspace;
pub mod avatar;
pub mod avif_brand;
pub mod blurhash;
pub mod bots;
pub mod card;
pub mod convert_targets;
pub mod derived_cache;
pub mod download_menu;
pub mod embed;
pub mod ffmpeg;
pub mod image;
pub mod link_formats;
pub mod mime;
pub mod motion_preview;
pub mod part_a;
pub mod paste_type;
pub mod paths;
pub mod playback_health;
pub mod probe;
pub mod render_ticket;
pub mod serve;
pub mod sharp_runtime;
pub mod sniff;
pub mod transform_cost;
pub mod transform_queue;
pub mod urls;
pub mod vips;
