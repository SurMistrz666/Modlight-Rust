//! `AdvancedColorPickerControl`: the colour box and the slider beside it, in six draw
//! styles (hue, saturation, brightness, red, green, blue). What each pixel shows, where
//! the markers sit, and which colour a press at a point picks.

use crate::color::{self, Color};

/// `DrawStyle`, in the order of the "Color mode" drop-down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawStyle {
    #[default]
    Hue,
    Saturation,
    Brightness,
    Red,
    Green,
    Blue,
}

impl DrawStyle {
    pub const ALL: [DrawStyle; 6] = [
        DrawStyle::Hue,
        DrawStyle::Saturation,
        DrawStyle::Brightness,
        DrawStyle::Red,
        DrawStyle::Green,
        DrawStyle::Blue,
    ];
}

/// `SliderWidth` and `Gap` of the control.
pub const SLIDER_WIDTH: f64 = 30.0;
pub const GAP: f64 = 12.0;

fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

/// `ToByte`: 0..1 to 0..255, rounded half to even like `Math.Round`.
pub fn to_byte(value: f64) -> u8 {
    ((clamp01(value) * 255.0).round_ties_even() as i64).clamp(0, 255) as u8
}

/// `FromHsv`: the control's own HSV conversion (not `HSBToColor`).
pub fn from_hsv(hue: f64, saturation: f64, value: f64, alpha: u8) -> Color {
    let hue = ((hue % 1.0) + 1.0) % 1.0;
    let saturation = clamp01(saturation);
    let value = clamp01(value);
    let scaled = hue * 6.0;
    let sector = (scaled.floor() as i64).rem_euclid(6);
    let fraction = scaled - scaled.floor();
    let p = value * (1.0 - saturation);
    let q = value * (1.0 - fraction * saturation);
    let t = value * (1.0 - (1.0 - fraction) * saturation);
    let (r, g, b) = match sector {
        0 => (value, t, p),
        1 => (q, value, p),
        2 => (p, value, t),
        3 => (p, q, value),
        4 => (t, p, value),
        _ => (value, p, q),
    };
    Color::argb(alpha, to_byte(r), to_byte(g), to_byte(b))
}

/// The two parts of a control `width` x `height`: the box and the slider (x, width).
pub fn regions(width: f64) -> ((f64, f64), (f64, f64)) {
    let box_width = (width - SLIDER_WIDTH - GAP).max(1.0);
    ((0.0, box_width), (box_width + GAP, SLIDER_WIDTH))
}

/// `GetPixelColor`: the colour at (x, y), both 0..1, of the box or the slider.
pub fn pixel_color(style: DrawStyle, selected: Color, x: f64, y: f64, slider: bool) -> Color {
    let hsb = color::to_hsb(selected);
    if slider {
        return match style {
            DrawStyle::Hue => from_hsv(1.0 - y, 1.0, 1.0, 255),
            DrawStyle::Saturation => from_hsv(hsb.hue, 1.0 - y, hsb.brightness, 255),
            DrawStyle::Brightness => from_hsv(hsb.hue, hsb.saturation, 1.0 - y, 255),
            DrawStyle::Red => Color::rgb(to_byte(1.0 - y), selected.g, selected.b),
            DrawStyle::Green => Color::rgb(selected.r, to_byte(1.0 - y), selected.b),
            DrawStyle::Blue => Color::rgb(selected.r, selected.g, to_byte(1.0 - y)),
        };
    }
    match style {
        DrawStyle::Hue => from_hsv(hsb.hue, x, 1.0 - y, 255),
        DrawStyle::Saturation => from_hsv(x, hsb.saturation, 1.0 - y, 255),
        DrawStyle::Brightness => from_hsv(x, 1.0 - y, hsb.brightness, 255),
        DrawStyle::Red => Color::rgb(selected.r, to_byte(1.0 - y), to_byte(x)),
        DrawStyle::Green => Color::rgb(to_byte(1.0 - y), selected.g, to_byte(x)),
        DrawStyle::Blue => Color::rgb(to_byte(x), to_byte(1.0 - y), selected.b),
    }
}

/// `CreateBitmap`: the box or the slider as RGBA pixels.
pub fn bitmap(style: DrawStyle, selected: Color, width: u32, height: u32, slider: bool) -> Vec<u8> {
    let width = width.max(1);
    let height = height.max(1);
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        let ny = y as f64 / (height.max(2) - 1) as f64;
        for x in 0..width {
            let nx = x as f64 / (width.max(2) - 1) as f64;
            let c = pixel_color(style, selected, nx, ny, slider);
            out.extend_from_slice(&[c.r, c.g, c.b, 255]);
        }
    }
    out
}

