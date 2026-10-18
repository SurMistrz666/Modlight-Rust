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
