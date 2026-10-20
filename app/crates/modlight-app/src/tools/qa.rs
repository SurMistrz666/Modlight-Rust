//! QA fixtures for phase 5's tool windows: each screen opens one window with fixture
//! data and draws it headlessly for `--qa-shot` and the snapshot tests. Nothing here
//! reads the screen, the clipboard, a model or a network.

use std::sync::Arc;

use egui::{Ui, Vec2, vec2};
use modlight_core::capture::geometry::IRect;
use modlight_tools::color::Color;
use modlight_tools::settings::{ColorPickerOptions, ScreenColorPickerOptions};

use crate::state::AppState;

/// The screens of this module, in `qa::SCREENS` order.
pub const SCREENS: &[&str] = &[
    "color-picker",
    "color-picker-standard",
    "screen-color-picker",
    "qr-code",
    "qr-code-empty",
    "image-viewer",
    "pin-startup",
    "pin-startup-error",
    "pin-options",
    "pinned-image",
];

/// A picture to show: a sunset like the fixture library's, drawn here.
pub fn sunset(w: u32, h: u32) -> image::RgbaImage {
    let mut image = image::RgbaImage::new(w, h);
    let lerp = |a: f32, b: f32, t: f32| (a + (b - a) * t) as u8;
    for (x, y, p) in image.enumerate_pixels_mut() {
        let t = y as f32 / h as f32;
        let ridge = h as f32 * 0.6 + 30.0 * ((x as f32) / 50.0).sin();
        *p = if (y as f32) > ridge {
            image::Rgba([
                lerp(120.0, 70.0, t),
                lerp(80.0, 50.0, t),
                lerp(130.0, 90.0, t),
                255,
            ])
        } else {
            image::Rgba([
                lerp(240.0, 200.0, t),
                lerp(170.0, 120.0, t),
                lerp(140.0, 150.0, t),
                255,
            ])
        };
        let d = ((x as f32 - w as f32 * 0.3).powi(2) + (y as f32 - h as f32 * 0.25).powi(2)).sqrt();
        if d < h as f32 * 0.1 {
            *p = image::Rgba([250, 230, 170, 255]);
        }
    }
    image
}

/// A few recent colours.
fn recent() -> Vec<Color> {
    vec![
        Color::rgb(18, 140, 200),
        Color::rgb(255, 106, 0),
        Color::rgb(46, 204, 113),
        Color::rgb(155, 89, 182),
        Color::rgb(241, 196, 15),
        Color::rgb(52, 73, 94),
        Color::rgb(231, 76, 60),
        Color::WHITE,
        Color::BLACK,
    ]
}

/// Whether `screen` is one of these (drawn as a window of its own).
pub fn is_dialog(screen: &str) -> bool {
    SCREENS.contains(&screen)
}

