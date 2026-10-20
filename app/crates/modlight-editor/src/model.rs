//! The annotations (`Core/Annotations/**`): what each one stores, its bounds and its hit
//! test, with upstream's defaults. One `Annotation` struct carries the fields of the
//! abstract base class; `Kind` carries what each concrete class adds.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use image::RgbaImage;

use crate::geom::{Point, Rect, distance_to_segment, point_in_triangle};

pub type AnnotationId = u64;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// `Guid.NewGuid()`: a fresh identity. Clones keep theirs (`Annotation.Clone`).
pub fn new_id() -> AnnotationId {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// `EditorTool`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum EditorTool {
    #[default]
    Select,
    Rectangle,
    Ellipse,
    Line,
    Arrow,
    Freehand,
    Text,
    SpeechBalloon,
    Step,
    Image,
    Emoji,
    Cursor,
    Highlight,
    SmartEraser,
    Blur,
    Pixelate,
    Magnify,
    Spotlight,
    Crop,
    CutOut,
}

impl EditorTool {
    pub const ALL: [EditorTool; 20] = [
        EditorTool::Select,
        EditorTool::Rectangle,
        EditorTool::Ellipse,
        EditorTool::Line,
        EditorTool::Arrow,
        EditorTool::Freehand,
        EditorTool::Text,
        EditorTool::SpeechBalloon,
        EditorTool::Step,
        EditorTool::Image,
        EditorTool::Emoji,
        EditorTool::Cursor,
        EditorTool::Highlight,
        EditorTool::SmartEraser,
        EditorTool::Blur,
        EditorTool::Pixelate,
        EditorTool::Magnify,
        EditorTool::Spotlight,
        EditorTool::Crop,
        EditorTool::CutOut,
    ];

    /// The enum name (`ToString()`), used as the toolbar item id.
    pub fn name(self) -> &'static str {
        match self {
            EditorTool::Select => "Select",
            EditorTool::Rectangle => "Rectangle",
            EditorTool::Ellipse => "Ellipse",
            EditorTool::Line => "Line",
            EditorTool::Arrow => "Arrow",
            EditorTool::Freehand => "Freehand",
            EditorTool::Text => "Text",
            EditorTool::SpeechBalloon => "SpeechBalloon",
            EditorTool::Step => "Step",
            EditorTool::Image => "Image",
            EditorTool::Emoji => "Emoji",
            EditorTool::Cursor => "Cursor",
            EditorTool::Highlight => "Highlight",
            EditorTool::SmartEraser => "SmartEraser",
            EditorTool::Blur => "Blur",
            EditorTool::Pixelate => "Pixelate",
            EditorTool::Magnify => "Magnify",
            EditorTool::Spotlight => "Spotlight",
            EditorTool::Crop => "Crop",
            EditorTool::CutOut => "CutOut",
        }
    }

    pub fn parse(name: &str) -> Option<EditorTool> {
        EditorTool::ALL
            .into_iter()
            .find(|tool| tool.name().eq_ignore_ascii_case(name))
    }
}

/// `AnnotationCategory`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnnotationCategory {
    Shapes,
    Effects,
    Text,
}

macro_rules! simple_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident),* $(,)? } default $default:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
        pub enum $name {
            #[default]
            $default,
            $($variant),*
        }

        impl $name {
            pub fn name(self) -> &'static str {
                match self {
                    $name::$default => stringify!($default),
                    $($name::$variant => stringify!($variant)),*
                }
            }

            pub fn parse(text: &str) -> Option<$name> {
                if text.eq_ignore_ascii_case(stringify!($default)) {
                    return Some($name::$default);
                }
                $(if text.eq_ignore_ascii_case(stringify!($variant)) {
                    return Some($name::$variant);
                })*
                None
            }
        }
    };
}

simple_enum!(
    /// `ArrowStyle` (declaration order is the numeric value: Classic = 0 ... Line = 4).
    ArrowStyle { Modern, Double, Basic, Line } default Classic
);

impl ArrowStyle {
    /// The picker's order (`BuildAvailableArrowStyles`).
    pub const PICKER_ORDER: [ArrowStyle; 5] = [
        ArrowStyle::Classic,
        ArrowStyle::Double,
        ArrowStyle::Modern,
        ArrowStyle::Basic,
        ArrowStyle::Line,
    ];
}

simple_enum!(
    /// `BorderStyle`.
    BorderStyle { Dash, Dot, DashDot, DashDotDot } default Solid
);

impl BorderStyle {
    pub const ALL: [BorderStyle; 5] = [
        BorderStyle::Solid,
        BorderStyle::Dash,
        BorderStyle::Dot,
        BorderStyle::DashDot,
        BorderStyle::DashDotDot,
    ];

    /// `BorderStyleDashHelper.CreateStrokeDashArray`, in multiples of the stroke width.
    pub fn dash_array(self) -> &'static [f32] {
        match self {
            BorderStyle::Solid => &[],
            BorderStyle::Dash => &[3.0, 1.0],
            BorderStyle::Dot => &[1.0, 1.0],
            BorderStyle::DashDot => &[3.0, 1.0, 1.0, 1.0],
            BorderStyle::DashDotDot => &[3.0, 1.0, 1.0, 1.0, 1.0, 1.0],
        }
    }

    /// `BorderStyleDashHelper.CreateStrokeLineCap`: round for solid lines, flat otherwise.
    pub fn round_cap(self) -> bool {
        self == BorderStyle::Solid
    }
}

simple_enum!(
    /// `StepType`.
    StepType { UppercaseLetter, LowercaseLetter, UppercaseRoman, LowercaseRoman } default Numeric
);

impl StepType {
    pub const ALL: [StepType; 5] = [
        StepType::Numeric,
        StepType::UppercaseLetter,
        StepType::LowercaseLetter,
        StepType::UppercaseRoman,
        StepType::LowercaseRoman,
    ];

    /// `StepTypeFormatter.Format`.
    pub fn format(self, value: i32) -> String {
        if value <= 0 {
            return value.to_string();
        }
        match self {
            StepType::Numeric => value.to_string(),
            StepType::UppercaseLetter => alphabetic(value),
            StepType::LowercaseLetter => alphabetic(value).to_lowercase(),
            StepType::UppercaseRoman => roman(value),
            StepType::LowercaseRoman => roman(value).to_lowercase(),
        }
    }

    /// `StepTypeFormatter.GetPreviewText`.
    pub fn preview_text(self) -> &'static str {
        match self {
            StepType::Numeric => "1, 2, 3 ...",
            StepType::UppercaseLetter => "A, B, C ...",
            StepType::LowercaseLetter => "a, b, c ...",
            StepType::UppercaseRoman => "I, II, III ...",
            StepType::LowercaseRoman => "i, ii, iii ...",
        }
    }
}

