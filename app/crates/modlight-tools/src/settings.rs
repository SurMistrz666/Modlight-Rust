//! The tools' settings as upstream keeps them: `TaskSettings.ToolsSettings` (screen colour
//! picker, pin to screen, indexer, image combiner, video converter, borderless window,
//! AI, background remover), `CaptureSettings.OCROptions`,
//! `CaptureSettings.SurfaceOptions.ColorPickerOptions` and the application's
//! `HistorySettings` / `ImageHistorySettings`.
//!
//! Every type reads with Newtonsoft's leniency (missing members take upstream's
//! defaults, enums by name or number) and is written back over the member it came from,
//! so whatever the port does not model stays in the file.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::color::Color;

/// An enum ShareX writes by name (`StringEnumConverter`); numbers and unknown names read
/// leniently.
macro_rules! named_enum {
    ($(#[$doc:meta])* $name:ident { $($(#[$vdoc:meta])* $variant:ident = $text:literal),+ $(,)? } default $default:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name { $($(#[$vdoc])* $variant),+ }

        impl Default for $name {
            fn default() -> Self {
                $name::$default
            }
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            /// The member's name in the settings file.
            pub fn name(self) -> &'static str {
                match self { $($name::$variant => $text),+ }
            }

            pub fn parse(text: &str) -> Option<$name> {
                $name::ALL.iter().copied().find(|v| v.name().eq_ignore_ascii_case(text.trim()))
            }

            pub fn index(self) -> usize {
                $name::ALL.iter().position(|v| *v == self).unwrap_or(0)
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = <::serde_json::Value as ::serde::Deserialize>::deserialize(deserializer)?;
                Ok(match &value {
                    ::serde_json::Value::String(text) => $name::parse(text).unwrap_or_default(),
                    ::serde_json::Value::Number(n) => n
                        .as_u64()
                        .and_then(|i| $name::from_number(i))
                        .unwrap_or_default(),
                    _ => $name::default(),
                })
            }
        }

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.name())
            }
        }
    };
}

#[allow(unused_imports)]
pub(crate) use named_enum;

named_enum! {
    /// `System.Drawing.ContentAlignment` (its numbers are flags: 1, 2, 4, 16, ... 1024).
    ContentAlignment {
        TopLeft = "TopLeft", TopCenter = "TopCenter", TopRight = "TopRight",
        MiddleLeft = "MiddleLeft", MiddleCenter = "MiddleCenter", MiddleRight = "MiddleRight",
        BottomLeft = "BottomLeft", BottomCenter = "BottomCenter", BottomRight = "BottomRight"
    } default BottomRight
}

impl ContentAlignment {
    fn from_number(n: u64) -> Option<ContentAlignment> {
        use ContentAlignment::*;
        Some(match n {
            1 => TopLeft,
            2 => TopCenter,
            4 => TopRight,
            16 => MiddleLeft,
            32 => MiddleCenter,
            64 => MiddleRight,
            256 => BottomLeft,
            512 => BottomCenter,
            1024 => BottomRight,
            _ => return None,
        })
    }
}

/// `System.Drawing.Color` in a settings file ("R, G, B", "A, R, G, B" or a name).
pub mod color_serde {
    use super::*;

    pub fn serialize<S: serde::Serializer>(color: &Color, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&crate::color::to_dotnet_string(*color))
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Color, D::Error> {
        let value = Value::deserialize(d)?;
        Ok(match value {
            Value::String(text) => crate::color::parse_dotnet_string(&text).unwrap_or_default(),
            _ => Color::default(),
        })
    }
}

/// `System.Drawing.Size`, written as "W, H".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Size {
    pub width: i64,
    pub height: i64,
}

impl Serialize for Size {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("{}, {}", self.width, self.height))
    }
}

impl<'de> Deserialize<'de> for Size {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Size, D::Error> {
        let value = Value::deserialize(d)?;
        let parse = |text: &str| -> Option<Size> {
            let mut parts = text.split(',').map(|p| p.trim().parse::<i64>());
            Some(Size {
                width: parts.next()?.ok()?,
                height: parts.next()?.ok()?,
            })
        };
        Ok(match &value {
            Value::String(text) => parse(text).unwrap_or_default(),
            Value::Object(map) => Size {
                width: map.get("Width").and_then(Value::as_i64).unwrap_or(0),
                height: map.get("Height").and_then(Value::as_i64).unwrap_or(0),
            },
            _ => Size::default(),
        })
    }
}

