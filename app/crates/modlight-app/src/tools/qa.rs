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
];

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
        _ => None,
    }
}

/// Draw `screen`'s window; `Some(height)` for windows whose height follows the content.
pub fn draw(screen: &str, ui: &mut Ui, state: &mut AppState) -> Option<Option<f32>> {
    if !is_dialog(screen) {
        return None;
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
