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

/// The main screen's area free of the menu bar and the Dock, in points, with the origin
/// at the top left: (x, y, width, height). `None` off the main thread.
#[cfg(target_os = "macos")]
pub fn visible_frame() -> Option<(f32, f32, f32, f32)> {
    use objc2_app_kit::NSScreen;
    use objc2_foundation::MainThreadMarker;

    let mtm = MainThreadMarker::new()?;
    let screen = NSScreen::mainScreen(mtm)?;
    let full = screen.frame();
    let visible = screen.visibleFrame();
    let top = full.size.height - (visible.origin.y + visible.size.height);
    Some((
        visible.origin.x as f32,
        top as f32,
        visible.size.width as f32,
        visible.size.height as f32,
    ))
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