// ------------------------------------------------------------------ the options

/// `ScreenColorPickerOptions`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ScreenColorPickerOptions {
    pub format: String,
    pub format_ctrl: String,
    pub info_text: String,
    pub show_magnifier: bool,
}

impl Default for ScreenColorPickerOptions {
    fn default() -> Self {
        ScreenColorPickerOptions {
            format: "$HEX".into(),
            format_ctrl: "$r255, $g255, $b255".into(),
            info_text: "#$HEX".into(),
            show_magnifier: true,
        }
    }
}

/// `ColorPickerOptions` (`CaptureSettings.SurfaceOptions.ColorPickerOptions`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ColorPickerOptions {
    pub recent_colors_selected: bool,
}

impl Default for ColorPickerOptions {
    fn default() -> Self {
        ColorPickerOptions {
            recent_colors_selected: true,
        }
    }
}

/// `PinToScreenOptions`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PinToScreenOptions {
    pub initial_scale: i64,
    pub scale_step: i64,
    pub high_quality_scale: bool,
    pub initial_opacity: i64,
    pub opacity_step: i64,
    pub placement: ContentAlignment,
    pub placement_offset: i64,
    pub top_most: bool,
    pub keep_center_location: bool,
    #[serde(with = "color_serde")]
    pub background_color: Color,
    pub shadow: bool,
    pub border: bool,
    pub border_size: i64,
    #[serde(with = "color_serde")]
    pub border_color: Color,
    pub minimize_size: Size,
}

impl Default for PinToScreenOptions {
    fn default() -> Self {
        PinToScreenOptions {
            initial_scale: 100,
            scale_step: 10,
            high_quality_scale: true,
            initial_opacity: 100,
            opacity_step: 10,
            placement: ContentAlignment::BottomRight,
            placement_offset: 10,
            top_most: true,
            keep_center_location: true,
            background_color: Color::WHITE,
            shadow: true,
            border: true,
            border_size: 2,
            border_color: Color::CORNFLOWER_BLUE,
            minimize_size: Size {
                width: 100,
                height: 100,
            },
        }
    }
}

named_enum! {
    /// `IndexerOutput`.
    IndexerOutput { Txt = "Txt", Html = "Html", Xml = "Xml", Json = "Json" } default Html
}

impl IndexerOutput {
    fn from_number(n: u64) -> Option<IndexerOutput> {
        IndexerOutput::ALL.get(n as usize).copied()
    }

    /// The `[Description]` the menus show.
    pub fn description(self) -> &'static str {
        match self {
            IndexerOutput::Txt => "Text",
            IndexerOutput::Html => "HTML",
            IndexerOutput::Xml => "XML",
            IndexerOutput::Json => "JSON",
        }
    }

    /// `Output.ToString().ToLower()`: the extension the uploaded index gets.
    pub fn extension(self) -> &'static str {
        match self {
            IndexerOutput::Txt => "txt",
            IndexerOutput::Html => "html",
            IndexerOutput::Xml => "xml",
            IndexerOutput::Json => "json",
        }
    }
}

/// `IndexerSettings`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct IndexerSettings {
    pub output: IndexerOutput,
    pub skip_hidden_folders: bool,
    pub skip_hidden_files: bool,
    pub skip_files: bool,
    pub max_depth_level: i64,
    pub show_size_info: bool,
    pub add_footer: bool,
    pub indentation_text: String,
    pub add_empty_line_after_folders: bool,
    #[serde(rename = "UseCustomCSSFile")]
    pub use_custom_css_file: bool,
    pub display_path: bool,
    pub display_path_limited: bool,
    #[serde(rename = "CustomCSSFilePath")]
    pub custom_css_file_path: String,
    pub use_attribute: bool,
    pub create_parseable_json: bool,
    /// `[JsonIgnore] BinaryUnits`: copied from the application setting before indexing.
    #[serde(skip)]
    pub binary_units: bool,
}

