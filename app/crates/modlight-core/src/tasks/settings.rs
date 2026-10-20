//! The task settings an upload runs with (`TaskSettings` and its sub-settings) and the
//! application settings the task flow reads (`ApplicationConfig`), read out of
//! `ApplicationConfig.json` with upstream's member names and defaults.
//!
//! Read-only views: the settings file stays a JSON object (see `config.rs`) so members
//! this build has no screen for survive. ShareX writes its config with a string enum
//! converter, so enums arrive as names and flag sets as `"A, B, C"`; numbers are
//! accepted too, because that is what Newtonsoft would accept.

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::AppConfig;
use crate::tasks::actions::ExternalProgram;
use crate::watch::WatchFolderSettings;

/// A member Newtonsoft may have written as `null` (`TaskSettings.Cleanup` nulls every
/// group a hotkey takes from the defaults): read as the default.
pub fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + serde::de::DeserializeOwned,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_null() {
        return Ok(T::default());
    }
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// A `[Flags]` enum as a set of bits, with the names upstream gives them.
pub trait FlagNames: Sized + Copy {
    const NAMES: &'static [(&'static str, u32)];
    fn from_bits(bits: u32) -> Self;
    fn bits(self) -> u32;

    fn parse_value(value: &Value) -> Option<Self> {
        match value {
            Value::Number(n) => n.as_u64().map(|n| Self::from_bits(n as u32)),
            Value::String(text) => {
                let mut bits = 0u32;
                for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                    if let Ok(number) = part.parse::<u32>() {
                        bits |= number;
                    } else if let Some((_, bit)) = Self::NAMES
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case(part))
                    {
                        bits |= bit;
                    }
                }
                Some(Self::from_bits(bits))
            }
            _ => None,
        }
    }

    fn names(self) -> String {
        let bits = self.bits();
        if bits == 0 {
            return "None".into();
        }
        Self::NAMES
            .iter()
            .filter(|(_, bit)| *bit != 0 && bits & bit == *bit)
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

macro_rules! flags {
    ($(#[$doc:meta])* $name:ident { $($(#[$fdoc:meta])* $flag:ident = $bit:expr),* $(,)? }) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        pub struct $name(pub u32);

        #[allow(non_upper_case_globals)]
        impl $name {
            pub const None: $name = $name(0);
            $($(#[$fdoc])* pub const $flag: $name = $name($bit);)*

            pub fn has(self, flag: $name) -> bool {
                flag.0 != 0 && self.0 & flag.0 == flag.0
            }

            pub fn with(self, flag: $name) -> $name {
                $name(self.0 | flag.0)
            }

            pub fn remove(self, flag: $name) -> $name {
                $name(self.0 & !flag.0)
            }
        }

        impl $crate::tasks::settings::FlagNames for $name {
            const NAMES: &'static [(&'static str, u32)] = &[$((stringify!($flag), $bit)),*];
            fn from_bits(bits: u32) -> Self {
                $name(bits)
            }
            fn bits(self) -> u32 {
                self.0
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = <::serde_json::Value as ::serde::Deserialize>::deserialize(deserializer)?;
                Ok(<$name as $crate::tasks::settings::FlagNames>::parse_value(&value).unwrap_or_default())
            }
        }

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.names())
            }
        }
    };
}

flags! {
    /// `AfterCaptureTasks`.
    AfterCaptureTasks {
        ShowQuickTaskMenu = 1,
        ShowAfterCaptureWindow = 1 << 1,
        BeautifyImage = 1 << 2,
        AddImageEffects = 1 << 3,
        AnnotateImage = 1 << 4,
        CopyImageToClipboard = 1 << 5,
        PinToScreen = 1 << 6,
        SendImageToPrinter = 1 << 7,
        SaveImageToFile = 1 << 8,
        SaveImageToFileWithDialog = 1 << 9,
        SaveThumbnailImageToFile = 1 << 10,
        PerformActions = 1 << 11,
        CopyFileToClipboard = 1 << 12,
        CopyFilePathToClipboard = 1 << 13,
        CopyFolderPathToClipboard = 1 << 14,
        ShowInExplorer = 1 << 15,
        AnalyzeImage = 1 << 16,
        ScanQRCode = 1 << 17,
        DoOCR = 1 << 18,
        ShowBeforeUploadWindow = 1 << 19,
        UploadImageToHost = 1 << 20,
        DeleteFile = 1 << 21,
    }
}

