//! What the image editor asks of the tools (`EDITOR.md`, "What a host provides"):
//! a colour from the screen, printing, pinning its picture.

use modlight_editor::editor::ColorTarget;
use modlight_tools::color::Color;
use modlight_tools::settings::ScreenColorPickerOptions;

use super::screen_picker::PickTarget;
use crate::state::AppState;

impl AppState {
    /// `Request::PickScreenColor(target)`: the screen colour picker, its colour into the
    /// editor that asked.
    pub fn editor_pick_screen_color(&mut self, key: u64, target: ColorTarget) {
        let settings = self.default_task_settings();
        let options: ScreenColorPickerOptions =
            super::jobs::tool_options(&settings, "ScreenColorPickerOptions");
        self.start_screen_pick(PickTarget::Editor { key, target }, options, 0.0);
    }

    /// The picked colour, as the editor's own pickers would set it.
    pub fn editor_screen_color_picked(&mut self, key: u64, target: ColorTarget, color: Color) {
        let Some(session) = self
            .windows
            .editors
            .sessions
            .iter_mut()
            .find(|s| s.key == key)
        else {
            return;
        };
        let hex = format!("#FF{:02X}{:02X}{:02X}", color.r, color.g, color.b);
        let editor = &mut session.editor;
        match target {
            ColorTarget::Stroke => editor.set_selected_color(&hex),
            ColorTarget::Fill => editor.set_fill_color(&hex),
            ColorTarget::Text => editor.set_text_color(&hex),
            ColorTarget::Shadow => {
                let (blur, opacity, ox, oy) = (
                    editor.tools.shadow_blur_radius,
                    editor.tools.shadow_opacity,
                    editor.tools.shadow_offset_x,
                    editor.tools.shadow_offset_y,
                );
                editor.set_shadow_settings(&hex, blur, opacity, ox, oy, true);
            }
            ColorTarget::Background => editor.update_background(|s| s.color = hex),
            ColorTarget::Gradient1 => editor.update_background(|s| s.gradient1 = hex),
            ColorTarget::Gradient2 => editor.update_background(|s| s.gradient2 = hex),
        }
    }
}