impl Default for IndexerSettings {
    fn default() -> Self {
        IndexerSettings {
            output: IndexerOutput::Html,
            skip_hidden_folders: true,
            skip_hidden_files: true,
            skip_files: false,
            max_depth_level: 0,
            show_size_info: true,
            add_footer: true,
            indentation_text: "|___".into(),
            add_empty_line_after_folders: false,
            use_custom_css_file: false,
            display_path: false,
            display_path_limited: false,
            custom_css_file_path: String::new(),
            use_attribute: true,
            create_parseable_json: true,
            binary_units: false,
        }
    }
}

named_enum! {
    /// `ImageCombinerOrientation`.
    ImageCombinerOrientation { Horizontal = "Horizontal", Vertical = "Vertical" } default Vertical
}

impl ImageCombinerOrientation {
    fn from_number(n: u64) -> Option<Self> {
        Self::ALL.get(n as usize).copied()
    }
}

named_enum! {
    /// `ImageCombinerAlignment`.
    ImageCombinerAlignment { LeftOrTop = "LeftOrTop", Center = "Center", RightOrBottom = "RightOrBottom" } default LeftOrTop
}

impl ImageCombinerAlignment {
    fn from_number(n: u64) -> Option<Self> {
        Self::ALL.get(n as usize).copied()
    }
}

/// `ImageCombinerOptions`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ImageCombinerOptions {
    pub orientation: ImageCombinerOrientation,
    pub alignment: ImageCombinerAlignment,
    pub space: i64,
    pub wrap_after: i64,
    pub auto_fill_background: bool,
}

impl Default for ImageCombinerOptions {
    fn default() -> Self {
        ImageCombinerOptions {
            orientation: ImageCombinerOrientation::Vertical,
            alignment: ImageCombinerAlignment::LeftOrTop,
            space: 0,
            wrap_after: 0,
            auto_fill_background: true,
        }
    }
}

named_enum! {
    /// `ConverterVideoCodecs`.
    ConverterVideoCodecs {
        X264 = "x264", X265 = "x265", H264Nvenc = "h264_nvenc", HevcNvenc = "hevc_nvenc",
        H264Amf = "h264_amf", HevcAmf = "hevc_amf", H264Qsv = "h264_qsv", HevcQsv = "hevc_qsv",
        Vp8 = "vp8", Vp9 = "vp9", Av1 = "av1", Xvid = "xvid", Gif = "gif", Webp = "webp", Apng = "apng"
    } default X264
}

impl ConverterVideoCodecs {
    fn from_number(n: u64) -> Option<Self> {
        Self::ALL.get(n as usize).copied()
    }

    /// The `[Description]`.
    pub fn description(self) -> &'static str {
        use ConverterVideoCodecs::*;
        match self {
            X264 => "H.264 / x264",
            X265 => "H.265 / x265",
            H264Nvenc => "H.264 / NVENC",
            HevcNvenc => "HEVC / NVENC",
            H264Amf => "H.264 / AMF",
            HevcAmf => "HEVC / AMF",
            H264Qsv => "H.264 / Quick Sync",
            HevcQsv => "HEVC / Quick Sync",
            Vp8 => "VP8",
            Vp9 => "VP9",
            Av1 => "AV1",
            Xvid => "MPEG-4 / Xvid",
            Gif => "GIF",
            Webp => "WebP",
            Apng => "APNG",
        }
    }
}

/// `VideoConverterOptions` (the members the settings file carries).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct VideoConverterOptions {
    pub input_file_path: Option<String>,
    pub output_folder_path: Option<String>,
    pub output_file_name: Option<String>,
    pub video_codec: ConverterVideoCodecs,
    pub video_quality: i64,
    pub video_quality_use_bitrate: bool,
    pub video_quality_bitrate: i64,
    pub use_custom_arguments: bool,
    pub custom_arguments: String,
    pub auto_open_folder: bool,
}

impl Default for VideoConverterOptions {
    fn default() -> Self {
        VideoConverterOptions {
            input_file_path: None,
            output_folder_path: None,
            output_file_name: None,
            video_codec: ConverterVideoCodecs::X264,
            video_quality: 23,
            video_quality_use_bitrate: false,
            video_quality_bitrate: 3000,
            use_custom_arguments: false,
            custom_arguments: String::new(),
            auto_open_folder: true,
        }
    }
}

/// `BorderlessWindowSettings`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct BorderlessWindowSettings {
    pub exclude_taskbar_area: bool,
}