fn alphabetic(value: i32) -> String {
    let mut out = Vec::new();
    let mut current = value;
    while current > 0 {
        current -= 1;
        out.insert(0, (b'A' + (current % 26) as u8) as char);
        current /= 26;
    }
    out.into_iter().collect()
}

fn roman(value: i32) -> String {
    const SYMBOLS: [(i32, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    let mut remaining = value;
    for (amount, symbol) in SYMBOLS {
        while remaining >= amount {
            out.push_str(symbol);
            remaining -= amount;
        }
    }
    out
}

simple_enum!(
    /// `TextHorizontalAlignment`.
    TextAlign { Center, Right } default Left
);

impl TextAlign {
    pub const ALL: [TextAlign; 3] = [TextAlign::Left, TextAlign::Center, TextAlign::Right];
}

simple_enum!(
    /// `CursorType`.
    CursorType {
        AppStarting, Arrow, Cross, Hand, Help, HSplit, IBeam, No, NoMove2D, NoMoveHoriz,
        NoMoveVert, PanEast, PanNE, PanNorth, PanNW, PanSE, PanSouth, PanSW, PanWest, SizeAll,
        SizeNESW, SizeNS, SizeNWSE, SizeWE, UpArrow, VSplit, WaitCursor
    } default Default
);

impl CursorType {
    /// The picker's order (`BuildAvailableCursorTypes`, alphabetical like the enum).
    pub const ALL: [CursorType; 28] = [
        CursorType::AppStarting,
        CursorType::Arrow,
        CursorType::Cross,
        CursorType::Default,
        CursorType::Hand,
        CursorType::Help,
        CursorType::HSplit,
        CursorType::IBeam,
        CursorType::No,
        CursorType::NoMove2D,
        CursorType::NoMoveHoriz,
        CursorType::NoMoveVert,
        CursorType::PanEast,
        CursorType::PanNE,
        CursorType::PanNorth,
        CursorType::PanNW,
        CursorType::PanSE,
        CursorType::PanSouth,
        CursorType::PanSW,
        CursorType::PanWest,
        CursorType::SizeAll,
        CursorType::SizeNESW,
        CursorType::SizeNS,
        CursorType::SizeNWSE,
        CursorType::SizeWE,
        CursorType::UpArrow,
        CursorType::VSplit,
        CursorType::WaitCursor,
    ];
}

/// `SmartEraserFillMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SmartEraserFillMode {
    #[default]
    SolidColor,
    StretchHorizontally,
    StretchVertically,
}

/// The raster effects of `Core/Annotations/Effects` (subclasses of `BaseEffectAnnotation`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    Blur,
    Pixelate,
    Highlight,
    Magnify,
}

pub const DEFAULT_SHADOW_COLOR: &str = "#80000000";
pub const DEFAULT_SHADOW_BLUR_RADIUS: f64 = 4.0;
pub const DEFAULT_SHADOW_OPACITY: f64 = 1.0;
pub const DEFAULT_SHADOW_OFFSET_X: f64 = 3.0;
pub const DEFAULT_SHADOW_OFFSET_Y: f64 = 3.0;

/// The drop shadow fields of `Annotation`.
#[derive(Clone, Debug, PartialEq)]
pub struct Shadow {
    pub enabled: bool,
    pub color: String,
    pub blur_radius: f64,
    pub opacity: f64,
    pub offset_x: f64,
    pub offset_y: f64,
}

impl Default for Shadow {
    fn default() -> Self {
        Shadow {
            enabled: false,
            color: DEFAULT_SHADOW_COLOR.to_string(),
            blur_radius: DEFAULT_SHADOW_BLUR_RADIUS,
            opacity: DEFAULT_SHADOW_OPACITY,
            offset_x: DEFAULT_SHADOW_OFFSET_X,
            offset_y: DEFAULT_SHADOW_OFFSET_Y,
        }
    }
}

/// `CurvePoint` and `CurvePointActivated` of lines and arrows (`ICurvedSegmentAnnotation`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Curve {
    pub point: Point,
    pub activated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextData {
    pub text: String,
    pub text_color: String,
    pub font_size: f32,
    pub font_family: String,
    pub bold: bool,
    pub italic: bool,
    pub align: TextAlign,
}

