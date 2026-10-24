//! The classic effects' data (`ShareX.ImageEffectsLib`): one schema per effect class
//! (its properties in declaration order, with upstream's types, defaults and setter
//! limits), the values a preset holds, and the JSON Newtonsoft writes for them.
//!
//! Presets live in the task settings as `ImageEffectPresets` and travel as `Config.json`
//! inside `.sxie` packages. Settings carry `"$type": "ShareX.ImageEffectsLib.Canvas,
//! ShareX.ImageEffectsLib"`; exported presets carry the short `"$type": "Canvas"`
//! (`ImageEffectsSerializationBinder`). Both are read. Colours, points, sizes, paddings
//! and fonts are the strings their `TypeConverter`s write ("235, 235, 235", "5, 5",
//! "Arial, 11.25pt"), enums their member names.

use modlight_core::culture::Culture;
use serde_json::{Map, Value as Json, json};

use crate::sk::C;

/// The effect classes, in the catalogue's order (`ImageEffectCatalog`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    // Drawings
    DrawBackground,
    DrawBackgroundImage,
    DrawBorder,
    DrawCheckerboard,
    DrawImage,
    DrawParticles,
    DrawTextEx,
    DrawText,
    // Manipulations
    AutoCrop,
    Canvas,
    Crop,
    Flip,
    ForceProportions,
    Resize,
    Rotate,
    RoundedCorners,
    Scale,
    Skew,
    // Adjustments
    Alpha,
    BlackWhite,
    Brightness,
    MatrixColor,
    Colorize,
    Contrast,
    Gamma,
    Grayscale,
    Hue,
    Inverse,
    Polaroid,
    ReplaceColor,
    Saturation,
    SelectiveColor,
    Sepia,
    // Filters
    Blur,
    ColorDepth,
    MatrixConvolution,
    EdgeDetect,
    Emboss,
    GaussianBlur,
    Glow,
    MeanRemoval,
    Outline,
    Pixelate,
    Reflection,
    RGBSplit,
    Shadow,
    Sharpen,
    Slice,
    Smooth,
    TornEdge,
    WaveEdge,
}

/// The catalogue's groups: the category's string key and its classes.
pub const CATALOG: &[(&str, &[Kind])] = &[
    (
        "ImageEffectCategory_Drawings",
        &[
            Kind::DrawBackground,
            Kind::DrawBackgroundImage,
            Kind::DrawBorder,
            Kind::DrawCheckerboard,
            Kind::DrawImage,
            Kind::DrawParticles,
            Kind::DrawTextEx,
            Kind::DrawText,
        ],
    ),
    (
        "ImageEffectCategory_Manipulations",
        &[
            Kind::AutoCrop,
            Kind::Canvas,
            Kind::Crop,
            Kind::Flip,
            Kind::ForceProportions,
            Kind::Resize,
            Kind::Rotate,
            Kind::RoundedCorners,
            Kind::Scale,
            Kind::Skew,
        ],
    ),
    (
        "ImageEffectCategory_Adjustments",
        &[
            Kind::Alpha,
            Kind::BlackWhite,
            Kind::Brightness,
            Kind::MatrixColor,
            Kind::Colorize,
            Kind::Contrast,
            Kind::Gamma,
            Kind::Grayscale,
            Kind::Hue,
            Kind::Inverse,
            Kind::Polaroid,
            Kind::ReplaceColor,
            Kind::Saturation,
            Kind::SelectiveColor,
            Kind::Sepia,
        ],
    ),
    (
        "ImageEffectCategory_Filters",
        &[
            Kind::Blur,
            Kind::ColorDepth,
            Kind::MatrixConvolution,
            Kind::EdgeDetect,
            Kind::Emboss,
            Kind::GaussianBlur,
            Kind::Glow,
            Kind::MeanRemoval,
            Kind::Outline,
            Kind::Pixelate,
            Kind::Reflection,
            Kind::RGBSplit,
            Kind::Shadow,
            Kind::Sharpen,
            Kind::Slice,
            Kind::Smooth,
            Kind::TornEdge,
            Kind::WaveEdge,
        ],
    ),
];

impl Kind {
    pub fn all() -> impl Iterator<Item = Kind> {
        CATALOG.iter().flat_map(|(_, kinds)| kinds.iter().copied())
    }

    /// The C# class name (`Type.Name`).
    pub fn type_name(self) -> &'static str {
        use Kind::*;
        match self {
            DrawBackground => "DrawBackground",
            DrawBackgroundImage => "DrawBackgroundImage",
            DrawBorder => "DrawBorder",
            DrawCheckerboard => "DrawCheckerboard",
            DrawImage => "DrawImage",
            DrawParticles => "DrawParticles",
            DrawTextEx => "DrawTextEx",
            DrawText => "DrawText",
            AutoCrop => "AutoCrop",
            Canvas => "Canvas",
            Crop => "Crop",
            Flip => "Flip",
            ForceProportions => "ForceProportions",
            Resize => "Resize",
            Rotate => "Rotate",
            RoundedCorners => "RoundedCorners",
            Scale => "Scale",
            Skew => "Skew",
            Alpha => "Alpha",
            BlackWhite => "BlackWhite",
            Brightness => "Brightness",
            MatrixColor => "MatrixColor",
            Colorize => "Colorize",
            Contrast => "Contrast",
            Gamma => "Gamma",
            Grayscale => "Grayscale",
            Hue => "Hue",
            Inverse => "Inverse",
            Polaroid => "Polaroid",
            ReplaceColor => "ReplaceColor",
            Saturation => "Saturation",
            SelectiveColor => "SelectiveColor",
            Sepia => "Sepia",
            Blur => "Blur",
            ColorDepth => "ColorDepth",
            MatrixConvolution => "MatrixConvolution",
            EdgeDetect => "EdgeDetect",
            Emboss => "Emboss",
            GaussianBlur => "GaussianBlur",
            Glow => "Glow",
            MeanRemoval => "MeanRemoval",
            Outline => "Outline",
            Pixelate => "Pixelate",
            Reflection => "Reflection",
            RGBSplit => "RGBSplit",
            Shadow => "Shadow",
            Sharpen => "Sharpen",
            Slice => "Slice",
            Smooth => "Smooth",
            TornEdge => "TornEdge",
            WaveEdge => "WaveEdge",
        }
    }

    /// The class for a `$type`: "Canvas", "ShareX.ImageEffectsLib.Canvas" or
    /// "ShareX.ImageEffectsLib.Canvas, ShareX.ImageEffectsLib".
    pub fn from_type_name(name: &str) -> Option<Kind> {
        let name = name.split(',').next().unwrap_or("").trim();
        let short = name.rsplit('.').next().unwrap_or(name);
        Kind::all().find(|k| k.type_name() == short)
    }

    /// The catalogue category's string key.
    pub fn category_key(self) -> &'static str {
        CATALOG
            .iter()
            .find(|(_, kinds)| kinds.contains(&self))
            .map(|(key, _)| *key)
            .unwrap_or("ImageEffectCategory_Filters")
    }
}

/// `AnchorStyles` flags.
pub const ANCHOR: &[(&str, u32)] = &[("Top", 1), ("Bottom", 2), ("Left", 4), ("Right", 8)];
pub const ANCHOR_ALL: u32 = 15;