named_enum! {
    /// `AIProvider`.
    AiProvider { OpenAI = "OpenAI", Gemini = "Gemini", OpenRouter = "OpenRouter", OpenAILegacy = "OpenAILegacy" } default OpenAI
}

impl AiProvider {
    fn from_number(n: u64) -> Option<Self> {
        Self::ALL.get(n as usize).copied()
    }
}

/// `AIOptions`. The three keys are `[JsonEncrypt]` upstream (DPAPI); here they live in
/// the Keychain and the settings file keeps them empty (see `analyze::keys`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct AiOptions {
    pub provider: AiProvider,
    #[serde(skip)]
    pub openai_api_key: Option<String>,
    #[serde(rename = "OpenAIModel")]
    pub openai_model: String,
    #[serde(rename = "OpenAICustomURL")]
    pub openai_custom_url: Option<String>,
    #[serde(rename = "OpenAIReasoningEffort")]
    pub openai_reasoning_effort: String,
    #[serde(rename = "OpenAIVerbosity")]
    pub openai_verbosity: String,
    #[serde(skip)]
    pub gemini_api_key: Option<String>,
    pub gemini_model: String,
    #[serde(skip)]
    pub open_router_api_key: Option<String>,
    pub open_router_model: String,
    pub input: String,
    pub auto_start_region: bool,
    pub auto_start_analyze: bool,
    pub auto_copy_result: bool,
}

impl Default for AiOptions {
    fn default() -> Self {
        AiOptions {
            provider: AiProvider::OpenAI,
            openai_api_key: None,
            openai_model: "gpt-5-mini".into(),
            openai_custom_url: None,
            openai_reasoning_effort: "minimal".into(),
            openai_verbosity: "medium".into(),
            gemini_api_key: None,
            gemini_model: "gemini-1.5-flash-latest".into(),
            open_router_api_key: None,
            open_router_model: "google/gemini-flash-1.5".into(),
            input: modlight_core::l10n::t(
                modlight_core::l10n::tools::AIOptions_What_is_in_this_image,
            )
            .to_string(),
            auto_start_region: true,
            auto_start_analyze: true,
            auto_copy_result: false,
        }
    }
}

impl AiOptions {
    /// `HasAPIKey`.
    pub fn has_api_key(&self) -> bool {
        let has = |k: &Option<String>| k.as_deref().is_some_and(|k| !k.trim().is_empty());
        match self.provider {
            AiProvider::OpenAI | AiProvider::OpenAILegacy => has(&self.openai_api_key),
            AiProvider::Gemini => has(&self.gemini_api_key),
            AiProvider::OpenRouter => has(&self.open_router_api_key),
        }
    }
}

named_enum! {
    /// `BackgroundRemovalDevice`.
    BackgroundRemovalDevice { Auto = "Auto", Gpu = "GPU", Cpu = "CPU" } default Auto
}

impl BackgroundRemovalDevice {
    fn from_number(n: u64) -> Option<Self> {
        Self::ALL.get(n as usize).copied()
    }
}

/// `BackgroundRemoverOptions`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct BackgroundRemoverOptions {
    pub selected_model_file_name: String,
    pub selected_device: BackgroundRemovalDevice,
}

/// `OCRServiceLinkOption`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct OcrServiceLink {
    pub name: String,
    #[serde(rename = "URL")]
    pub url: String,
}

/// `OCROptions`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct OcrOptions {
    pub language: String,
    pub scale_factor: f64,
    pub single_line: bool,
    pub silent: bool,
    pub auto_copy: bool,
    pub service_links: Vec<OcrServiceLink>,
    pub close_window_after_opening_service_link: bool,
    pub selected_service_link: i64,
}

impl Default for OcrOptions {
    fn default() -> Self {
        OcrOptions {
            language: "en".into(),
            scale_factor: 2.0,
            single_line: false,
            silent: false,
            auto_copy: false,
            service_links: OcrOptions::default_service_links(),
            close_window_after_opening_service_link: false,
            selected_service_link: 0,
        }
    }
}

