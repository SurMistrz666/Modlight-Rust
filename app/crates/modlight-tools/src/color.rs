//! Colours as ShareX handles them (`ShareX.HelpersLib` `ColorHelpers`, `HSB`, `CMYK`,
//! `MyColor`, `System.Drawing.KnownColor`): the conversions the colour picker shows, the
//! texts it copies, the names it gives, and the way `ApplicationConfig.RecentColors`
//! writes a colour into the settings file.

use std::fmt::Write as _;

use modlight_core::culture::Culture;

/// `System.Drawing.Color` without the "known colour" bookkeeping: alpha and RGB.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color {
    pub a: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { a: 255, r, g, b }
    }

    pub const fn argb(a: u8, r: u8, g: u8, b: u8) -> Color {
        Color { a, r, g, b }
    }

    /// `Color.FromArgb(int a, int r, int g, int b)` with values ShareX has already
    /// clamped; anything outside 0..255 is clamped here instead of throwing.
    pub fn from_ints(a: i64, r: i64, g: i64, b: i64) -> Color {
        let c = |v: i64| v.clamp(0, 255) as u8;
        Color::argb(c(a), c(r), c(g), c(b))
    }

    pub fn to_array(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }

    pub fn from_rgba(rgba: [u8; 4]) -> Color {
        Color::argb(rgba[3], rgba[0], rgba[1], rgba[2])
    }

    /// `Color.Red`, what a fresh colour picker starts with.
    pub const RED: Color = Color::rgb(255, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const CORNFLOWER_BLUE: Color = Color::rgb(100, 149, 237);
}

/// `ColorFormat`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorFormat {
    Rgb,
    Rgba,
    Argb,
}

/// `HSB`: hue, saturation and brightness from 0 to 1, alpha 0 to 255.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hsb {
    pub hue: f64,
    pub saturation: f64,
    pub brightness: f64,
    pub alpha: u8,
}

impl Hsb {
    /// `new HSB(double, double, double, int)`: each part clamped to 0..1.
    pub fn new(hue: f64, saturation: f64, brightness: f64, alpha: i64) -> Hsb {
        Hsb {
            hue: valid(hue),
            saturation: valid(saturation),
            brightness: valid(brightness),
            alpha: alpha.clamp(0, 255) as u8,
        }
    }

    /// `new HSB(int hue, int saturation, int brightness, int alpha)`: degrees and
    /// percentages, the way the colour picker's boxes hold them.
    pub fn from_ints(hue360: i64, saturation100: i64, brightness100: i64, alpha: i64) -> Hsb {
        Hsb::new(
            hue360 as f64 / 360.0,
            saturation100 as f64 / 100.0,
            brightness100 as f64 / 100.0,
            alpha,
        )
    }

    pub fn hue360(&self) -> f64 {
        self.hue * 360.0
    }

    pub fn saturation100(&self) -> f64 {
        self.saturation * 100.0
    }

    pub fn brightness100(&self) -> f64 {
        self.brightness * 100.0
    }

    /// `ColorHelpers.HSBToColor`.
    pub fn to_color(&self) -> Color {
        let max = (self.brightness * 255.0).round_ties_even() as i64;
        let min =
            ((1.0 - self.saturation) * (self.brightness / 1.0) * 255.0).round_ties_even() as i64;
        let q = (max - min) as f64 / 255.0;
        let h = self.hue;
        let a = self.alpha as i64;
        let mid = |offset: f64, base: i64, up: bool| -> i64 {
            let delta = (h - offset) * q * 1530.0;
            if up {
                (delta + base as f64).round_ties_even() as i64
            } else {
                (-delta + base as f64).round_ties_even() as i64
            }
        };
        if (0.0..=1.0 / 6.0).contains(&h) {
            return Color::from_ints(a, max, mid(0.0, min, true), min);
        }
        if h <= 1.0 / 3.0 {
            return Color::from_ints(a, mid(1.0 / 6.0, max, false), max, min);
        }
        if h <= 0.5 {
            return Color::from_ints(a, min, max, mid(1.0 / 3.0, min, true));
        }
        if h <= 2.0 / 3.0 {
            return Color::from_ints(a, min, mid(0.5, max, false), max);
        }
        if h <= 5.0 / 6.0 {
            return Color::from_ints(a, mid(2.0 / 3.0, min, true), min, max);
        }
        if h <= 1.0 {
            return Color::from_ints(a, max, min, mid(5.0 / 6.0, max, false));
        }
        Color::from_ints(a, 0, 0, 0)
    }
}

/// `CMYK`: cyan, magenta, yellow and key from 0 to 1, alpha 0 to 255.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cmyk {
    pub cyan: f64,
    pub magenta: f64,
    pub yellow: f64,
    pub key: f64,
    pub alpha: u8,
}

