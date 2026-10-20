//! Printing a picture (`PrintHelper` and `PrintSettings` of ShareX.HelpersLib): where the
//! picture goes on the page. .NET draws in hundredths of an inch with a picture pixel as
//! one unit; the same units are kept here and turned into points (1/72 inch) by the
//! caller, so a picture prints at the size it printed on Windows.

use image::RgbaImage;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `PrintSettings` (`ApplicationConfig.PrintSettings`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PrintSettings {
    pub margin: i64,
    pub auto_rotate_image: bool,
    pub auto_scale_image: bool,
    pub allow_enlarge_image: bool,
    pub center_image: bool,
    /// `XmlFont` for printing text, kept as it was written.
    #[serde(skip_serializing_if = "Value::is_null")]
    pub text_font: Value,
    pub show_print_dialog: bool,
    pub default_printer_override: String,
}

impl Default for PrintSettings {
    fn default() -> Self {
        PrintSettings {
            margin: 5,
            auto_rotate_image: true,
            auto_scale_image: true,
            allow_enlarge_image: false,
            center_image: false,
            text_font: Value::Null,
            show_print_dialog: true,
            default_printer_override: String::new(),
        }
    }
}

/// Hundredths of an inch in a point.
pub const UNITS_PER_POINT: f64 = 100.0 / 72.0;

/// Where the picture goes on a page (hundredths of an inch from the page's top left).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// Turned a quarter clockwise first (`Rotate90FlipNone`).
    pub rotate: bool,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Without auto scaling, only this much of the picture's top left is drawn (pixels),
    /// one pixel a unit.
    pub crop: Option<(u32, u32)>,
}

/// `PrintImage(PrintPageEventArgs)` for a page of `page` units.
pub fn place(page: (f64, f64), image: (u32, u32), settings: &PrintSettings) -> Placement {
    let margin = settings.margin as f64;
    let (rx, ry) = (margin, margin);
    let (rw, rh) = (page.0 - 2.0 * margin, page.1 - 2.0 * margin);
    let (iw, ih) = image;
    let rotate = settings.auto_rotate_image && ((rw > rh && iw < ih) || (rw < rh && iw > ih));
    let (iw, ih) = if rotate { (ih, iw) } else { (iw, ih) };
    if !settings.auto_scale_image {
        // `DrawImage(img, rect, new Rectangle(0, 0, rect.Width, rect.Height), Pixel)`.
        let cw = (rw.max(0.0) as u32).min(iw);
        let ch = (rh.max(0.0) as u32).min(ih);
        return Placement {
            rotate,
            x: rx,
            y: ry,
            width: cw as f64,
            height: ch as f64,
            crop: Some((cw, ch)),
        };
    }
    // `DrawAutoScaledImage`.
    let (iwf, ihf) = (iw as f64, ih as f64);
    let (ratio, nw, nh) = if !settings.allow_enlarge_image && iwf <= rw && ihf <= rh {
        (1.0, iw as i64, ih as i64)
    } else {
        let ratio = (rw / iwf).min(rh / ihf);
        (ratio, (iwf * ratio) as i64, (ihf * ratio) as i64)
    };
    let (mut x, mut y) = (rx as i64, ry as i64);
    if settings.center_image {
        x += ((rw - iwf * ratio) / 2.0) as i64;
        y += ((rh - ihf * ratio) / 2.0) as i64;
    }
    Placement {
        rotate,
        x: x as f64,
        y: y as f64,
        width: nw as f64,
        height: nh as f64,
        crop: None,
    }
}

/// The picture as it is drawn: turned and cut as the placement says.
pub fn prepared(image: &RgbaImage, placement: &Placement) -> RgbaImage {
    let turned = if placement.rotate {
        image::imageops::rotate90(image)
    } else {
        image.clone()
    };
    match placement.crop {
        Some((w, h)) if (w, h) != turned.dimensions() && w > 0 && h > 0 => {
            image::imageops::crop_imm(&turned, 0, 0, w, h).to_image()
        }
        _ => turned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letter() -> (f64, f64) {
        (850.0, 1100.0)
    }

    #[test]
    fn small_pictures_stay_their_size() {
        let s = PrintSettings::default();
        let p = place(letter(), (300, 200), &s);
        // Wider than tall on a tall page: turned.
        assert!(p.rotate);
        assert_eq!((p.x, p.y, p.width, p.height), (5.0, 5.0, 200.0, 300.0));
        let centred = PrintSettings {
            center_image: true,
            auto_rotate_image: false,
            ..s.clone()
        };
        let p = place(letter(), (300, 200), &centred);
        assert!(!p.rotate);
        assert_eq!((p.x, p.y), (5.0 + 270.0, 5.0 + 445.0));
    }

    #[test]
    fn large_pictures_shrink_to_the_page() {
        let s = PrintSettings::default();
        let p = place(letter(), (1600, 3000), &s);
        assert!(!p.rotate);
        let ratio = (840.0 / 1600.0f64).min(1090.0 / 3000.0);
        assert_eq!(p.width, (1600.0 * ratio) as i64 as f64);
        assert_eq!(p.height, (3000.0 * ratio) as i64 as f64);
        let enlarge = PrintSettings {
            allow_enlarge_image: true,
            auto_rotate_image: false,
            ..s.clone()
        };
        let p = place(letter(), (84, 50), &enlarge);
        assert_eq!(p.width, 840.0);
    }

    #[test]
    fn without_scaling_the_corner_is_cut() {
        let s = PrintSettings {
            auto_scale_image: false,
            auto_rotate_image: false,
            margin: 10,
            ..PrintSettings::default()
        };
        let p = place(letter(), (2000, 500), &s);
        assert_eq!(p.crop, Some((830, 500)));
        let image = RgbaImage::new(2000, 500);
        assert_eq!(prepared(&image, &p).dimensions(), (830, 500));
        let turned = Placement {
            rotate: true,
            crop: None,
            ..p
        };
        assert_eq!(prepared(&image, &turned).dimensions(), (500, 2000));
    }

    #[test]
    fn settings_keep_their_font() {
        let text = r#"{"Margin":12,"TextFont":{"Name":"Arial","Size":10},"ShowPrintDialog":false}"#;
        let s: PrintSettings = serde_json::from_str(text).unwrap();
        assert_eq!(s.margin, 12);
        assert!(!s.show_print_dialog);
        assert!(s.auto_scale_image);
        let back = serde_json::to_value(&s).unwrap();
        assert_eq!(back["TextFont"]["Name"], "Arial");
        let none = serde_json::to_value(PrintSettings::default()).unwrap();
        assert!(none.get("TextFont").is_none());
    }
}