/// Set the state up for `screen`; the size the window wants.
pub fn prepare(screen: &str, state: &mut AppState, ctx: &egui::Context) -> Option<Vec2> {
    let _ = ctx;
    if !is_dialog(screen) {
        return None;
    }
    use super::color_picker::{ColorPickerWindow, Purpose};
    match screen {
        "color-picker" | "color-picker-standard" => {
            let options = ColorPickerOptions {
                recent_colors_selected: screen == "color-picker",
            };
            state.tools.open_keyed(|key| {
                let mut window = ColorPickerWindow::new(
                    key,
                    Purpose::Tool,
                    Color::rgb(18, 140, 200),
                    options,
                    ScreenColorPickerOptions::default(),
                    recent(),
                );
                window.previous = Color::RED;
                window.has_previous = true;
                Box::new(window)
            });
            Some(vec2(super::color_picker::WIDTH, 526.0))
        }
        "screen-color-picker" => {
            let source = state.captures.source.clone();
            let screen_rect = IRect::new(0, 0, 800, 500);
            let image = source
                .capture_rect(source.screen_bounds(), false)
                .unwrap_or_else(|_| image::RgbaImage::new(800, 500));
            let mut screens = source.screens();
            screens.truncate(1);
            if let Some(first) = screens.first_mut() {
                first.bounds = screen_rect;
            }
            let desktop = super::screen_picker::Desktop {
                image: Arc::new(image),
                bounds: source.screen_bounds(),
                scale: 1.0,
                screens,
                pointer: (163, 123),
            };
            let options = ScreenColorPickerOptions {
                info_text: "#$HEX$nX: $x Y: $y".into(),
                ..ScreenColorPickerOptions::default()
            };
            state.tools.screen_picker = Some(super::screen_picker::ScreenPicker::new(
                super::screen_picker::PickTarget::ColorPicker(0),
                options,
                desktop,
            ));
            Some(vec2(800.0, 500.0))
        }
        "qr-code" | "qr-code-empty" => {
            let text =
                (screen == "qr-code").then(|| "https://files.example.com/alex/q8".to_string());
            state.tools.open_keyed(|key| {
                Box::new(super::qr::QrWindow::new(
                    key,
                    super::qr::QrOptions {
                        initial_text: text,
                        ..Default::default()
                    },
                ))
            });
            Some(vec2(940.0, 610.0))
        }
        "image-viewer" => {
            let folder = std::env::temp_dir().join("modlight-qa-image-viewer");
            let _ = std::fs::create_dir_all(&folder);
            for (name, w, h) in [
                ("aurora.png", 800, 500),
                ("mountain-sunset.png", 1200, 760),
                ("zebra.png", 600, 600),
            ] {
                let path = folder.join(name);
                if !path.is_file() {
                    let _ = modlight_tools::images::save(&sunset(w, h), &path);
                }
            }
            let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(1040.0, 700.0));
            let mut window = super::image_viewer::ImageViewerWindow::new_on(screen);
            window.load_file(&folder.join("mountain-sunset.png"));
            state.tools.open_keyed(|key| {
                window.key = key;
                Box::new(window)
            });
            Some(vec2(1040.0, 700.0))
        }
        "pin-startup" | "pin-startup-error" => {
            let settings = state.default_task_settings();
            let error = screen == "pin-startup-error";
            state.tools.open_keyed(|key| {
                let mut window = super::pin::PinStartupWindow::new(key, settings);
                if error {
                    window.error = modlight_core::l10n::t(
                        modlight_core::l10n::tools::PinToScreenStartupViewModel_Clipboard_no_image,
                    )
                    .to_string();
                }
                Box::new(window)
            });
            Some(vec2(560.0, 230.0))
        }
        "pin-options" => {
            state.open_pin_options(None);
            Some(vec2(760.0, 560.0))
        }
        "pinned-image" => {
            let settings = state.default_task_settings();
            state.pin_picture(Arc::new(sunset(420, 260)), &settings);
            let pin = state.tools.pins.last_mut().expect("pinned");
            pin.show_toolbar = true;
            Some(pin.size)
        }
        _ => None,
    }
}

/// Draw `screen`'s window; `Some(height)` for windows whose height follows the content.
pub fn draw(screen: &str, ui: &mut Ui, state: &mut AppState) -> Option<Option<f32>> {
    if !is_dialog(screen) {
        return None;
    }
    if screen == "pinned-image" {
        let colors = state.colors.clone();
        let now = state.time;
        if let Some(pin) = state.tools.pins.last_mut() {
            // The desktop behind a transparent window.
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, egui::Color32::from_rgb(58, 74, 120));
            let _ = pin.ui(ui, &colors, now);
        }
        return Some(None);
    }
    if screen == "screen-color-picker" {
        let colors = state.colors.clone();
        if let Some(picker) = state.tools.screen_picker.as_mut()
            && let Some(first) = picker.desktop.screens.first().cloned()
        {
            picker.ui(ui, &colors, &first);
        }
        return Some(None);
    }
    let mut windows = std::mem::take(&mut state.tools.windows);
    let mut wanted = None;
    if let Some(hosted) = windows.first_mut() {
        let _ = hosted.window.ui(ui, state);
        if hosted.window.spec().fit_height {
            wanted = hosted.window.content_height();
        }
    }
    windows.append(&mut state.tools.windows);
    state.tools.windows = windows;
    Some(wanted)
}