impl Cmyk {
    pub fn new(cyan: f64, magenta: f64, yellow: f64, key: f64, alpha: i64) -> Cmyk {
        Cmyk {
            cyan: valid(cyan),
            magenta: valid(magenta),
            yellow: valid(yellow),
            key: valid(key),
            alpha: alpha.clamp(0, 255) as u8,
        }
    }

    /// `new CMYK(int, int, int, int, int)`: percentages.
    pub fn from_ints(c100: i64, m100: i64, y100: i64, k100: i64, alpha: i64) -> Cmyk {
        Cmyk::new(
            c100 as f64 / 100.0,
            m100 as f64 / 100.0,
            y100 as f64 / 100.0,
            k100 as f64 / 100.0,
            alpha,
        )
    }

    pub fn cyan100(&self) -> f64 {
        self.cyan * 100.0
    }

    pub fn magenta100(&self) -> f64 {
        self.magenta * 100.0
    }

    pub fn yellow100(&self) -> f64 {
        self.yellow * 100.0
    }

    pub fn key100(&self) -> f64 {
        self.key * 100.0
    }

    /// `ColorHelpers.CMYKToColor`.
    pub fn to_color(&self) -> Color {
        if self.cyan == 0.0 && self.magenta == 0.0 && self.yellow == 0.0 && self.key == 1.0 {
            return Color::argb(self.alpha, 0, 0, 0);
        }
        let c = self.cyan * (1.0 - self.key) + self.key;
        let m = self.magenta * (1.0 - self.key) + self.key;
        let y = self.yellow * (1.0 - self.key) + self.key;
        Color::from_ints(
            self.alpha as i64,
            ((1.0 - c) * 255.0).round_ties_even() as i64,
            ((1.0 - m) * 255.0).round_ties_even() as i64,
            ((1.0 - y) * 255.0).round_ties_even() as i64,
        )
    }
}

fn valid(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

/// `ColorHelpers.ColorToHSB`.
pub fn to_hsb(color: Color) -> Hsb {
    let (r, g, b) = (color.r as i64, color.g as i64, color.b as i64);
    let (mut max, mut min) = if r > g { (r, g) } else { (g, r) };
    if b > max {
        max = b;
    } else if b < min {
        min = b;
    }
    let diff = max - min;
    let brightness = max as f64 / 255.0;
    let saturation = if max == 0 {
        0.0
    } else {
        diff as f64 / max as f64
    };
    let q = if diff == 0 { 0.0 } else { 60.0 / diff as f64 };
    let hue = if max == r {
        if g < b {
            (360.0 + q * (g - b) as f64) / 360.0
        } else {
            q * (g - b) as f64 / 360.0
        }
    } else if max == g {
        (120.0 + q * (b - r) as f64) / 360.0
    } else if max == b {
        (240.0 + q * (r - g) as f64) / 360.0
    } else {
        0.0
    };
    Hsb::new(hue, saturation, brightness, color.a as i64)
}

/// `ColorHelpers.ColorToCMYK`.
pub fn to_cmyk(color: Color) -> Cmyk {
    if color.r == 0 && color.g == 0 && color.b == 0 {
        return Cmyk::new(0.0, 0.0, 0.0, 1.0, color.a as i64);
    }
    let c = 1.0 - color.r as f64 / 255.0;
    let m = 1.0 - color.g as f64 / 255.0;
    let y = 1.0 - color.b as f64 / 255.0;
    let k = c.min(m.min(y));
    Cmyk::new(
        (c - k) / (1.0 - k),
        (m - k) / (1.0 - k),
        (y - k) / (1.0 - k),
        k,
        color.a as i64,
    )
}

/// `ColorHelpers.ColorToHex`: upper-case digits, no `#`.
pub fn to_hex(color: Color, format: ColorFormat) -> String {
    match format {
        ColorFormat::Rgb => format!("{:02X}{:02X}{:02X}", color.r, color.g, color.b),
        ColorFormat::Rgba => format!(
            "{:02X}{:02X}{:02X}{:02X}",
            color.r, color.g, color.b, color.a
        ),
        ColorFormat::Argb => format!(
            "{:02X}{:02X}{:02X}{:02X}",
            color.a, color.r, color.g, color.b
        ),
    }
}

/// `ColorHelpers.ColorToDecimal` (an `int`: RGBA and ARGB may come out negative).
pub fn to_decimal(color: Color, format: ColorFormat) -> i32 {
    let (a, r, g, b) = (
        color.a as u32,
        color.r as u32,
        color.g as u32,
        color.b as u32,
    );
    (match format {
        ColorFormat::Rgb => (r << 16) | (g << 8) | b,
        ColorFormat::Rgba => (r << 24) | (g << 16) | (b << 8) | a,
        ColorFormat::Argb => (a << 24) | (r << 16) | (g << 8) | b,
    }) as i32
}

/// `ColorHelpers.HexToColor`: `None` stands for `Color.Empty`.
pub fn hex_to_color(hex: &str, format: ColorFormat) -> Option<Color> {
    let mut hex = hex;
    if hex.is_empty() {
        return None;
    }
    if let Some(rest) = hex.strip_prefix('#') {
        hex = rest;
    } else if hex.get(..2).is_some_and(|p| p.eq_ignore_ascii_case("0x")) {
        hex = &hex[2..];
    }
    let wanted = if format == ColorFormat::Rgb { 6 } else { 8 };
    if hex.chars().count() != wanted || !hex.is_ascii() {
        return None;
    }
    let part = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some(match format {
        ColorFormat::Rgb => Color::rgb(part(0)?, part(2)?, part(4)?),
        ColorFormat::Rgba => Color::argb(part(6)?, part(0)?, part(2)?, part(4)?),
        ColorFormat::Argb => Color::argb(part(0)?, part(2)?, part(4)?, part(6)?),
    })
}

/// `ColorHelpers.DecimalToColor`.
pub fn decimal_to_color(dec: i64, format: ColorFormat) -> Color {
    let dec = dec as i32 as u32;
    let byte = |shift: u32| ((dec >> shift) & 0xFF) as u8;
    match format {
        ColorFormat::Rgb => Color::rgb(byte(16), byte(8), byte(0)),
        ColorFormat::Rgba => Color::argb(byte(0), byte(24), byte(16), byte(8)),
        ColorFormat::Argb => Color::argb(byte(24), byte(16), byte(8), byte(0)),
    }
}

/// `ColorHelpers.ParseColor`: `#RRGGBB`, `0xRRGGBB`, `RRGGBB`, `rgb(r, g, b)` or
/// `r g b` / `r, g, b`, at most 20 characters after trimming.
pub fn parse_color(text: &str) -> Option<Color> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > 20 {
        return None;
    }
    let hex = regex::Regex::new(r"(?i)^(?:#|0x)?((?:[0-9A-F]{2}){3})$").expect("valid");
    if let Some(captures) = hex.captures(text) {
        return hex_to_color(&captures[1], ColorFormat::Rgb);
    }
    let component = r"([1]?[0-9]?[0-9]|2[0-4][0-9]|25[0-5])";
    let rgb = regex::Regex::new(&format!(
        r"^(?:rgb\()?{component}(?:\s|,)+{component}(?:\s|,)+{component}\)?$"
    ))
    .expect("valid");
    let captures = rgb.captures(text)?;
    let value = |i: usize| captures[i].parse::<u8>().ok();
    Some(Color::rgb(value(1)?, value(2)?, value(3)?))
}