impl OcrOptions {
    /// `OCROptions.DefaultServiceLinks`.
    pub fn default_service_links() -> Vec<OcrServiceLink> {
        use modlight_core::l10n::{t, tools};
        let link = |name, url: &str| OcrServiceLink {
            name: t(name).to_string(),
            url: url.to_string(),
        };
        vec![
            link(
                tools::OCROptions_Google_Translate,
                "https://translate.google.com/?sl=auto&tl=en&text={0}&op=translate",
            ),
            link(
                tools::OCROptions_Google_Search,
                "https://www.google.com/search?q={0}",
            ),
            link(
                tools::OCROptions_Google_Images,
                "https://www.google.com/search?q={0}&tbm=isch",
            ),
            link(tools::OCROptions_Bing, "https://www.bing.com/search?q={0}"),
            link(
                tools::OCROptions_DuckDuckGo,
                "https://duckduckgo.com/?q={0}",
            ),
            link(
                tools::OCROptions_DeepL,
                "https://www.deepl.com/translator#auto/en/{0}",
            ),
        ]
    }

    /// `Helpers.IsDefaultSettings(ServiceLinks, DefaultServiceLinks, (x, y) => x.Name == y.Name)`
    /// in any language: links still named like the defaults are refreshed to this
    /// language's defaults before the window opens.
    pub fn refresh_default_links(&mut self) {
        use modlight_core::l10n::{Language, text_in, tools};
        let keys = [
            tools::OCROptions_Google_Translate,
            tools::OCROptions_Google_Search,
            tools::OCROptions_Google_Images,
            tools::OCROptions_Bing,
            tools::OCROptions_DuckDuckGo,
            tools::OCROptions_DeepL,
        ];
        let defaults = |language| -> Vec<&'static str> {
            keys.iter().map(|k| text_in(*k, language)).collect()
        };
        let names: Vec<&str> = self.service_links.iter().map(|l| l.name.as_str()).collect();
        if names == defaults(Language::English) || names == defaults(Language::Polish) {
            self.service_links = OcrOptions::default_service_links();
        }
    }
}

/// `WindowState` (`ShareX.HelpersLib`): where a window was.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct WindowState {
    pub location: Option<String>,
    pub size: Option<String>,
    pub is_maximized: bool,
}

/// `HistorySettings`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct HistorySettings {
    pub remember_window_state: bool,
    pub window_state: WindowState,
    pub splitter_distance: i64,
    pub remember_search_text: bool,
    pub search_text: String,
    pub favorites: bool,
    pub history_list_width: f64,
}

impl Default for HistorySettings {
    fn default() -> Self {
        HistorySettings {
            remember_window_state: true,
            window_state: WindowState::default(),
            splitter_distance: 550,
            remember_search_text: false,
            search_text: String::new(),
            favorites: false,
            history_list_width: 720.0,
        }
    }
}

named_enum! {
    /// `ImageHistoryThumbnailHeaderPosition`.
    ThumbnailHeaderPosition { None = "None", Top = "Top", Bottom = "Bottom" } default Top
}

impl ThumbnailHeaderPosition {
    fn from_number(n: u64) -> Option<Self> {
        Self::ALL.get(n as usize).copied()
    }
}

/// `ImageHistorySettings`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ImageHistorySettings {
    pub remember_window_state: bool,
    pub window_state: WindowState,
    pub thumbnail_size: Size,
    pub thumbnail_header_position: ThumbnailHeaderPosition,
    pub max_item_count: i64,
    pub auto_load_more_items: bool,
    pub filter_missing_files: bool,
    pub image_only: bool,
    pub remember_search_text: bool,
    pub search_text: String,
    pub favorites: bool,
}

impl Default for ImageHistorySettings {
    fn default() -> Self {
        ImageHistorySettings {
            remember_window_state: true,
            window_state: WindowState::default(),
            thumbnail_size: Size {
                width: 250,
                height: 150,
            },
            thumbnail_header_position: ThumbnailHeaderPosition::Top,
            max_item_count: 500,
            auto_load_more_items: true,
            filter_missing_files: false,
            image_only: true,
            remember_search_text: false,
            search_text: String::new(),
            favorites: false,
        }
    }
}

// ------------------------------------------------------------------ reading and writing