impl Default for TextData {
    fn default() -> Self {
        TextData {
            text: String::new(),
            text_color: "#FF000000".into(),
            font_size: 48.0,
            font_family: "Segoe UI".into(),
            bold: false,
            italic: false,
            align: TextAlign::Center,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BalloonData {
    pub tail_point: Point,
    pub tail_initialized: bool,
    pub tail_enabled: bool,
    pub text: TextData,
    pub corner_radius: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NumberData {
    pub number: i32,
    pub step_type: StepType,
    pub font_size: f32,
    pub text_color: String,
    pub bold: bool,
    pub tail_point: Point,
    pub tail_initialized: bool,
    pub tail_enabled: bool,
}

#[derive(Clone, Debug, Default)]
pub struct EffectData {
    pub effect: Option<EffectKind>,
    pub amount: f32,
    pub is_freehand: bool,
    pub is_ellipse: bool,
    /// `EffectBitmap`: the pre-rendered effect for the current bounds.
    pub bitmap: Option<Arc<RgbaImage>>,
}

impl PartialEq for EffectData {
    fn eq(&self, other: &Self) -> bool {
        self.effect == other.effect
            && self.amount == other.amount
            && self.is_freehand == other.is_freehand
            && self.is_ellipse == other.is_ellipse
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpotlightData {
    pub darken_opacity: u8,
    pub blur_amount: f32,
    pub is_ellipse: bool,
    pub canvas_size: (f32, f32),
}

/// What each concrete annotation class adds to the base class.
#[derive(Clone, Debug)]
pub enum Kind {
    Rectangle {
        border_style: BorderStyle,
        corner_radius: i32,
    },
    /// `SmartEraserAnnotation : RectangleAnnotation`.
    SmartEraser {
        fill_mode: SmartEraserFillMode,
        edge_pixels: Option<Arc<Vec<u32>>>,
    },
    Ellipse {
        border_style: BorderStyle,
    },
    Line {
        border_style: BorderStyle,
        curve: Curve,
    },
    Arrow {
        style: ArrowStyle,
        curve: Curve,
    },
    Freehand {
        points: Vec<Point>,
        border_style: BorderStyle,
        smoothing_tolerance: f32,
    },
    Text(TextData),
    SpeechBalloon(BalloonData),
    Number(NumberData),
    /// `ImageAnnotation`.
    Image {
        path: String,
        bitmap: Option<Arc<RgbaImage>>,
    },
    /// `CursorAnnotation : ImageAnnotation`.
    Cursor {
        cursor_type: CursorType,
        bitmap: Option<Arc<RgbaImage>>,
    },
    /// `EmojiAnnotation : ImageAnnotation`.
    Emoji {
        unicode: String,
        display_name: String,
        bitmap: Option<Arc<RgbaImage>>,
    },
    Effect(EffectData),
    Spotlight(SpotlightData),
    Crop,
    CutOut {
        is_vertical: bool,
    },
}

/// `Annotation`.
#[derive(Clone, Debug)]
pub struct Annotation {
    pub id: AnnotationId,
    pub tool: EditorTool,
    pub stroke_color: String,
    pub stroke_width: f32,
    pub fill_color: String,
    pub shadow: Shadow,
    pub start: Point,
    pub end: Point,
    pub rotation: f32,
    pub kind: Kind,
}

impl Annotation {
    fn base(tool: EditorTool, kind: Kind) -> Annotation {
        Annotation {
            id: new_id(),
            tool,
            stroke_color: "#ef4444".into(),
            stroke_width: 4.0,
            fill_color: "#00000000".into(),
            shadow: Shadow::default(),
            start: Point::default(),
            end: Point::default(),
            rotation: 0.0,
            kind,
        }
    }

    pub fn rectangle() -> Annotation {
        Annotation::base(
            EditorTool::Rectangle,
            Kind::Rectangle {
                border_style: BorderStyle::Solid,
                corner_radius: 0,
            },
        )
    }

    pub fn ellipse() -> Annotation {
        Annotation::base(
            EditorTool::Ellipse,
            Kind::Ellipse {
                border_style: BorderStyle::Solid,
            },
        )
    }

    pub fn line() -> Annotation {
        Annotation::base(
            EditorTool::Line,
            Kind::Line {
                border_style: BorderStyle::Solid,
                curve: Curve::default(),
            },
        )
    }

    pub fn arrow() -> Annotation {
        Annotation::base(
            EditorTool::Arrow,
            Kind::Arrow {
                style: ArrowStyle::Classic,
                curve: Curve::default(),
            },
        )
    }

    pub fn freehand() -> Annotation {
        Annotation::base(
            EditorTool::Freehand,
            Kind::Freehand {
                points: Vec::new(),
                border_style: BorderStyle::Solid,
                smoothing_tolerance: 2.0,
            },
        )
    }

    pub fn text() -> Annotation {
        Annotation::base(EditorTool::Text, Kind::Text(TextData::default()))
    }

    pub fn speech_balloon() -> Annotation {
        let mut a = Annotation::base(
            EditorTool::SpeechBalloon,
            Kind::SpeechBalloon(BalloonData {
                tail_point: Point::default(),
                tail_initialized: false,
                tail_enabled: true,
                text: TextData {
                    font_size: 20.0,
                    ..TextData::default()
                },
                corner_radius: 10,
            }),
        );
        a.stroke_width = 2.0;
        a.stroke_color = "#FF000000".into();
        a.fill_color = "#FFFFFFFF".into();
        a
    }

    pub fn number() -> Annotation {
        Annotation::base(
            EditorTool::Step,
            Kind::Number(NumberData {
                number: 1,
                step_type: StepType::Numeric,
                font_size: 24.0,
                text_color: "#FFFFFFFF".into(),
                bold: true,
                tail_point: Point::default(),
                tail_initialized: false,
                tail_enabled: true,
            }),
        )
    }

    pub fn image(bitmap: Option<Arc<RgbaImage>>) -> Annotation {
        let mut a = Annotation::base(
            EditorTool::Image,
            Kind::Image {
                path: String::new(),
                bitmap,
            },
        );
        a.stroke_width = 0.0;
        a
    }

    pub fn cursor(cursor_type: CursorType) -> Annotation {
        let mut a = Annotation::base(
            EditorTool::Cursor,
            Kind::Cursor {
                cursor_type,
                bitmap: None,
            },
        );
        a.stroke_width = 0.0;
        a
    }

    pub fn emoji(unicode: &str, display_name: &str) -> Annotation {
        let mut a = Annotation::base(
            EditorTool::Emoji,
            Kind::Emoji {
                unicode: unicode.into(),
                display_name: display_name.into(),
                bitmap: None,
            },
        );
        a.stroke_width = 0.0;
        a
    }

    pub fn smart_eraser() -> Annotation {
        let mut a = Annotation::base(
            EditorTool::SmartEraser,
            Kind::SmartEraser {
                fill_mode: SmartEraserFillMode::SolidColor,
                edge_pixels: None,
            },
        );
        a.stroke_color = "#80FF0000".into();
        a.fill_color = "#80FF0000".into();
        a.stroke_width = 0.0;
        a
    }

    pub fn effect(effect: EffectKind) -> Annotation {
        let (tool, amount) = match effect {
            EffectKind::Blur => (EditorTool::Blur, 10.0),
            EffectKind::Pixelate => (EditorTool::Pixelate, 10.0),
            EffectKind::Highlight => (EditorTool::Highlight, 10.0),
            EffectKind::Magnify => (EditorTool::Magnify, 2.0),
        };
        let mut a = Annotation::base(
            tool,
            Kind::Effect(EffectData {
                effect: Some(effect),
                amount,
                ..EffectData::default()
            }),
        );
        a.stroke_width = 0.0;
        match effect {
            EffectKind::Highlight => {
                a.stroke_color = "Transparent".into();
                a.fill_color = "#FFFF00".into();
            }
            _ => a.stroke_color = "#00000000".into(),
        }
        a
    }

    pub fn spotlight() -> Annotation {
        Annotation::base(
            EditorTool::Spotlight,
            Kind::Spotlight(SpotlightData {
                darken_opacity: 180,
                blur_amount: 0.0,
                is_ellipse: false,
                canvas_size: (0.0, 0.0),
            }),
        )
    }

    pub fn crop() -> Annotation {
        Annotation::base(EditorTool::Crop, Kind::Crop)
    }

    pub fn cut_out() -> Annotation {
        Annotation::base(EditorTool::CutOut, Kind::CutOut { is_vertical: false })
    }

    /// `Annotation.Category`.
    pub fn category(&self) -> AnnotationCategory {
        match &self.kind {
            Kind::Effect(_) | Kind::Spotlight(_) => AnnotationCategory::Effects,
            Kind::Text(_) | Kind::SpeechBalloon(_) | Kind::Number(_) => AnnotationCategory::Text,
            _ => AnnotationCategory::Shapes,
        }
    }

    /// `is RectangleAnnotation` (the smart eraser derives from it).
    pub fn is_rectangle(&self) -> bool {
        matches!(self.kind, Kind::Rectangle { .. } | Kind::SmartEraser { .. })
    }

    pub fn is_smart_eraser(&self) -> bool {
        matches!(self.kind, Kind::SmartEraser { .. })
    }

    /// `is ImageAnnotation` (cursors and emoji derive from it).
    pub fn is_image(&self) -> bool {
        matches!(
            self.kind,
            Kind::Image { .. } | Kind::Cursor { .. } | Kind::Emoji { .. }
        )
    }

    /// `is BaseEffectAnnotation`.
    pub fn is_effect(&self) -> bool {
        matches!(self.kind, Kind::Effect(_))
    }

    pub fn effect_kind(&self) -> Option<EffectKind> {
        match &self.kind {
            Kind::Effect(data) => data.effect,
            _ => None,
        }
    }

    /// `is ICurvedSegmentAnnotation`.
    pub fn is_curved_segment(&self) -> bool {
        matches!(self.kind, Kind::Line { .. } | Kind::Arrow { .. })
    }

    pub fn curve(&self) -> Option<Curve> {
        match &self.kind {
            Kind::Line { curve, .. } | Kind::Arrow { curve, .. } => Some(*curve),
            _ => None,
        }
    }

    fn curve_mut(&mut self) -> Option<&mut Curve> {
        match &mut self.kind {
            Kind::Line { curve, .. } | Kind::Arrow { curve, .. } => Some(curve),
            _ => None,
        }
    }

    pub fn border_style(&self) -> Option<BorderStyle> {
        match &self.kind {
            Kind::Rectangle { border_style, .. }
            | Kind::Ellipse { border_style }
            | Kind::Line { border_style, .. }
            | Kind::Freehand { border_style, .. } => Some(*border_style),
            Kind::SmartEraser { .. } => Some(BorderStyle::Solid),
            _ => None,
        }
    }

    pub fn set_border_style(&mut self, style: BorderStyle) {
        match &mut self.kind {
            Kind::Rectangle { border_style, .. }
            | Kind::Ellipse { border_style }
            | Kind::Line { border_style, .. }
            | Kind::Freehand { border_style, .. } => *border_style = style,
            _ => {}
        }
    }

    pub fn corner_radius(&self) -> Option<i32> {
        match &self.kind {
            Kind::Rectangle { corner_radius, .. } => Some(*corner_radius),
            Kind::SmartEraser { .. } => Some(0),
            Kind::SpeechBalloon(b) => Some(b.corner_radius),
            _ => None,
        }
    }

    pub fn freehand_points(&self) -> Option<&[Point]> {
        match &self.kind {
            Kind::Freehand { points, .. } => Some(points),
            _ => None,
        }
    }

    pub fn freehand_points_mut(&mut self) -> Option<&mut Vec<Point>> {
        match &mut self.kind {
            Kind::Freehand { points, .. } => Some(points),
            _ => None,
        }
    }

    pub fn text_data(&self) -> Option<&TextData> {
        match &self.kind {
            Kind::Text(t) => Some(t),
            Kind::SpeechBalloon(b) => Some(&b.text),
            _ => None,
        }
    }

    pub fn text_data_mut(&mut self) -> Option<&mut TextData> {
        match &mut self.kind {
            Kind::Text(t) => Some(t),
            Kind::SpeechBalloon(b) => Some(&mut b.text),
            _ => None,
        }
    }

    pub fn image_bitmap(&self) -> Option<&Arc<RgbaImage>> {
        match &self.kind {
            Kind::Image { bitmap, .. }
            | Kind::Cursor { bitmap, .. }
            | Kind::Emoji { bitmap, .. } => bitmap.as_ref(),
            _ => None,
        }
    }

    /// `ImageAnnotation.SetImage`.
    pub fn set_image(&mut self, image: Option<Arc<RgbaImage>>) {
        match &mut self.kind {
            Kind::Image { bitmap, .. }
            | Kind::Cursor { bitmap, .. }
            | Kind::Emoji { bitmap, .. } => *bitmap = image,
            _ => {}
        }
    }

    pub fn effect_data(&self) -> Option<&EffectData> {
        match &self.kind {
            Kind::Effect(e) => Some(e),
            _ => None,
        }
    }

    pub fn effect_data_mut(&mut self) -> Option<&mut EffectData> {
        match &mut self.kind {
            Kind::Effect(e) => Some(e),
            _ => None,
        }
    }

    /// `Annotation.Clone`: the same identity; effect bitmaps are left to be regenerated.
    pub fn clone_for_history(&self) -> Annotation {
        let mut clone = self.clone();
        if let Kind::Effect(data) = &mut clone.kind {
            data.bitmap = None;
        }
        clone
    }

    /// `Annotation.GetBounds` and its overrides.
    pub fn bounds(&self) -> Rect {
        match &self.kind {
            Kind::Line { .. } | Kind::Arrow { .. } => curve_bounds(self),
            Kind::Freehand { points, .. } => {
                if points.is_empty() {
                    return Rect::EMPTY;
                }
                let mut r = Rect::new(points[0].x, points[0].y, points[0].x, points[0].y);
                for p in points {
                    r.left = r.left.min(p.x);
                    r.top = r.top.min(p.y);
                    r.right = r.right.max(p.x);
                    r.bottom = r.bottom.max(p.y);
                }
                r
            }
            Kind::Text(_) => {
                let r = Rect::from_points(self.start, self.end);
                const MIN_SIZE: f32 = 10.0;
                let right = if r.right - r.left < MIN_SIZE {
                    r.left + MIN_SIZE
                } else {
                    r.right
                };
                let bottom = if r.bottom - r.top < MIN_SIZE {
                    r.top + MIN_SIZE
                } else {
                    r.bottom
                };
                Rect::new(r.left, r.top, right, bottom)
            }
            Kind::Number(n) => {
                let radius = number_radius(n);
                Rect::new(
                    self.start.x - radius,
                    self.start.y - radius,
                    self.start.x + radius,
                    self.start.y + radius,
                )
            }
            _ => Rect::from_points(self.start, self.end),
        }
    }

    /// `HitTest(point, tolerance)` with each class's default tolerance (5, except the base
    /// class's 10 which no concrete class uses).
    pub fn hit_test(&self, point: Point) -> bool {
        self.hit_test_with(point, 5.0)
    }

    pub fn hit_test_with(&self, point: Point, tolerance: f32) -> bool {
        match &self.kind {
            Kind::Line { .. } | Kind::Arrow { .. } => distance_to_path(self, point) <= tolerance,
            Kind::Rectangle { .. } | Kind::SmartEraser { .. } => {
                let rect = self.bounds();
                let p = unrotate(point, &rect, self.rotation);
                rect.inflate(tolerance, tolerance).contains(p)
            }
            Kind::Ellipse { .. } => {
                let rect = self.bounds();
                let p = unrotate(point, &rect, self.rotation);
                if !rect.inflate(tolerance, tolerance).contains(p) {
                    return false;
                }
                let rx = rect.width() / 2.0 + tolerance;
                let ry = rect.height() / 2.0 + tolerance;
                if rx <= 0.0 || ry <= 0.0 {
                    return false;
                }
                let dx = p.x - rect.mid_x();
                let dy = p.y - rect.mid_y();
                (dx * dx) / (rx * rx) + (dy * dy) / (ry * ry) <= 1.0
            }
            Kind::Freehand { points, .. } => {
                let bounds = self.bounds();
                if !bounds.inflate(tolerance, tolerance).contains(point) {
                    return false;
                }
                points
                    .windows(2)
                    .any(|w| freehand_distance(point, w[0], w[1]) <= tolerance)
            }
            Kind::Text(_) | Kind::Image { .. } | Kind::Cursor { .. } | Kind::Emoji { .. } => {
                let rect = self.bounds();
                let p = unrotate(point, &rect, self.rotation);
                rect.inflate(tolerance, tolerance).contains(p)
            }
            Kind::SpeechBalloon(_) => {
                let rect = self.bounds();
                let p = unrotate(point, &rect, self.rotation);
                if rect.inflate(tolerance, tolerance).contains(p) {
                    return true;
                }
                match balloon_tail_polygon(self) {
                    Some((a, tip, b)) => {
                        point_in_triangle(p, a, tip, b)
                            || distance_to_segment_exact(p, a, tip) <= tolerance
                            || distance_to_segment_exact(p, tip, b) <= tolerance
                            || distance_to_segment_exact(p, b, a) <= tolerance
                    }
                    None => false,
                }
            }
            Kind::Number(n) => {
                if point.distance(self.start) <= number_radius(n) + tolerance {
                    return true;
                }
                match number_tail_polygon(self) {
                    Some((a, tip, b)) => {
                        point_in_triangle(point, a, tip, b)
                            || distance_to_segment_exact(point, a, tip) <= tolerance
                            || distance_to_segment_exact(point, tip, b) <= tolerance
                            || distance_to_segment_exact(point, b, a) <= tolerance
                    }
                    None => false,
                }
            }
            Kind::Effect(data) => {
                let rect = self.bounds();
                let p = unrotate(point, &rect, self.rotation);
                let inflated = rect.inflate(tolerance, tolerance);
                if !data.is_ellipse {
                    return inflated.contains(p);
                }
                ellipse_contains(&inflated, p)
            }
            Kind::Spotlight(data) => {
                let inflated = self.bounds().inflate(tolerance, tolerance);
                if !data.is_ellipse {
                    return inflated.contains(point);
                }
                ellipse_contains(&inflated, point)
            }
            Kind::Crop => {
                let rect = self.bounds();
                rect.inflate(tolerance, tolerance).contains(point)
                    && !rect.inflate(-tolerance, -tolerance).contains(point)
            }
            Kind::CutOut { is_vertical } => {
                let rect = self.bounds();
                if *is_vertical {
                    (point.x - rect.mid_x()).abs() <= tolerance
                } else {
                    (point.y - rect.mid_y()).abs() <= tolerance
                }
            }
        }
    }

    /// Moves every point of the annotation (start, end, freehand points, curve point and
    /// tails), as the selection controller's `HandleMove` does for each kind.
    pub fn translate(&mut self, dx: f32, dy: f32) {
        self.start = self.start.offset(dx, dy);
        self.end = self.end.offset(dx, dy);
        match &mut self.kind {
            Kind::Freehand { points, .. } => {
                for p in points.iter_mut() {
                    *p = p.offset(dx, dy);
                }
            }
            Kind::Line { .. } | Kind::Arrow { .. } => {
                offset_curve_point(self, dx, dy);
            }
            Kind::Number(n) if n.tail_initialized => {
                n.tail_point = n.tail_point.offset(dx, dy);
            }
            _ => {}
        }
        if matches!(self.kind, Kind::SpeechBalloon(_)) {
            // `HandleMove` gives a balloon an explicit tail at the moved effective point.
            let tail = balloon_effective_tail_point_before_move(self, dx, dy);
            set_balloon_tail(self, tail);
        }
    }
}

fn unrotate(point: Point, rect: &Rect, rotation: f32) -> Point {
    if rotation == 0.0 {
        point
    } else {
        point.rotate_around(rect.center(), -rotation)
    }
}

fn ellipse_contains(rect: &Rect, p: Point) -> bool {
    let rx = rect.width() / 2.0;
    let ry = rect.height() / 2.0;
    if rx <= 0.0 || ry <= 0.0 {
        return false;
    }
    let nx = (p.x - rect.mid_x()) / rx;
    let ny = (p.y - rect.mid_y()) / ry;
    nx * nx + ny * ny <= 1.0
}

/// `FreehandAnnotation.DistanceToSegment` (no epsilon on the degenerate case).
fn freehand_distance(p: Point, v: Point, w: Point) -> f32 {
    let l2 = (v.x - w.x) * (v.x - w.x) + (v.y - w.y) * (v.y - w.y);
    if l2 == 0.0 {
        return p.distance(v);
    }
    let t = (((p.x - v.x) * (w.x - v.x) + (p.y - v.y) * (w.y - v.y)) / l2).clamp(0.0, 1.0);
    p.distance(Point::new(v.x + t * (w.x - v.x), v.y + t * (w.y - v.y)))
}

/// The number and balloon `DistanceToSegment` (epsilon 0.001 on the squared length).
fn distance_to_segment_exact(p: Point, a: Point, b: Point) -> f32 {
    distance_to_segment(p, a, b)
}

// ---------------------------------------------------------------------------------------
// CurvedSegmentHelper

const CURVE_TOLERANCE_EPSILON: f32 = 0.5;

pub fn midpoint(a: Point, b: Point) -> Point {
    Point::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5)
}

/// `CurvedSegmentHelper.SupportsCurve`: everything but the modern arrow.
pub fn supports_curve(a: &Annotation) -> bool {
    !matches!(
        a.kind,
        Kind::Arrow {
            style: ArrowStyle::Modern,
            ..
        }
    )
}

pub fn effective_curve_point(a: &Annotation) -> Point {
    match a.curve() {
        Some(curve) if supports_curve(a) && curve.activated => curve.point,
        _ => midpoint(a.start, a.end),
    }
}

pub fn quadratic_control_point(a: &Annotation) -> Point {
    control_point_for(a.start, a.end, effective_curve_point(a))
}

pub fn control_point_for(start: Point, end: Point, anchor: Point) -> Point {
    Point::new(
        2.0 * anchor.x - 0.5 * (start.x + end.x),
        2.0 * anchor.y - 0.5 * (start.y + end.y),
    )
}

pub fn reset_curve_point(a: &mut Annotation) {
    let mid = midpoint(a.start, a.end);
    if let Some(curve) = a.curve_mut() {
        curve.point = mid;
        curve.activated = false;
    }
}

pub fn has_curve(a: &Annotation) -> bool {
    let Some(curve) = a.curve() else {
        return false;
    };
    if !supports_curve(a) || !curve.activated {
        return false;
    }
    let mid = midpoint(a.start, a.end);
    let dx = curve.point.x - mid.x;
    let dy = curve.point.y - mid.y;
    dx * dx + dy * dy > CURVE_TOLERANCE_EPSILON * CURVE_TOLERANCE_EPSILON
}

pub fn ensure_curve_activated(a: &mut Annotation) {
    if !supports_curve(a) {
        reset_curve_point(a);
        return;
    }
    let mid = midpoint(a.start, a.end);
    if let Some(curve) = a.curve_mut()
        && !curve.activated
    {
        curve.point = mid;
        curve.activated = true;
    }
}

pub fn set_curve_point(a: &mut Annotation, point: Point) {
    if !supports_curve(a) {
        reset_curve_point(a);
        return;
    }
    if let Some(curve) = a.curve_mut() {
        curve.point = point;
        curve.activated = true;
    }
}

pub fn offset_curve_point(a: &mut Annotation, dx: f32, dy: f32) {
    if !supports_curve(a) {
        return;
    }
    if let Some(curve) = a.curve_mut()
        && curve.activated
    {
        curve.point = curve.point.offset(dx, dy);
    }
}

/// `CurvedSegmentHelper.SetEndpoints`.
pub fn set_endpoints(a: &mut Annotation, start: Point, end: Point) {
    let keep_neutral = supports_curve(a) && a.curve().is_some_and(|c| c.activated) && !has_curve(a);
    a.start = start;
    a.end = end;
    if keep_neutral {
        if let Some(curve) = a.curve_mut() {
            curve.point = midpoint(start, end);
        }
    } else if !supports_curve(a) {
        reset_curve_point(a);
    }
}

/// Sets the arrow style the way the `Style` setter does (a modern arrow drops its curve).
pub fn set_arrow_style(a: &mut Annotation, new_style: ArrowStyle) {
    if let Kind::Arrow { style, .. } = &mut a.kind {
        *style = new_style;
    }
    if new_style == ArrowStyle::Modern {
        reset_curve_point(a);
    }
}

pub fn curve_path_points(a: &Annotation, segments: usize) -> Vec<Point> {
    if !has_curve(a) {
        return vec![a.start, a.end];
    }
    sample_quadratic(a.start, quadratic_control_point(a), a.end, segments)
}

fn sample_quadratic(start: Point, control: Point, end: Point, segments: usize) -> Vec<Point> {
    let count = segments.max(2);
    (0..=count)
        .map(|i| {
            let t = i as f32 / count as f32;
            let u = 1.0 - t;
            Point::new(
                u * u * start.x + 2.0 * u * t * control.x + t * t * end.x,
                u * u * start.y + 2.0 * u * t * control.y + t * t * end.y,
            )
        })
        .collect()
}

pub fn distance_to_path(a: &Annotation, point: Point) -> f32 {
    let points = curve_path_points(a, 24);
    match points.len() {
        0 => f32::MAX,
        1 => point.distance(points[0]),
        _ => points
            .windows(2)
            .map(|w| distance_to_segment(point, w[0], w[1]))
            .fold(f32::MAX, f32::min),
    }
}

fn curve_bounds(a: &Annotation) -> Rect {
    let points = curve_path_points(a, 24);
    let mut r = Rect::new(points[0].x, points[0].y, points[0].x, points[0].y);
    for p in &points {
        r.left = r.left.min(p.x);
        r.top = r.top.min(p.y);
        r.right = r.right.max(p.x);
        r.bottom = r.bottom.max(p.y);
    }
    r
}

pub fn tangent_at_start(a: &Annotation) -> Point {
    let c = quadratic_control_point(a);
    let mut t = Point::new(c.x - a.start.x, c.y - a.start.y);
    if t.x.abs() < 0.001 && t.y.abs() < 0.001 {
        t = Point::new(a.end.x - a.start.x, a.end.y - a.start.y);
    }
    t
}

pub fn tangent_at_end(a: &Annotation) -> Point {
    let c = quadratic_control_point(a);
    let mut t = Point::new(a.end.x - c.x, a.end.y - c.y);
    if t.x.abs() < 0.001 && t.y.abs() < 0.001 {
        t = Point::new(a.end.x - a.start.x, a.end.y - a.start.y);
    }
    t
}

// ---------------------------------------------------------------------------------------
// NumberAnnotation

const GEOMETRY_EPSILON: f32 = 0.001;
const TEXT_PADDING_MULTIPLIER: f32 = 0.35;

/// `NumberAnnotation.Radius` (`CalculateRadius`): big enough for the label at
/// `FontSize * 0.6` in the default typeface.
pub fn number_radius(n: &NumberData) -> f32 {
    let base = (n.font_size * 0.7).max(12.0);
    let label = n.step_type.format(n.number);
    let size = (n.font_size * 0.6).max(1.0);
    let metrics = crate::text::measure_default_line(&label, size, n.bold);
    let padding = (n.font_size * TEXT_PADDING_MULTIPLIER).max(6.0);
    let measured = metrics.0.max(metrics.1) * 0.5 + padding;
    base.max(measured)
}

pub fn number_display_text(n: &NumberData) -> String {
    n.step_type.format(n.number)
}

/// `NumberAnnotation.GetTailHandlePoint`.
pub fn number_tail_handle(a: &Annotation) -> Point {
    match &a.kind {
        Kind::Number(n) if n.tail_initialized => n.tail_point,
        _ => {
            let b = a.bounds();
            Point::new(b.right, b.bottom)
        }
    }
}

/// `NumberAnnotation.TryGetTailPolygon`.
pub fn number_tail_polygon(a: &Annotation) -> Option<(Point, Point, Point)> {
    let Kind::Number(n) = &a.kind else {
        return None;
    };
    if !n.tail_enabled || !n.tail_initialized {
        return None;
    }
    let center = a.start;
    let radius = number_radius(n);
    if radius <= GEOMETRY_EPSILON {
        return None;
    }
    let tip = n.tail_point;
    let dx = tip.x - center.x;
    let dy = tip.y - center.y;
    let length = (dx * dx + dy * dy).sqrt();
    if length <= radius + GEOMETRY_EPSILON {
        return None;
    }
    let ux = dx / length;
    let uy = dy / length;
    let perp = Point::new(-uy, ux);
    let projection = radius * radius / length;
    let offset = radius * (length * length - radius * radius).sqrt() / length;
    let tangent_center = Point::new(center.x + ux * projection, center.y + uy * projection);
    Some((
        Point::new(
            tangent_center.x + perp.x * offset,
            tangent_center.y + perp.y * offset,
        ),
        tip,
        Point::new(
            tangent_center.x - perp.x * offset,
            tangent_center.y - perp.y * offset,
        ),
    ))
}

pub fn number_tail_visible(a: &Annotation) -> bool {
    matches!(&a.kind, Kind::Number(n) if n.tail_enabled) && number_tail_polygon(a).is_some()
}

/// `GetInteractionBounds` of numbers and balloons: the body and the tail.
pub fn interaction_bounds(a: &Annotation, tolerance: f32) -> Rect {
    let mut bounds = a.bounds();
    let tail = match &a.kind {
        Kind::Number(_) => number_tail_polygon(a),
        Kind::SpeechBalloon(_) => balloon_tail_polygon(a),
        _ => None,
    };
    if let Some((s, tip, e)) = tail {
        let tail_rect = Rect::new(
            s.x.min(tip.x.min(e.x)),
            s.y.min(tip.y.min(e.y)),
            s.x.max(tip.x.max(e.x)),
            s.y.max(tip.y.max(e.y)),
        );
        bounds = bounds.union(&tail_rect);
    }
    if tolerance > 0.0 {
        bounds = bounds.inflate(tolerance, tolerance);
    }
    bounds
}

pub fn set_number_tail(a: &mut Annotation, point: Point) {
    if let Kind::Number(n) = &mut a.kind {
        n.tail_point = point;
        n.tail_initialized = true;
    }
}

// ---------------------------------------------------------------------------------------
// SpeechBalloonAnnotation

pub const BALLOON_DEFAULT_TAIL_OFFSET: f32 = 30.0;
pub const BALLOON_TAIL_WIDTH_MULTIPLIER: f32 = 0.3;

/// `HasTailPoint`: initialized, or a point other than (0, 0) from an older document.
pub fn balloon_has_tail_point(a: &Annotation) -> bool {
    match &a.kind {
        Kind::SpeechBalloon(b) => b.tail_initialized || b.tail_point != Point::default(),
        _ => false,
    }
}

pub fn balloon_default_tail_point(a: &Annotation) -> Point {
    let b = a.bounds();
    Point::new(b.mid_x(), b.bottom + BALLOON_DEFAULT_TAIL_OFFSET)
}

/// The effective tail before a move of (`dx`, `dy`), moved along with the body.
fn balloon_effective_tail_point_before_move(a: &Annotation, dx: f32, dy: f32) -> Point {
    if balloon_has_tail_point(a) {
        let p = balloon_effective_tail_point(a);
        p.offset(dx, dy)
    } else {
        // The default tail follows the body, which has already moved.
        balloon_default_tail_point(a)
    }
}

pub fn balloon_effective_tail_point(a: &Annotation) -> Point {
    match &a.kind {
        Kind::SpeechBalloon(b) if balloon_has_tail_point(a) => b.tail_point,
        _ => balloon_default_tail_point(a),
    }
}

pub fn set_balloon_tail(a: &mut Annotation, point: Point) {
    if let Kind::SpeechBalloon(b) = &mut a.kind {
        b.tail_point = point;
        b.tail_initialized = true;
    }
}

/// `EnsureTailPointInitialized`.
pub fn ensure_balloon_tail(a: &mut Annotation) {
    if !balloon_has_tail_point(a) {
        let p = balloon_default_tail_point(a);
        set_balloon_tail(a, p);
    } else if let Kind::SpeechBalloon(b) = &mut a.kind
        && !b.tail_initialized
        && b.tail_point != Point::default()
    {
        b.tail_initialized = true;
    }
}

pub fn balloon_tail_visible(a: &Annotation) -> bool {
    match &a.kind {
        Kind::SpeechBalloon(b) if b.tail_enabled => {
            let bounds = a.bounds();
            !bounds.contains(balloon_effective_tail_point(a))
        }
        _ => false,
    }
}

/// `SpeechBalloonAnnotation.TryGetTailPolygon`.
pub fn balloon_tail_polygon(a: &Annotation) -> Option<(Point, Point, Point)> {
    let Kind::SpeechBalloon(data) = &a.kind else {
        return None;
    };
    if !data.tail_enabled {
        return None;
    }
    let bounds = a.bounds();
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return None;
    }
    let tip = balloon_effective_tail_point(a);
    if bounds.contains(tip) {
        return None;
    }
    let center = bounds.center();
    let dx = tip.x - center.x;
    let dy = tip.y - center.y;
    let length = (dx * dx + dy * dy).sqrt();
    if length <= GEOMETRY_EPSILON {
        return None;
    }
    let ux = dx / length;
    let uy = dy / length;
    let average = (bounds.width() + bounds.height()) / 2.0;
    let tail_width =
        (BALLOON_TAIL_WIDTH_MULTIPLIER * average).min(bounds.width().min(bounds.height()));
    if tail_width <= GEOMETRY_EPSILON {
        return None;
    }
    let half = tail_width / 2.0;
    let perp = Point::new(-uy, ux);
    let base_start = Point::new(center.x + perp.x * half, center.y + perp.y * half);
    let base_end = Point::new(center.x - perp.x * half, center.y - perp.y * half);
    let s = segment_exit_point(&bounds, base_start, tip)?;
    let e = segment_exit_point(&bounds, base_end, tip)?;
    Some((s, tip, e))
}

fn segment_exit_point(bounds: &Rect, start: Point, end: Point) -> Option<Point> {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let mut best_t = f32::INFINITY;
    let mut best = Point::default();
    let mut consider = |t: f32| {
        if t < -GEOMETRY_EPSILON || t > 1.0 + GEOMETRY_EPSILON || t >= best_t {
            return;
        }
        let x = start.x + dx * t;
        let y = start.y + dy * t;
        if x < bounds.left - GEOMETRY_EPSILON
            || x > bounds.right + GEOMETRY_EPSILON
            || y < bounds.top - GEOMETRY_EPSILON
            || y > bounds.bottom + GEOMETRY_EPSILON
        {
            return;
        }
        best_t = t.clamp(0.0, 1.0);
        best = Point::new(
            x.clamp(bounds.left, bounds.right),
            y.clamp(bounds.top, bounds.bottom),
        );
    };
    if dx.abs() > GEOMETRY_EPSILON {
        consider((bounds.left - start.x) / dx);
        consider((bounds.right - start.x) / dx);
    }
    if dy.abs() > GEOMETRY_EPSILON {
        consider((bounds.top - start.y) / dy);
        consider((bounds.bottom - start.y) / dy);
    }
    if best_t.is_infinite() {
        None
    } else {
        Some(best)
    }
}

// ---------------------------------------------------------------------------------------
// SmartEraserAnnotation

/// `SmartEraserAnnotation.ConfigureFill`: matching left/right columns stretch
/// horizontally, matching top/bottom rows vertically, else the top-left pixel.
pub fn configure_smart_eraser(a: &mut Annotation, source: &RgbaImage) {
    let (w, h) = (source.width() as i32, source.height() as i32);
    if w <= 0 || h <= 0 {
        return;
    }
    let bounds = a.bounds();
    let left = (bounds.left.floor() as i32).clamp(0, w - 1);
    let top = (bounds.top.floor() as i32).clamp(0, h - 1);
    let right = (left.max(bounds.right.ceil() as i32 - 1)).clamp(0, w - 1);
    let bottom = (top.max(bounds.bottom.ceil() as i32 - 1)).clamp(0, h - 1);
    let pixel = |x: i32, y: i32| {
        let p = source.get_pixel(x as u32, y as u32).0;
        crate::color::Color::rgba(p[0], p[1], p[2], p[3])
    };
    let fallback = pixel(left, top);
    let hex = if fallback.a == 255 {
        fallback.to_hex_rgb()
    } else {
        fallback.to_hex_argb()
    };
    a.stroke_color = hex.clone();
    a.fill_color = hex;
    let mut mode = SmartEraserFillMode::SolidColor;
    let mut edge: Option<Vec<u32>> = None;
    if (top..=bottom).all(|y| pixel(left, y) == pixel(right, y)) {
        mode = SmartEraserFillMode::StretchHorizontally;
        edge = Some((top..=bottom).map(|y| pixel(left, y).pack()).collect());
    } else if (left..=right).all(|x| pixel(x, top) == pixel(x, bottom)) {
        mode = SmartEraserFillMode::StretchVertically;
        edge = Some((left..=right).map(|x| pixel(x, top).pack()).collect());
    }
    if let Kind::SmartEraser {
        fill_mode,
        edge_pixels,
    } = &mut a.kind
    {
        *fill_mode = mode;
        *edge_pixels = edge.map(Arc::new);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_labels_follow_the_formatter() {
        assert_eq!(StepType::UppercaseLetter.format(1), "A");
        assert_eq!(StepType::UppercaseLetter.format(27), "AA");
        assert_eq!(StepType::LowercaseRoman.format(14), "xiv");
        assert_eq!(StepType::Numeric.format(0), "0");
        assert_eq!(StepType::UppercaseRoman.format(1994), "MCMXCIV");
    }

    #[test]
    fn rectangle_hit_test_respects_rotation_and_tolerance() {
        let mut r = Annotation::rectangle();
        r.start = Point::new(10.0, 10.0);
        r.end = Point::new(110.0, 30.0);
        assert!(r.hit_test(Point::new(60.0, 20.0)));
        assert!(r.hit_test(Point::new(112.0, 20.0)));
        assert!(!r.hit_test(Point::new(60.0, 60.0)));
        r.rotation = 90.0;
        assert!(r.hit_test(Point::new(60.0, 60.0)));
        assert!(!r.hit_test(Point::new(105.0, 20.0)));
    }

    #[test]
    fn curve_needs_a_real_offset_to_count() {
        let mut line = Annotation::line();
        line.start = Point::new(0.0, 0.0);
        line.end = Point::new(100.0, 0.0);
        ensure_curve_activated(&mut line);
        assert!(!has_curve(&line));
        set_curve_point(&mut line, Point::new(50.0, 40.0));
        assert!(has_curve(&line));
        // The quadratic passes through the anchor point at t = 0.5.
        let points = curve_path_points(&line, 24);
        let mid = points[12];
        assert!((mid.y - 40.0).abs() < 0.01, "{mid:?}");
        assert!(line.hit_test(Point::new(50.0, 41.0)));
        assert!(!line.hit_test(Point::new(50.0, 2.0)));
        let b = line.bounds();
        assert!((b.bottom - 40.0).abs() < 0.1);
    }

    #[test]
    fn modern_arrow_cannot_curve() {
        let mut arrow = Annotation::arrow();
        arrow.end = Point::new(100.0, 0.0);
        set_curve_point(&mut arrow, Point::new(50.0, 40.0));
        assert!(has_curve(&arrow));
        set_arrow_style(&mut arrow, ArrowStyle::Modern);
        assert!(!has_curve(&arrow));
        assert!(!arrow.curve().unwrap().activated);
    }

    #[test]
    fn balloon_tail_leaves_the_body_on_its_edge() {
        let mut b = Annotation::speech_balloon();
        b.start = Point::new(0.0, 0.0);
        b.end = Point::new(100.0, 50.0);
        assert!(!balloon_has_tail_point(&b));
        assert_eq!(balloon_effective_tail_point(&b), Point::new(50.0, 80.0));
        let (s, tip, e) = balloon_tail_polygon(&b).unwrap();
        assert_eq!(tip, Point::new(50.0, 80.0));
        assert!((s.y - 50.0).abs() < 0.01 && (e.y - 50.0).abs() < 0.01);
        assert!(b.hit_test(Point::new(50.0, 75.0)));
        set_balloon_tail(&mut b, Point::new(50.0, 25.0));
        assert!(balloon_tail_polygon(&b).is_none());
    }

    #[test]
    fn text_bounds_have_a_minimum_size() {
        let mut t = Annotation::text();
        t.start = Point::new(5.0, 5.0);
        t.end = Point::new(5.0, 5.0);
        assert_eq!(t.bounds(), Rect::new(5.0, 5.0, 15.0, 15.0));
    }

    #[test]
    fn smart_eraser_prefers_matching_columns() {
        let mut img = RgbaImage::from_pixel(10, 10, image::Rgba([255, 255, 255, 255]));
        for y in 0..10 {
            img.put_pixel(2, y, image::Rgba([0, 0, y as u8, 255]));
            img.put_pixel(7, y, image::Rgba([0, 0, y as u8, 255]));
        }
        let mut e = Annotation::smart_eraser();
        e.start = Point::new(2.0, 1.0);
        e.end = Point::new(8.0, 5.0);
        configure_smart_eraser(&mut e, &img);
        let Kind::SmartEraser {
            fill_mode,
            edge_pixels,
        } = &e.kind
        else {
            unreachable!()
        };
        assert_eq!(*fill_mode, SmartEraserFillMode::StretchHorizontally);
        assert_eq!(edge_pixels.as_ref().unwrap().len(), 4);
        assert_eq!(e.fill_color, "#000001");
    }
}
