//! My space without the window: paging, the fetch order of previews, the memory
//! ceilings, and what a file and a folder say about themselves.

pub mod budget;
pub mod caches;
pub mod convert;
pub mod files;
pub mod folders;
pub mod pager;
pub mod queue;

pub use budget::{ThumbnailBudget, motion};
pub use caches::{MotionCache, ThumbnailCache};
pub use files::{CloudFile, LinkFormat, TypeFamily};
pub use folders::{CloudFolder, Crumb, FolderKind, LIBRARY_VIEWS, LibraryView, crumbs, roll_up};
pub use pager::Pager;
pub use queue::{ThumbnailQueue, visible_band};

/// Fetch slots for previews.
pub const THUMBNAIL_SLOTS: usize = 4;

/// How long a folder tile glows after a file lands in it.
pub const FLASH_MILLIS: u64 = 1600;

/// Previews are decoded no wider than this (the 180 px card at 1.6x).
pub const THUMBNAIL_DECODE_WIDTH: u32 = 288;

/// Decoded previews kept at once.
pub const THUMBNAIL_BUDGET_BYTES: u64 = 64 * 1024 * 1024;

/// Cards either side of the visible run that keep a picture.
pub const VISIBLE_BAND_MARGIN: usize = 40;

/// How long the pointer must rest on a card before it starts moving.
pub const MOTION_DWELL_MILLIS: u64 = 320;

/// Frame delays of animated previews: unknown, and the clamp.
pub const MOTION_FALLBACK_DELAY_MS: u32 = 120;
pub const MOTION_MIN_DELAY_MS: u32 = 40;
pub const MOTION_MAX_DELAY_MS: u32 = 2000;

/// The details drawer's preview request edge and decode width.
pub const DETAILS_PREVIEW_EDGE: u32 = 1024;

/// A thumbnail URL asking for the drawer's larger rendering (`ModlightDetailsPreview.Enlarge`).
pub fn enlarge_thumbnail_url(url: &str) -> String {
    if url.is_empty() {
        return String::new();
    }
    let edge = DETAILS_PREVIEW_EDGE;
    let Some(mark) = url.find('?') else {
        return format!("{url}?w={edge}&h={edge}");
    };
    let mut kept = Vec::new();
    let (mut saw_width, mut saw_height) = (false, false);
    for part in url[mark + 1..].split('&') {
        if part.starts_with("w=") {
            saw_width = true;
            kept.push(format!("w={edge}"));
        } else if part.starts_with("h=") {
            saw_height = true;
            kept.push(format!("h={edge}"));
        } else if !part.is_empty() {
            kept.push(part.to_string());
        }
    }
    if !saw_width {
        kept.push(format!("w={edge}"));
    }
    if !saw_height {
        kept.push(format!("h={edge}"));
    }
    format!("{}?{}", &url[..mark], kept.join("&"))
}

/// A frame delay as the motion player uses it.
pub fn clamp_frame_delay(duration_ms: i64) -> u32 {
    if duration_ms <= 0 {
        MOTION_FALLBACK_DELAY_MS
    } else {
        (duration_ms as u32).clamp(MOTION_MIN_DELAY_MS, MOTION_MAX_DELAY_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_drawer_asks_for_a_larger_preview() {
        assert_eq!(
            enlarge_thumbnail_url("https://h/x/raw?w=480"),
            "https://h/x/raw?w=1024&h=1024"
        );
        assert_eq!(
            enlarge_thumbnail_url("https://h/x/raw?w=480&h=480&fit=cover"),
            "https://h/x/raw?w=1024&h=1024&fit=cover"
        );
        assert_eq!(
            enlarge_thumbnail_url("https://h/x/raw"),
            "https://h/x/raw?w=1024&h=1024"
        );
        assert_eq!(enlarge_thumbnail_url(""), "");
        assert_eq!(clamp_frame_delay(0), 120);
        assert_eq!(clamp_frame_delay(10), 40);
        assert_eq!(clamp_frame_delay(83), 83);
        assert_eq!(clamp_frame_delay(9000), 2000);
    }
}