/// `ColorHelpers.StandardColors`: the palette's 32 swatches.
pub const STANDARD_COLORS: [Color; 32] = [
    Color::rgb(0, 0, 0),
    Color::rgb(64, 64, 64),
    Color::rgb(255, 0, 0),
    Color::rgb(255, 106, 0),
    Color::rgb(255, 216, 0),
    Color::rgb(182, 255, 0),
    Color::rgb(76, 255, 0),
    Color::rgb(0, 255, 33),
    Color::rgb(0, 255, 144),
    Color::rgb(0, 255, 255),
    Color::rgb(0, 148, 255),
    Color::rgb(0, 38, 255),
    Color::rgb(72, 0, 255),
    Color::rgb(178, 0, 255),
    Color::rgb(255, 0, 220),
    Color::rgb(255, 0, 110),
    Color::rgb(255, 255, 255),
    Color::rgb(128, 128, 128),
    Color::rgb(127, 0, 0),
    Color::rgb(127, 51, 0),
    Color::rgb(127, 106, 0),
    Color::rgb(91, 127, 0),
    Color::rgb(38, 127, 0),
    Color::rgb(0, 127, 14),
    Color::rgb(0, 127, 70),
    Color::rgb(0, 127, 127),
    Color::rgb(0, 74, 127),
    Color::rgb(0, 19, 127),
    Color::rgb(33, 0, 127),
    Color::rgb(87, 0, 127),
    Color::rgb(127, 0, 110),
    Color::rgb(127, 0, 55),
];

/// `HelpersOptions.RecentColorsMax`.
pub const RECENT_COLORS_MAX: usize = 32;