/// A property's type, as the options panel and the JSON see it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PType {
    Bool,
    Int,
    Float,
    Double,
    Byte,
    Color,
    Point,
    Size,
    Padding,
    /// A string; `File`/`Folder` get a Browse button, `Parsed` goes through the name
    /// parser.
    Text(TextKind),
    /// An enum: its C# type name and member names in declaration order.
    Enum(&'static str, &'static [&'static str]),
    /// A `[Flags]` enum (`AnchorStyles`): edited as text like upstream's panel.
    Flags(&'static str, &'static [(&'static str, u32)]),
    Font,
    Gradient,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextKind {
    Plain,
    File,
    Folder,
    Parsed,
}

pub const CONTENT_ALIGNMENT: &[&str] = &[
    "TopLeft",
    "TopCenter",
    "TopRight",
    "MiddleLeft",
    "MiddleCenter",
    "MiddleRight",
    "BottomLeft",
    "BottomCenter",
    "BottomRight",
];
pub const LINEAR_GRADIENT_MODE: &[&str] = &[
    "Horizontal",
    "Vertical",
    "ForwardDiagonal",
    "BackwardDiagonal",
];
const BORDER_TYPE: &[&str] = &["Outside", "Inside"];
const DASH_STYLE: &[&str] = &["Solid", "Dash", "Dot", "DashDot", "DashDotDot", "Custom"];
const CANVAS_MARGIN_MODE: &[&str] = &["AbsoluteSize", "PercentageOfCanvas"];
const FORCE_PROPORTIONS_METHOD: &[&str] = &["Grow", "Crop"];
const RESIZE_MODE: &[&str] = &["ResizeAll", "ResizeIfBigger", "ResizeIfSmaller"];
const DRAW_IMAGE_SIZE_MODE: &[&str] = &[
    "DontResize",
    "AbsoluteSize",
    "PercentageOfWatermark",
    "PercentageOfCanvas",
];
const IMAGE_ROTATE_FLIP_TYPE: &[&str] = &[
    "None",
    "Rotate90",
    "Rotate180",
    "Rotate270",
    "FlipX",
    "Rotate90FlipX",
    "FlipY",
    "Rotate90FlipY",
];
const IMAGE_INTERPOLATION_MODE: &[&str] = &[
    "HighQualityBicubic",
    "Bicubic",
    "HighQualityBilinear",
    "Bilinear",
    "NearestNeighbor",
];
const COMPOSITING_MODE: &[&str] = &["SourceOver", "SourceCopy"];
const TEXT_RENDERING_HINT: &[&str] = &[
    "SystemDefault",
    "SingleBitPerPixelGridFit",
    "SingleBitPerPixel",
    "AntiAliasGridFit",
    "AntiAlias",
    "ClearTypeGridFit",
];

/// One property: its C# name and type.
#[derive(Clone, Copy, Debug)]
pub struct PropDef {
    pub name: &'static str,
    pub ty: PType,
}

const fn p(name: &'static str, ty: PType) -> PropDef {
    PropDef { name, ty }
}

const fn e(name: &'static str, ty: &'static str, members: &'static [&'static str]) -> PropDef {
    PropDef {
        name,
        ty: PType::Enum(ty, members),
    }
}

const SIDES: PropDef = PropDef {
    name: "Sides",
    ty: PType::Flags("AnchorStyles", ANCHOR),
};

/// The browsable properties of a class in declaration order (`Enabled` and `Name` are
/// not browsable).
pub fn props(kind: Kind) -> &'static [PropDef] {
    use Kind::*;
    use PType::*;
    match kind {
        Alpha => const { &[p("Value", Float), p("Addition", Float)] },
        BlackWhite | Inverse | Polaroid | EdgeDetect | Emboss | MeanRemoval | Sharpen | Smooth => {
            &[]
        }
        Brightness | Contrast | Gamma | Grayscale | Saturation | Sepia => {
            const { &[p("Value", Float)] }
        }
        Colorize => const { &[p("Color", Color), p("Value", Float)] },
        Hue => const { &[p("Angle", Float)] },
        MatrixColor => {
            const {
                &[
                    p("Rr", Float),
                    p("Rg", Float),
                    p("Rb", Float),
                    p("Ra", Float),
                    p("Ro", Float),
                    p("Gr", Float),
                    p("Gg", Float),
                    p("Gb", Float),
                    p("Ga", Float),
                    p("Go", Float),
                    p("Br", Float),
                    p("Bg", Float),
                    p("Bb", Float),
                    p("Ba", Float),
                    p("Bo", Float),
                    p("Ar", Float),
                    p("Ag", Float),
                    p("Ab", Float),
                    p("Aa", Float),
                    p("Ao", Float),
                ]
            }
        }
        ReplaceColor => {
            const {
                &[
                    p("SourceColor", Color),
                    p("AutoSourceColor", Bool),
                    p("TargetColor", Color),
                    p("Threshold", Int),
                ]
            }
        }
        SelectiveColor => {
            const {
                &[
                    p("LightColor", Color),
                    p("DarkColor", Color),
                    p("PaletteSize", Int),
                ]
            }
        }
        Blur | GaussianBlur => const { &[p("Radius", Int)] },
        ColorDepth => const { &[p("BitsPerChannel", Int)] },
        MatrixConvolution => {
            const {
                &[
                    p("X0Y0", Int),
                    p("X1Y0", Int),
                    p("X2Y0", Int),
                    p("X0Y1", Int),
                    p("X1Y1", Int),
                    p("X2Y1", Int),
                    p("X0Y2", Int),
                    p("X1Y2", Int),
                    p("X2Y2", Int),
                    p("Factor", Double),
                    p("Offset", Byte),
                ]
            }
        }
        Glow => {
            const {
                &[
                    p("Size", Int),
                    p("Strength", Float),
                    p("Color", Color),
                    p("UseGradient", Bool),
                    p("Gradient", Gradient),
                    p("Offset", Point),
                ]
            }
        }
        Outline => {
            const {
                &[
                    p("Size", Int),
                    p("Padding", Int),
                    p("Color", Color),
                    p("OutlineOnly", Bool),
                ]
            }
        }
        Pixelate => {
            const {
                &[
                    p("Size", Int),
                    p("BorderSize", Int),
                    p("BorderColor", Color),
                ]
            }
        }
        RGBSplit => {
            const {
                &[
                    p("OffsetRed", Point),
                    p("OffsetGreen", Point),
                    p("OffsetBlue", Point),
                ]
            }
        }
        Reflection => {
            const {
                &[
                    p("Percentage", Int),
                    p("MaxAlpha", Int),
                    p("MinAlpha", Int),
                    p("Offset", Int),
                    p("Skew", Bool),
                    p("SkewSize", Int),
                ]
            }
        }
        Shadow => {
            const {
                &[
                    p("Opacity", Float),
                    p("Size", Int),
                    p("Darkness", Float),
                    p("Color", Color),
                    p("Offset", Point),
                    p("AutoResize", Bool),
                ]
            }
        }
        Slice => {
            const {
                &[
                    p("MinSliceHeight", Int),
                    p("MaxSliceHeight", Int),
                    p("MinSliceShift", Int),
                    p("MaxSliceShift", Int),
                ]
            }
        }
        TornEdge => {
            const {
                &[
                    p("Depth", Int),
                    p("Range", Int),
                    SIDES,
                    p("CurvedEdges", Bool),
                ]
            }
        }
        WaveEdge => const { &[p("Depth", Int), p("Range", Int), SIDES] },
        AutoCrop => const { &[SIDES, p("Padding", Int)] },
        Canvas => {
            const {
                &[
                    p("Margin", Padding),
                    e("MarginMode", "CanvasMarginMode", CANVAS_MARGIN_MODE),
                    p("Color", Color),
                ]
            }
        }
        Crop => const { &[p("Margin", Padding)] },
        Flip => const { &[p("Horizontally", Bool), p("Vertically", Bool)] },
        ForceProportions => {
            const {
                &[
                    p("ProportionalWidth", Int),
                    p("ProportionalHeight", Int),
                    e("Method", "ForceProportionsMethod", FORCE_PROPORTIONS_METHOD),
                    p("GrowFillColor", Color),
                ]
            }
        }
        Resize => {
            const {
                &[
                    p("Width", Int),
                    p("Height", Int),
                    e("Mode", "ResizeMode", RESIZE_MODE),
                ]
            }
        }
        Rotate => const { &[p("Angle", Float), p("Upsize", Bool), p("Clip", Bool)] },
        RoundedCorners => const { &[p("CornerRadius", Int)] },
        Scale => const { &[p("WidthPercentage", Float), p("HeightPercentage", Float)] },
        Skew => const { &[p("Horizontally", Int), p("Vertically", Int)] },
        DrawBackground => {
            const {
                &[
                    p("Color", Color),
                    p("UseGradient", Bool),
                    p("Gradient", Gradient),
                ]
            }
        }
        DrawBackgroundImage => {
            const {
                &[
                    p("ImageFilePath", Text(TextKind::File)),
                    p("Center", Bool),
                    p("Tile", Bool),
                ]
            }
        }
        DrawBorder => {
            const {
                &[
                    e("Type", "BorderType", BORDER_TYPE),
                    p("Size", Int),
                    e("DashStyle", "DashStyle", DASH_STYLE),
                    p("Color", Color),
                    p("UseGradient", Bool),
                    p("Gradient", Gradient),
                ]
            }
        }
        DrawCheckerboard => const { &[p("Size", Int), p("Color", Color), p("Color2", Color)] },
        DrawImage => {
            const {
                &[
                    p("ImageLocation", Text(TextKind::File)),
                    e("Placement", "ContentAlignment", CONTENT_ALIGNMENT),
                    p("Offset", Point),
                    e("SizeMode", "DrawImageSizeMode", DRAW_IMAGE_SIZE_MODE),
                    p("Size", Size),
                    e("RotateFlip", "ImageRotateFlipType", IMAGE_ROTATE_FLIP_TYPE),
                    p("Tile", Bool),
                    p("AutoHide", Bool),
                    e(
                        "InterpolationMode",
                        "ImageInterpolationMode",
                        IMAGE_INTERPOLATION_MODE,
                    ),
                    e("CompositingMode", "CompositingMode", COMPOSITING_MODE),
                    p("Opacity", Int),
                ]
            }
        }
        DrawParticles => {
            const {
                &[
                    p("ImageFolder", Text(TextKind::Folder)),
                    p("ImageCount", Int),
                    p("Background", Bool),
                    p("RandomSize", Bool),
                    p("RandomSizeMin", Int),
                    p("RandomSizeMax", Int),
                    p("RandomAngle", Bool),
                    p("RandomAngleMin", Int),
                    p("RandomAngleMax", Int),
                    p("RandomOpacity", Bool),
                    p("RandomOpacityMin", Int),
                    p("RandomOpacityMax", Int),
                    p("NoOverlap", Bool),
                    p("NoOverlapOffset", Int),
                    p("EdgeOverlap", Bool),
                ]
            }
        }
        DrawText => {
            const {
                &[
                    p("Text", Text(TextKind::Parsed)),
                    e("Placement", "ContentAlignment", CONTENT_ALIGNMENT),
                    p("Offset", Point),
                    p("AutoHide", Bool),
                    p("TextFont", Font),
                    e(
                        "TextRenderingMode",
                        "TextRenderingHint",
                        TEXT_RENDERING_HINT,
                    ),
                    p("TextColor", Color),
                    p("DrawTextShadow", Bool),
                    p("TextShadowColor", Color),
                    p("TextShadowOffset", Point),
                    p("CornerRadius", Int),
                    p("Padding", Padding),
                    p("DrawBorder", Bool),
                    p("BorderColor", Color),
                    p("BorderSize", Int),
                    p("DrawBackground", Bool),
                    p("BackgroundColor", Color),
                    p("UseGradient", Bool),
                    p("Gradient", Gradient),
                ]
            }
        }
        DrawTextEx => {
            const {
                &[
                    p("Text", Text(TextKind::Parsed)),
                    e("Placement", "ContentAlignment", CONTENT_ALIGNMENT),
                    p("Offset", Point),
                    p("Angle", Int),
                    p("AutoHide", Bool),
                    p("Font", Font),
                    p("Color", Color),
                    p("UseGradient", Bool),
                    p("Gradient", Gradient),
                    p("Outline", Bool),
                    p("OutlineSize", Int),
                    p("OutlineColor", Color),
                    p("OutlineUseGradient", Bool),
                    p("OutlineGradient", Gradient),
                    p("Shadow", Bool),
                    p("ShadowOffset", Point),
                    p("ShadowColor", Color),
                    p("ShadowUseGradient", Bool),
                    p("ShadowGradient", Gradient),
                ]
            }
        }
    }
}

// ---------- values ----------

/// `System.Drawing.Color`: its ARGB and, for a known colour, its name (which
/// `ColorConverter` writes back).
#[derive(Clone, Debug, PartialEq)]
pub struct GdiColor {
    pub c: C,
    pub name: Option<String>,
}

impl GdiColor {
    pub fn argb(a: u8, r: u8, g: u8, b: u8) -> GdiColor {
        GdiColor {
            c: C::new(r, g, b, a),
            name: None,
        }
    }

    pub fn rgb(r: u8, g: u8, b: u8) -> GdiColor {
        GdiColor::argb(255, r, g, b)
    }

    /// A known colour by name ("Black", "Transparent").
    pub fn named(name: &str) -> GdiColor {
        let c = known_color(name).unwrap_or(C::BLACK);
        GdiColor {
            c,
            name: Some(canonical_color_name(name).unwrap_or(name).to_string()),
        }
    }

    /// `ColorConverter.ConvertToString` with the invariant culture.
    pub fn to_converter_string(&self) -> String {
        if let Some(n) = &self.name {
            // `Color.Empty` converts to "".
            if n == "Empty" {
                return String::new();
            }
            return n.clone();
        }
        let c = self.c;
        if c.a == 255 {
            format!("{}, {}, {}", c.r, c.g, c.b)
        } else {
            format!("{}, {}, {}, {}", c.a, c.r, c.g, c.b)
        }
    }

    /// `ColorConverter.ConvertFromString`: a known name, "#RRGGBB"/"#AARRGGBB", or
    /// "R, G, B" / "A, R, G, B" (also with `;` separators, as a Polish list separator
    /// writes them).
    pub fn parse(text: &str) -> Option<GdiColor> {
        let t = text.trim();
        if t.is_empty() {
            return Some(GdiColor {
                c: C::ZERO,
                name: Some("Empty".into()),
            });
        }
        if let Some(name) = canonical_color_name(t) {
            return Some(GdiColor::named(name));
        }
        if let Some(hex) = t.strip_prefix('#') {
            let v = u32::from_str_radix(hex, 16).ok()?;
            let a = if hex.len() <= 6 { 255 } else { (v >> 24) as u8 };
            return Some(GdiColor::argb(a, (v >> 16) as u8, (v >> 8) as u8, v as u8));
        }
        let parts: Vec<i64> = t
            .split([',', ';'])
            .map(|p| p.trim().parse::<i64>())
            .collect::<Result<_, _>>()
            .ok()?;
        let b = |v: i64| u8::try_from(v).ok();
        match parts.as_slice() {
            [v] => {
                let v = *v as u32;
                Some(GdiColor::argb(
                    (v >> 24) as u8,
                    (v >> 16) as u8,
                    (v >> 8) as u8,
                    v as u8,
                ))
            }
            [r, g, bl] => Some(GdiColor::rgb(b(*r)?, b(*g)?, b(*bl)?)),
            [a, r, g, bl] => Some(GdiColor::argb(b(*a)?, b(*r)?, b(*g)?, b(*bl)?)),
            _ => None,
        }
    }
}

/// `System.Drawing.Font` as `FontConverter` writes it: "Arial, 11.25pt" or
/// "Arial, 36pt, style=Bold, Italic".
#[derive(Clone, Debug, PartialEq)]
pub struct FontSpec {
    pub family: String,
    pub size_pt: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikeout: bool,
}

impl FontSpec {
    pub fn new(family: &str, size_pt: f32) -> FontSpec {
        FontSpec {
            family: family.to_string(),
            size_pt,
            bold: false,
            italic: false,
            underline: false,
            strikeout: false,
        }
    }

    /// `FontConverter.ConvertToInvariantString`, what Newtonsoft writes into the settings
    /// and packages whatever the language or region: "Arial, 11.25pt".
    pub fn to_converter_string(&self) -> String {
        self.to_string_with(',', '.')
    }

    /// `ConvertToString(null, CurrentCulture, font)`, the options panel's text: the
    /// region's list separator and decimal ("Arial; 11,25pt" in pl-PL).
    pub fn to_culture_string(&self, culture: &Culture) -> String {
        self.to_string_with(list_separator(culture), culture.decimal)
    }

    fn to_string_with(&self, separator: char, decimal: char) -> String {
        let size = invariant_float(self.size_pt).replace('.', &decimal.to_string());
        let mut s = format!("{}{separator} {size}pt", self.family);
        let mut styles = Vec::new();
        if self.bold {
            styles.push("Bold");
        }
        if self.italic {
            styles.push("Italic");
        }
        if self.underline {
            styles.push("Underline");
        }
        if self.strikeout {
            styles.push("Strikeout");
        }
        if !styles.is_empty() {
            s.push(separator);
            s.push_str(" style=");
            s.push_str(&styles.join(", "));
        }
        s
    }

    /// `FontConverter.ConvertFromInvariantString` (the settings and packages).
    pub fn parse(text: &str) -> Option<FontSpec> {
        Self::parse_with(text, ',', '.')
    }

    /// `ConvertFromString(null, CurrentCulture, text)` (the options panel's text box).
    pub fn parse_in(text: &str, culture: &Culture) -> Option<FontSpec> {
        Self::parse_with(text, list_separator(culture), culture.decimal)
    }

    fn parse_with(text: &str, separator: char, decimal: char) -> Option<FontSpec> {
        let t = text.trim();
        if t.is_empty() {
            return None;
        }
        let (main, style) = match t.find("style=") {
            Some(i) => (t[..i].trim_end_matches([separator, ' ']), Some(&t[i + 6..])),
            None => (t, None),
        };
        let mut parts = main.splitn(2, separator).map(str::trim);
        let family = parts.next()?.to_string();
        let mut size = 8.25f32;
        if let Some(sz) = parts.next() {
            let num: String = sz
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == decimal)
                .map(|c| if c == decimal { '.' } else { c })
                .collect();
            let mut v: f32 = num.parse().ok()?;
            if sz.ends_with("px") {
                v *= 72.0 / 96.0;
            }
            size = v;
        }
        let mut f = FontSpec::new(&family, size);
        if let Some(style) = style {
            for s in style.split([',', separator]).map(str::trim) {
                match s {
                    "Bold" => f.bold = true,
                    "Italic" => f.italic = true,
                    "Underline" => f.underline = true,
                    "Strikeout" => f.strikeout = true,
                    _ => {}
                }
            }
        }
        Some(f)
    }

    /// The size in pixels at 96 DPI (`SizeInPoints * 96 / 72`).
    pub fn size_px(&self) -> f32 {
        self.size_pt * 96.0 / 72.0
    }
}

/// `GradientStop`.
#[derive(Clone, Debug, PartialEq)]
pub struct Stop {
    pub color: GdiColor,
    /// 0..100.
    pub location: f32,
}

/// `GradientInfo`.
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    /// `LinearGradientMode` member name.
    pub mode: String,
    pub stops: Vec<Stop>,
}

impl Gradient {
    pub fn new(mode: &str, stops: &[(C, f32)]) -> Gradient {
        Gradient {
            mode: mode.to_string(),
            stops: stops
                .iter()
                .map(|(c, l)| Stop {
                    color: GdiColor { c: *c, name: None },
                    location: *l,
                })
                .collect(),
        }
    }

    /// `IsValid`: at least one stop.
    pub fn is_valid(&self) -> bool {
        !self.stops.is_empty()
    }

    /// `IsVisible`: a stop that is not fully transparent.
    pub fn is_visible(&self) -> bool {
        self.is_valid() && self.stops.iter().any(|s| s.color.c.a > 0)
    }

    /// `Sort` (stable, by location).
    pub fn sort(&mut self) {
        self.stops.sort_by(|a, b| {
            a.location
                .partial_cmp(&b.location)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    /// `GetColorBlend`: the stops by location, with 0 and 100 added when missing.
    pub fn color_blend(&self) -> (Vec<C>, Vec<f32>) {
        let mut stops = self.stops.clone();
        stops.sort_by(|a, b| {
            a.location
                .partial_cmp(&b.location)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if !stops.iter().any(|s| s.location == 0.0) {
            let first = stops[0].clone();
            stops.insert(
                0,
                Stop {
                    location: 0.0,
                    ..first
                },
            );
        }
        if !stops.iter().any(|s| s.location == 100.0) {
            let last = stops[stops.len() - 1].clone();
            stops.push(Stop {
                location: 100.0,
                ..last
            });
        }
        (
            stops.iter().map(|s| s.color.c).collect(),
            stops.iter().map(|s| s.location / 100.0).collect(),
        )
    }

    fn to_json(&self) -> Json {
        json!({
            "Type": self.mode,
            "Colors": self.stops.iter().map(|s| json!({
                "Color": s.color.to_converter_string(),
                "Location": s.location,
            })).collect::<Vec<_>>(),
        })
    }

    fn from_json(v: &Json) -> Option<Gradient> {
        let o = v.as_object()?;
        let mode = o
            .get("Type")
            .and_then(Json::as_str)
            .filter(|m| LINEAR_GRADIENT_MODE.contains(m))
            .unwrap_or("Vertical")
            .to_string();
        let stops = o
            .get("Colors")
            .and_then(Json::as_array)
            .map(|list| {
                list.iter()
                    .map(|s| {
                        let color = s
                            .get("Color")
                            .and_then(Json::as_str)
                            .and_then(GdiColor::parse)
                            .unwrap_or_else(|| GdiColor::named("Black"));
                        let location = s
                            .get("Location")
                            .and_then(Json::as_f64)
                            .unwrap_or(0.0)
                            .clamp(0.0, 100.0) as f32;
                        Stop { color, location }
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(Gradient { mode, stops })
    }
}

/// `WinForms.Padding`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Padding {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Padding {
    pub fn new(left: i32, top: i32, right: i32, bottom: i32) -> Padding {
        Padding {
            left,
            top,
            right,
            bottom,
        }
    }

    /// `Padding.All`: the common value, or -1.
    pub fn all(&self) -> i32 {
        if self.left == self.top && self.top == self.right && self.right == self.bottom {
            self.left
        } else {
            -1
        }
    }

    pub fn horizontal(&self) -> i32 {
        self.left + self.right
    }

    pub fn vertical(&self) -> i32 {
        self.top + self.bottom
    }
}

/// A property's value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i32),
    Float(f32),
    Double(f64),
    Byte(u8),
    Color(GdiColor),
    Point(i32, i32),
    Size(i32, i32),
    Padding(Padding),
    Text(String),
    Enum(String),
    Flags(u32),
    Font(FontSpec),
    Gradient(Gradient),
}

/// The default gradient of Background, Border and Text watermark.
fn blue_gradient() -> Gradient {
    Gradient::new(
        "Vertical",
        &[
            (C::rgb(68, 120, 194), 0.0),
            (C::rgb(13, 58, 122), 50.0),
            (C::rgb(6, 36, 78), 50.0),
            (C::rgb(23, 89, 174), 100.0),
        ],
    )
}

/// The default gradient of Glow and Text (`RandomFast.Next(0, 2)`: one of three).
fn random_gradient(mode: &str) -> Gradient {
    let pick = crate::dotnet::shared_next(0, 3);
    let (a, b) = match pick {
        0 => (C::rgb(0, 187, 138), C::rgb(0, 105, 163)),
        1 => (C::rgb(255, 3, 135), C::rgb(255, 143, 3)),
        _ => (C::rgb(184, 11, 195), C::rgb(98, 54, 255)),
    };
    Gradient::new(mode, &[(a, 0.0), (b, 100.0)])
}

fn named(name: &str) -> Value {
    Value::Color(GdiColor::named(name))
}

fn rgb(r: u8, g: u8, b: u8) -> Value {
    Value::Color(GdiColor::rgb(r, g, b))
}

fn en(member: &str) -> Value {
    Value::Enum(member.to_string())
}

/// A class's values as its constructor leaves them.
pub fn defaults(kind: Kind) -> Vec<Value> {
    use Kind::*;
    use Value::*;
    match kind {
        Alpha => vec![Float(1.0), Float(0.0)],
        BlackWhite | Inverse | Polaroid | EdgeDetect | Emboss | MeanRemoval | Sharpen | Smooth => {
            vec![]
        }
        Brightness => vec![Float(0.0)],
        Contrast | Gamma | Grayscale | Saturation | Sepia => vec![Float(1.0)],
        Colorize => vec![named("Red"), Float(0.0)],
        Hue => vec![Float(0.0)],
        MatrixColor => {
            let mut v = vec![Float(0.0); 20];
            for i in [0, 6, 12, 18] {
                v[i] = Float(1.0);
            }
            v
        }
        ReplaceColor => vec![named("White"), Bool(false), named("Transparent"), Int(0)],
        SelectiveColor => vec![named("White"), named("Black"), Int(10)],
        Blur | GaussianBlur => vec![Int(15)],
        ColorDepth => vec![Int(4)],
        MatrixConvolution => {
            let mut v = vec![Int(0); 9];
            v[4] = Int(1);
            v.push(Double(1.0));
            v.push(Byte(0));
            v
        }
        Glow => vec![
            Int(20),
            Float(1.0),
            named("White"),
            Bool(false),
            Gradient(random_gradient("ForwardDiagonal")),
            Point(0, 0),
        ],
        Outline => vec![Int(1), Int(0), named("Black"), Bool(false)],
        Pixelate => vec![Int(10), Int(0), named("Black")],
        RGBSplit => vec![Point(-5, 0), Point(0, 0), Point(5, 0)],
        Reflection => vec![Int(20), Int(255), Int(0), Int(0), Bool(false), Int(25)],
        Shadow => vec![
            Float(0.6),
            Int(10),
            Float(0.0),
            named("Black"),
            Point(0, 0),
            Bool(true),
        ],
        Slice => vec![Int(10), Int(100), Int(0), Int(10)],
        TornEdge => vec![Int(15), Int(20), Flags(ANCHOR_ALL), Bool(true)],
        WaveEdge => vec![Int(15), Int(20), Flags(ANCHOR_ALL)],
        AutoCrop => vec![Flags(ANCHOR_ALL), Int(0)],
        Canvas => vec![
            Padding(self::Padding::default()),
            en("AbsoluteSize"),
            named("Transparent"),
        ],
        Crop => vec![Padding(self::Padding::default())],
        Flip => vec![Bool(false), Bool(false)],
        ForceProportions => vec![Int(1), Int(1), en("Grow"), named("Transparent")],
        Resize => vec![Int(250), Int(0), en("ResizeAll")],
        Rotate => vec![Float(0.0), Bool(true), Bool(false)],
        RoundedCorners => vec![Int(20)],
        Scale => vec![Float(100.0), Float(0.0)],
        Skew => vec![Int(0), Int(0)],
        DrawBackground => vec![named("Black"), Bool(false), Gradient(blue_gradient())],
        DrawBackgroundImage => vec![Text(String::new()), Bool(true), Bool(false)],
        DrawBorder => vec![
            en("Outside"),
            Int(1),
            en("Solid"),
            named("Black"),
            Bool(false),
            Gradient(blue_gradient()),
        ],
        DrawCheckerboard => vec![Int(10), named("LightGray"), named("White")],
        DrawImage => vec![
            Text(String::new()),
            en("TopLeft"),
            Point(0, 0),
            en("DontResize"),
            Size(0, 0),
            en("None"),
            Bool(false),
            Bool(false),
            en("HighQualityBicubic"),
            en("SourceOver"),
            Int(100),
        ],
        DrawParticles => vec![
            Text(String::new()),
            Int(1),
            Bool(false),
            Bool(false),
            Int(64),
            Int(128),
            Bool(false),
            Int(0),
            Int(360),
            Bool(false),
            Int(0),
            Int(100),
            Bool(false),
            Int(0),
            Bool(false),
        ],
        DrawText => vec![
            Text(
                modlight_core::l10n::t(
                    modlight_core::l10n::effects::ImageEffectDefault_Text_watermark,
                )
                .to_string(),
            ),
            en("BottomRight"),
            Point(5, 5),
            Bool(false),
            Font(FontSpec::new("Arial", 11.25)),
            en("SystemDefault"),
            rgb(235, 235, 235),
            Bool(true),
            named("Black"),
            Point(-1, -1),
            Int(4),
            Padding(self::Padding::new(5, 5, 5, 5)),
            Bool(true),
            named("Black"),
            Int(1),
            Bool(true),
            rgb(42, 47, 56),
            Bool(false),
            Gradient(blue_gradient()),
        ],
        DrawTextEx => vec![
            Text(
                modlight_core::l10n::t(modlight_core::l10n::effects::ImageEffectDefault_Text)
                    .to_string(),
            ),
            en("TopLeft"),
            Point(0, 0),
            Int(0),
            Bool(false),
            Font(FontSpec::new("Arial", 36.0)),
            rgb(235, 235, 235),
            Bool(false),
            Gradient(random_gradient("Horizontal")),
            Bool(false),
            Int(5),
            rgb(235, 0, 0),
            Bool(false),
            Gradient(random_gradient("Horizontal")),
            Bool(false),
            Point(0, 5),
            Value::Color(GdiColor::argb(125, 0, 0, 0)),
            Bool(false),
            Gradient(random_gradient("Horizontal")),
        ],
    }
}

/// The property setters' limits (`value.Max(3)`, `Clamp(1, 8)`, ...). Returns false
/// when the setter ignores the value (Crop's negative margins).
pub fn constrain(kind: Kind, prop: &str, value: &mut Value) -> bool {
    use Kind::*;
    let max = |v: &mut Value, m: i32| {
        if let Value::Int(i) = v {
            *i = (*i).max(m);
        }
    };
    let clamp = |v: &mut Value, lo: i32, hi: i32| {
        if let Value::Int(i) = v {
            *i = (*i).clamp(lo, hi);
        }
    };
    match (kind, prop) {
        (Blur, "Radius") => {
            if let Value::Int(i) = value {
                *i = (*i).max(3);
                if *i % 2 == 0 {
                    *i += 1;
                }
            }
        }
        (ColorDepth, "BitsPerChannel") => clamp(value, 1, 8),
        (GaussianBlur, "Radius") => max(value, 1),
        (Glow, "Size") => max(value, 0),
        (Glow, "Strength") => {
            if let Value::Float(f) = value {
                *f = f.max(0.1);
            }
        }
        (Outline, "Size") => max(value, 1),
        (Outline, "Padding") => max(value, 0),
        (Pixelate, "Size") => max(value, 2),
        (Pixelate, "BorderSize") => max(value, 0),
        (Reflection, "Percentage") => clamp(value, 1, 100),
        (Reflection, "MaxAlpha") | (Reflection, "MinAlpha") => clamp(value, 0, 255),
        (Shadow, "Opacity") => {
            if let Value::Float(f) = value {
                *f = f.clamp(0.1, 1.0);
            }
        }
        (Shadow, "Size") => max(value, 0),
        (Slice, "MinSliceHeight") | (Slice, "MaxSliceHeight") => max(value, 1),
        (Crop, "Margin") => {
            if let Value::Padding(p) = value {
                return p.left >= 0 && p.top >= 0 && p.right >= 0 && p.bottom >= 0;
            }
        }
        (ForceProportions, "ProportionalWidth") | (ForceProportions, "ProportionalHeight") => {
            max(value, 1)
        }
        (RoundedCorners, "CornerRadius") => max(value, 0),
        (DrawBorder, "Size") => max(value, 1),
        (DrawCheckerboard, "Size") => max(value, 1),
        (DrawImage, "Opacity") => clamp(value, 0, 100),
        (DrawParticles, "ImageCount") => clamp(value, 1, 1000),
        (DrawText, "CornerRadius") => max(value, 0),
        _ => {}
    }
    true
}

// ---------- effects and presets ----------

/// One effect of a preset (`ImageEffect`).
#[derive(Clone, Debug, PartialEq)]
pub struct ClassicEffect {
    pub kind: Kind,
    pub enabled: bool,
    /// The custom name (`Name`), empty for none.
    pub name: String,
    pub values: Vec<Value>,
}

impl ClassicEffect {
    pub fn new(kind: Kind) -> ClassicEffect {
        ClassicEffect {
            kind,
            enabled: true,
            name: String::new(),
            values: defaults(kind),
        }
    }

    fn index(&self, prop: &str) -> Option<usize> {
        props(self.kind).iter().position(|p| p.name == prop)
    }

    pub fn get(&self, prop: &str) -> Option<&Value> {
        self.index(prop).and_then(|i| self.values.get(i))
    }

    /// Sets a property through its setter's limits.
    pub fn set(&mut self, prop: &str, mut value: Value) {
        if let Some(i) = self.index(prop)
            && constrain(self.kind, prop, &mut value)
        {
            self.values[i] = value;
        }
    }

    pub fn bool(&self, prop: &str) -> bool {
        matches!(self.get(prop), Some(Value::Bool(true)))
    }

    pub fn int(&self, prop: &str) -> i32 {
        match self.get(prop) {
            Some(Value::Int(v)) => *v,
            Some(Value::Byte(v)) => *v as i32,
            _ => 0,
        }
    }

    pub fn float(&self, prop: &str) -> f32 {
        match self.get(prop) {
            Some(Value::Float(v)) => *v,
            Some(Value::Double(v)) => *v as f32,
            Some(Value::Int(v)) => *v as f32,
            _ => 0.0,
        }
    }

    pub fn double(&self, prop: &str) -> f64 {
        match self.get(prop) {
            Some(Value::Double(v)) => *v,
            Some(Value::Float(v)) => *v as f64,
            _ => 0.0,
        }
    }

    pub fn color(&self, prop: &str) -> C {
        match self.get(prop) {
            Some(Value::Color(c)) => c.c,
            _ => C::ZERO,
        }
    }

    pub fn point(&self, prop: &str) -> (i32, i32) {
        match self.get(prop) {
            Some(Value::Point(x, y)) | Some(Value::Size(x, y)) => (*x, *y),
            _ => (0, 0),
        }
    }

    pub fn padding(&self, prop: &str) -> Padding {
        match self.get(prop) {
            Some(Value::Padding(p)) => *p,
            _ => Padding::default(),
        }
    }

    pub fn text(&self, prop: &str) -> String {
        match self.get(prop) {
            Some(Value::Text(s)) | Some(Value::Enum(s)) => s.clone(),
            _ => String::new(),
        }
    }

    pub fn flags(&self, prop: &str) -> u32 {
        match self.get(prop) {
            Some(Value::Flags(f)) => *f,
            _ => 0,
        }
    }

    pub fn font(&self, prop: &str) -> FontSpec {
        match self.get(prop) {
            Some(Value::Font(f)) => f.clone(),
            _ => FontSpec::new("Arial", 8.25),
        }
    }

    pub fn gradient(&self, prop: &str) -> Option<&Gradient> {
        match self.get(prop) {
            Some(Value::Gradient(g)) => Some(g),
            _ => None,
        }
    }

    pub fn gradient_mut(&mut self, prop: &str) -> Option<&mut Gradient> {
        let i = self.index(prop)?;
        match self.values.get_mut(i) {
            Some(Value::Gradient(g)) => Some(g),
            _ => None,
        }
    }

    /// The effect as Newtonsoft writes it; `short_type` for exports (`$type` without
    /// namespace and assembly).
    pub fn to_json(&self, short_type: bool) -> Json {
        let mut o = Map::new();
        let ty = if short_type {
            self.kind.type_name().to_string()
        } else {
            format!(
                "ShareX.ImageEffectsLib.{}, ShareX.ImageEffectsLib",
                self.kind.type_name()
            )
        };
        o.insert("$type".into(), Json::String(ty));
        for (def, value) in props(self.kind).iter().zip(&self.values) {
            o.insert(def.name.into(), value_to_json(def, value));
        }
        o.insert("Enabled".into(), Json::Bool(self.enabled));
        if !self.name.is_empty() {
            o.insert("Name".into(), Json::String(self.name.clone()));
        }
        Json::Object(o)
    }

    /// An effect from JSON; `None` for an unknown `$type`. Missing or unreadable
    /// properties keep their defaults; values go through the setters.
    pub fn from_json(v: &Json) -> Option<ClassicEffect> {
        let o = v.as_object()?;
        let kind = Kind::from_type_name(o.get("$type")?.as_str()?)?;
        let mut effect = ClassicEffect::new(kind);
        for def in props(kind) {
            if let Some(raw) = o.get(def.name)
                && let Some(value) = value_from_json(def, raw)
            {
                effect.set(def.name, value);
            }
        }
        if let Some(Json::Bool(b)) = o.get("Enabled") {
            effect.enabled = *b;
        }
        if let Some(Json::String(n)) = o.get("Name") {
            effect.name = n.clone();
        }
        Some(effect)
    }
}

fn value_to_json(def: &PropDef, value: &Value) -> Json {
    match value {
        Value::Bool(b) => Json::Bool(*b),
        Value::Int(i) => json!(i),
        Value::Float(f) => json!(*f),
        Value::Double(d) => json!(d),
        Value::Byte(b) => json!(b),
        Value::Color(c) => Json::String(c.to_converter_string()),
        Value::Point(x, y) | Value::Size(x, y) => Json::String(format!("{x}, {y}")),
        Value::Padding(p) => {
            Json::String(format!("{}, {}, {}, {}", p.left, p.top, p.right, p.bottom))
        }
        Value::Text(s) | Value::Enum(s) => Json::String(s.clone()),
        Value::Flags(f) => Json::String(flags_to_string(def, *f)),
        Value::Font(f) => Json::String(f.to_converter_string()),
        Value::Gradient(g) => g.to_json(),
    }
}

/// `Enum.ToString()` of a flags value: "Top, Bottom, Left, Right", or "None".
pub fn flags_to_string(def: &PropDef, f: u32) -> String {
    let PType::Flags(_, members) = def.ty else {
        return f.to_string();
    };
    let names: Vec<&str> = members
        .iter()
        .filter(|(_, bit)| f & bit != 0)
        .map(|(n, _)| *n)
        .collect();
    if names.is_empty() {
        "None".into()
    } else {
        names.join(", ")
    }
}

pub fn flags_from_string(def: &PropDef, s: &str) -> Option<u32> {
    let PType::Flags(_, members) = def.ty else {
        return None;
    };
    let mut f = 0;
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if part.eq_ignore_ascii_case("None") {
            continue;
        }
        if let Ok(n) = part.parse::<u32>() {
            f |= n;
            continue;
        }
        let (_, bit) = members.iter().find(|(n, _)| n.eq_ignore_ascii_case(part))?;
        f |= bit;
    }
    Some(f)
}

/// Parses "x, y" (`PointConverter`, `SizeConverter`).
pub fn parse_pair(s: &str) -> Option<(i32, i32)> {
    let v: Vec<i32> = s
        .split([',', ';'])
        .map(|p| p.trim().parse::<i32>())
        .collect::<Result<_, _>>()
        .ok()?;
    match v.as_slice() {
        [x, y] => Some((*x, *y)),
        _ => None,
    }
}

/// Parses "l, t, r, b" or a single "all" (`PaddingConverter`).
pub fn parse_padding(s: &str) -> Option<Padding> {
    let v: Vec<i32> = s
        .split([',', ';'])
        .map(|p| p.trim().parse::<i32>())
        .collect::<Result<_, _>>()
        .ok()?;
    match v.as_slice() {
        [a] => Some(Padding::new(*a, *a, *a, *a)),
        [l, t, r, b] => Some(Padding::new(*l, *t, *r, *b)),
        _ => None,
    }
}

fn value_from_json(def: &PropDef, raw: &Json) -> Option<Value> {
    Some(match def.ty {
        PType::Bool => Value::Bool(raw.as_bool()?),
        PType::Int => Value::Int(raw.as_f64()?.round() as i32),
        PType::Float => Value::Float(raw.as_f64()? as f32),
        PType::Double => Value::Double(raw.as_f64()?),
        PType::Byte => Value::Byte(raw.as_f64()?.clamp(0.0, 255.0) as u8),
        PType::Color => Value::Color(GdiColor::parse(raw.as_str()?)?),
        PType::Point => {
            let (x, y) = parse_pair(raw.as_str()?)?;
            Value::Point(x, y)
        }
        PType::Size => {
            let (x, y) = parse_pair(raw.as_str()?)?;
            Value::Size(x, y)
        }
        PType::Padding => Value::Padding(parse_padding(raw.as_str()?)?),
        PType::Text(_) => Value::Text(raw.as_str()?.to_string()),
        PType::Enum(_, members) => {
            let s = match raw {
                Json::String(s) => members
                    .iter()
                    .find(|m| m.eq_ignore_ascii_case(s))?
                    .to_string(),
                Json::Number(n) => members.get(n.as_u64()? as usize)?.to_string(),
                _ => return None,
            };
            Value::Enum(s)
        }
        PType::Flags(..) => match raw {
            Json::String(s) => Value::Flags(flags_from_string(def, s)?),
            Json::Number(n) => Value::Flags(n.as_u64()? as u32),
            _ => return None,
        },
        PType::Font => Value::Font(FontSpec::parse(raw.as_str()?)?),
        PType::Gradient => Value::Gradient(Gradient::from_json(raw)?),
    })
}

/// `ImageEffectPreset`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Preset {
    pub name: String,
    pub effects: Vec<ClassicEffect>,
}

impl Preset {
    /// `GetDefaultPreset`: a 30 px canvas under the picture and a text watermark with
    /// its gradient on.
    pub fn default_preset() -> Preset {
        let mut canvas = ClassicEffect::new(Kind::Canvas);
        canvas.set("Margin", Value::Padding(Padding::new(0, 0, 0, 30)));
        let mut text = ClassicEffect::new(Kind::DrawText);
        text.set("Offset", Value::Point(0, 0));
        text.set("UseGradient", Value::Bool(true));
        Preset {
            name: String::new(),
            effects: vec![canvas, text],
        }
    }

    pub fn to_json(&self, short_type: bool) -> Json {
        json!({
            "Name": self.name,
            "Effects": self.effects.iter().map(|e| e.to_json(short_type)).collect::<Vec<_>>(),
        })
    }

    /// A preset from JSON; unknown effect classes are skipped (the settings loader's
    /// error handler does the same).
    pub fn from_json(v: &Json) -> Option<Preset> {
        let o = v.as_object()?;
        let name = o
            .get("Name")
            .and_then(Json::as_str)
            .unwrap_or("")
            .to_string();
        let effects = o
            .get("Effects")
            .and_then(Json::as_array)
            .map(|list| list.iter().filter_map(ClassicEffect::from_json).collect())
            .unwrap_or_default();
        Some(Preset { name, effects })
    }

    /// A preset from an exported `Config.json` string.
    pub fn from_json_str(text: &str) -> Option<Preset> {
        let v: Json = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
        Preset::from_json(&v)
    }

    /// `JsonHelpers.SerializeToString(preset, binder)`: indented, short `$type`s.
    pub fn to_export_string(&self) -> String {
        serde_json::to_string_pretty(&self.to_json(true)).unwrap_or_default()
    }
}

/// The presets of the task settings (`ImageEffectPresets`); missing gives upstream's
/// default list (one default preset).
pub fn presets_from_settings(v: Option<&Json>) -> Vec<Preset> {
    match v {
        Some(Json::Array(list)) => list.iter().filter_map(Preset::from_json).collect(),
        _ => vec![Preset::default_preset()],
    }
}

pub fn presets_to_settings(presets: &[Preset]) -> Json {
    Json::Array(presets.iter().map(|p| p.to_json(false)).collect())
}

// ---------- names ----------

/// .NET's `float.ToString()`: the shortest text that reads back as the same float, in
/// the current culture's decimal separator (the region's format, not the interface
/// language: `modlight_core::culture`).
pub fn fmt_float(v: f32) -> String {
    let decimal = Culture::current().decimal;
    invariant_float(v).replace('.', &decimal.to_string())
}

/// `float.ToString(CultureInfo.InvariantCulture)`.
fn invariant_float(v: f32) -> String {
    let s = format!("{v}");
    if s.contains('e') {
        format!("{}", v as f64)
    } else {
        s
    }
}

/// `TextInfo.ListSeparator` of the two modelled cultures (Windows' NLS data, which
/// upstream runs with: `;` where the decimal separator is a comma).
fn list_separator(culture: &Culture) -> char {
    if culture.decimal == ',' { ';' } else { ',' }
}

/// `Helpers.GetProperName`: "BitsPerChannel" is "Bits per channel".
pub fn proper_name(name: &str) -> String {
    let mut out = String::new();
    let mut number = false;
    for (i, c) in name.chars().enumerate() {
        if i > 0 && (c.is_uppercase() || (!number && c.is_ascii_digit())) {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
        number = c.is_ascii_digit();
    }
    out
}

fn effects_string(key: &str) -> Option<&'static str> {
    modlight_core::l10n::lookup(&format!("ShareX.ImageEffectsLib:{key}"))
        .map(modlight_core::l10n::t)
}

/// `ImageEffectsLocalization.GetEffectName`.
pub fn effect_name(kind: Kind) -> String {
    effects_string(&format!("ImageEffect_{}", kind.type_name()))
        .map(str::to_string)
        .unwrap_or_else(|| proper_name(kind.type_name()))
}

/// The catalogue category's text.
pub fn category_name(key: &str) -> String {
    effects_string(key).unwrap_or(key).to_string()
}

/// `GetPropertyName`.
pub fn property_name(prop: &str) -> String {
    effects_string(&format!("ImageEffectProperty_{prop}"))
        .map(str::to_string)
        .unwrap_or_else(|| proper_name(prop))
}

/// `GetPropertyDescription` (empty when the property has no `[Description]`).
pub fn property_description(kind: Kind, prop: &str) -> String {
    effects_string(&format!(
        "ImageEffectPropertyDescription_{}_{prop}",
        kind.type_name()
    ))
    .unwrap_or("")
    .to_string()
}

/// `GetEnumValue`.
pub fn enum_value_name(type_name: &str, member: &str) -> String {
    effects_string(&format!("ImageEffectEnum_{type_name}_{member}"))
        .map(str::to_string)
        .unwrap_or_else(|| proper_name(member))
}

/// `FileHelpers.GetFileNameSafe`.
fn file_name_safe(path: &str) -> String {
    let p = path.trim_end_matches(['/', '\\']);
    p.rsplit(['/', '\\']).next().unwrap_or(p).to_string()
}

/// `StringExtensions.Truncate(20, "...")`.
fn truncate(s: &str, max: usize, ending: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() > max {
        let keep = max.saturating_sub(ending.chars().count());
        if keep > 0 {
            return chars[..keep].iter().collect::<String>() + ending;
        }
    }
    s.to_string()
}

/// `GetSummary`.
pub fn summary(e: &ClassicEffect) -> Option<String> {
    use Kind::*;
    let f = |p: &str| fmt_float(e.float(p));
    let margin = |p: Padding| {
        if p.all() == -1 {
            format!("{}, {}, {}, {}", p.left, p.top, p.right, p.bottom)
        } else {
            p.all().to_string()
        }
    };
    let b = |v: bool| if v { "True" } else { "False" };
    Some(match e.kind {
        Alpha => format!("{}, {}", f("Value"), f("Addition")),
        Brightness | Contrast | Gamma | Grayscale | Saturation | Sepia => f("Value"),
        Colorize => {
            let c = e.color("Color");
            format!("{}, {}, {}", c.r, c.g, c.b)
        }
        Hue => format!("{}°", f("Angle")),
        Blur | GaussianBlur => e.int("Radius").to_string(),
        ColorDepth => {
            let n = e.int("BitsPerChannel");
            format!("{n} bit{}", if n > 1 { "s" } else { "" })
        }
        Glow | Outline | Pixelate | Shadow => e.int("Size").to_string(),
        Reflection => e.int("Percentage").to_string(),
        Slice => format!("{}, {}", e.int("MinSliceHeight"), e.int("MaxSliceHeight")),
        TornEdge | WaveEdge => format!("{}, {}", e.int("Depth"), e.int("Range")),
        AutoCrop => {
            let p = e.int("Padding");
            if p > 0 {
                p.to_string()
            } else {
                return None;
            }
        }
        Canvas | Crop => margin(e.padding("Margin")),
        Flip => format!("{}, {}", b(e.bool("Horizontally")), b(e.bool("Vertically"))),
        ForceProportions => format!(
            "{}, {}",
            e.int("ProportionalWidth"),
            e.int("ProportionalHeight")
        ),
        Resize => {
            let (w, h) = (e.int("Width"), e.int("Height"));
            format!(
                "{w}{}, {h}{}",
                if w > 0 { "px" } else { "" },
                if h > 0 { "px" } else { "" }
            )
        }
        Rotate => format!("{}°", f("Angle")),
        RoundedCorners => e.int("CornerRadius").to_string(),
        Scale => {
            let (w, h) = (e.float("WidthPercentage"), e.float("HeightPercentage"));
            format!(
                "{}{}, {}{}",
                fmt_float(w),
                if w > 0.0 { "%" } else { "" },
                fmt_float(h),
                if h > 0.0 { "%" } else { "" }
            )
        }
        Skew => format!("{}px, {}px", e.int("Horizontally"), e.int("Vertically")),
        DrawBackground => {
            if e.bool("UseGradient") {
                return None;
            }
            let c = e.color("Color");
            format!("{}, {}, {}", c.r, c.g, c.b)
        }
        DrawBackgroundImage | DrawImage | DrawParticles => {
            let key = match e.kind {
                DrawBackgroundImage => "ImageFilePath",
                DrawImage => "ImageLocation",
                _ => "ImageFolder",
            };
            let path = e.text(key);
            if path.is_empty() {
                return None;
            }
            file_name_safe(&path)
        }
        DrawBorder => format!("{}px", e.int("Size")),
        DrawCheckerboard => {
            let s = e.int("Size");
            format!("{s}x{s}")
        }
        DrawText | DrawTextEx => {
            let t = e.text("Text");
            if t.is_empty() {
                return None;
            }
            truncate(&t, 20, "...")
        }
        _ => return None,
    })
}

/// `ImageEffect.ToString()`: the custom name, or the class name with its summary.
pub fn display_name(e: &ClassicEffect) -> String {
    if !e.name.is_empty() {
        return e.name.clone();
    }
    let name = effect_name(e.kind);
    match summary(e) {
        Some(s) if !s.is_empty() => format!("{name}: {s}"),
        _ => name,
    }
}

// ---------- known colours ----------

/// `KnownColor` web colours and `Transparent` (System.Drawing names, case-insensitive).
const KNOWN_COLORS: &[(&str, u32)] = &[
    ("Transparent", 0x00FF_FFFF),
    ("AliceBlue", 0xFFF0_F8FF),
    ("AntiqueWhite", 0xFFFA_EBD7),
    ("Aqua", 0xFF00_FFFF),
    ("Aquamarine", 0xFF7F_FFD4),
    ("Azure", 0xFFF0_FFFF),
    ("Beige", 0xFFF5_F5DC),
    ("Bisque", 0xFFFF_E4C4),
    ("Black", 0xFF00_0000),
    ("BlanchedAlmond", 0xFFFF_EBCD),
    ("Blue", 0xFF00_00FF),
    ("BlueViolet", 0xFF8A_2BE2),
    ("Brown", 0xFFA5_2A2A),
    ("BurlyWood", 0xFFDE_B887),
    ("CadetBlue", 0xFF5F_9EA0),
    ("Chartreuse", 0xFF7F_FF00),
    ("Chocolate", 0xFFD2_691E),
    ("Coral", 0xFFFF_7F50),
    ("CornflowerBlue", 0xFF64_95ED),
    ("Cornsilk", 0xFFFF_F8DC),
    ("Crimson", 0xFFDC_143C),
    ("Cyan", 0xFF00_FFFF),
    ("DarkBlue", 0xFF00_008B),
    ("DarkCyan", 0xFF00_8B8B),
    ("DarkGoldenrod", 0xFFB8_860B),
    ("DarkGray", 0xFFA9_A9A9),
    ("DarkGreen", 0xFF00_6400),
    ("DarkKhaki", 0xFFBD_B76B),
    ("DarkMagenta", 0xFF8B_008B),
    ("DarkOliveGreen", 0xFF55_6B2F),
    ("DarkOrange", 0xFFFF_8C00),
    ("DarkOrchid", 0xFF99_32CC),
    ("DarkRed", 0xFF8B_0000),
    ("DarkSalmon", 0xFFE9_967A),
    ("DarkSeaGreen", 0xFF8F_BC8B),
    ("DarkSlateBlue", 0xFF48_3D8B),
    ("DarkSlateGray", 0xFF2F_4F4F),
    ("DarkTurquoise", 0xFF00_CED1),
    ("DarkViolet", 0xFF94_00D3),
    ("DeepPink", 0xFFFF_1493),
    ("DeepSkyBlue", 0xFF00_BFFF),
    ("DimGray", 0xFF69_6969),
    ("DodgerBlue", 0xFF1E_90FF),
    ("Firebrick", 0xFFB2_2222),
    ("FloralWhite", 0xFFFF_FAF0),
    ("ForestGreen", 0xFF22_8B22),
    ("Fuchsia", 0xFFFF_00FF),
    ("Gainsboro", 0xFFDC_DCDC),
    ("GhostWhite", 0xFFF8_F8FF),
    ("Gold", 0xFFFF_D700),
    ("Goldenrod", 0xFFDA_A520),
    ("Gray", 0xFF80_8080),
    ("Green", 0xFF00_8000),
    ("GreenYellow", 0xFFAD_FF2F),
    ("Honeydew", 0xFFF0_FFF0),
    ("HotPink", 0xFFFF_69B4),
    ("IndianRed", 0xFFCD_5C5C),
    ("Indigo", 0xFF4B_0082),
    ("Ivory", 0xFFFF_FFF0),
    ("Khaki", 0xFFF0_E68C),
    ("Lavender", 0xFFE6_E6FA),
    ("LavenderBlush", 0xFFFF_F0F5),
    ("LawnGreen", 0xFF7C_FC00),
    ("LemonChiffon", 0xFFFF_FACD),
    ("LightBlue", 0xFFAD_D8E6),
    ("LightCoral", 0xFFF0_8080),
    ("LightCyan", 0xFFE0_FFFF),
    ("LightGoldenrodYellow", 0xFFFA_FAD2),
    ("LightGray", 0xFFD3_D3D3),
    ("LightGreen", 0xFF90_EE90),
    ("LightPink", 0xFFFF_B6C1),
    ("LightSalmon", 0xFFFF_A07A),
    ("LightSeaGreen", 0xFF20_B2AA),
    ("LightSkyBlue", 0xFF87_CEFA),
    ("LightSlateGray", 0xFF77_8899),
    ("LightSteelBlue", 0xFFB0_C4DE),
    ("LightYellow", 0xFFFF_FFE0),
    ("Lime", 0xFF00_FF00),
    ("LimeGreen", 0xFF32_CD32),
    ("Linen", 0xFFFA_F0E6),
    ("Magenta", 0xFFFF_00FF),
    ("Maroon", 0xFF80_0000),
    ("MediumAquamarine", 0xFF66_CDAA),
    ("MediumBlue", 0xFF00_00CD),
    ("MediumOrchid", 0xFFBA_55D3),
    ("MediumPurple", 0xFF93_70DB),
    ("MediumSeaGreen", 0xFF3C_B371),
    ("MediumSlateBlue", 0xFF7B_68EE),
    ("MediumSpringGreen", 0xFF00_FA9A),
    ("MediumTurquoise", 0xFF48_D1CC),
    ("MediumVioletRed", 0xFFC7_1585),
    ("MidnightBlue", 0xFF19_1970),
    ("MintCream", 0xFFF5_FFFA),
    ("MistyRose", 0xFFFF_E4E1),
    ("Moccasin", 0xFFFF_E4B5),
    ("NavajoWhite", 0xFFFF_DEAD),
    ("Navy", 0xFF00_0080),
    ("OldLace", 0xFFFD_F5E6),
    ("Olive", 0xFF80_8000),
    ("OliveDrab", 0xFF6B_8E23),
    ("Orange", 0xFFFF_A500),
    ("OrangeRed", 0xFFFF_4500),
    ("Orchid", 0xFFDA_70D6),
    ("PaleGoldenrod", 0xFFEE_E8AA),
    ("PaleGreen", 0xFF98_FB98),
    ("PaleTurquoise", 0xFFAF_EEEE),
    ("PaleVioletRed", 0xFFDB_7093),
    ("PapayaWhip", 0xFFFF_EFD5),
    ("PeachPuff", 0xFFFF_DAB9),
    ("Peru", 0xFFCD_853F),
    ("Pink", 0xFFFF_C0CB),
    ("Plum", 0xFFDD_A0DD),
    ("PowderBlue", 0xFFB0_E0E6),
    ("Purple", 0xFF80_0080),
    ("RebeccaPurple", 0xFF66_3399),
    ("Red", 0xFFFF_0000),
    ("RosyBrown", 0xFFBC_8F8F),
    ("RoyalBlue", 0xFF41_69E1),
    ("SaddleBrown", 0xFF8B_4513),
    ("Salmon", 0xFFFA_8072),
    ("SandyBrown", 0xFFF4_A460),
    ("SeaGreen", 0xFF2E_8B57),
    ("SeaShell", 0xFFFF_F5EE),
    ("Sienna", 0xFFA0_522D),
    ("Silver", 0xFFC0_C0C0),
    ("SkyBlue", 0xFF87_CEEB),
    ("SlateBlue", 0xFF6A_5ACD),
    ("SlateGray", 0xFF70_8090),
    ("Snow", 0xFFFF_FAFA),
    ("SpringGreen", 0xFF00_FF7F),
    ("SteelBlue", 0xFF46_82B4),
    ("Tan", 0xFFD2_B48C),
    ("Teal", 0xFF00_8080),
    ("Thistle", 0xFFD8_BFD8),
    ("Tomato", 0xFFFF_6347),
    ("Turquoise", 0xFF40_E0D0),
    ("Violet", 0xFFEE_82EE),
    ("Wheat", 0xFFF5_DEB3),
    ("White", 0xFFFF_FFFF),
    ("WhiteSmoke", 0xFFF5_F5F5),
    ("Yellow", 0xFFFF_FF00),
    ("YellowGreen", 0xFF9A_CD32),
];

fn canonical_color_name(name: &str) -> Option<&'static str> {
    KNOWN_COLORS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(n, _)| *n)
}

fn known_color(name: &str) -> Option<C> {
    KNOWN_COLORS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| C::new((v >> 16) as u8, (v >> 8) as u8, *v as u8, (v >> 24) as u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_class_has_values_for_its_properties() {
        for kind in Kind::all() {
            assert_eq!(props(kind).len(), defaults(kind).len(), "{kind:?}");
        }
        assert_eq!(Kind::all().count(), 51);
    }

    #[test]
    fn settings_json_reads_back() {
        let preset = Preset::default_preset();
        let json = preset.to_json(false);
        assert_eq!(
            json["Effects"][0]["$type"],
            "ShareX.ImageEffectsLib.Canvas, ShareX.ImageEffectsLib"
        );
        assert_eq!(json["Effects"][0]["Margin"], "0, 0, 0, 30");
        assert_eq!(json["Effects"][0]["Color"], "Transparent");
        assert_eq!(json["Effects"][1]["TextFont"], "Arial, 11.25pt");
        assert_eq!(json["Effects"][1]["TextColor"], "235, 235, 235");
        let back = Preset::from_json(&json).unwrap();
        assert_eq!(back, preset);
    }

    #[test]
    fn exported_presets_use_short_types() {
        let text = r#"{"Name":"Shadowy","Effects":[{"$type":"Shadow","Opacity":2.0,"Size":12,
            "Darkness":0.0,"Color":"128, 0, 0, 0","Offset":"3, -4","AutoResize":false,"Enabled":true},
            {"$type":"Unknown","Enabled":true},{"$type":"Blur","Radius":4,"Enabled":false,"Name":"Soft"}]}"#;
        let p = Preset::from_json_str(text).unwrap();
        assert_eq!(p.name, "Shadowy");
        assert_eq!(p.effects.len(), 2);
        let shadow = &p.effects[0];
        assert_eq!(shadow.float("Opacity"), 1.0, "the setter clamps to 1");
        assert_eq!(shadow.point("Offset"), (3, -4));
        assert_eq!(shadow.color("Color"), C::new(0, 0, 0, 128));
        assert_eq!(
            p.effects[1].int("Radius"),
            5,
            "even radius goes to the next odd one"
        );
        assert!(!p.effects[1].enabled);
        assert_eq!(display_name(&p.effects[1]), "Soft");
        assert!(p.to_export_string().contains("\"$type\": \"Shadow\""));
    }

    /// `Color.Empty` is written as "" (`ColorConverter`), which Windows reads back as
    /// empty; "Empty" is no colour name there, so it would fail and keep the default.
    #[test]
    fn the_empty_colour_round_trips_as_an_empty_string() {
        let empty = GdiColor::parse("").unwrap();
        assert_eq!(empty.c, C::ZERO);
        assert_eq!(empty.to_converter_string(), "");
        assert_eq!(GdiColor::parse(&empty.to_converter_string()), Some(empty));
    }

    #[test]
    fn converters_round_trip() {
        assert_eq!(GdiColor::parse("Red").unwrap().to_converter_string(), "Red");
        assert_eq!(GdiColor::parse("red").unwrap().c, C::RED);
        assert_eq!(
            GdiColor::parse("125, 0, 0, 0")
                .unwrap()
                .to_converter_string(),
            "125, 0, 0, 0"
        );
        let f = FontSpec::parse("Segoe UI, 36pt, style=Bold, Italic").unwrap();
        assert!(f.bold && f.italic);
        assert_eq!(
            f.to_converter_string(),
            "Segoe UI, 36pt, style=Bold, Italic"
        );
        let sides = PropDef {
            name: "Sides",
            ty: PType::Flags("AnchorStyles", ANCHOR),
        };
        assert_eq!(flags_to_string(&sides, 15), "Top, Bottom, Left, Right");
        assert_eq!(flags_from_string(&sides, "Left, Top"), Some(5));
        assert_eq!(
            parse_padding("0, 0, 0, 30"),
            Some(Padding::new(0, 0, 0, 30))
        );
    }

    /// Newtonsoft writes a font with `ConvertToInvariantString`: "Arial, 11.25pt" whatever
    /// the interface language or the region, so Windows and this app read it back.
    #[test]
    fn fonts_are_written_invariant_in_every_language_and_region() {
        use modlight_core::culture::{Culture, set_thread_culture};
        use modlight_core::l10n::{Language, set_thread_language};
        for (language, culture) in [
            (Language::Polish, Culture::polish()),
            (Language::Polish, Culture::english()),
            (Language::English, Culture::polish()),
        ] {
            set_thread_language(Some(language));
            set_thread_culture(Some(culture));
            let mut text = ClassicEffect::new(Kind::DrawText);
            let mut font = FontSpec::new("Arial", 11.25);
            font.bold = true;
            text.set("TextFont", Value::Font(font.clone()));
            let json = text.to_json(false);
            assert_eq!(json["TextFont"], "Arial, 11.25pt, style=Bold");
            let back = ClassicEffect::from_json(&json).unwrap();
            assert_eq!(back.font("TextFont"), font);
        }
        set_thread_language(None);
        set_thread_culture(None);
    }

    /// The options panel's text box shows and reads a font in the region's culture, as
    /// `ConvertToString(null, CurrentCulture, ...)` and `ConvertFromString` do.
    #[test]
    fn fonts_in_the_panel_follow_the_region() {
        use modlight_core::culture::Culture;
        let mut font = FontSpec::new("Segoe UI", 11.25);
        font.italic = true;
        let polish = font.to_culture_string(&Culture::polish());
        assert_eq!(polish, "Segoe UI; 11,25pt; style=Italic");
        assert_eq!(
            FontSpec::parse_in(&polish, &Culture::polish()),
            Some(font.clone())
        );
        let english = font.to_culture_string(&Culture::english());
        assert_eq!(english, "Segoe UI, 11.25pt, style=Italic");
        assert_eq!(
            FontSpec::parse_in(&english, &Culture::english()),
            Some(font)
        );
    }

    /// `GetSummary` formats numbers with the region's culture (`$"{Value}"`), which the
    /// interface language does not decide.
    #[test]
    fn summaries_follow_the_region_not_the_language() {
        use modlight_core::culture::{Culture, set_thread_culture};
        use modlight_core::l10n::{Language, set_thread_language};
        let mut gamma = ClassicEffect::new(Kind::Gamma);
        gamma.set("Value", Value::Float(1.5));
        set_thread_language(Some(Language::English));
        set_thread_culture(Some(Culture::polish()));
        assert_eq!(summary(&gamma).as_deref(), Some("1,5"));
        set_thread_language(Some(Language::Polish));
        set_thread_culture(Some(Culture::english()));
        assert_eq!(summary(&gamma).as_deref(), Some("1.5"));
        set_thread_language(None);
        set_thread_culture(None);
    }

    #[test]
    fn names_follow_the_string_table() {
        assert_eq!(proper_name("BitsPerChannel"), "Bits per channel");
        assert_eq!(proper_name("X0Y1"), "X 0 y 1");
        let mut e = ClassicEffect::new(Kind::Resize);
        assert_eq!(
            display_name(&e),
            format!("{}: 250px, 0", effect_name(Kind::Resize))
        );
        e.name = "Mine".into();
        assert_eq!(display_name(&e), "Mine");
        assert_eq!(
            truncate("Text watermark of a long kind", 20, "..."),
            "Text watermark of..."
        );
    }
}
