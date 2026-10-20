//! The capture settings a task runs with (`TaskSettingsCapture`), the region capture's
//! own options (`RegionCaptureOptions`) and the scrolling capture's
//! (`ScrollingCaptureOptions`), read with upstream's member names and defaults.
//!
//! Like the rest of the task settings these are read-only views over the JSON settings
//! file; members this build has no use for stay in the file untouched.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::geometry::{IPoint, IRect};
use crate::recording::FFmpegOptions;
use crate::tasks::settings::{Size, named_enum};

named_enum! {
    /// `RegionCaptureAction`.
    RegionCaptureAction {
        None, CancelCapture, RemoveShapeCancelCapture, RemoveShape, SwapToolType,
        CaptureFullscreen, CaptureActiveMonitor, CaptureLastRegion
    } default None
}

impl RegionCaptureAction {
    pub fn text(self) -> String {
        crate::l10n::enum_text("RegionCaptureAction", self.name(), None)
    }
}

named_enum! {
    /// `ScrollMethod`.
    ScrollMethod { MouseWheel, DownArrow, PageDown, ScrollMessage } default MouseWheel
}

/// `SnapSize`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct SnapSize {
    pub width: i64,
    pub height: i64,
}

fn default_snap_sizes() -> Vec<SnapSize> {
    [
        (426, 240),
        (640, 360),
        (854, 480),
        (1280, 720),
        (1920, 1080),
    ]
    .into_iter()
    .map(|(width, height)| SnapSize { width, height })
    .collect()
}

/// `RegionCaptureOptions`: the members the region capture and its settings page use.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct RegionCaptureOptions {
    pub quick_crop: bool,
    pub minimum_size: i64,
    pub region_capture_action_right_click: RegionCaptureAction,
    pub region_capture_action_middle_click: RegionCaptureAction,
    #[serde(rename = "RegionCaptureActionX1Click")]
    pub region_capture_action_x1_click: RegionCaptureAction,
    #[serde(rename = "RegionCaptureActionX2Click")]
    pub region_capture_action_x2_click: RegionCaptureAction,
    pub detect_windows: bool,
    pub detect_controls: bool,
    pub use_dimming: bool,
    pub background_dim_strength: i64,
    pub use_custom_info_text: bool,
    pub custom_info_text: String,
    #[serde(default = "default_snap_sizes")]
    pub snap_sizes: Vec<SnapSize>,
    pub show_info: bool,
    pub show_magnifier: bool,
    pub use_square_magnifier: bool,
    pub magnifier_pixel_count: i64,
    pub magnifier_pixel_size: i64,
    pub show_center_crosshair: bool,
    pub show_crosshair: bool,
    pub use_light_resize_nodes: bool,
    pub enable_animations: bool,
    pub is_fixed_size: bool,
    pub fixed_size: Size,
    #[serde(rename = "ShowFPS")]
    pub show_fps: bool,
    #[serde(rename = "FPSLimit")]
    pub fps_limit: i64,
    pub menu_icon_size: i64,
    pub menu_locked: bool,
    pub remember_menu_state: bool,
    pub menu_collapsed: bool,
    pub menu_position: IPoint,
    pub input_delay: i64,
    pub switch_to_drawing_tool_after_selection: bool,
    pub switch_to_selection_tool_after_drawing: bool,
    pub active_monitor_mode: bool,
    /// `ColorPickerOptions`, as the file has it (the colour picker tool reads it).
    pub color_picker_options: Value,
}

