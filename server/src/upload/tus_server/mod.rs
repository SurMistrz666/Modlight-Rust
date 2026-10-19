//! Port of the tus libraries `lib/upload/tus.ts` builds on: `@tus/server`
//! 2.4.1 (the protocol handlers), `@tus/file-store` 2.1.0 and
//! `@tus/s3-store` 2.0.4 (the datastores), with the shared models of
//! `@tus/utils` 0.7.0 in [`store`].
//!
//! Not a Modlight module upstream (they are npm dependencies), but their wire
//! behaviour is what Uppy and the desktop apps' resumable uploader talk to,
//! so it is ported as faithfully as the Modlight code itself.

pub mod store;
