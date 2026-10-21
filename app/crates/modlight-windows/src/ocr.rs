//! Text recognition with Windows OCR (`Windows.Media.Ocr`), as upstream. A picture in,
//! the recognised lines out.

/// The languages Windows OCR has installed, as BCP 47 tags ("en-US").
pub fn languages() -> Vec<String> {
    Vec::new()
}

/// Recognise the text of a picture (PNG or any format the `image` crate reads) in
/// `language`. Each line, top to bottom.
pub fn recognize(image: &[u8], language: &str) -> Result<Vec<String>, String> {
    let _ = (image, language);
    Err("not implemented yet".into())
}
