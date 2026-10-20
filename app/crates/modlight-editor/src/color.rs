//! Colours as upstream keeps them: hex strings (`"#AARRGGBB"`) on the annotations,
//! parsed with `SKColor.Parse` / Avalonia's `Color.Parse`.

/// An sRGB colour with straight (not premultiplied) alpha.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }

    pub const fn argb(a: u8, r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a }
    }

    /// `Color.Parse`: `#RGB`, `#ARGB`, `#RRGGBB`, `#AARRGGBB` or a known colour name.
    pub fn parse(text: &str) -> Option<Color> {
        let text = text.trim();
        if let Some(hex) = text.strip_prefix('#') {
            let digits: Vec<u8> = hex
                .chars()
                .map(|c| c.to_digit(16).map(|d| d as u8))
                .collect::<Option<Vec<u8>>>()?;
            let pair = |i: usize| digits[i] * 16 + digits[i + 1];
            let twice = |i: usize| digits[i] * 17;
            return match digits.len() {
                3 => Some(Color::rgb(twice(0), twice(1), twice(2))),
                4 => Some(Color::argb(twice(0), twice(1), twice(2), twice(3))),
                6 => Some(Color::rgb(pair(0), pair(2), pair(4))),
                8 => Some(Color::argb(pair(0), pair(2), pair(4), pair(6))),
                _ => None,
            };
        }
        named(text)
    }

    /// `Color.Parse` that falls back to transparent, for values the UI never produces.
    pub fn parse_or_transparent(text: &str) -> Color {
        Color::parse(text).unwrap_or(Color::TRANSPARENT)
    }

    /// `$"#{A:X2}{R:X2}{G:X2}{B:X2}"`, the form the view model writes back.
    pub fn to_hex_argb(&self) -> String {
        format!("#{:02X}{:02X}{:02X}{:02X}", self.a, self.r, self.g, self.b)
    }

    /// `#RRGGBB` (the colour sampler's form).
    pub fn to_hex_rgb(&self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    /// `ARGB` packed into a `u32`, as `SmartEraserAnnotation.PackColor` does.
    pub fn pack(&self) -> u32 {
        (self.a as u32) << 24 | (self.r as u32) << 16 | (self.g as u32) << 8 | self.b as u32
    }

    pub fn unpack(value: u32) -> Color {
        Color::argb(
            (value >> 24) as u8,
            (value >> 16) as u8,
            (value >> 8) as u8,
            value as u8,
        )
    }

    pub fn with_alpha(self, a: u8) -> Color {
        Color { a, ..self }
    }

    /// Luminance used by the editor's contrast decisions (`IsColorLight`).
    pub fn luminance(&self) -> f64 {
        (0.299 * self.r as f64 + 0.587 * self.g as f64 + 0.114 * self.b as f64) / 255.0
    }

    pub fn to_skia(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba8(self.r, self.g, self.b, self.a)
    }

    pub fn to_egui(self) -> egui::Color32 {
        egui::Color32::from_rgba_unmultiplied(self.r, self.g, self.b, self.a)
    }

    pub fn from_egui(color: egui::Color32) -> Color {
        let [r, g, b, a] = color.to_srgba_unmultiplied();
        Color::rgba(r, g, b, a)
    }
}

/// The Avalonia `KnownColor` names the editor uses or that a settings file may carry.
fn named(name: &str) -> Option<Color> {
    let color = match name.to_ascii_lowercase().as_str() {
        "transparent" => Color::TRANSPARENT,
        "black" => Color::BLACK,
        "white" => Color::WHITE,
        "red" => Color::rgb(255, 0, 0),
        "green" => Color::rgb(0, 128, 0),
        "lime" => Color::rgb(0, 255, 0),
        "blue" => Color::rgb(0, 0, 255),
        "yellow" => Color::rgb(255, 255, 0),
        "orange" => Color::rgb(255, 165, 0),
        "gray" | "grey" => Color::rgb(128, 128, 128),
        "lightgray" | "lightgrey" => Color::rgb(211, 211, 211),
        "darkgray" | "darkgrey" => Color::rgb(169, 169, 169),
        "cyan" | "aqua" => Color::rgb(0, 255, 255),
        "magenta" | "fuchsia" => Color::rgb(255, 0, 255),
        "purple" => Color::rgb(128, 0, 128),
        "pink" => Color::rgb(255, 192, 203),
        "brown" => Color::rgb(165, 42, 42),
        _ => return None,
    };
    Some(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_hex_form() {
        assert_eq!(Color::parse("#ef4444"), Some(Color::rgb(0xef, 0x44, 0x44)));
        assert_eq!(
            Color::parse("#80FF0000"),
            Some(Color::argb(0x80, 0xff, 0, 0))
        );
        assert_eq!(Color::parse("#fff"), Some(Color::WHITE));
        assert_eq!(Color::parse("#8f00"), Some(Color::argb(0x88, 0xff, 0, 0)));
        assert_eq!(Color::parse("Transparent"), Some(Color::TRANSPARENT));
        assert_eq!(Color::parse("#12345"), None);
    }

    #[test]
    fn writes_argb_hex_like_the_view_model() {
        assert_eq!(Color::rgb(0xF2, 0x3C, 0x3C).to_hex_argb(), "#FFF23C3C");
        assert_eq!(
            Color::unpack(Color::argb(1, 2, 3, 4).pack()),
            Color::argb(1, 2, 3, 4)
        );
    }
}