flags! {
    /// `AfterUploadTasks`.
    AfterUploadTasks {
        ShowAfterUploadWindow = 1,
        UseURLShortener = 1 << 1,
        ShareURL = 1 << 2,
        CopyURLToClipboard = 1 << 3,
        OpenURL = 1 << 4,
        ShowQRCode = 1 << 5,
    }
}

/// An enum ShareX writes by name. Unknown names read as the default.
macro_rules! named_enum {
    ($(#[$doc:meta])* $name:ident { $($(#[$vdoc:meta])* $variant:ident),+ $(,)? } default $default:ident) => {
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

            pub fn name(self) -> &'static str {
                match self { $($name::$variant => stringify!($variant)),+ }
            }

            pub fn parse(text: &str) -> Option<$name> {
                $name::ALL.iter().copied().find(|v| v.name().eq_ignore_ascii_case(text.trim()))
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = <::serde_json::Value as ::serde::Deserialize>::deserialize(deserializer)?;
                Ok(match &value {
                    ::serde_json::Value::String(text) => $name::parse(text).unwrap_or_default(),
                    ::serde_json::Value::Number(n) => n
                        .as_u64()
                        .and_then(|i| $name::ALL.get(i as usize).copied())
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
pub(crate) use flags;
pub(crate) use named_enum;

named_enum! {
    /// `ImageDestination`.
    ImageDestination { Modlight, FileUploader } default Modlight
}

named_enum! {
    /// `TextDestination`.
    TextDestination { Modlight, FileUploader } default Modlight
}

named_enum! {
    /// `FileDestination`.
    FileDestination { Modlight } default Modlight
}

named_enum! {
    /// `UrlShortenerType`.
    UrlShortenerType { Modlight } default Modlight
}

named_enum! {
    /// `URLSharingServices`.
    UrlSharingServices { GoogleImageSearch, BingVisualSearch } default GoogleImageSearch
}

named_enum! {
    /// `EImageFormat`.
    ImageFormat { PNG, JPEG, GIF, BMP, TIFF } default PNG
}

impl ImageFormat {
    /// The extension upstream's `[Description]` gives each format.
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::PNG => "png",
            ImageFormat::JPEG => "jpg",
            ImageFormat::GIF => "gif",
            ImageFormat::BMP => "bmp",
            ImageFormat::TIFF => "tif",
        }
    }
}

named_enum! {
    /// `PNGBitDepth`.
    PngBitDepth { Default, Automatic, Bit32, Bit24 } default Default
}

named_enum! {
    /// `GIFQuality`.
    GifQuality { Default, Bit8, Bit4, Grayscale } default Default
}

named_enum! {
    /// `FileExistAction`.
    FileExistAction { Ask, Overwrite, UniqueName, Cancel } default Ask
}

named_enum! {
    /// `ToastClickAction`.
    ToastClickAction {
        CloseNotification, AnnotateImage, CopyImageToClipboard, CopyFile, CopyFilePath,
        CopyUrl, OpenFile, OpenFolder, OpenUrl, Upload, PinToScreen, DeleteFile
    } default CloseNotification
}

named_enum! {
    /// `System.Drawing.ContentAlignment`: where the notification sits.
    ContentAlignment {
        TopLeft, TopCenter, TopRight, MiddleLeft, MiddleCenter, MiddleRight,
        BottomLeft, BottomCenter, BottomRight
    } default BottomRight
}

/// `System.Drawing.Size`, written by ShareX as `"400, 300"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub width: i64,
    pub height: i64,
}

impl<'de> Deserialize<'de> for Size {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let pair = match &value {
            Value::String(text) => text
                .split_once(',')
                .and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?))),
            Value::Object(map) => Some((
                map.get("Width").and_then(Value::as_i64).unwrap_or(0),
                map.get("Height").and_then(Value::as_i64).unwrap_or(0),
            )),
            _ => None,
        };
        pair.map(|(width, height)| Size { width, height })
            .ok_or_else(|| de::Error::custom("not a size"))
    }
}

impl Serialize for Size {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("{}, {}", self.width, self.height))
    }
}

/// `NotificationActionButton`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct NotificationActionButton {
    pub action: ToastClickAction,
    pub label: Option<String>,
    pub icon: Option<String>,
    pub dismiss_notification: bool,
}

impl Default for NotificationActionButton {
    fn default() -> Self {
        NotificationActionButton {
            action: ToastClickAction::CloseNotification,
            label: None,
            icon: None,
            dismiss_notification: true,
        }
    }
}

