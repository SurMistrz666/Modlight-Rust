//! `SKColor`: an sRGB colour with straight alpha, and SkiaSharp's HSL/HSV conversions.

use modlight_editor::color::Color;

/// `SKColor` (straight, not premultiplied).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct C {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

const EPSILON: f32 = 0.001;

impl C {
    /// `SKColors.Transparent` is white with zero alpha (`0x00FFFFFF`); gradients that
    /// fade to it pass through grey, as upstream's do.
    pub const TRANSPARENT: C = C::new(255, 255, 255, 0);
    /// `default(SKColor)` and empty memory.
    pub const ZERO: C = C::new(0, 0, 0, 0);
    pub const BLACK: C = C::rgb(0, 0, 0);
    pub const WHITE: C = C::rgb(255, 255, 255);
    pub const RED: C = C::rgb(255, 0, 0);
    pub const ORANGE: C = C::rgb(255, 165, 0);
    pub const GRAY: C = C::rgb(128, 128, 128);
    pub const YELLOW: C = C::rgb(255, 255, 0);
    pub const GOLD: C = C::rgb(255, 215, 0);
    pub const BLUE: C = C::rgb(0, 0, 255);
    pub const CYAN: C = C::rgb(0, 255, 255);
    pub const DARK_GRAY: C = C::rgb(169, 169, 169);
    pub const LIGHT_GRAY: C = C::rgb(211, 211, 211);
    pub const SILVER: C = C::rgb(192, 192, 192);
    pub const LIME: C = C::rgb(0, 255, 0);
    pub const MAGENTA: C = C::rgb(255, 0, 255);
    pub const GREEN: C = C::rgb(0, 128, 0);
    pub const PURPLE: C = C::rgb(128, 0, 128);
    pub const DEEP_SKY_BLUE: C = C::rgb(0, 191, 255);
    pub const DODGER_BLUE: C = C::rgb(30, 144, 255);
    pub const HOT_PINK: C = C::rgb(255, 105, 180);

    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> C {
        C { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> C {
        C { r, g, b, a: 255 }
    }

    /// `new SKColor(uint argb)`.
    pub const fn argb(v: u32) -> C {
        C {
            a: (v >> 24) as u8,
            r: (v >> 16) as u8,
            g: (v >> 8) as u8,
            b: v as u8,
        }
    }

    pub fn with_alpha(self, a: u8) -> C {
        C { a, ..self }
    }

    pub fn red(self) -> u8 {
        self.r
    }

    pub fn from_editor(c: Color) -> C {
        C::new(c.r, c.g, c.b, c.a)
    }

    pub fn to_editor(self) -> Color {
        Color::rgba(self.r, self.g, self.b, self.a)
    }

    pub fn to_skia(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba8(self.r, self.g, self.b, self.a)
    }

    /// `SKColor.FromHsl(h 0..360, s 0..100, l 0..100, a)`.
    pub fn from_hsl(h: f32, s: f32, l: f32, a: u8) -> C {
        let h = h / 360.0;
        let s = s / 100.0;
        let l = l / 100.0;
        let mut r = l * 255.0;
        let mut g = l * 255.0;
        let mut b = l * 255.0;
        if s.abs() > EPSILON {
            let v2 = if l < 0.5 {
                l * (1.0 + s)
            } else {
                (l + s) - (s * l)
            };
            let v1 = 2.0 * l - v2;
            r = 255.0 * hue_to_rgb(v1, v2, h + 1.0 / 3.0);
            g = 255.0 * hue_to_rgb(v1, v2, h);
            b = 255.0 * hue_to_rgb(v1, v2, h - 1.0 / 3.0);
        }
        C::new(r as u8, g as u8, b as u8, a)
    }

    /// `SKColor.FromHsv(h 0..360, s 0..100, v 0..100, a)`.
    pub fn from_hsv(h: f32, s: f32, v: f32, a: u8) -> C {
        let mut h = h / 360.0;
        let s = s / 100.0;
        let v = v / 100.0;
        let (mut r, mut g, mut b) = (v, v, v);
        if s.abs() > EPSILON {
            h *= 6.0;
            if (h - 6.0).abs() < EPSILON {
                h = 0.0;
            }
            let hi = h as i32;
            let v1 = v * (1.0 - s);
            let v2 = v * (1.0 - s * (h - hi as f32));
            let v3 = v * (1.0 - s * (1.0 - (h - hi as f32)));
            (r, g, b) = match hi {
                0 => (v, v3, v1),
                1 => (v2, v, v1),
                2 => (v1, v, v3),
                3 => (v1, v2, v),
                4 => (v3, v1, v),
                _ => (v, v1, v2),
            };
        }
        C::new((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, a)
    }

    /// `SKColor.ToHsl`: (h 0..360, s 0..100, l 0..100).
    pub fn to_hsl(self) -> (f32, f32, f32) {
        let r = self.r as f32 / 255.0;
        let g = self.g as f32 / 255.0;
        let b = self.b as f32 / 255.0;
        let min = r.min(g).min(b);
        let max = r.max(g).max(b);
        let delta = max - min;
        let mut h = 0.0;
        let mut s = 0.0;
        let l = (max + min) / 2.0;
        if delta.abs() > EPSILON {
            s = if l < 0.5 {
                delta / (max + min)
            } else {
                delta / (2.0 - max - min)
            };
            h = hue_of(r, g, b, max, delta);
        }
        (h * 360.0, s * 100.0, l * 100.0)
    }

    /// `SKColor.ToHsv`: (h 0..360, s 0..100, v 0..100).
    pub fn to_hsv(self) -> (f32, f32, f32) {
        let r = self.r as f32 / 255.0;
        let g = self.g as f32 / 255.0;
        let b = self.b as f32 / 255.0;
        let min = r.min(g).min(b);
        let max = r.max(g).max(b);
        let delta = max - min;
        let mut h = 0.0;
        let mut s = 0.0;
        if delta.abs() > EPSILON {
            s = delta / max;
            h = hue_of(r, g, b, max, delta);
        }
        (h * 360.0, s * 100.0, max * 100.0)
    }
}

fn hue_of(r: f32, g: f32, b: f32, max: f32, delta: f32) -> f32 {
    let dr = (((max - r) / 6.0) + (delta / 2.0)) / delta;
    let dg = (((max - g) / 6.0) + (delta / 2.0)) / delta;
    let db = (((max - b) / 6.0) + (delta / 2.0)) / delta;
    let mut h = if (r - max).abs() < EPSILON {
        db - dg
    } else if (g - max).abs() < EPSILON {
        (1.0 / 3.0) + dr - db
    } else {
        (2.0 / 3.0) + dg - dr
    };
    if h < 0.0 {
        h += 1.0;
    }
    if h > 1.0 {
        h -= 1.0;
    }
    h
}

fn hue_to_rgb(v1: f32, v2: f32, mut vh: f32) -> f32 {
    if vh < 0.0 {
        vh += 1.0;
    }
    if vh > 1.0 {
        vh -= 1.0;
    }
    if 6.0 * vh < 1.0 {
        return v1 + (v2 - v1) * 6.0 * vh;
    }
    if 2.0 * vh < 1.0 {
        return v2;
    }
    if 3.0 * vh < 2.0 {
        return v1 + (v2 - v1) * ((2.0 / 3.0) - vh) * 6.0;
    }
    v1
}

/// `MathF.Round` / `Math.Round`: half to even.
#[inline]
pub fn round(v: f32) -> f32 {
    v.round_ties_even()
}

#[inline]
pub fn roundd(v: f64) -> f64 {
    v.round_ties_even()
}

/// `(byte)value` in C#: truncation, saturating at the ends (.NET's conversion).
#[inline]
pub fn byte(v: f32) -> u8 {
    v as u8
}

/// `ClampToByte` as the effects write it: 0 and 255 at the ends, `MathF.Round` between.
#[inline]
pub fn clamp_to_byte(v: f32) -> u8 {
    if v <= 0.0 {
        0
    } else if v >= 255.0 {
        255
    } else {
        round(v) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsl_round_trip_of_primaries() {
        assert_eq!(C::from_hsl(0.0, 100.0, 50.0, 255), C::rgb(255, 0, 0));
        assert_eq!(C::from_hsl(120.0, 100.0, 50.0, 255), C::rgb(0, 255, 0));
        assert_eq!(C::from_hsv(240.0, 100.0, 100.0, 255), C::rgb(0, 0, 255));
        let (h, s, l) = C::rgb(0, 0, 255).to_hsl();
        assert!((h - 240.0).abs() < 0.01 && (s - 100.0).abs() < 0.01 && (l - 50.0).abs() < 0.01);
        assert_eq!(round(2.5), 2.0);
        assert_eq!(round(3.5), 4.0);
    }
}
