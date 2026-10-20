//! The Mac's counterparts of upstream's Windows integration (`StartupManager`,
//! `IntegrationHelpers`): the login item instead of the Startup folder shortcut, and the
//! Finder service "Upload with Modlight" instead of the Explorer context menu.
//!
//! Both only work from the `Modlight.app` bundle (`scripts/build-app.sh`): a login item
//! is registered for a bundle, and Finder reads services from a bundle's Info.plist.
//! Registering a login item may show the system's "Login item added" notice and appear in
//! System Settings; it is never done by QA or automated runs.

use std::path::PathBuf;
use std::sync::Mutex;

/// `StartupState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupState {
    Disabled,
    DisabledByUser,
    Enabled,
    /// Waiting for approval in System Settings (`SMAppServiceStatusRequiresApproval`).
    RequiresApproval,
    /// Not running from an app bundle: there is nothing to register.
    NotAvailable,
}

/// Whether the running binary is inside an `.app` bundle.
pub fn in_app_bundle() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .is_some_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"))
}

/// `StartupManager.State` (read).
pub fn startup_state() -> StartupState {
    if !in_app_bundle() {
        return StartupState::NotAvailable;
    }
    #[cfg(target_os = "macos")]
    {
        use objc2_service_management::{SMAppService, SMAppServiceStatus};
        // SAFETY: a plain status read of this app's own service.
        let status = unsafe { SMAppService::mainAppService().status() };
        match status {
            SMAppServiceStatus::Enabled => StartupState::Enabled,
            SMAppServiceStatus::RequiresApproval => StartupState::RequiresApproval,
            SMAppServiceStatus::NotFound => StartupState::NotAvailable,
            _ => StartupState::Disabled,
        }
    }
    #[cfg(not(target_os = "macos"))]
    StartupState::NotAvailable
}

/// `StartupManager.State` (write): register or unregister the app as a login item.
pub fn set_startup(enabled: bool) -> Result<StartupState, String> {
    if !in_app_bundle() {
        return Err("Modlight is not running from Modlight.app".into());
    }
    #[cfg(target_os = "macos")]
    {
        use objc2_service_management::SMAppService;
        // SAFETY: registering this app's own main service.
        let service = unsafe { SMAppService::mainAppService() };
        let result = if enabled {
            unsafe { service.registerAndReturnError() }
        } else {
            unsafe { service.unregisterAndReturnError() }
        };
        result.map_err(|e| e.localizedDescription().to_string())?;
        Ok(startup_state())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = enabled;
        Err("login items need macOS".into())
    }
}

/// Open System Settings at Login Items (where a login item waiting for approval is).
pub fn open_login_items_settings() {
    #[cfg(target_os = "macos")]
    {
        use objc2_service_management::SMAppService;
        // SAFETY: opens System Settings; no state of ours is touched.
        unsafe { SMAppService::openSystemSettingsLoginItems() };
    }
}

/// Files Finder handed to the "Upload with Modlight" service, waiting for the window.
static RECEIVED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// The files the service received since the last call.
pub fn take_service_files() -> Vec<PathBuf> {
    std::mem::take(&mut *RECEIVED.lock().unwrap())
}

#[cfg(target_os = "macos")]
mod provider {
    use objc2::rc::Retained;
    use objc2::runtime::NSObjectProtocol;
    use objc2::{AllocAnyThread, define_class, msg_send};
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeFileURL};
    use objc2_foundation::{NSObject, NSString, NSURL};

    define_class!(
        // SAFETY: NSObject subclass without ivars.
        #[unsafe(super(NSObject))]
        #[name = "ModlightServicesProvider"]
        pub struct ServicesProvider;

        unsafe impl NSObjectProtocol for ServicesProvider {}

        impl ServicesProvider {
            /// `NSMessage = uploadFiles` in Info.plist's `NSServices`.
            #[unsafe(method(uploadFiles:userData:error:))]
            fn upload_files(&self, pasteboard: &NSPasteboard, _user_data: Option<&NSString>, _error: *mut *mut NSString) {
                let mut files = Vec::new();
                if let Some(items) = pasteboard.pasteboardItems() {
                    for item in items.iter() {
                        // SAFETY: the type constant is a valid pasteboard type.
                        let kind = unsafe { NSPasteboardTypeFileURL };
                        if let Some(text) = item.stringForType(kind)
                            && let Some(url) = NSURL::URLWithString(&text)
                            && let Some(path) = url.path()
                        {
                            files.push(std::path::PathBuf::from(path.to_string()));
                        }
                    }
                }
                log::info!("Finder service: {} file(s) to upload", files.len());
                super::RECEIVED.lock().unwrap().extend(files);
            }
        }
    );

    impl ServicesProvider {
        pub fn new() -> Retained<ServicesProvider> {
            let this = ServicesProvider::alloc().set_ivars(());
            // SAFETY: NSObject's init on a fresh object.
            unsafe { msg_send![super(this), init] }
        }
    }
}

/// Become the provider of the "Upload with Modlight" service. Normal mode only, on the
/// main thread, once.
pub fn install_services_provider() {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSApplication;
        use objc2_foundation::MainThreadMarker;
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let provider = provider::ServicesProvider::new();
        let app = NSApplication::sharedApplication(mtm);
        // SAFETY: the provider implements the method Info.plist names; it is kept alive
        // for the life of the app below.
        unsafe { app.setServicesProvider(Some(&provider)) };
        std::mem::forget(provider);
        objc2_app_kit::NSUpdateDynamicServices();
    }
}

/// Open System Settings where services are switched on and off.
pub fn open_services_settings() -> bool {
    std::process::Command::new("/usr/bin/open")
        .arg("x-apple.systempreferences:com.apple.preference.keyboard?Services")
        .spawn()
        .is_ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_test_binary_is_not_a_bundle() {
        assert!(!super::in_app_bundle());
        assert_eq!(super::startup_state(), super::StartupState::NotAvailable);
        assert!(super::set_startup(true).is_err());
        assert!(super::take_service_files().is_empty());
    }
}
