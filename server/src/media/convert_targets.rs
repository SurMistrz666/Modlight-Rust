//! Port of `lib/media/convert-targets.ts` - PARTIAL: only the target
//! vocabulary `transform_cost` is written in. The menu rules
//! (`convertTargetsFor`, `canConvert`, `keepsAnimation`, ...) need
//! `lib/media/mime.ts` and arrive with the media agent's port.

/// `ConvertTarget`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConvertTarget {
    Pdf,
    Png,
    Jpg,
    Webp,
    Avif,
}

impl ConvertTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            ConvertTarget::Pdf => "pdf",
            ConvertTarget::Png => "png",
            ConvertTarget::Jpg => "jpg",
            ConvertTarget::Webp => "webp",
            ConvertTarget::Avif => "avif",
        }
    }
}

/// Every target, in menu order. There is no "jpeg" alias: it was in the type but
/// never in this list, so `isConvertTarget("jpeg")` was false and the encoder's
/// `case "jpeg"` was unreachable code pretending to be a supported format.
pub const IMAGE_TARGETS: [ConvertTarget; 5] =
    [ConvertTarget::Pdf, ConvertTarget::Png, ConvertTarget::Jpg, ConvertTarget::Webp, ConvertTarget::Avif];
