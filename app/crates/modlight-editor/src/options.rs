//! `ImageEditorOptions`: the editor's remembered settings, kept by the host in
//! `TaskSettings.ToolsSettings.ImageEditorOptions` with upstream's member names (Newtonsoft,
//! enums as names, colours as `#AARRGGBB`). Members this build does not know are kept.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::color::Color;
use crate::model::{ArrowStyle, BorderStyle, CursorType, EditorTool, StepType, TextAlign};

pub const PRIMARY_COLOR: Color = Color::argb(255, 242, 60, 60);
pub const SECONDARY_COLOR: Color = Color::argb(255, 250, 250, 250);

/// `DefaultFavoriteEffects`.
pub const DEFAULT_FAVORITE_EFFECTS: [&str; 9] = [
    "resize_image",
    "resize_canvas",
    "crop_image",
    "auto_crop_image",
    "rotate_90_clockwise",
    "rotate_90_counter_clockwise",
    "rotate_180",
    "flip_horizontal",
    "flip_vertical",
];

/// `ImageEditorToolbarItemOptions`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ToolbarItemOptions {
    pub id: String,
    pub begin_group: Option<bool>,
    #[serde(default = "yes")]
    pub is_visible: bool,
    pub hotkey: String,
}

fn yes() -> bool {
    true
}

macro_rules! enum_serde {
    ($ty:ty) => {
        impl Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.name())
            }
        }
        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let value = Value::deserialize(d)?;
                Ok(match value {
                    Value::String(s) => <$ty>::parse(&s).unwrap_or_default(),
                    Value::Number(n) => n
                        .as_u64()
                        .and_then(|i| <$ty>::from_index(i as usize))
                        .unwrap_or_default(),
                    _ => <$ty>::default(),
                })
            }
        }
    };
}

trait FromIndex: Sized {
    fn from_index(i: usize) -> Option<Self>;
}

impl FromIndex for EditorTool {
    fn from_index(i: usize) -> Option<Self> {
        EditorTool::ALL.get(i).copied()
    }
}

impl EditorTool {
    fn from_index(i: usize) -> Option<Self> {
        <Self as FromIndex>::from_index(i)
    }
}

impl ArrowStyle {
    fn from_index(i: usize) -> Option<Self> {
        [
            ArrowStyle::Classic,
            ArrowStyle::Modern,
            ArrowStyle::Double,
            ArrowStyle::Basic,
            ArrowStyle::Line,
        ]
        .get(i)
        .copied()
    }
}

impl BorderStyle {
    fn from_index(i: usize) -> Option<Self> {
        BorderStyle::ALL.get(i).copied()
    }
}

impl CursorType {
    fn from_index(i: usize) -> Option<Self> {
        CursorType::ALL.get(i).copied()
    }
}

impl StepType {
    fn from_index(i: usize) -> Option<Self> {
        StepType::ALL.get(i).copied()
    }
}

impl TextAlign {
    fn from_index(i: usize) -> Option<Self> {
        TextAlign::ALL.get(i).copied()
    }
}

impl EditorTool {
    fn parse_or_default(text: &str) -> Option<EditorTool> {
        EditorTool::parse(text)
    }
}

impl Serialize for EditorTool {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for EditorTool {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(d)?;
        Ok(match value {
            Value::String(s) => EditorTool::parse_or_default(&s).unwrap_or(EditorTool::Rectangle),
            Value::Number(n) => n
                .as_u64()
                .and_then(|i| EditorTool::from_index(i as usize))
                .unwrap_or(EditorTool::Rectangle),
            _ => EditorTool::Rectangle,
        })
    }
}

enum_serde!(ArrowStyle);
enum_serde!(BorderStyle);
enum_serde!(CursorType);
enum_serde!(StepType);
enum_serde!(TextAlign);

fn hex(c: Color) -> String {
    c.to_hex_argb()
}

