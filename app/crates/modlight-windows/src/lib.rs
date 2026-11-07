//! The parts of Modlight App that are Windows: where the token is kept, the screen and
//! its windows, global hotkeys, the Explorer integration, sounds, printing, text
//! recognition, and what the system says about language, region and motion.
//!
//! The module names and functions are those of `modlight-macos`, so the app calls the
//! same API on either system (`platform::...`). Upstream (`Modlight-App`) is a Windows
//! program: where it used a Windows mechanism, this uses the same mechanism again;
//! `docs/WINDOWS.md` lists what differs from it and from the macOS port.
//!
//! The crate is Windows only: on any other system all that builds is `duplication_plan`,
//! plain logic without a system call, so that its tests run on a Mac as well.

#[cfg(windows)]
pub mod ax;
#[cfg(windows)]
pub mod capture;
#[cfg(windows)]
pub mod credentials;
#[cfg(windows)]
pub mod editor;
#[cfg(windows)]
pub mod hotkeys;
#[cfg(windows)]
pub mod input;
#[cfg(windows)]
pub mod integration;
#[cfg(windows)]
pub mod keys;
#[cfg(windows)]
pub mod network;
#[cfg(windows)]
pub mod ocr;
#[cfg(windows)]
pub mod pasteboard;
#[cfg(windows)]
pub mod print;
#[cfg(windows)]
pub mod sound;
#[cfg(windows)]
pub mod system;
#[cfg(windows)]
pub mod webview;
#[cfg(windows)]
pub mod windows;

#[cfg(windows)]
#[allow(dead_code)]
mod wide;

#[cfg_attr(not(windows), allow(dead_code))]
mod duplication_plan;

#[cfg(windows)]
pub use credentials::SecretStore;