/// `System.Drawing.KnownColor`, in the enum's order, with the values .NET gives them.
/// The system colours (the first 26 and those after `YellowGreen` but `RebeccaPurple`)
/// follow the Windows theme on Windows; they carry the default Windows 10/11 values here,
/// which is what a Windows install with the default theme matches against.
pub const KNOWN_COLORS: &[(&str, Color)] = &[
    ("ActiveBorder", Color::rgb(180, 180, 180)),
    ("ActiveCaption", Color::rgb(153, 180, 209)),
    ("ActiveCaptionText", Color::rgb(0, 0, 0)),
    ("AppWorkspace", Color::rgb(171, 171, 171)),
    ("Control", Color::rgb(240, 240, 240)),
    ("ControlDark", Color::rgb(160, 160, 160)),
    ("ControlDarkDark", Color::rgb(105, 105, 105)),
    ("ControlLight", Color::rgb(227, 227, 227)),
    ("ControlLightLight", Color::rgb(255, 255, 255)),
    ("ControlText", Color::rgb(0, 0, 0)),
    ("Desktop", Color::rgb(0, 0, 0)),
    ("GrayText", Color::rgb(109, 109, 109)),
    ("Highlight", Color::rgb(0, 120, 215)),
    ("HighlightText", Color::rgb(255, 255, 255)),
    ("HotTrack", Color::rgb(0, 102, 204)),
    ("InactiveBorder", Color::rgb(244, 247, 252)),
    ("InactiveCaption", Color::rgb(191, 205, 219)),
    ("InactiveCaptionText", Color::rgb(0, 0, 0)),
    ("Info", Color::rgb(255, 255, 225)),
    ("InfoText", Color::rgb(0, 0, 0)),
    ("Menu", Color::rgb(240, 240, 240)),
    ("MenuText", Color::rgb(0, 0, 0)),
    ("ScrollBar", Color::rgb(200, 200, 200)),
    ("Window", Color::rgb(255, 255, 255)),
    ("WindowFrame", Color::rgb(100, 100, 100)),
    ("WindowText", Color::rgb(0, 0, 0)),
    ("Transparent", Color::argb(0, 255, 255, 255)),
    ("AliceBlue", Color::rgb(240, 248, 255)),
    ("AntiqueWhite", Color::rgb(250, 235, 215)),
    ("Aqua", Color::rgb(0, 255, 255)),
    ("Aquamarine", Color::rgb(127, 255, 212)),
    ("Azure", Color::rgb(240, 255, 255)),
    ("Beige", Color::rgb(245, 245, 220)),
    ("Bisque", Color::rgb(255, 228, 196)),
    ("Black", Color::rgb(0, 0, 0)),
    ("BlanchedAlmond", Color::rgb(255, 235, 205)),
    ("Blue", Color::rgb(0, 0, 255)),
    ("BlueViolet", Color::rgb(138, 43, 226)),
    ("Brown", Color::rgb(165, 42, 42)),
    ("BurlyWood", Color::rgb(222, 184, 135)),
    ("CadetBlue", Color::rgb(95, 158, 160)),
    ("Chartreuse", Color::rgb(127, 255, 0)),
    ("Chocolate", Color::rgb(210, 105, 30)),
    ("Coral", Color::rgb(255, 127, 80)),
    ("CornflowerBlue", Color::rgb(100, 149, 237)),
    ("Cornsilk", Color::rgb(255, 248, 220)),
    ("Crimson", Color::rgb(220, 20, 60)),
    ("Cyan", Color::rgb(0, 255, 255)),
    ("DarkBlue", Color::rgb(0, 0, 139)),
    ("DarkCyan", Color::rgb(0, 139, 139)),
    ("DarkGoldenrod", Color::rgb(184, 134, 11)),
    ("DarkGray", Color::rgb(169, 169, 169)),
    ("DarkGreen", Color::rgb(0, 100, 0)),
    ("DarkKhaki", Color::rgb(189, 183, 107)),
    ("DarkMagenta", Color::rgb(139, 0, 139)),
    ("DarkOliveGreen", Color::rgb(85, 107, 47)),
    ("DarkOrange", Color::rgb(255, 140, 0)),
    ("DarkOrchid", Color::rgb(153, 50, 204)),
    ("DarkRed", Color::rgb(139, 0, 0)),
    ("DarkSalmon", Color::rgb(233, 150, 122)),
    ("DarkSeaGreen", Color::rgb(143, 188, 139)),
    ("DarkSlateBlue", Color::rgb(72, 61, 139)),
    ("DarkSlateGray", Color::rgb(47, 79, 79)),
    ("DarkTurquoise", Color::rgb(0, 206, 209)),
    ("DarkViolet", Color::rgb(148, 0, 211)),
    ("DeepPink", Color::rgb(255, 20, 147)),
    ("DeepSkyBlue", Color::rgb(0, 191, 255)),
    ("DimGray", Color::rgb(105, 105, 105)),
    ("DodgerBlue", Color::rgb(30, 144, 255)),
    ("Firebrick", Color::rgb(178, 34, 34)),
    ("FloralWhite", Color::rgb(255, 250, 240)),
    ("ForestGreen", Color::rgb(34, 139, 34)),
    ("Fuchsia", Color::rgb(255, 0, 255)),
    ("Gainsboro", Color::rgb(220, 220, 220)),
    ("GhostWhite", Color::rgb(248, 248, 255)),
    ("Gold", Color::rgb(255, 215, 0)),
    ("Goldenrod", Color::rgb(218, 165, 32)),
    ("Gray", Color::rgb(128, 128, 128)),
    ("Green", Color::rgb(0, 128, 0)),
    ("GreenYellow", Color::rgb(173, 255, 47)),
    ("Honeydew", Color::rgb(240, 255, 240)),
    ("HotPink", Color::rgb(255, 105, 180)),
    ("IndianRed", Color::rgb(205, 92, 92)),
    ("Indigo", Color::rgb(75, 0, 130)),
    ("Ivory", Color::rgb(255, 255, 240)),
    ("Khaki", Color::rgb(240, 230, 140)),
    ("Lavender", Color::rgb(230, 230, 250)),
    ("LavenderBlush", Color::rgb(255, 240, 245)),
    ("LawnGreen", Color::rgb(124, 252, 0)),
    ("LemonChiffon", Color::rgb(255, 250, 205)),
    ("LightBlue", Color::rgb(173, 216, 230)),
    ("LightCoral", Color::rgb(240, 128, 128)),
    ("LightCyan", Color::rgb(224, 255, 255)),
    ("LightGoldenrodYellow", Color::rgb(250, 250, 210)),
    ("LightGray", Color::rgb(211, 211, 211)),
    ("LightGreen", Color::rgb(144, 238, 144)),
    ("LightPink", Color::rgb(255, 182, 193)),
    ("LightSalmon", Color::rgb(255, 160, 122)),
    ("LightSeaGreen", Color::rgb(32, 178, 170)),
    ("LightSkyBlue", Color::rgb(135, 206, 250)),
    ("LightSlateGray", Color::rgb(119, 136, 153)),
    ("LightSteelBlue", Color::rgb(176, 196, 222)),
    ("LightYellow", Color::rgb(255, 255, 224)),
    ("Lime", Color::rgb(0, 255, 0)),
    ("LimeGreen", Color::rgb(50, 205, 50)),
    ("Linen", Color::rgb(250, 240, 230)),
    ("Magenta", Color::rgb(255, 0, 255)),
    ("Maroon", Color::rgb(128, 0, 0)),
    ("MediumAquamarine", Color::rgb(102, 205, 170)),
    ("MediumBlue", Color::rgb(0, 0, 205)),
    ("MediumOrchid", Color::rgb(186, 85, 211)),
    ("MediumPurple", Color::rgb(147, 112, 219)),
    ("MediumSeaGreen", Color::rgb(60, 179, 113)),
    ("MediumSlateBlue", Color::rgb(123, 104, 238)),
    ("MediumSpringGreen", Color::rgb(0, 250, 154)),
    ("MediumTurquoise", Color::rgb(72, 209, 204)),
    ("MediumVioletRed", Color::rgb(199, 21, 133)),
    ("MidnightBlue", Color::rgb(25, 25, 112)),
    ("MintCream", Color::rgb(245, 255, 250)),
    ("MistyRose", Color::rgb(255, 228, 225)),
    ("Moccasin", Color::rgb(255, 228, 181)),
    ("NavajoWhite", Color::rgb(255, 222, 173)),
    ("Navy", Color::rgb(0, 0, 128)),
    ("OldLace", Color::rgb(253, 245, 230)),
    ("Olive", Color::rgb(128, 128, 0)),
    ("OliveDrab", Color::rgb(107, 142, 35)),
    ("Orange", Color::rgb(255, 165, 0)),
    ("OrangeRed", Color::rgb(255, 69, 0)),
    ("Orchid", Color::rgb(218, 112, 214)),
    ("PaleGoldenrod", Color::rgb(238, 232, 170)),
    ("PaleGreen", Color::rgb(152, 251, 152)),
    ("PaleTurquoise", Color::rgb(175, 238, 238)),
    ("PaleVioletRed", Color::rgb(219, 112, 147)),
    ("PapayaWhip", Color::rgb(255, 239, 213)),
    ("PeachPuff", Color::rgb(255, 218, 185)),
    ("Peru", Color::rgb(205, 133, 63)),
    ("Pink", Color::rgb(255, 192, 203)),
    ("Plum", Color::rgb(221, 160, 221)),
    ("PowderBlue", Color::rgb(176, 224, 230)),
    ("Purple", Color::rgb(128, 0, 128)),
    ("Red", Color::rgb(255, 0, 0)),
    ("RosyBrown", Color::rgb(188, 143, 143)),
    ("RoyalBlue", Color::rgb(65, 105, 225)),
    ("SaddleBrown", Color::rgb(139, 69, 19)),
    ("Salmon", Color::rgb(250, 128, 114)),
    ("SandyBrown", Color::rgb(244, 164, 96)),
    ("SeaGreen", Color::rgb(46, 139, 87)),
    ("SeaShell", Color::rgb(255, 245, 238)),
    ("Sienna", Color::rgb(160, 82, 45)),
    ("Silver", Color::rgb(192, 192, 192)),
    ("SkyBlue", Color::rgb(135, 206, 235)),
    ("SlateBlue", Color::rgb(106, 90, 205)),
    ("SlateGray", Color::rgb(112, 128, 144)),
    ("Snow", Color::rgb(255, 250, 250)),
    ("SpringGreen", Color::rgb(0, 255, 127)),
    ("SteelBlue", Color::rgb(70, 130, 180)),
    ("Tan", Color::rgb(210, 180, 140)),
    ("Teal", Color::rgb(0, 128, 128)),
    ("Thistle", Color::rgb(216, 191, 216)),
    ("Tomato", Color::rgb(255, 99, 71)),
    ("Turquoise", Color::rgb(64, 224, 208)),
    ("Violet", Color::rgb(238, 130, 238)),
    ("Wheat", Color::rgb(245, 222, 179)),
    ("White", Color::rgb(255, 255, 255)),
    ("WhiteSmoke", Color::rgb(245, 245, 245)),
    ("Yellow", Color::rgb(255, 255, 0)),
    ("YellowGreen", Color::rgb(154, 205, 50)),
    ("ButtonFace", Color::rgb(240, 240, 240)),
    ("ButtonHighlight", Color::rgb(255, 255, 255)),
    ("ButtonShadow", Color::rgb(160, 160, 160)),
    ("GradientActiveCaption", Color::rgb(185, 209, 234)),
    ("GradientInactiveCaption", Color::rgb(215, 228, 242)),
    ("MenuBar", Color::rgb(240, 240, 240)),
    ("MenuHighlight", Color::rgb(0, 120, 215)),
    ("RebeccaPurple", Color::rgb(102, 51, 153)),
];

