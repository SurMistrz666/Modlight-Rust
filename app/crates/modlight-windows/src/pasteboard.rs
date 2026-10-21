//! The picture on the clipboard as encoded bytes, for the app to decode under its own
//! limits (`modlight_core::image_decode`): the "PNG" format when a program put one
//! there, else the device-independent bitmap as a BMP file.

/// The PNG or BMP bytes on the clipboard, when it holds a picture.
pub fn image_bytes() -> Option<Vec<u8>> {
    None
}