impl NotificationActionButton {
    pub fn new(action: ToastClickAction) -> Self {
        NotificationActionButton {
            action,
            ..Default::default()
        }
    }

    /// Copy, and copy the link (`CreateDefaultButtons`).
    pub fn defaults() -> Vec<NotificationActionButton> {
        vec![
            NotificationActionButton::new(ToastClickAction::CopyImageToClipboard),
            NotificationActionButton::new(ToastClickAction::CopyUrl),
        ]
    }
}

fn default_buttons() -> Vec<NotificationActionButton> {
    NotificationActionButton::defaults()
}

/// `TaskSettingsGeneral`: sounds and the notification.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct GeneralSettings {
    pub play_sound_after_capture: bool,
    pub play_sound_after_upload: bool,
    pub play_sound_after_action: bool,
    pub show_toast_notification_after_task_completed: bool,
    pub toast_window_duration: f64,
    pub toast_window_fade_duration: f64,
    pub toast_window_placement: ContentAlignment,
    pub toast_window_size: Size,
    pub toast_window_left_click_action: ToastClickAction,
    pub toast_window_right_click_action: ToastClickAction,
    pub toast_window_middle_click_action: ToastClickAction,
    pub toast_window_button_size: i64,
    #[serde(default = "default_buttons")]
    pub toast_window_buttons: Vec<NotificationActionButton>,
    pub toast_window_auto_hide: bool,
    pub disable_notifications_on_fullscreen: bool,
    pub use_custom_capture_sound: bool,
    pub custom_capture_sound_path: Option<String>,
    pub use_custom_task_completed_sound: bool,
    pub custom_task_completed_sound_path: Option<String>,
    pub use_custom_action_completed_sound: bool,
    pub custom_action_completed_sound_path: Option<String>,
    pub use_custom_error_sound: bool,
    pub custom_error_sound_path: Option<String>,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        GeneralSettings {
            play_sound_after_capture: true,
            play_sound_after_upload: true,
            play_sound_after_action: true,
            show_toast_notification_after_task_completed: true,
            toast_window_duration: 3.0,
            toast_window_fade_duration: 1.0,
            toast_window_placement: ContentAlignment::BottomRight,
            toast_window_size: Size {
                width: 400,
                height: 300,
            },
            toast_window_left_click_action: ToastClickAction::OpenUrl,
            toast_window_right_click_action: ToastClickAction::CloseNotification,
            toast_window_middle_click_action: ToastClickAction::AnnotateImage,
            toast_window_button_size: 40,
            toast_window_buttons: NotificationActionButton::defaults(),
            toast_window_auto_hide: true,
            disable_notifications_on_fullscreen: false,
            use_custom_capture_sound: false,
            custom_capture_sound_path: None,
            use_custom_task_completed_sound: false,
            custom_task_completed_sound_path: None,
            use_custom_action_completed_sound: false,
            custom_action_completed_sound_path: None,
            use_custom_error_sound: false,
            custom_error_sound_path: None,
        }
    }
}

/// `TaskSettingsImage`: the parts the upload path reads.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ImageSettings {
    pub image_format: ImageFormat,
    #[serde(rename = "ImagePNGBitDepth")]
    pub image_png_bit_depth: PngBitDepth,
    #[serde(rename = "ImageJPEGQuality")]
    pub image_jpeg_quality: i64,
    #[serde(rename = "ImageGIFQuality")]
    pub image_gif_quality: GifQuality,
    #[serde(rename = "ImageAutoUseJPEG")]
    pub image_auto_use_jpeg: bool,
    #[serde(rename = "ImageAutoUseJPEGSize")]
    pub image_auto_use_jpeg_size: i64,
    #[serde(rename = "ImageAutoJPEGQuality")]
    pub image_auto_jpeg_quality: bool,
    pub file_exist_action: FileExistAction,
    pub selected_image_effect_preset: i64,
    pub show_image_effects_window_after_capture: bool,
    pub image_effect_only_region_capture: bool,
    pub use_random_image_effect: bool,
    pub thumbnail_width: i64,
    pub thumbnail_height: i64,
    pub thumbnail_name: String,
    pub thumbnail_check_size: bool,
}