impl Default for RegionCaptureOptions {
    fn default() -> Self {
        RegionCaptureOptions {
            quick_crop: true,
            minimum_size: Self::DEFAULT_MINIMUM_SIZE,
            region_capture_action_right_click: RegionCaptureAction::RemoveShapeCancelCapture,
            region_capture_action_middle_click: RegionCaptureAction::SwapToolType,
            region_capture_action_x1_click: RegionCaptureAction::CaptureFullscreen,
            region_capture_action_x2_click: RegionCaptureAction::CaptureActiveMonitor,
            detect_windows: true,
            detect_controls: true,
            use_dimming: true,
            background_dim_strength: 20,
            use_custom_info_text: false,
            custom_info_text: "X: $x, Y: $y$nR: $r, G: $g, B: $b$nHex: $hex".into(),
            snap_sizes: default_snap_sizes(),
            show_info: true,
            show_magnifier: true,
            use_square_magnifier: false,
            magnifier_pixel_count: 15,
            magnifier_pixel_size: 10,
            show_center_crosshair: false,
            show_crosshair: false,
            use_light_resize_nodes: false,
            enable_animations: true,
            is_fixed_size: false,
            fixed_size: Size {
                width: 250,
                height: 250,
            },
            show_fps: false,
            fps_limit: 100,
            menu_icon_size: 0,
            menu_locked: false,
            remember_menu_state: false,
            menu_collapsed: false,
            menu_position: IPoint::default(),
            input_delay: 500,
            switch_to_drawing_tool_after_selection: false,
            switch_to_selection_tool_after_drawing: false,
            active_monitor_mode: false,
            color_picker_options: Value::Null,
        }
    }
}

impl RegionCaptureOptions {
    pub const DEFAULT_MINIMUM_SIZE: i64 = 5;
    pub const MAGNIFIER_PIXEL_COUNT_MINIMUM: i64 = 3;
    pub const MAGNIFIER_PIXEL_COUNT_MAXIMUM: i64 = 35;
    pub const MAGNIFIER_PIXEL_SIZE_MINIMUM: i64 = 3;
    pub const MAGNIFIER_PIXEL_SIZE_MAXIMUM: i64 = 30;
    pub const SNAP_DISTANCE: i64 = 30;
    pub const MOVE_SPEED_MINIMUM: f64 = 1.0;
    pub const MOVE_SPEED_MAXIMUM: f64 = 10.0;

    /// `RegionCaptureWindow.GetDimAlpha`.
    pub fn dim_alpha(&self) -> u8 {
        if !self.use_dimming || self.background_dim_strength <= 0 {
            return 0;
        }
        ((self.background_dim_strength as f64 / 100.0 * 255.0).round() as i64).clamp(0, 255) as u8
    }
}

/// `ScrollingCaptureOptions`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ScrollingCaptureOptions {
    pub start_delay: i64,
    pub auto_scroll_top: bool,
    pub scroll_delay: i64,
    pub scroll_method: ScrollMethod,
    pub scroll_amount: i64,
    pub auto_ignore_bottom_edge: bool,
    pub auto_upload: bool,
    pub show_region: bool,
}

impl Default for ScrollingCaptureOptions {
    fn default() -> Self {
        ScrollingCaptureOptions {
            start_delay: 300,
            auto_scroll_top: false,
            scroll_delay: 300,
            scroll_method: ScrollMethod::MouseWheel,
            scroll_amount: 2,
            auto_ignore_bottom_edge: true,
            auto_upload: false,
            show_region: true,
        }
    }
}

/// `TaskSettingsCapture`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct CaptureSettings {
    pub show_cursor: bool,
    /// Seconds (a `decimal` upstream).
    pub screenshot_delay: f64,
    pub capture_transparent: bool,
    pub capture_shadow: bool,
    pub capture_shadow_offset: i64,
    pub capture_client_area: bool,
    pub capture_auto_hide_taskbar: bool,
    pub capture_auto_hide_desktop_icons: bool,
    #[serde(rename = "HDRScreenshotColorCorrection")]
    pub hdr_screenshot_color_correction: bool,
    pub capture_custom_region: IRect,
    pub capture_custom_window: String,
    pub surface_options: RegionCaptureOptions,
    #[serde(rename = "FFmpegOptions")]
    pub ffmpeg_options: FFmpegOptions,
    #[serde(rename = "ScreenRecordFPS")]
    pub screen_record_fps: i64,
    #[serde(rename = "GIFFPS")]
    pub gif_fps: i64,
    pub screen_record_show_cursor: bool,
    pub screen_record_show_timer: bool,
    pub screen_record_show_button_labels: bool,
    pub screen_record_auto_start: bool,
    pub screen_record_start_delay: f64,
    pub screen_record_fixed_duration: bool,
    pub screen_record_duration: f64,
    pub screen_record_two_pass_encoding: bool,
    pub screen_record_ask_confirmation_on_abort: bool,
    pub screen_record_transparent_region: bool,
    pub scrolling_capture_options: ScrollingCaptureOptions,
    /// `OCROptions`: kept as the file has it until the OCR tool arrives (phase 5).
    #[serde(rename = "OCROptions")]
    pub ocr_options: Value,
}

