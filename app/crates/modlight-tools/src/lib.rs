//! The ShareX tools of Modlight App (`ShareX.Tools`, the colour picker of
//! `ShareX.HelpersLib`, the screen colour picker of `ShareX.Avalonia`, the history views of
//! `ShareX.HistoryLib`) without their windows: what they compute, convert, build and
//! parse. The windows are in `modlight-app`; the macOS calls in `modlight-macos`.

pub mod color;
pub mod color_format;
pub mod color_picker;
pub mod combine;
pub mod images;
pub mod metadata;
pub mod pin;
pub mod qr;
pub mod settings;
pub mod video;