impl Default for ImageSettings {
    fn default() -> Self {
        ImageSettings {
            image_format: ImageFormat::PNG,
            image_png_bit_depth: PngBitDepth::Default,
            image_jpeg_quality: 90,
            image_gif_quality: GifQuality::Default,
            image_auto_use_jpeg: true,
            image_auto_use_jpeg_size: 2048,
            image_auto_jpeg_quality: false,
            file_exist_action: FileExistAction::Ask,
            selected_image_effect_preset: 0,
            show_image_effects_window_after_capture: false,
            image_effect_only_region_capture: false,
            use_random_image_effect: false,
            thumbnail_width: 200,
            thumbnail_height: 0,
            thumbnail_name: "-thumbnail".into(),
            thumbnail_check_size: false,
        }
    }
}

/// `TaskSettingsUpload`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct UploadSettings {
    pub modlight_folder_id: Option<String>,
    pub modlight_folder_name: Option<String>,
    pub use_custom_time_zone: bool,
    pub name_format_pattern: String,
    pub name_format_pattern_active_window: String,
    pub file_upload_use_name_pattern: bool,
    pub file_upload_replace_problematic_characters: bool,
    #[serde(rename = "URLRegexReplace")]
    pub url_regex_replace: bool,
    #[serde(rename = "URLRegexReplacePattern")]
    pub url_regex_replace_pattern: String,
    #[serde(rename = "URLRegexReplaceReplacement")]
    pub url_regex_replace_replacement: String,
    #[serde(rename = "ClipboardUploadURLContents")]
    pub clipboard_upload_url_contents: bool,
    #[serde(rename = "ClipboardUploadShortenURL")]
    pub clipboard_upload_shorten_url: bool,
    #[serde(rename = "ClipboardUploadShareURL")]
    pub clipboard_upload_share_url: bool,
    pub clipboard_upload_auto_index_folder: bool,
}

impl Default for UploadSettings {
    fn default() -> Self {
        UploadSettings {
            modlight_folder_id: None,
            modlight_folder_name: None,
            use_custom_time_zone: false,
            name_format_pattern: "%ra{10}".into(),
            name_format_pattern_active_window: "%pn_%ra{10}".into(),
            file_upload_use_name_pattern: false,
            file_upload_replace_problematic_characters: false,
            url_regex_replace: false,
            url_regex_replace_pattern: "^https?://(.+)$".into(),
            url_regex_replace_replacement: "https://$1".into(),
            clipboard_upload_url_contents: false,
            clipboard_upload_shorten_url: false,
            clipboard_upload_share_url: false,
            clipboard_upload_auto_index_folder: false,
        }
    }
}

/// `TaskSettingsAdvanced`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct AdvancedSettings {
    pub process_images_during_file_upload: bool,
    pub process_images_during_clipboard_upload: bool,
    pub use_after_capture_tasks_during_file_upload: bool,
    pub text_task_save_as_file: bool,
    pub auto_clear_clipboard: bool,
    pub region_capture_disable_annotation: bool,
    pub image_extensions: Option<Vec<String>>,
    pub text_extensions: Option<Vec<String>>,
    #[serde(rename = "EarlyCopyURL")]
    pub early_copy_url: bool,
    pub text_file_extension: String,
    pub text_format: String,
    pub text_custom: String,
    pub text_custom_encode_input: bool,
    #[serde(rename = "ResultForceHTTPS")]
    pub result_force_https: bool,
    pub clipboard_content_format: String,
    pub balloon_tip_content_format: String,
    #[serde(rename = "OpenURLFormat")]
    pub open_url_format: String,
    #[serde(rename = "AutoShortenURLLength")]
    pub auto_shorten_url_length: i64,
    pub auto_close_after_upload_form: bool,
    pub name_pattern_max_length: i64,
    pub name_pattern_max_title_length: i64,
}

impl Default for AdvancedSettings {
    fn default() -> Self {
        AdvancedSettings {
            process_images_during_file_upload: false,
            process_images_during_clipboard_upload: false,
            use_after_capture_tasks_during_file_upload: true,
            text_task_save_as_file: true,
            auto_clear_clipboard: false,
            region_capture_disable_annotation: false,
            image_extensions: None,
            text_extensions: None,
            early_copy_url: false,
            text_file_extension: "txt".into(),
            text_format: "text".into(),
            text_custom: String::new(),
            text_custom_encode_input: true,
            result_force_https: false,
            clipboard_content_format: "$result".into(),
            balloon_tip_content_format: "$result".into(),
            open_url_format: "$result".into(),
            auto_shorten_url_length: 0,
            auto_close_after_upload_form: false,
            name_pattern_max_length: 100,
            name_pattern_max_title_length: 50,
        }
    }
}