/// Read `T` from a settings node; anything missing or unreadable is upstream's default.
pub fn read<T: DeserializeOwned + Default>(value: Option<&Value>) -> T {
    value
        .filter(|v| !v.is_null())
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

/// The node at `path` under `root`, created as objects on the way.
pub fn node_mut<'a>(root: &'a mut Map<String, Value>, path: &[&str]) -> &'a mut Map<String, Value> {
    let mut current = root;
    for member in path {
        let entry = current
            .entry((*member).to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        current = entry.as_object_mut().expect("an object");
    }
    current
}

/// The node at `path` under `root`, if it is there.
pub fn node<'a>(root: &'a Map<String, Value>, path: &[&str]) -> Option<&'a Value> {
    let (last, parents) = path.split_last()?;
    let mut current = root;
    for member in parents {
        current = current.get(*member)?.as_object()?;
    }
    current.get(*last)
}

/// Write `value` over the object at `path`: its members replace the ones there, the
/// others stay.
pub fn write<T: Serialize>(root: &mut Map<String, Value>, path: &[&str], value: &T) {
    let Ok(Value::Object(members)) = serde_json::to_value(value) else {
        return;
    };
    let target = node_mut(root, path);
    for (key, member) in members {
        target.insert(key, member);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_are_upstreams() {
        let pin: PinToScreenOptions = read(None);
        assert_eq!(pin.initial_scale, 100);
        assert_eq!(pin.border_color, Color::CORNFLOWER_BLUE);
        assert_eq!(
            pin.minimize_size,
            Size {
                width: 100,
                height: 100
            }
        );
        let indexer: IndexerSettings = read(Some(&json!({})));
        assert_eq!(indexer.indentation_text, "|___");
        assert_eq!(indexer.output, IndexerOutput::Html);
        let video: VideoConverterOptions = read(Some(&Value::Null));
        assert_eq!(video.video_quality, 23);
        assert!(video.auto_open_folder);
    }

    #[test]
    fn enums_read_by_name_or_number() {
        let pin: PinToScreenOptions = read(Some(&json!({"Placement": 1, "BorderColor": "Red"})));
        assert_eq!(pin.placement, ContentAlignment::TopLeft);
        assert_eq!(pin.border_color, Color::RED);
        let pin: PinToScreenOptions = read(Some(&json!({"Placement": "MiddleCenter"})));
        assert_eq!(pin.placement, ContentAlignment::MiddleCenter);
        let video: VideoConverterOptions = read(Some(&json!({"VideoCodec": "hevc_nvenc"})));
        assert_eq!(video.video_codec, ConverterVideoCodecs::HevcNvenc);
        let video: VideoConverterOptions = read(Some(&json!({"VideoCodec": 9})));
        assert_eq!(video.video_codec, ConverterVideoCodecs::Vp9);
        assert_eq!(
            serde_json::to_value(ConverterVideoCodecs::H264Qsv).unwrap(),
            json!("h264_qsv")
        );
    }

    #[test]
    fn writing_keeps_what_is_not_modelled() {
        let mut root = json!({"DefaultTaskSettings": {"ToolsSettings": {"PinToScreenOptions": {"Future": 1, "InitialScale": 50}}}})
            .as_object()
            .unwrap()
            .clone();
        let path = ["DefaultTaskSettings", "ToolsSettings", "PinToScreenOptions"];
        let mut pin: PinToScreenOptions = read(node(&root, &path));
        assert_eq!(pin.initial_scale, 50);
        pin.initial_scale = 75;
        write(&mut root, &path, &pin);
        let written = node(&root, &path).unwrap();
        assert_eq!(written["Future"], 1);
        assert_eq!(written["InitialScale"], 75);
        assert_eq!(written["BorderColor"], "100, 149, 237");
        assert_eq!(written["MinimizeSize"], "100, 100");
    }

    #[test]
    fn ai_keys_are_never_written() {
        let options = AiOptions {
            openai_api_key: Some("sk-secret".into()),
            ..AiOptions::default()
        };
        let text = serde_json::to_string(&options).unwrap();
        assert!(!text.contains("sk-secret"));
        assert!(!text.contains("APIKey"));
        // Keys in an imported settings file (DPAPI blobs upstream) are not read either.
        let read: AiOptions =
            serde_json::from_str(r#"{"OpenAIAPIKey":"AQAAANCMnd8B","Provider":"Gemini"}"#).unwrap();
        assert_eq!(read.openai_api_key, None);
        assert_eq!(read.provider, AiProvider::Gemini);
    }
}
