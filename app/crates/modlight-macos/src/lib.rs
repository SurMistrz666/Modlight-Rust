//! The parts of Modlight App that are macOS: where the token is kept, how a link is
//! opened, and what the system says about language, region and motion.
//!
//! Everything here goes through the system's own command-line tools, so no build of the
//! app becomes the owner of a Keychain item (an item created by the app binary is tied
//! to that exact build, and every rebuild would stop at a password prompt).

pub mod capture;
pub mod editor;
pub mod hotkeys;
pub mod input;
pub mod integration;
pub mod keychain;
pub mod keys;
pub mod network;
pub mod sound;
pub mod system;
pub mod windows;
// Phase 3, media: the YouTube sign-in window.
pub mod webview;
// Phase 5, tools: text recognition.
pub mod ocr;

pub use keychain::KeychainStore;
