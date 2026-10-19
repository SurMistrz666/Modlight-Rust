//! Port of `lib/media/probe.ts`: can libvips open this, and how big is it
//! really?
//!
//! Split out of convert.ts so the thumbnail and metadata paths can share it. They
//! used to open buffers with a bare `sharp(buf)` and therefore inherited none of
//! the things this function knows: the `avis` brand workaround, the difference
//! between a frame and a stacked strip, and the fact that "libvips cannot read
//! this" is an answer rather than a crash.

use bytes::Bytes;

use crate::media::avif_brand::with_still_avif_brand;
use crate::media::vips::{self, FailOn, Metadata, Sharp, VipsError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageProbe {
    /// The buffer to hand libvips - possibly a patched copy, see avif_brand.rs.
    pub input: Bytes,
    pub width: u32,
    /// Height of ONE frame (sharp's `pageHeight`), not of the stacked strip.
    pub height: u32,
    pub pages: u32,
    pub animated: bool,
    /// Pixels in one frame, and in the whole sequence.
    pub frame_pixels: u64,
    pub total_pixels: u64,
    /// True when only the cover frame is reachable, so no target can keep the
    /// animation however capable the encoder is.
    pub cover_only: bool,
}

fn read_metadata(buf: &Bytes) -> Option<Metadata> {
    Sharp::new(buf.clone()).fail_on(FailOn::None).limit_input_pixels(None).metadata_blocking().ok()
}

/// Returns `None` when libvips cannot decode the file. Callers answer 415 with
/// that, rather than the catch-all 500 a thrown encoder produces: "we cannot
/// convert this format" is a fact the user can act on, "conversion failed" is not.
pub async fn probe_image(buf: Bytes) -> Option<ImageProbe> {
    vips::run_blocking(move || Ok::<_, VipsError>(probe_image_blocking(&buf))).await.ok().flatten()
}

/// [`probe_image`] for code already on the blocking pool.
pub fn probe_image_blocking(buf: &Bytes) -> Option<ImageProbe> {
    let mut input = buf.clone();
    let mut cover_only = false;
    let mut meta = read_metadata(buf);

    if meta.is_none() {
        let patched = with_still_avif_brand(buf);
        let patched_meta = patched.as_ref().and_then(read_metadata);
        if let (Some(patched), Some(patched_meta)) = (patched, patched_meta) {
            input = patched;
            meta = Some(patched_meta);
            cover_only = true;
        }
    }

    let meta = meta?;
    if meta.width == 0 || meta.height == 0 {
        return None;
    }

    let pages = meta.pages.filter(|&p| p > 0).unwrap_or(1);
    let height = meta.page_height.unwrap_or(meta.height);
    let frame_pixels = meta.width as u64 * height as u64;

    Some(ImageProbe {
        input,
        width: meta.width,
        height,
        pages,
        animated: pages > 1 && !cover_only,
        frame_pixels,
        total_pixels: frame_pixels * pages as u64,
        cover_only,
    })
}