impl Default for CaptureSettings {
    fn default() -> Self {
        CaptureSettings {
            show_cursor: true,
            screenshot_delay: 0.0,
            capture_transparent: false,
            capture_shadow: true,
            capture_shadow_offset: 100,
            capture_client_area: false,
            capture_auto_hide_taskbar: false,
            capture_auto_hide_desktop_icons: false,
            hdr_screenshot_color_correction: false,
            capture_custom_region: IRect::default(),
            capture_custom_window: String::new(),
            surface_options: RegionCaptureOptions::default(),
            ffmpeg_options: FFmpegOptions::default(),
            screen_record_fps: 30,
            gif_fps: 15,
            screen_record_show_cursor: true,
            screen_record_show_timer: true,
            screen_record_show_button_labels: true,
            screen_record_auto_start: true,
            screen_record_start_delay: 0.0,
            screen_record_fixed_duration: false,
            screen_record_duration: 3.0,
            screen_record_two_pass_encoding: false,
            screen_record_ask_confirmation_on_abort: false,
            screen_record_transparent_region: false,
            scrolling_capture_options: ScrollingCaptureOptions::default(),
            ocr_options: Value::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn region_options_take_upstreams_defaults() {
        let options: RegionCaptureOptions = serde_json::from_value(json!({
            "ShowMagnifier": false,
            "RegionCaptureActionRightClick": "CancelCapture",
            "RegionCaptureActionX1Click": "CaptureLastRegion",
            "FixedSize": "640, 480",
            "BackgroundDimStrength": 50
        }))
        .unwrap();
        assert!(!options.show_magnifier);
        assert!(options.show_info);
        assert_eq!(
            options.region_capture_action_right_click,
            RegionCaptureAction::CancelCapture
        );
        assert_eq!(
            options.region_capture_action_x1_click,
            RegionCaptureAction::CaptureLastRegion
        );
        assert_eq!(
            options.region_capture_action_middle_click,
            RegionCaptureAction::SwapToolType
        );
        assert_eq!(options.fixed_size.width, 640);
        assert_eq!(options.magnifier_pixel_count, 15);
        assert_eq!(options.input_delay, 500);
        assert_eq!(options.snap_sizes.len(), 5);
        assert_eq!(options.dim_alpha(), 128);
        assert_eq!(RegionCaptureOptions::default().dim_alpha(), 51);
    }

    #[test]
    fn capture_settings_read_from_a_sharex_file() {
        let settings: CaptureSettings = serde_json::from_value(json!({
            "ShowCursor": false,
            "ScreenshotDelay": 1.5,
            "CaptureCustomRegion": "10, 20, 300, 200",
            "SurfaceOptions": { "QuickCrop": false },
            "GIFFPS": 20,
            "OCROptions": { "Language": "pl" }
        }))
        .unwrap();
        assert!(!settings.show_cursor);
        assert_eq!(settings.screenshot_delay, 1.5);
        assert_eq!(settings.capture_custom_region, IRect::new(10, 20, 300, 200));
        assert!(!settings.surface_options.quick_crop);
        assert_eq!(settings.gif_fps, 20);
        assert_eq!(settings.screen_record_fps, 30);
        assert!(settings.capture_shadow);
        assert_eq!(settings.ocr_options["Language"], "pl");
    }
}
