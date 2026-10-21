//! The parts of Modlight App that are Windows: where the token is kept, the screen and
//! its windows, global hotkeys, the Explorer integration, sounds, printing, text
//! recognition, and what the system says about language, region and motion.
//!
//! The module names and functions are those of `modlight-macos`, so the app calls the
//! same API on either system (`platform::...`). Upstream (`Modlight-App`) is a Windows
//! program: where it used a Windows mechanism, this uses the same mechanism again;
//! `docs/WINDOWS.md` lists what differs from it and from the macOS port.
//!
//! The whole crate is Windows only; on any other system it is empty.

#![cfg(windows)]

pub mod ax;
pub mod capture;
pub mod credentials;
pub mod editor;
pub mod hotkeys;
pub mod input;
pub mod integration;
pub mod keys;
pub mod network;
pub mod ocr;
pub mod pasteboard;
pub mod print;
pub mod sound;
pub mod system;
pub mod webview;
pub mod windows;

#[allow(dead_code)]
mod wide;

pub use credentials::SecretStore;
