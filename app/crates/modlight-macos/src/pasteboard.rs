//! The picture on the general pasteboard as encoded bytes, for the app to decode under
//! its own limits (`modlight_core::image_decode`). arboard decodes it itself, without
//! any limit of ours.

/// The PNG or TIFF bytes on the pasteboard, when it holds a picture.
pub fn image_bytes() -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        mac::image_bytes()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypePNG, NSPasteboardTypeTIFF};

    pub fn image_bytes() -> Option<Vec<u8>> {
        autoreleasepool(|_| {
            let pasteboard = NSPasteboard::generalPasteboard();
            // SAFETY: the type constants are AppKit's own.
            let (png, tiff) = unsafe { (NSPasteboardTypePNG, NSPasteboardTypeTIFF) };
            // A type the pasteboard holds as it is comes first: asking for TIFF when only
            // PNG is there has the system convert it, decoding it with no limit of ours.
            if let Some(types) = pasteboard.types() {
                for kind in [png, tiff] {
                    if types.iter().any(|t| &*t == kind)
                        && let Some(data) = pasteboard.dataForType(kind)
                    {
                        return Some(data.to_vec());
                    }
                }
            }
            // Anything else the system can turn into TIFF (what arboard asked for).
            pasteboard.dataForType(tiff).map(|data| data.to_vec())
        })
    }
}
