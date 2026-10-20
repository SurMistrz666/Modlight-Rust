//! QA fixtures for phase 5's tool windows: each screen opens one window with fixture
//! data and draws it headlessly for `--qa-shot` and the snapshot tests. Nothing here
//! reads the screen, the clipboard, a model or a network.

use egui::{Ui, Vec2};

use crate::state::AppState;

/// The screens of this module, in `qa::SCREENS` order.
pub const SCREENS: &[&str] = &[];

/// Whether `screen` is one of these (drawn as a window of its own).
pub fn is_dialog(screen: &str) -> bool {
    SCREENS.contains(&screen)
}

/// Set the state up for `screen`; the size the window wants.
pub fn prepare(screen: &str, state: &mut AppState, ctx: &egui::Context) -> Option<Vec2> {
    let _ = (state, ctx);
    if !is_dialog(screen) {
        return None;
    }
    None
}

/// Draw `screen`'s window; `Some(height)` for windows whose height follows the content.
pub fn draw(screen: &str, ui: &mut Ui, state: &mut AppState) -> Option<Option<f32>> {
    if !is_dialog(screen) {
        return None;
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