/// `ImageEditorOptions` with upstream's defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ImageEditorOptions {
    pub last_used_annotation_tool: EditorTool,
    pub remember_window_state: bool,
    pub is_window_maximized: bool,
    pub window_width: f64,
    pub window_height: f64,
    pub show_exit_confirmation: bool,
    pub zoom_to_fit_on_open: bool,
    pub quick_crop: bool,
    pub auto_close_editor_on_task: bool,
    pub auto_copy_image_to_clipboard: bool,
    pub show_insert_image_dialog: bool,
    pub show_notifications: bool,
    pub toolbar_items: Vec<ToolbarItemOptions>,

    pub border_color_hex: String,
    pub fill_color_hex: String,
    pub thickness: i32,
    pub corner_radius: i32,
    pub shadow: bool,
    pub shadow_color_hex: String,
    pub shadow_blur_radius: f64,
    pub shadow_opacity: f64,
    pub shadow_offset_x: f64,
    pub shadow_offset_y: f64,
    pub border_style: BorderStyle,
    pub arrow_style: ArrowStyle,
    pub cursor_type: CursorType,

    pub text_border_color_hex: String,
    pub text_text_color_hex: String,
    pub text_thickness: i32,
    pub text_font_size: f32,
    pub text_font_family: String,
    pub text_horizontal_alignment: TextAlign,
    pub text_bold: bool,
    pub text_italic: bool,

    pub speech_balloon_border_color_hex: String,
    pub speech_balloon_fill_color_hex: String,
    pub speech_balloon_text_color_hex: String,
    pub speech_balloon_thickness: i32,
    pub speech_balloon_font_size: f32,
    pub speech_balloon_font_family: String,
    pub speech_balloon_text_horizontal_alignment: TextAlign,
    pub speech_balloon_text_bold: bool,
    pub speech_balloon_text_italic: bool,
    pub speech_balloon_tail: bool,

    pub step_border_color_hex: String,
    pub step_fill_color_hex: String,
    pub step_text_color_hex: String,
    pub step_thickness: i32,
    pub step_font_size: f32,
    pub step_text_bold: bool,
    pub step_type: StepType,
    pub step_tail: bool,

    pub highlight_fill_color_hex: String,

    pub blur_strength: f32,
    pub pixelate_strength: f32,
    pub magnifier_strength: f32,
    pub magnifier_ellipse: bool,
    pub spotlight_strength: f32,
    pub spotlight_blur: f32,
    pub spotlight_ellipse: bool,

    pub background_margin: f64,
    pub background_padding: f64,
    pub background_smart_padding: bool,
    pub background_rounded_corner: f64,
    pub background_shadow_radius: f64,
    pub background_type: String,
    pub background_gradient_color1_hex: String,
    pub background_gradient_color2_hex: String,
    pub background_color_hex: String,
    pub background_image_path: String,

    pub recent_image_files: Vec<String>,
    pub max_recent_image_files: i32,
    pub recent_effects: Vec<String>,
    pub favorite_effects: Vec<String>,

    /// Members this build does not know, kept as they came.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for ImageEditorOptions {
    fn default() -> Self {
        ImageEditorOptions {
            last_used_annotation_tool: EditorTool::Rectangle,
            remember_window_state: true,
            is_window_maximized: true,
            window_width: 1280.0,
            window_height: 720.0,
            show_exit_confirmation: true,
            zoom_to_fit_on_open: false,
            quick_crop: true,
            auto_close_editor_on_task: false,
            auto_copy_image_to_clipboard: false,
            show_insert_image_dialog: true,
            show_notifications: true,
            toolbar_items: Vec::new(),
            border_color_hex: hex(PRIMARY_COLOR),
            fill_color_hex: hex(Color::TRANSPARENT),
            thickness: 4,
            corner_radius: 4,
            shadow: false,
            shadow_color_hex: crate::model::DEFAULT_SHADOW_COLOR.into(),
            shadow_blur_radius: crate::model::DEFAULT_SHADOW_BLUR_RADIUS,
            shadow_opacity: crate::model::DEFAULT_SHADOW_OPACITY,
            shadow_offset_x: crate::model::DEFAULT_SHADOW_OFFSET_X,
            shadow_offset_y: crate::model::DEFAULT_SHADOW_OFFSET_Y,
            border_style: BorderStyle::Solid,
            arrow_style: ArrowStyle::Classic,
            cursor_type: CursorType::Default,
            text_border_color_hex: hex(PRIMARY_COLOR),
            text_text_color_hex: hex(SECONDARY_COLOR),
            text_thickness: 8,
            text_font_size: 48.0,
            text_font_family: "Segoe UI".into(),
            text_horizontal_alignment: TextAlign::Center,
            text_bold: true,
            text_italic: false,
            speech_balloon_border_color_hex: hex(Color::TRANSPARENT),
            speech_balloon_fill_color_hex: hex(PRIMARY_COLOR),
            speech_balloon_text_color_hex: hex(SECONDARY_COLOR),
            speech_balloon_thickness: 4,
            speech_balloon_font_size: 48.0,
            speech_balloon_font_family: "Segoe UI".into(),
            speech_balloon_text_horizontal_alignment: TextAlign::Center,
            speech_balloon_text_bold: true,
            speech_balloon_text_italic: false,
            speech_balloon_tail: true,
            step_border_color_hex: hex(Color::TRANSPARENT),
            step_fill_color_hex: hex(PRIMARY_COLOR),
            step_text_color_hex: hex(SECONDARY_COLOR),
            step_thickness: 4,
            step_font_size: 30.0,
            step_text_bold: true,
            step_type: StepType::Numeric,
            step_tail: true,
            highlight_fill_color_hex: hex(Color::rgb(255, 255, 0)),
            blur_strength: 30.0,
            pixelate_strength: 20.0,
            magnifier_strength: 2.0,
            magnifier_ellipse: false,
            spotlight_strength: 30.0,
            spotlight_blur: 0.0,
            spotlight_ellipse: false,
            background_margin: 80.0,
            background_padding: 40.0,
            background_smart_padding: true,
            background_rounded_corner: 20.0,
            background_shadow_radius: 30.0,
            background_type: "Transparent".into(),
            background_gradient_color1_hex: hex(Color::argb(255, 221, 36, 118)),
            background_gradient_color2_hex: hex(Color::argb(255, 255, 81, 47)),
            background_color_hex: hex(Color::argb(255, 34, 34, 34)),
            background_image_path: String::new(),
            recent_image_files: Vec::new(),
            max_recent_image_files: 10,
            recent_effects: Vec::new(),
            favorite_effects: DEFAULT_FAVORITE_EFFECTS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            extra: Map::new(),
        }
    }
}

