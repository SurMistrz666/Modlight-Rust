//! `lib/crypto/*`: end-to-end encrypted uploads, the server's side of them.
//!
//! The browser encrypts before a byte leaves the tab and decrypts after the
//! last one arrives; the key lives only in the URL fragment, which no browser
//! ever transmits (`lib/crypto/link.ts`, run by the SPA verbatim). The server
//! stores an opaque container and decides exactly one thing about it: that a
//! row IS one, so the preview page hands it to the in-browser viewer instead
//! of rendering, embedding, thumbnailing or converting it.

pub mod envelope;
