//! QA screens of phase 4: the image editor's window drawn alone, through the same
//! session code the real window uses (`editor_window`), with the editor's own fixtures.

use egui::{Ui, Vec2, vec2};

use crate::editor_window::{EditorSession, Finish};
use crate::state::AppState;

pub const SCREENS: &[&str] = &[
    "image-editor",
    "image-editor-start",
    "image-editor-annotate",
    // The editor with the effect library registered (phase 5, image effects).
    "image-editor-effects",
    "image-editor-effect-dialog",
];

pub fn is_dialog(screen: &str) -> bool {
    SCREENS.contains(&screen)
}

/// Opens the editor the screen shows. Returns the window size.
pub fn prepare(screen: &str, state: &mut AppState) -> Option<Vec2> {
    if !SCREENS.contains(&screen) {
        return None;
    }
    match screen {
        "image-editor" => {
            let scene = modlight_editor::qa::scene("editor-select")?;
            state.open_editor(None, None, false, false, Finish::RunImageTask);
            if let Some(session) = state.windows.editors.sessions.last_mut() {
                adopt(session, scene);
            }
        }
        "image-editor-start" => {
            state.open_image_editor_window();
        }
        "image-editor-effects" | "image-editor-effect-dialog" => {
            let mut scene = modlight_editor::qa::scene("editor-select")?;
            if screen == "image-editor-effects" {
                scene.editor.toggle_effects_panel();
            } else {
                scene.editor.run_effect("vignette");
            }
            state.open_editor(None, None, false, false, Finish::RunImageTask);
            if let Some(session) = state.windows.editors.sessions.last_mut() {
                adopt(session, scene);
            }
        }
        _ => {
            // Task mode (`AnnotateImage` after a capture): Continue and Cancel, no start screen.
            let (reply, _answer) = tokio::sync::oneshot::channel();
            let image = modlight_editor::qa::sample_image(960, 600);
            state.annotate_for_task(std::sync::Arc::new(image), false, reply);
        }
    }
    Some(vec2(
        modlight_editor::qa::SIZE.0,
        modlight_editor::qa::SIZE.1,
    ))
}

fn adopt(session: &mut EditorSession, scene: modlight_editor::qa::Scene) {
    let options = session.editor.options.clone();
    session.editor = scene.editor;
    session.editor.options.recent_image_files = options.recent_image_files;
}

/// Draws the screen's editor into `ui`.
pub fn draw(screen: &str, ui: &mut Ui, state: &mut AppState) -> Option<Option<f32>> {
    if !SCREENS.contains(&screen) {
        return None;
    }
    let colors = modlight_editor::ui::Colors::new(&state.theme);
    if let Some(session) = state.windows.editors.sessions.last_mut() {
        crate::editor_window::editor_ui(ui, session, &colors);
        let key = session.key;
        state.editor_requests(key);
    }
    Some(None)
}