/// `KnownColor.AliceBlue ..= KnownColor.YellowGreen`: the 140 web colours.
pub fn web_colors() -> &'static [(&'static str, Color)] {
    let start = KNOWN_COLORS
        .iter()
        .position(|(n, _)| *n == "AliceBlue")
        .expect("listed");
    let end = KNOWN_COLORS
        .iter()
        .position(|(n, _)| *n == "YellowGreen")
        .expect("listed");
    &KNOWN_COLORS[start..=end]
}

/// `ColorHelpers.ColorDifference`: the sum of the RGB distances.
pub fn difference(a: Color, b: Color) -> i64 {
    (a.r as i64 - b.r as i64).abs()
        + (a.g as i64 - b.g as i64).abs()
        + (a.b as i64 - b.b as i64).abs()
}

/// `ColorHelpers.GetColorName`: the closest web colour, "Alice blue" style.
pub fn color_name(color: Color) -> String {
    // `Aggregate(Color.Black, ...)`: Black first, a later colour wins only when closer.
    let mut best = ("Black", Color::BLACK);
    for (name, known) in web_colors() {
        if difference(color, *known) < difference(color, best.1) {
            best = (name, *known);
        }
    }
    proper_name(best.0)
}

/// The `KnownColor` with exactly this colour, first in enum order
/// (`ScreenColorPickerTextFormatter.GetColorName`), or nothing.
pub fn exact_known_name(color: Color) -> &'static str {
    KNOWN_COLORS
        .iter()
        .find(|(_, known)| *known == color)
        .map(|(name, _)| *name)
        .unwrap_or("")
}