impl AdvancedSettings {
    pub fn image_extensions(&self) -> Vec<String> {
        self.image_extensions.clone().unwrap_or_else(|| {
            crate::names::file_helpers::IMAGE_FILE_EXTENSIONS
                .iter()
                .map(|e| e.to_string())
                .collect()
        })
    }

    pub fn text_extensions(&self) -> Vec<String> {
        self.text_extensions.clone().unwrap_or_else(|| {
            crate::names::file_helpers::TEXT_FILE_EXTENSIONS
                .iter()
                .map(|e| e.to_string())
                .collect()
        })
    }
}

/// `TaskSettings`: what one task runs with.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct TaskSettings {
    pub description: Option<String>,
    /// The hotkey's job (`HotkeyType`) when these are a hotkey's settings.
    pub job: crate::hotkeys::HotkeyType,
    pub use_default_after_capture_job: bool,
    pub after_capture_job: AfterCaptureTasks,
    pub use_default_after_upload_job: bool,
    pub after_upload_job: AfterUploadTasks,
    pub use_default_destinations: bool,
    pub image_destination: ImageDestination,
    pub image_file_destination: FileDestination,
    pub text_destination: TextDestination,
    pub text_file_destination: FileDestination,
    pub file_destination: FileDestination,
    #[serde(rename = "URLShortenerDestination")]
    pub url_shortener_destination: UrlShortenerType,
    #[serde(rename = "URLSharingServiceDestination")]
    pub url_sharing_service_destination: UrlSharingServices,
    pub override_screenshots_folder: bool,
    pub screenshots_folder: Option<String>,
    pub use_default_general_settings: bool,
    #[serde(deserialize_with = "null_default")]
    pub general_settings: GeneralSettings,
    pub use_default_image_settings: bool,
    #[serde(deserialize_with = "null_default")]
    pub image_settings: ImageSettings,
    pub use_default_capture_settings: bool,
    #[serde(deserialize_with = "null_default")]
    pub capture_settings: crate::capture::options::CaptureSettings,
    pub use_default_upload_settings: bool,
    #[serde(deserialize_with = "null_default")]
    pub upload_settings: UploadSettings,
    pub use_default_actions: bool,
    #[serde(deserialize_with = "null_default")]
    pub external_programs: Vec<ExternalProgram>,
    pub use_default_tools_settings: bool,
    /// `ToolsSettings` as the file has it (phase 5's tools read their options from it
    /// through `modlight_tools::settings`).
    #[serde(rename = "ToolsSettings")]
    pub tools_settings: Value,
    pub use_default_advanced_settings: bool,
    #[serde(deserialize_with = "null_default")]
    pub advanced_settings: AdvancedSettings,
    pub watch_folder_enabled: bool,
    #[serde(deserialize_with = "null_default")]
    pub watch_folder_list: Vec<WatchFolderSettings>,
    /// Where THIS upload is going (a folder tile that was dropped on). Not a setting;
    /// never read from or written to the file.
    #[serde(skip)]
    pub modlight_folder_id: Option<String>,
    /// What the Shorten URL dialog asked for. Memory only: it can carry a password.
    #[serde(skip)]
    pub modlight_short_link: Option<modlight_api::ShortLinkOptions>,
}

impl Default for TaskSettings {
    fn default() -> Self {
        TaskSettings {
            description: None,
            job: crate::hotkeys::HotkeyType::None,
            use_default_after_capture_job: true,
            use_default_after_upload_job: true,
            use_default_destinations: true,
            use_default_general_settings: true,
            use_default_image_settings: true,
            use_default_capture_settings: true,
            use_default_upload_settings: true,
            use_default_actions: true,
            use_default_tools_settings: true,
            tools_settings: Value::Null,
            use_default_advanced_settings: true,
            capture_settings: crate::capture::options::CaptureSettings::default(),
            external_programs: Vec::new(),
            // UploadImageToHost is on, unlike ShareX: this is a client for a server that
            // holds the screenshots; the file on disk is what survives a refusal.
            after_capture_job: AfterCaptureTasks::CopyImageToClipboard
                .with(AfterCaptureTasks::SaveImageToFile)
                .with(AfterCaptureTasks::UploadImageToHost),
            after_upload_job: AfterUploadTasks::CopyURLToClipboard,
            image_destination: ImageDestination::Modlight,
            image_file_destination: FileDestination::Modlight,
            text_destination: TextDestination::Modlight,
            text_file_destination: FileDestination::Modlight,
            file_destination: FileDestination::Modlight,
            url_shortener_destination: UrlShortenerType::Modlight,
            url_sharing_service_destination: UrlSharingServices::GoogleImageSearch,
            override_screenshots_folder: false,
            screenshots_folder: None,
            general_settings: GeneralSettings::default(),
            image_settings: ImageSettings::default(),
            upload_settings: UploadSettings::default(),
            advanced_settings: AdvancedSettings::default(),
            watch_folder_enabled: false,
            watch_folder_list: Vec::new(),
            modlight_folder_id: None,
            modlight_short_link: None,
        }
    }
}

