//! Upstream's Windows integration (`StartupManager`, `IntegrationHelpers`), back on
//! Windows: "Start with Windows" (the `Run` key of this user), the Explorer context menu
//! "Upload with Modlight" and the "Send to" entry, and the single running instance that
//! receives the files those hand to a second start of the program.
//!
//! The function names are the macOS port's (`in_app_bundle`, `take_service_files`...),
//! so the app calls one API; each says what it means here.

use std::path::PathBuf;
use std::sync::Mutex;

/// `StartupState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupState {
    Disabled,
    /// Turned off in Task Manager's Startup apps (`StartupApproved\Run`).
    DisabledByUser,
    Enabled,
    /// Kept for the macOS API; Windows never waits for an approval.
    RequiresApproval,
    /// Not running from an installed copy: a development build in a cargo target folder.
    NotAvailable,
}

/// Whether this is a copy that may register itself with Windows (start with Windows,
/// the Explorer menu): any build that does not run from a cargo `target` folder.
pub fn in_app_bundle() -> bool {
    false
}

/// `StartupManager.State` (read).
pub fn startup_state() -> StartupState {
    StartupState::NotAvailable
}

/// `StartupManager.State` (write).
pub fn set_startup(enabled: bool) -> Result<StartupState, String> {
    let _ = enabled;
    Err("not implemented yet".into())
}

/// Open Settings at Startup apps.
pub fn open_login_items_settings() {}

/// Files handed over by Explorer (the context menu, "Send to", a second start of the
/// program with files on its command line), waiting for the window.
static RECEIVED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// The files received since the last call.
pub fn take_service_files() -> Vec<PathBuf> {
    std::mem::take(&mut *RECEIVED.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Queue files for the window (the first instance's own command line).
pub fn receive_files(files: Vec<PathBuf>) {
    if !files.is_empty() {
        RECEIVED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(files);
    }
}

/// Start listening for files from later starts of the program. Normal mode only, once.
pub fn install_services_provider() {}

/// Kept for the macOS API: there is no system page for the Explorer menu.
pub fn open_services_settings() -> bool {
    false
}

/// `ShellContextMenu`: whether "Upload with Modlight" is in Explorer's context menu.
pub fn context_menu_enabled() -> bool {
    false
}

/// Add or remove "Upload with Modlight" in Explorer's context menu.
pub fn set_context_menu(enabled: bool) -> Result<(), String> {
    let _ = enabled;
    Err("not implemented yet".into())
}

/// `SendTo`: whether Modlight is in Explorer's "Send to" menu.
pub fn send_to_enabled() -> bool {
    false
}

/// Add or remove Modlight in Explorer's "Send to" menu.
pub fn set_send_to(enabled: bool) -> Result<(), String> {
    let _ = enabled;
    Err("not implemented yet".into())
}

/// Hand `files` to a Modlight already running for this user. `true` when one took
/// them: this start then has nothing left to do and exits.
pub fn forward_to_running_instance(files: &[PathBuf]) -> bool {
    let _ = files;
    false
}

/// Bring the windows of a Modlight already running forward. `true` when one answered.
pub fn activate_running_instance() -> bool {
    false
}
