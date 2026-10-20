//! Printing a picture with the macOS print system (`PrintDocument`, `PrintDialog` and
//! `PrintPreviewDialog` on Windows): one page, the picture placed by the caller,
//! `NSPrintOperation` with or without the print panel; the preview is the page as a PDF
//! opened in Preview. Main thread only; never used by automated runs.

use image::RgbaImage;

/// The page in points: (width, height) of the paper the print settings name.
pub fn page_size() -> (f64, f64) {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSPrintInfo;
        let size = NSPrintInfo::sharedPrintInfo().paperSize();
        (size.width, size.height)
    }
    #[cfg(not(target_os = "macos"))]
    {
        (612.0, 792.0)
    }
}

/// What a print came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Printed {
    Done,
    /// The panel was cancelled.
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
    #[cfg(target_os = "macos")]
    {
        mac::run(
            picture,
            rect,
            Output::Printer {
                show_panel,
                printer,
            },
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (picture, rect, show_panel, printer);
        Err("printing needs macOS".into())
    }
}

/// Write the page as a PDF (the preview).
pub fn page_pdf(
    picture: &RgbaImage,
    rect: (f64, f64, f64, f64),
    path: &std::path::Path,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        mac::run(picture, rect, Output::Pdf(path)).map(|_| ())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (picture, rect, path);
        Err("printing needs macOS".into())
    }
}

#[cfg(target_os = "macos")]
enum Output<'a> {
    Printer {
        show_panel: bool,
        printer: Option<&'a str>,
    },
    Pdf(&'a std::path::Path),
}

#[cfg(target_os = "macos")]
mod mac {
    use image::RgbaImage;
    use objc2::rc::Retained;
    use objc2::{AnyThread, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSImage, NSImageScaling, NSImageView, NSPrintInfo, NSPrintOperation, NSPrinter,
        NSPrintingPaginationMode, NSView,
    };
    use objc2_foundation::{NSCopying, NSData, NSPoint, NSRect, NSSize, NSString};

    use super::{Output, Printed};

    fn ns_image(picture: &RgbaImage) -> Result<Retained<NSImage>, String> {
        let png = {
            let mut bytes = Vec::new();
            image::DynamicImage::ImageRgba8(picture.clone())
                .write_to(
                    &mut std::io::Cursor::new(&mut bytes),
                    image::ImageFormat::Png,
                )
                .map_err(|e| e.to_string())?;
            bytes
        };
        let data = NSData::with_bytes(&png);
        NSImage::initWithData(NSImage::alloc(), &data)
            .ok_or_else(|| "the picture could not be read".to_string())
    }

    pub(super) fn run(
        picture: &RgbaImage,
        rect: (f64, f64, f64, f64),
        output: Output,
    ) -> Result<Printed, String> {
        let mtm = MainThreadMarker::new().ok_or("printing runs on the main thread")?;
        let info = NSPrintInfo::sharedPrintInfo();
        let info: Retained<NSPrintInfo> = info.copy();
        let mut result = Printed::Done;
        if let Output::Printer {
            printer: Some(name),
            ..
        } = &output
            && !name.trim().is_empty()
        {
            match NSPrinter::printerWithName(&NSString::from_str(name)) {
                Some(printer) => info.setPrinter(&printer),
                None => result = Printed::UnknownPrinter,
            }
        }
        // The whole sheet: the placement already holds the margin.
        info.setLeftMargin(0.0);
        info.setRightMargin(0.0);
        info.setTopMargin(0.0);
        info.setBottomMargin(0.0);
        info.setHorizontalPagination(NSPrintingPaginationMode::Clip);
        info.setVerticalPagination(NSPrintingPaginationMode::Clip);
        let paper = info.paperSize();
        let page = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), paper),
        );
        let view = NSImageView::initWithFrame(
            NSImageView::alloc(mtm),
            NSRect::new(
                // The view's origin is the bottom left.
                NSPoint::new(rect.0, paper.height - rect.1 - rect.3),
                NSSize::new(rect.2, rect.3),
            ),
        );
        view.setImageScaling(NSImageScaling::ScaleAxesIndependently);
        let image = ns_image(picture)?;
        view.setImage(Some(&image));
        page.addSubview(&view);
        let operation = match output {
            Output::Pdf(path) => {
                let path = NSString::from_str(&path.to_string_lossy());
                NSPrintOperation::PDFOperationWithView_insideRect_toPath_printInfo(
                    &page,
                    NSRect::new(NSPoint::new(0.0, 0.0), paper),
                    &path,
                    &info,
                )
            }
            Output::Printer { show_panel, .. } => {
                let operation = NSPrintOperation::printOperationWithView_printInfo(&page, &info);
                operation.setShowsPrintPanel(show_panel);
                operation.setShowsProgressPanel(show_panel);
                operation
            }
        };
        if operation.runOperation() {
            Ok(result)
        } else {
            Ok(Printed::Cancelled)
        }
    }
}