/// `Helpers.GetProperName`: "TopLeft" becomes "Top left", "Rotate180" "Rotate 180".
pub fn proper_name(name: &str) -> String {
    let mut out = String::new();
    let mut number = false;
    for (i, c) in name.chars().enumerate() {
        if i > 0 && (c.is_uppercase() || (!number && c.is_numeric())) {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
        number = c.is_numeric();
    }
    out
}

/// `MyColor.ToString()`: "Copy all details".
pub fn details_text(color: Color) -> String {
    let culture = Culture::current();
    let f1 = |v: f64| culture.fixed(v, 1);
    let hsb = to_hsb(color);
    let cmyk = to_cmyk(color);
    let mut out = String::new();
    let _ = write!(
        out,
        "RGBA (Red, Green, Blue, Alpha) = {}, {}, {}, {}\r\n",
        color.r, color.g, color.b, color.a
    );
    let _ = write!(
        out,
        "HSB (Hue, Saturation, Brightness) = {}°, {}%, {}%\r\n",
        f1(hsb.hue360()),
        f1(hsb.saturation100()),
        f1(hsb.brightness100())
    );
    let _ = write!(
        out,
        "CMYK (Cyan, Magenta, Yellow, Key) = {}%, {}%, {}%, {}%\r\n",
        f1(cmyk.cyan100()),
        f1(cmyk.magenta100()),
        f1(cmyk.yellow100()),
        f1(cmyk.key100())
    );
    let _ = write!(
        out,
        "Hex (RGB, RGBA, ARGB) = #{}, #{}, #{}\r\n",
        to_hex(color, ColorFormat::Rgb),
        to_hex(color, ColorFormat::Rgba),
        to_hex(color, ColorFormat::Argb)
    );
    let _ = write!(
        out,
        "Decimal (RGB, RGBA, ARGB) = {}, {}, {}",
        to_decimal(color, ColorFormat::Rgb),
        to_decimal(color, ColorFormat::Rgba),
        to_decimal(color, ColorFormat::Argb)
    );
    out
}

/// "Copy HSB": `{0:0.0}°, {1:0.0}%, {2:0.0}%`.
pub fn hsb_text(color: Color) -> String {
    let culture = Culture::current();
    let hsb = to_hsb(color);
    format!(
        "{}°, {}%, {}%",
        culture.fixed(hsb.hue360(), 1),
        culture.fixed(hsb.saturation100(), 1),
        culture.fixed(hsb.brightness100(), 1)
    )
}

/// "Copy CMYK": `{0:0.0}%, {1:0.0}%, {2:0.0}%, {3:0.0}%`.
pub fn cmyk_text(color: Color) -> String {
    let culture = Culture::current();
    let cmyk = to_cmyk(color);
    format!(
        "{}%, {}%, {}%, {}%",
        culture.fixed(cmyk.cyan100(), 1),
        culture.fixed(cmyk.magenta100(), 1),
        culture.fixed(cmyk.yellow100(), 1),
        culture.fixed(cmyk.key100(), 1)
    )
}

/// `ColorHelpers.PerceivedBrightness`.
pub fn perceived_brightness(color: Color) -> i64 {
    let (r, g, b) = (color.r as f64, color.g as f64, color.b as f64);
    (r * r * 0.299 + g * g * 0.587 + b * b * 0.114).sqrt() as i64
}

/// `ColorHelpers.IsLightColor`.
pub fn is_light(color: Color) -> bool {
    perceived_brightness(color) > 130
}

/// How Newtonsoft writes a `System.Drawing.Color` (`ColorConverter.ConvertToInvariantString`):
/// a known colour by name, an opaque one as "R, G, B", anything else as "A, R, G, B".
/// Colours the port creates are never "known", so only the numeric forms are written.
pub fn to_dotnet_string(color: Color) -> String {
    if color.a == 255 {
        format!("{}, {}, {}", color.r, color.g, color.b)
    } else {
        format!("{}, {}, {}, {}", color.a, color.r, color.g, color.b)
    }
}

/// The reverse of [`to_dotnet_string`], also reading known colour names and `#RRGGBB`.
pub fn parse_dotnet_string(text: &str) -> Option<Color> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Some((_, color)) = KNOWN_COLORS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(text))
    {
        return Some(*color);
    }
    if let Some(digits) = text.strip_prefix('#') {
        let value = u32::from_str_radix(digits, 16).ok()?;
        return Some(match digits.len() {
            6 => Color::rgb((value >> 16) as u8, (value >> 8) as u8, value as u8),
            8 => Color::argb(
                (value >> 24) as u8,
                (value >> 16) as u8,
                (value >> 8) as u8,
                value as u8,
            ),
            _ => return None,
        });
    }
    let parts: Vec<i64> = text
        .split([',', ';'])
        .map(|p| p.trim().parse::<i64>())
        .collect::<Result<_, _>>()
        .ok()?;
    match parts.as_slice() {
        [r, g, b] => Some(Color::from_ints(255, *r, *g, *b)),
        [a, r, g, b] => Some(Color::from_ints(*a, *r, *g, *b)),
        // A single number is an ARGB integer.
        [argb] => Some(decimal_to_color(*argb, ColorFormat::Argb)),
        _ => None,
    }
}