impl TaskSettings {
    /// `TaskSettings.GetDefaultTaskSettings()`: a copy of the configuration's defaults.
    pub fn defaults_from(config: &AppConfig) -> TaskSettings {
        config
            .get("DefaultTaskSettings")
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .unwrap_or_default()
    }

    /// `GetFileDestinationByDataType`.
    pub fn file_destination_for(&self, data_type: super::info::DataType) -> FileDestination {
        match data_type {
            super::info::DataType::Image => self.image_file_destination,
            super::info::DataType::Text => self.text_file_destination,
            _ => self.file_destination,
        }
    }
}

/// The `ApplicationConfig` members the task flow reads.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct AppUploadSettings {
    pub upload_limit: i64,
    pub buffer_size_power: i64,
    pub max_upload_fail_retry: i64,
    pub show_large_file_size_warning: i64,
    pub show_multi_upload_warning: bool,
    pub binary_units: bool,
    pub history_save_tasks: bool,
    #[serde(rename = "HistoryCheckURL")]
    pub history_check_url: bool,
    pub recent_tasks_save: bool,
    pub recent_tasks_max_count: i64,
    pub recent_tasks_show_in_main_window: bool,
    pub recent_tasks_show_in_tray_menu: bool,
    pub recent_tasks_tray_menu_most_recent_first: bool,
    pub show_clipboard_content_viewer: bool,
    pub disable_upload: bool,
    pub file_upload_default_directory: Option<String>,
    pub name_parser_auto_increment_number: i64,
    pub use_custom_screenshots_path: bool,
    pub custom_screenshots_path: Option<String>,
    pub custom_screenshots_path2: Option<String>,
    pub save_image_sub_folder_pattern: Option<String>,
    pub save_image_sub_folder_pattern_window: Option<String>,
    pub save_settings_after_task_completed: bool,
    pub tray_icon_progress_enabled: bool,
    pub drop_size: i64,
    pub drop_offset: i64,
    pub drop_alignment: ContentAlignment,
    pub drop_opacity: i64,
    pub drop_hover_opacity: i64,
}

impl Default for AppUploadSettings {
    fn default() -> Self {
        AppUploadSettings {
            upload_limit: 0,
            buffer_size_power: 5,
            max_upload_fail_retry: 1,
            show_large_file_size_warning: 100,
            show_multi_upload_warning: true,
            binary_units: false,
            history_save_tasks: true,
            history_check_url: false,
            recent_tasks_save: false,
            recent_tasks_max_count: 10,
            recent_tasks_show_in_main_window: true,
            recent_tasks_show_in_tray_menu: true,
            recent_tasks_tray_menu_most_recent_first: false,
            show_clipboard_content_viewer: true,
            disable_upload: false,
            file_upload_default_directory: None,
            name_parser_auto_increment_number: 0,
            use_custom_screenshots_path: false,
            custom_screenshots_path: None,
            custom_screenshots_path2: None,
            save_image_sub_folder_pattern: Some("%y-%mo".into()),
            save_image_sub_folder_pattern_window: None,
            save_settings_after_task_completed: false,
            tray_icon_progress_enabled: true,
            drop_size: 150,
            drop_offset: 5,
            drop_alignment: ContentAlignment::BottomRight,
            drop_opacity: 100,
            drop_hover_opacity: 255,
        }
    }
}

impl AppUploadSettings {
    pub fn from_config(config: &AppConfig) -> AppUploadSettings {
        serde_json::from_value(Value::Object(config.values().clone())).unwrap_or_default()
    }

