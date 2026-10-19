//! The secondary windows of My space and the message box: prompt, folder delete, link
//! settings, duplicates, confirm. Each is a state struct plus a `ui` function that draws
//! it filling its window and returns what the person decided.

pub mod after_upload;
pub mod before_upload;
pub mod chrome;
pub mod clipboard_upload;
pub mod confirm;
pub mod drop_window;
pub mod duplicates;
pub mod file_exists;
pub mod folder_delete;
pub mod link_settings;
pub mod message;
pub mod paste;
pub mod prompt;
pub mod shorten_url;
pub mod text_upload;
pub mod upload_confirm;
pub mod url_upload;

use egui::{Color32, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::notification::Notification;
use crate::settings::SettingsWindow;
use crate::signin::SignInWindow;
use crate::theme::Colors;

/// Which secondary windows are open.
#[derive(Default)]
pub struct Windows {
    pub sign_in: Option<SignInWindow>,
    pub settings: Option<SettingsWindow>,
    pub prompt: Option<prompt::Prompt>,
    pub folder_delete: Option<folder_delete::FolderDelete>,
    pub link_settings: Option<link_settings::LinkSettings>,
    pub duplicates: Option<duplicates::Duplicates>,
    pub confirm: Option<confirm::Confirm>,
    pub notification: Option<Notification>,
}

impl Windows {
    /// A dialog that blocks the main window (`ShowDialog(owner)`).
    pub fn modal_open(&self) -> bool {
        self.prompt.is_some()
            || self.folder_delete.is_some()
            || self.link_settings.is_some()
            || self.duplicates.is_some()
            || self.confirm.is_some()
    }
}

/// The footer bar every dialog ends with: toolbar fill, a rule on top, 14,12 padding.
pub fn footer(ui: &mut Ui, colors: &Colors, height: f32) -> Rect {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    ui.painter()
        .rect_filled(rect, 0.0, colors.background_toolbar());
    ui.painter().hline(
        rect.left()..=rect.right(),
        rect.top() + 0.5,
        Stroke::new(1.0, colors.border()),
    );
    rect
}

/// The footer bar at an exact place, for dialogs laid out by hand.
pub fn footer_at(ui: &Ui, colors: &Colors, rect: Rect) -> Rect {
    ui.painter()
        .rect_filled(rect, 0.0, colors.background_toolbar());
    ui.painter().hline(
        rect.left()..=rect.right(),
        rect.top() + 0.5,
        Stroke::new(1.0, colors.border()),
    );
    rect
}

/// Fill the whole window with the main background.
pub fn background(ui: &Ui, colors: &Colors) {
    let rect = ui.max_rect();
    ui.painter()
        .rect_filled(rect.expand(1.0), 0.0, colors.background_main());
}

/// A content area inset by Avalonia margins (left, top, right, bottom).
pub fn inset(rect: Rect, left: f32, top: f32, right: f32, bottom: f32) -> Rect {
    Rect::from_min_max(
        pos2(rect.left() + left, rect.top() + top),
        pos2(rect.right() - right, rect.bottom() - bottom),
    )
}

/// The yellow and red of the message box icons (`MessageBoxWindow`).
pub const WARNING_COLOR: Color32 = Color32::from_rgb(0xF5, 0x9E, 0x0B);
pub const ERROR_COLOR: Color32 = Color32::from_rgb(0xEF, 0x44, 0x44);