/// `Math.Round(value, digits, MidpointRounding.AwayFromZero)`.
pub fn round_away(value: f64, digits: i32) -> f64 {
    let scale = 10f64.powi(digits);
    (value * scale).round() / scale
}

#[cfg(test)]
mod tests {
    use super::*;
    use modlight_core::culture::set_thread_culture;

    #[test]
    fn hsb_round_trips_like_sharex() {
        let red = to_hsb(Color::RED);
        assert_eq!(red.hue360(), 0.0);
        assert_eq!(red.saturation100(), 100.0);
        assert_eq!(red.brightness100(), 100.0);
        assert_eq!(red.to_color(), Color::RED);
        let c = Color::rgb(18, 140, 200);
        let hsb = to_hsb(c);
        assert!((hsb.hue360() - 199.78).abs() < 0.01);
        assert_eq!(hsb.to_color(), c);
        // The picker's boxes hold whole numbers.
        let back = Hsb::from_ints(200, 91, 78, 255).to_color();
        assert_eq!(back, Color::rgb(18, 139, 199));
    }

    #[test]
    fn cmyk_of_black_is_all_key() {
        let black = to_cmyk(Color::BLACK);
        assert_eq!(
            (black.cyan, black.magenta, black.yellow, black.key),
            (0.0, 0.0, 0.0, 1.0)
        );
        assert_eq!(black.to_color(), Color::BLACK);
        let c = Color::rgb(255, 106, 0);
        let cmyk = to_cmyk(c);
        assert_eq!(cmyk.to_color(), c);
        assert_eq!(
            Cmyk::from_ints(0, 58, 100, 0, 255).to_color(),
            Color::rgb(255, 107, 0)
        );
    }

