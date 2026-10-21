//! Printing a picture (`PrintDocument`, `PrintDialog` and `PrintPreviewDialog` upstream):
//! one page through GDI, with or without the print dialog; the preview is the page as a
//! PDF opened in the default viewer. Main thread only; never used by automated runs.

use image::RgbaImage;

/// The page in points: (width, height) of the default printer's paper.
pub fn page_size() -> (f64, f64) {
    (612.0, 792.0)
}

/// What a print came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Printed {
    Done,
    /// The dialog was cancelled.
    Cancelled,
    /// The printer named in the settings does not exist; printed on the default one.
    UnknownPrinter,
}

/// Print `picture` at `rect` (points, top left of the page) on one page.
pub fn print(
    picture: &RgbaImage,
    rect: (f64, f64, f64, f64),
    show_panel: bool,
    printer: Option<&str>,
) -> Result<Printed, String> {
    let _ = (picture, rect, show_panel, printer);
    Err("not implemented yet".into())
}

/// Write the page as a PDF (the preview).
pub fn page_pdf(
    picture: &RgbaImage,
    rect: (f64, f64, f64, f64),
    path: &std::path::Path,
) -> Result<(), String> {
    let _ = (picture, rect, path);
    Err("not implemented yet".into())
}
