//! The two things the app asks AppKit directly: bring its windows back from the tray,
//! and where on the screen a notice may go.

/// Unhide the app and bring every one of its windows forward. Must run on the main
/// thread (menu actions of the tray do); elsewhere it does nothing.
#[cfg(target_os = "macos")]
pub fn show_app_windows() {
    use objc2_app_kit::NSApplication;
    use objc2_foundation::MainThreadMarker;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    app.unhide(None);
    for window in app.windows().iter() {
        // Only what the tray hid: a window already on screen stays where it is.
        if !window.isVisible() {
            window.makeKeyAndOrderFront(None);
        }
    }
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
}

#[cfg(not(target_os = "macos"))]
pub fn show_app_windows() {}

/// A rectangle as Cocoa gives it: x, y (up from the bottom of the primary display),
/// width, height.
pub type CocoaRect = (f64, f64, f64, f64);

/// One screen as Cocoa gives it: its frame and its visible frame.
pub type CocoaScreen = (CocoaRect, CocoaRect);

/// The visible frame of `screens[index]` in the desktop's points, origin at the top
/// left of the primary display, y down (where CoreGraphics, winit and the capture code
/// put things). `screens` is in `NSScreen.screens` order: Cocoa's y runs up from the
/// bottom of the first one (the display with the menu bar, at (0, 0)), so its height
/// turns every screen's y around, not the height of the screen measured (that is
/// right only for the first display and those of its height).
pub fn visible_in_desktop(screens: &[CocoaScreen], index: usize) -> Option<CocoaRect> {
    let primary_height = screens.first()?.0.3;
    let (_, visible) = *screens.get(index)?;
    Some((
        visible.0,
        primary_height - (visible.1 + visible.3),
        visible.2,
        visible.3,
    ))
}

/// The screens in `NSScreen.screens` order with their display ids, and which of them is
/// the main screen (the one with the key window, `mainScreen`).
#[cfg(target_os = "macos")]
pub(crate) fn cocoa_screens(
    mtm: objc2_foundation::MainThreadMarker,
) -> (Vec<(u32, CocoaScreen)>, Option<usize>) {
    use objc2_app_kit::NSScreen;
    use objc2_foundation::{NSNumber, NSRect, NSString};

    let id_of = |screen: &NSScreen| -> u32 {
        screen
            .deviceDescription()
            .objectForKey(&NSString::from_str("NSScreenNumber"))
            .and_then(|n| n.downcast::<NSNumber>().ok())
            .map(|n| n.unsignedIntValue())
            .unwrap_or(0)
    };
    let rect = |r: NSRect| (r.origin.x, r.origin.y, r.size.width, r.size.height);
    let screens: Vec<(u32, CocoaScreen)> = NSScreen::screens(mtm)
        .iter()
        .map(|s| (id_of(&s), (rect(s.frame()), rect(s.visibleFrame()))))
        .collect();
    let main = NSScreen::mainScreen(mtm).map(|s| id_of(&s));
    let index = main.and_then(|id| screens.iter().position(|(i, _)| *i == id));
    (screens, index)
}

/// The main screen's area free of the menu bar and the Dock, in points, with the origin
/// at the top left: (x, y, width, height). `None` off the main thread.
#[cfg(target_os = "macos")]
pub fn visible_frame() -> Option<(f32, f32, f32, f32)> {
    use objc2_foundation::MainThreadMarker;

    let mtm = MainThreadMarker::new()?;
    let (screens, main) = cocoa_screens(mtm);
    let screens: Vec<CocoaScreen> = screens.into_iter().map(|(_, s)| s).collect();
    let (x, y, w, h) = visible_in_desktop(&screens, main.unwrap_or(0))?;
    Some((x as f32, y as f32, w as f32, h as f32))
}

#[cfg(not(target_os = "macos"))]
pub fn visible_frame() -> Option<(f32, f32, f32, f32)> {
    None
}

/// Whether the active app shows a window in full screen
/// (`CaptureHelpers.IsActiveWindowFullscreen`): the presentation options the active
/// app put into effect include full screen. `false` off the main thread.
#[cfg(target_os = "macos")]
pub fn active_window_fullscreen() -> bool {
    use objc2_app_kit::{NSApplication, NSApplicationPresentationOptions};
    use objc2_foundation::MainThreadMarker;

    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    NSApplication::sharedApplication(mtm)
        .currentSystemPresentationOptions()
        .contains(NSApplicationPresentationOptions::FullScreen)
}

#[cfg(not(target_os = "macos"))]
pub fn active_window_fullscreen() -> bool {
    false
}

/// Raise a window of this app, found by its title, above the menu bar and the Dock and
/// onto every Space, as the region capture overlay must be (upstream's overlay covers the
/// taskbar). Main thread only; returns whether the window was found.
#[cfg(target_os = "macos")]
pub fn raise_overlay(title: &str) -> bool {
    use objc2_app_kit::{NSApplication, NSWindowCollectionBehavior};
    use objc2_foundation::MainThreadMarker;

    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    let mut found = false;
    for window in app.windows().iter() {
        if window.title().to_string() == title {
            // NSScreenSaverWindowLevel: over the menu bar, the Dock and full-screen apps.
            window.setLevel(1000);
            window.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::FullScreenAuxiliary
                    | NSWindowCollectionBehavior::Stationary,
            );
            found = true;
        }
    }
    found
}

#[cfg(not(target_os = "macos"))]
pub fn raise_overlay(_title: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A MacBook (1440 by 900, the menu bar on it) with a 1920 by 1080 display to its
    /// right, tops aligned: Cocoa puts the external one at y = 900 - 1080 = -180. The
    /// key window is on the external display, so it is `mainScreen`.
    fn two_displays() -> Vec<CocoaScreen> {
        vec![
            ((0.0, 0.0, 1440.0, 900.0), (0.0, 70.0, 1440.0, 805.0)),
            (
                (1440.0, -180.0, 1920.0, 1080.0),
                (1440.0, -180.0, 1920.0, 1055.0),
            ),
        ]
    }

    #[test]
    fn visible_frames_are_measured_from_the_primary_display() {
        let screens = two_displays();
        // The primary display: under its 25 pt menu bar, above its 70 pt Dock.
        assert_eq!(
            visible_in_desktop(&screens, 0),
            Some((0.0, 25.0, 1440.0, 805.0))
        );
        // The external display's top is the desktop's 0 too; under the menu bar macOS
        // shows there (25 pt), not 205 pt down where the display's own height put it.
        assert_eq!(
            visible_in_desktop(&screens, 1),
            Some((1440.0, 25.0, 1920.0, 1055.0))
        );
        assert_eq!(visible_in_desktop(&screens, 2), None);
    }
}