    #[test]
    fn hex_and_decimal() {
        let c = Color::argb(0x80, 0x12, 0xAB, 0xEF);
        assert_eq!(to_hex(c, ColorFormat::Rgb), "12ABEF");
        assert_eq!(to_hex(c, ColorFormat::Rgba), "12ABEF80");
        assert_eq!(to_hex(c, ColorFormat::Argb), "8012ABEF");
        assert_eq!(to_decimal(c, ColorFormat::Rgb), 0x12ABEF);
        assert_eq!(to_decimal(c, ColorFormat::Argb), 0x8012ABEFu32 as i32);
        assert_eq!(
            hex_to_color("#12abef", ColorFormat::Rgb),
            Some(Color::rgb(0x12, 0xAB, 0xEF))
        );
        assert_eq!(
            hex_to_color("0x12ABEF80", ColorFormat::Rgba),
            Some(Color::argb(0x80, 0x12, 0xAB, 0xEF))
        );
        assert_eq!(hex_to_color("#12ABE", ColorFormat::Rgb), None);
        assert_eq!(
            decimal_to_color(0x12ABEF, ColorFormat::Rgb),
            Color::rgb(0x12, 0xAB, 0xEF)
        );
    }

    /// The colour picker's hex box hands over whatever is typed or pasted: text that is
    /// not hex is no colour, whatever its letters (a byte slice inside a letter panicked).
    #[test]
    fn hex_text_with_other_letters_is_no_colour() {
        for text in ["aé", "aé1234", "0ą", "#żółw", "é", "0xé1234", "ą1234567"] {
            assert_eq!(hex_to_color(text, ColorFormat::Rgb), None, "{text}");
            assert_eq!(hex_to_color(text, ColorFormat::Rgba), None, "{text}");
        }
    }

    #[test]
    fn parse_color_accepts_what_sharex_accepts() {
        assert_eq!(parse_color(" #FF8000 "), Some(Color::rgb(255, 128, 0)));
        assert_eq!(parse_color("0xff8000"), Some(Color::rgb(255, 128, 0)));
        assert_eq!(parse_color("ff8000"), Some(Color::rgb(255, 128, 0)));
        assert_eq!(parse_color("rgb(1, 2, 3)"), Some(Color::rgb(1, 2, 3)));
        assert_eq!(parse_color("10 20 30"), Some(Color::rgb(10, 20, 30)));
        assert_eq!(parse_color("256, 0, 0"), None);
        assert_eq!(parse_color("hello"), None);
        assert_eq!(parse_color("#FF80001"), None);
    }

    #[test]
    fn names() {
        assert_eq!(color_name(Color::rgb(240, 248, 255)), "Alice blue");
        assert_eq!(color_name(Color::rgb(1, 1, 1)), "Black");
        assert_eq!(color_name(Color::rgb(254, 1, 2)), "Red");
        assert_eq!(color_name(Color::rgb(100, 149, 237)), "Cornflower blue");
        assert_eq!(web_colors().len(), 140);
        assert_eq!(
            exact_known_name(Color::rgb(255, 255, 255)),
            "ControlLightLight"
        );
        assert_eq!(exact_known_name(Color::rgb(255, 0, 0)), "Red");
        assert_eq!(exact_known_name(Color::rgb(1, 2, 3)), "");
        assert_eq!(proper_name("Rotate180"), "Rotate 180");
    }

    #[test]
    fn details_follow_the_culture() {
        set_thread_culture(Some(Culture::polish()));
        let text = details_text(Color::rgb(255, 0, 0));
        assert!(text.contains("HSB (Hue, Saturation, Brightness) = 0,0°, 100,0%, 100,0%"));
        assert!(text.ends_with("Decimal (RGB, RGBA, ARGB) = 16711680, -16776961, -65536"));
        set_thread_culture(Some(Culture::english()));
        assert_eq!(
            cmyk_text(Color::rgb(255, 0, 0)),
            "0.0%, 100.0%, 100.0%, 0.0%"
        );
        set_thread_culture(None);
    }

    #[test]
    fn settings_file_form() {
        assert_eq!(to_dotnet_string(Color::rgb(1, 2, 3)), "1, 2, 3");
        assert_eq!(to_dotnet_string(Color::argb(4, 1, 2, 3)), "4, 1, 2, 3");
        assert_eq!(parse_dotnet_string("Red"), Some(Color::RED));
        assert_eq!(
            parse_dotnet_string("4, 1, 2, 3"),
            Some(Color::argb(4, 1, 2, 3))
        );
        assert_eq!(parse_dotnet_string("1, 2, 3"), Some(Color::rgb(1, 2, 3)));
        assert_eq!(
            parse_dotnet_string("CornflowerBlue"),
            Some(Color::CORNFLOWER_BLUE)
        );
    }
}
