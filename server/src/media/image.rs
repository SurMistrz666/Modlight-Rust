//! PLACEHOLDER for `lib/media/image.ts` until the media helpers branch
//! (srv-library-media: libvips binding, probe, blurhash) is merged; that
//! branch's module replaces this file. Same public signature.

/// `ImageMeta`
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageMeta {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub blurhash: Option<String>,
    pub frames: Option<u32>,
}

/// Best-effort image dimensions, frame count and blurhash. Never fails.
pub async fn image_meta_from_buffer(_buf: bytes::Bytes) -> ImageMeta {
    ImageMeta::default()
}