/// A colour property of the options (`HexToColor` / `ColorToHex`).
fn color_of(hex: &str) -> Color {
    Color::parse(hex).unwrap_or(Color::TRANSPARENT)
}

impl ImageEditorOptions {
    pub fn from_value(value: &Value) -> ImageEditorOptions {
        serde_json::from_value(value.clone()).unwrap_or_default()
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn border_color(&self) -> Color {
        color_of(&self.border_color_hex)
    }
    pub fn fill_color(&self) -> Color {
        color_of(&self.fill_color_hex)
    }
    pub fn shadow_color(&self) -> Color {
        color_of(&self.shadow_color_hex)
    }
    pub fn text_border_color(&self) -> Color {
        color_of(&self.text_border_color_hex)
    }
    pub fn text_text_color(&self) -> Color {
        color_of(&self.text_text_color_hex)
    }
    pub fn speech_balloon_border_color(&self) -> Color {
        color_of(&self.speech_balloon_border_color_hex)
    }
    pub fn speech_balloon_fill_color(&self) -> Color {
        color_of(&self.speech_balloon_fill_color_hex)
    }
    pub fn speech_balloon_text_color(&self) -> Color {
        color_of(&self.speech_balloon_text_color_hex)
    }
    pub fn step_border_color(&self) -> Color {
        color_of(&self.step_border_color_hex)
    }
    pub fn step_fill_color(&self) -> Color {
        color_of(&self.step_fill_color_hex)
    }
    pub fn step_text_color(&self) -> Color {
        color_of(&self.step_text_color_hex)
    }
    pub fn highlight_fill_color(&self) -> Color {
        color_of(&self.highlight_fill_color_hex)
    }

    /// `AddRecentImageFile`: most recent first, at most `MaxRecentImageFiles`.
    pub fn add_recent_image_file(&mut self, path: &str) {
        if path.is_empty() {
            return;
        }
        self.recent_image_files.retain(|p| p != path);
        self.recent_image_files.insert(0, path.to_string());
        let max = self.max_recent_image_files.max(0) as usize;
        self.recent_image_files.truncate(max);
    }

    /// `RecentImageFiles.Remove`.
    pub fn remove_recent_image_file(&mut self, path: &str) {
        self.recent_image_files.retain(|p| p != path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_upstream_member_names_and_keeps_unknown_ones() {
        let mut value = ImageEditorOptions::default().to_value();
        assert_eq!(value["BorderColorHex"], "#FFF23C3C");
        assert_eq!(value["ArrowStyle"], "Classic");
        assert_eq!(value["LastUsedAnnotationTool"], "Rectangle");
        value["FutureMember"] = serde_json::json!(42);
        value["ArrowStyle"] = serde_json::json!(4);
        let options = ImageEditorOptions::from_value(&value);
        assert_eq!(options.arrow_style, ArrowStyle::Line);
        assert_eq!(options.to_value()["FutureMember"], 42);
    }

    #[test]
    fn recent_files_are_capped_and_deduplicated() {
        let mut o = ImageEditorOptions {
            max_recent_image_files: 2,
            ..ImageEditorOptions::default()
        };
        o.add_recent_image_file("a");
        o.add_recent_image_file("b");
        o.add_recent_image_file("a");
        o.add_recent_image_file("c");
        assert_eq!(o.recent_image_files, vec!["c", "a"]);
    }
}