/// `UpdateFromPointer`: the colour a press at (x, y) of a control `width` x `height`
/// picks; `on_slider` is whether the drag started on the slider.
pub fn pick(
    style: DrawStyle,
    selected: Color,
    width: f64,
    height: f64,
    x: f64,
    y: f64,
    on_slider: bool,
) -> Color {
    let ((box_x, box_w), _) = regions(width);
    let nx = clamp01((x - box_x) / (box_w - 1.0).max(1.0));
    let ny = clamp01(y / (height - 1.0).max(1.0));
    let sy = clamp01(y / (height - 1.0).max(1.0));
    let alpha = selected.a;
    let hsb = color::to_hsb(selected);
    match (style, on_slider) {
        (DrawStyle::Hue, true) => from_hsv(1.0 - sy, hsb.saturation, hsb.brightness, alpha),
        (DrawStyle::Hue, false) => from_hsv(hsb.hue, nx, 1.0 - ny, alpha),
        (DrawStyle::Saturation, true) => from_hsv(hsb.hue, 1.0 - sy, hsb.brightness, alpha),
        (DrawStyle::Saturation, false) => from_hsv(nx, hsb.saturation, 1.0 - ny, alpha),
        (DrawStyle::Brightness, true) => from_hsv(hsb.hue, hsb.saturation, 1.0 - sy, alpha),
        (DrawStyle::Brightness, false) => from_hsv(nx, 1.0 - ny, hsb.brightness, alpha),
        (DrawStyle::Red, true) => Color::argb(alpha, to_byte(1.0 - sy), selected.g, selected.b),
        (DrawStyle::Red, false) => Color::argb(alpha, selected.r, to_byte(1.0 - ny), to_byte(nx)),
        (DrawStyle::Green, true) => Color::argb(alpha, selected.r, to_byte(1.0 - sy), selected.b),
        (DrawStyle::Green, false) => Color::argb(alpha, to_byte(1.0 - ny), selected.g, to_byte(nx)),
        (DrawStyle::Blue, true) => Color::argb(alpha, selected.r, selected.g, to_byte(1.0 - sy)),
        (DrawStyle::Blue, false) => Color::argb(alpha, to_byte(nx), to_byte(1.0 - ny), selected.b),
    }
}

/// `GetBoxPoint`: the circle's centre, as fractions of the box.
pub fn box_point(style: DrawStyle, selected: Color) -> (f64, f64) {
    let hsb = color::to_hsb(selected);
    let (r, g, b) = (
        selected.r as f64 / 255.0,
        selected.g as f64 / 255.0,
        selected.b as f64 / 255.0,
    );
    match style {
        DrawStyle::Hue => (hsb.saturation, 1.0 - hsb.brightness),
        DrawStyle::Saturation => (hsb.hue, 1.0 - hsb.brightness),
        DrawStyle::Brightness => (hsb.hue, 1.0 - hsb.saturation),
        DrawStyle::Red => (b, 1.0 - g),
        DrawStyle::Green => (b, 1.0 - r),
        DrawStyle::Blue => (r, 1.0 - g),
    }
}

/// `GetSliderY`: the marker's height, as a fraction of the slider from its top.
pub fn slider_y(style: DrawStyle, selected: Color) -> f64 {
    let hsb = color::to_hsb(selected);
    let value = match style {
        DrawStyle::Hue => hsb.hue,
        DrawStyle::Saturation => hsb.saturation,
        DrawStyle::Brightness => hsb.brightness,
        DrawStyle::Red => selected.r as f64 / 255.0,
        DrawStyle::Green => selected.g as f64 / 255.0,
        DrawStyle::Blue => selected.b as f64 / 255.0,
    };
    1.0 - value
}

/// `AddRecentColor`: to the front, once, at most 32.
pub fn add_recent(recent: &mut Vec<Color>, color: Color) {
    recent.retain(|c| *c != color);
    recent.insert(0, color);
    recent.truncate(color::RECENT_COLORS_MAX);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_sectors() {
        assert_eq!(from_hsv(0.0, 1.0, 1.0, 255), Color::RED);
        assert_eq!(from_hsv(1.0 / 3.0, 1.0, 1.0, 255), Color::rgb(0, 255, 0));
        assert_eq!(from_hsv(2.0 / 3.0, 1.0, 1.0, 255), Color::rgb(0, 0, 255));
        assert_eq!(from_hsv(1.0, 1.0, 1.0, 255), Color::RED);
        assert_eq!(from_hsv(0.5, 0.0, 0.5, 9), Color::argb(9, 128, 128, 128));
    }

    #[test]
    fn the_hue_slider_runs_red_to_red_upwards() {
        assert_eq!(
            pixel_color(DrawStyle::Hue, Color::RED, 0.0, 0.0, true),
            Color::RED
        );
        assert_eq!(
            pixel_color(DrawStyle::Hue, Color::RED, 0.0, 1.0, true),
            Color::RED
        );
        assert_eq!(
            pixel_color(DrawStyle::Hue, Color::RED, 0.0, 0.5, true),
            Color::rgb(0, 255, 255)
        );
    }

    #[test]
    fn a_press_in_the_box_keeps_hue_and_alpha() {
        let start = Color::argb(200, 255, 0, 0);
        // Top right of a 342 x 300 control's box: full saturation and brightness.
        let ((_, bw), _) = regions(342.0);
        let c = pick(DrawStyle::Hue, start, 342.0, 300.0, bw - 1.0, 0.0, false);
        assert_eq!(c, start);
        // Bottom: black.
        let c = pick(DrawStyle::Hue, start, 342.0, 300.0, 10.0, 299.0, false);
        assert_eq!(c, Color::argb(200, 0, 0, 0));
        // The red slider at the top: full red.
        let c = pick(
            DrawStyle::Red,
            Color::rgb(0, 10, 20),
            342.0,
            300.0,
            330.0,
            0.0,
            true,
        );
        assert_eq!(c, Color::rgb(255, 10, 20));
    }

    #[test]
    fn markers() {
        assert_eq!(box_point(DrawStyle::Hue, Color::RED), (1.0, 0.0));
        assert_eq!(slider_y(DrawStyle::Hue, Color::RED), 1.0);
        assert_eq!(slider_y(DrawStyle::Blue, Color::rgb(0, 0, 255)), 0.0);
    }

    #[test]
    fn recent_colours() {
        let mut recent = vec![Color::RED, Color::BLACK];
        add_recent(&mut recent, Color::BLACK);
        assert_eq!(recent, vec![Color::BLACK, Color::RED]);
        for i in 0..40 {
            add_recent(&mut recent, Color::rgb(i, 0, 0));
        }
        assert_eq!(recent.len(), 32);
        assert_eq!(recent[0], Color::rgb(39, 0, 0));
    }
}