    /// The upload buffer: 2^BufferSizePower KiB.
    pub fn buffer_size(&self) -> usize {
        1024usize << self.buffer_size_power.clamp(0, 14)
    }

    /// `ShowLargeFileSizeWarning` in bytes, with the units the person chose.
    pub fn large_file_bytes(&self) -> u64 {
        let size = self.show_large_file_size_warning.max(0) as u64;
        if self.binary_units {
            size * 1024 * 1024
        } else {
            size * 1000 * 1000
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flags_read_as_sharex_writes_them() {
        let job: AfterCaptureTasks = serde_json::from_value(json!(
            "CopyImageToClipboard, SaveImageToFile, UploadImageToHost"
        ))
        .unwrap();
        assert!(job.has(AfterCaptureTasks::SaveImageToFile));
        assert!(job.has(AfterCaptureTasks::UploadImageToHost));
        assert!(!job.has(AfterCaptureTasks::DeleteFile));
        assert_eq!(job, TaskSettings::default().after_capture_job);
        assert_eq!(
            serde_json::to_value(job).unwrap(),
            json!("CopyImageToClipboard, SaveImageToFile, UploadImageToHost")
        );
        let numeric: AfterUploadTasks = serde_json::from_value(json!(8)).unwrap();
        assert_eq!(numeric, AfterUploadTasks::CopyURLToClipboard);
        let none: AfterUploadTasks = serde_json::from_value(json!("None")).unwrap();
        assert_eq!(none, AfterUploadTasks::None);
        assert!(
            !none.has(AfterUploadTasks::None),
            "None is never a flag that is set"
        );
        assert_eq!(
            AfterCaptureTasks::SaveImageToFile
                .with(AfterCaptureTasks::DeleteFile)
                .remove(AfterCaptureTasks::DeleteFile),
            AfterCaptureTasks::SaveImageToFile
        );
    }

    #[test]
    fn missing_members_take_upstreams_defaults() {
        let mut config = AppConfig::in_memory();
        config.set(
            "DefaultTaskSettings",
            json!({
                "AfterUploadJob": "CopyURLToClipboard, OpenURL",
                "GeneralSettings": { "ToastWindowSize": "500, 200", "ToastWindowPlacement": "TopRight" },
                "ImageSettings": { "ImageFormat": "JPEG", "ImageJPEGQuality": 75 },
                "AdvancedSettings": { "ClipboardContentFormat": "<$url>" },
                "Unknown": 1
            }),
        );
        config.set("UploadLimit", json!(2));
        let settings = TaskSettings::defaults_from(&config);
        assert!(settings.after_upload_job.has(AfterUploadTasks::OpenURL));
        assert_eq!(settings.general_settings.toast_window_size.width, 500);
        assert_eq!(
            settings.general_settings.toast_window_placement,
            ContentAlignment::TopRight
        );
        assert_eq!(settings.general_settings.toast_window_duration, 3.0);
        assert_eq!(settings.image_settings.image_format, ImageFormat::JPEG);
        assert_eq!(settings.image_settings.image_jpeg_quality, 75);
        assert!(settings.image_settings.image_auto_use_jpeg);
        assert_eq!(
            settings.advanced_settings.clipboard_content_format,
            "<$url>"
        );
        assert!(settings.advanced_settings.text_task_save_as_file);
        assert_eq!(settings.upload_settings.name_format_pattern, "%ra{10}");
        assert_eq!(
            settings.general_settings.toast_window_buttons,
            NotificationActionButton::defaults()
        );
        let app = AppUploadSettings::from_config(&config);
        assert_eq!(app.upload_limit, 2);
        assert_eq!(app.max_upload_fail_retry, 1);
        assert_eq!(app.buffer_size(), 32 * 1024);
        assert_eq!(app.large_file_bytes(), 100_000_000);
    }

    #[test]
    fn a_broken_member_does_not_lose_the_rest() {
        let mut config = AppConfig::in_memory();
        config.set("DefaultTaskSettings", json!("not an object"));
        assert_eq!(
            TaskSettings::defaults_from(&config),
            TaskSettings::default()
        );
        let mut config = AppConfig::in_memory();
        config.set(
            "DefaultTaskSettings",
            json!({ "ImageSettings": { "ImageFormat": "WEBP" }, "AfterUploadJob": null }),
        );
        let settings = TaskSettings::defaults_from(&config);
        assert_eq!(settings.image_settings.image_format, ImageFormat::PNG);
    }
}
