//! What the image editor asks of the system, none of which needs a permission:
//!
//! - the cursors a cursor annotation shows (`NSCursor`, where upstream drew Windows'
//!   cursors with `WindowsCursorBitmapRenderer`);
//! - the desktop picture behind the Wallpaper background (`NSWorkspace`, where upstream
//!   asked System Events through `osascript`, which needs the Automation permission);
//! - pictures the `image` crate cannot read (HEIC wallpapers), through `NSImage`.

use std::path::{Path, PathBuf};

use image::RgbaImage;

/// How the desktop picture is laid out (`NSWorkspaceDesktopImageScalingKey` and
/// `NSWorkspaceDesktopImageAllowClippingKey`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WallpaperOptions {
    /// `NSImageScaling`: 0 proportionally down, 1 axes independently, 2 none,
    /// 3 proportionally up or down.
    pub scaling: i64,
    pub allow_clipping: bool,
}

/// The cursor a `CursorType` name stands for, as a picture at the screen's 2x size.
pub fn cursor_image(cursor_type: &str) -> Option<RgbaImage> {
    #[cfg(target_os = "macos")]
    {
        mac::cursor_image(cursor_type)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cursor_type;
        None
    }
}

/// The main screen's desktop picture and its layout.
pub fn desktop_wallpaper() -> Option<(PathBuf, WallpaperOptions)> {
    #[cfg(target_os = "macos")]
    {
        mac::desktop_wallpaper()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// A picture file read by `NSImage` (HEIC and every other format the system knows).
pub fn load_image(path: &Path) -> Option<RgbaImage> {
    #[cfg(target_os = "macos")]
    {
        mac::load_image(path)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        None
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::path::{Path, PathBuf};

    use image::RgbaImage;
    use objc2::rc::Retained;
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSCursor, NSImage, NSScreen, NSWorkspace,
        NSWorkspaceDesktopImageAllowClippingKey, NSWorkspaceDesktopImageScalingKey,
    };
    use objc2_foundation::{NSDictionary, NSNumber, NSRect, NSSize, NSString};

    use super::WallpaperOptions;

    /// The PNG bytes of an `NSImage`'s representation for `scale` (1 or 2), decoded.
    fn image_pixels(image: &NSImage, scale: f64) -> Option<RgbaImage> {
        let size: NSSize = image.size();
        let mut rect = NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            NSSize::new(size.width * scale, size.height * scale),
        );
        // SAFETY: a valid rect pointer and no hints.
        let cg = unsafe { image.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
        let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
        let properties = NSDictionary::new();
        // SAFETY: an empty properties dictionary.
        let data = unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
        }?;
        let bytes = data.to_vec();
        image::load_from_memory(&bytes).ok().map(|i| i.to_rgba8())
    }

    // The pre-macOS 15 resize cursors still draw; their successors only exist there.
    #[allow(deprecated)]
    pub fn cursor_image(cursor_type: &str) -> Option<RgbaImage> {
        // AppKit's cursors are main-thread objects of a running application (the
        // shared one exists in the app; asking for it makes sure).
        let mtm = MainThreadMarker::new()?;
        let _ = objc2_app_kit::NSApplication::sharedApplication(mtm);
        let cursor: Retained<NSCursor> = match cursor_type {
            "Cross" => NSCursor::crosshairCursor(),
            "Hand" => NSCursor::pointingHandCursor(),
            "IBeam" => NSCursor::IBeamCursor(),
            "No" => NSCursor::operationNotAllowedCursor(),
            "SizeAll" | "NoMove2D" => NSCursor::openHandCursor(),
            "SizeNS" | "NoMoveVert" | "PanNorth" | "PanSouth" | "HSplit" => {
                NSCursor::resizeUpDownCursor()
            }
            "SizeWE" | "NoMoveHoriz" | "PanEast" | "PanWest" | "VSplit" => {
                NSCursor::resizeLeftRightCursor()
            }
            "UpArrow" => NSCursor::resizeUpCursor(),
            "SizeNWSE" | "PanNW" | "PanSE" | "SizeNESW" | "PanNE" | "PanSW" => {
                diagonal_cursor(cursor_type).unwrap_or_else(NSCursor::crosshairCursor)
            }
            // Arrow, Default, AppStarting, Help, WaitCursor: macOS has one arrow.
            _ => NSCursor::arrowCursor(),
        };
        // The cursor's canvas has room to its right and below; keep the top-left (where
        // the hot spot is measured from) and drop the empty rest.
        image_pixels(&cursor.image(), 2.0).map(|image| trim_right_bottom(&image))
    }

    fn trim_right_bottom(image: &RgbaImage) -> RgbaImage {
        let (mut w, mut h) = (0, 0);
        for (x, y, p) in image.enumerate_pixels() {
            if p.0[3] > 0 {
                w = w.max(x + 1);
                h = h.max(y + 1);
            }
        }
        if w == 0 || h == 0 {
            return image.clone();
        }
        image::imageops::crop_imm(image, 0, 0, w, h).to_image()
    }

    /// The frame-resize cursors of macOS 15 for the diagonal kinds, when present.
    fn diagonal_cursor(cursor_type: &str) -> Option<Retained<NSCursor>> {
        use objc2::runtime::AnyClass;
        use objc2::{ClassType, sel};
        use objc2_app_kit::{NSCursorFrameResizeDirections, NSCursorFrameResizePosition};
        let class: &AnyClass = NSCursor::class();
        class.class_method(sel!(frameResizeCursorFromPosition:inDirections:))?;
        let position = match cursor_type {
            "SizeNWSE" | "PanNW" | "PanSE" => NSCursorFrameResizePosition::TopLeft,
            _ => NSCursorFrameResizePosition::TopRight,
        };
        Some(NSCursor::frameResizeCursorFromPosition_inDirections(
            position,
            NSCursorFrameResizeDirections::All,
        ))
    }

    pub fn desktop_wallpaper() -> Option<(PathBuf, WallpaperOptions)> {
        let mtm = MainThreadMarker::new()?;
        let screen = NSScreen::mainScreen(mtm)?;
        let workspace = NSWorkspace::sharedWorkspace();
        let url = workspace.desktopImageURLForScreen(&screen)?;
        let path = PathBuf::from(url.path()?.to_string());
        if !path.is_file() {
            return None;
        }
        let mut options = WallpaperOptions {
            scaling: 3,
            allow_clipping: true,
        };
        if let Some(dict) = workspace.desktopImageOptionsForScreen(&screen) {
            // SAFETY: the keys are AppKit's own constants.
            let (scaling_key, clipping_key) = unsafe {
                (
                    NSWorkspaceDesktopImageScalingKey,
                    NSWorkspaceDesktopImageAllowClippingKey,
                )
            };
            if let Some(n) = dict
                .objectForKey(scaling_key)
                .and_then(|v| v.downcast::<NSNumber>().ok())
            {
                options.scaling = n.integerValue() as i64;
            }
            if let Some(n) = dict
                .objectForKey(clipping_key)
                .and_then(|v| v.downcast::<NSNumber>().ok())
            {
                options.allow_clipping = n.boolValue();
            }
        }
        Some((path, options))
    }

    pub fn load_image(path: &Path) -> Option<RgbaImage> {
        let file = NSString::from_str(&path.to_string_lossy());
        let image = NSImage::initWithContentsOfFile(NSImage::alloc(), &file)?;
        // The picture's own pixels: the representation at its pixel size.
        let reps = image.representations();
        let scale = reps
            .iter()
            .map(|r| {
                let w = r.pixelsWide() as f64;
                let size = r.size();
                if size.width > 0.0 {
                    w / size.width
                } else {
                    1.0
                }
            })
            .fold(1.0f64, f64::max);
        image_pixels(&image, scale)
    }
}
