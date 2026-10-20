//! `EditorView`: the whole editor in one egui `Ui`. The Background sidebar, the canvas
//! and the right sidebar side by side; the built-in toolbars, the toast, the effect
//! browser and the modal dialogs over them; the context menu; the keyboard.
//!
//! The host draws it every frame with [`EditorView::show`] and then handles
//! `Editor::take_requests` (files, clipboard, save, upload, close).

use std::time::Instant;

use egui::{Color32, Id, Rect, Sense, Stroke, Ui, pos2, vec2};

use super::canvas::Canvas;
use super::dialogs::Dialogs;
use super::effects::EffectBrowser;
use super::menu::{self, Entry};
use super::sidebar::{self, SIDEBAR_WIDTH};
use super::style::{self, Colors, Weight};
use super::toolbars;
use crate::editor::input;
use crate::editor::{Editor, Request};
use crate::ui::icons;
use modlight_core::l10n::{editor as s, t};

/// The view's own state.
#[derive(Default)]
pub struct EditorView {
    pub canvas: Canvas,
    pub effect_browser: EffectBrowser,
    pub dialogs: Dialogs,
    /// The host watches Cmd+V itself (an AppKit key monitor, since egui reports a paste
    /// only when the clipboard holds text) and calls [`EditorView::paste_shortcut`].
    pub host_watches_paste: bool,
    context_menu_open: bool,
    /// The rectangles the toolbars took this frame (tests).
    pub toolbar_rects: Vec<Rect>,
}

impl EditorView {
    pub fn new() -> EditorView {
        EditorView::default()
    }

    /// Cmd+V seen by the host's key monitor.
    pub fn paste_shortcut(&mut self, editor: &mut Editor) {
        if editor.modal.is_none() && editor.text_editing.is_none() {
            editor.request(Request::Paste);
        }
    }

    /// Draws the editor into `ui`'s whole rectangle.
    pub fn show(&mut self, ui: &mut Ui, editor: &mut Editor, colors: &Colors) {
        let view = ui.max_rect();
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        ui.painter()
            .rect_filled(view, 0.0, colors.background_main());
        editor.pump();
        self.keyboard(ui, editor);

        let left = if editor.is_settings_panel_open {
            SIDEBAR_WIDTH
        } else {
            0.0
        };
        let right = if editor.is_right_sidebar_visible() {
            SIDEBAR_WIDTH
        } else {
            0.0
        };
        let canvas_area = Rect::from_min_max(
            pos2(view.left() + left, view.top()),
            pos2(view.right() - right, view.bottom()),
        );
        let blocked = editor.modal.is_some() || editor.is_effect_browser_visible();
        self.canvas.show(ui, editor, colors, canvas_area, blocked);
        if left > 0.0 {
            sidebar::background_panel(
                ui,
                colors,
                editor,
                Rect::from_min_max(view.min, pos2(view.left() + left, view.bottom())),
            );
        }
        if right > 0.0 {
            sidebar::right_panel(
                ui,
                colors,
                editor,
                Rect::from_min_max(pos2(view.right() - right, view.top()), view.max),
            );
        }
        self.toolbar_rects.clear();
        if editor.show_toolbars {
            let top = toolbars::top_toolbar(ui, colors, editor, view);
            let bottom = toolbars::bottom_toolbar(ui, colors, editor, &mut self.canvas, view);
            self.toolbar_rects.push(top);
            self.toolbar_rects.push(bottom);
        }
        self.context_menu(ui, colors, editor);
        self.toast(ui, colors, editor, view);
        if editor.is_effect_browser_visible() {
            self.effect_browser.show(ui, colors, editor, view);
        } else {
            self.effect_browser.reset();
        }
        self.dialogs.show(ui, colors, editor, view);
        editor.pump();
    }

    // ------------------------------------------------------------------ keyboard

    fn keyboard(&mut self, ui: &Ui, editor: &mut Editor) {
        let typing = ui.ctx().egui_wants_keyboard_input();
        let events = ui.input(|i| i.events.clone());
        for event in events {
            match event {
                egui::Event::Key {
                    key,
                    pressed,
                    modifiers,
                    ..
                } => {
                    if typing && key != egui::Key::Escape {
                        continue;
                    }
                    if typing {
                        // Escape in a text field: only the field's own handling.
                        continue;
                    }
                    if pressed {
                        input::key_down(editor, key, modifiers);
                    } else {
                        input::key_up(editor, key, modifiers);
                    }
                }
                egui::Event::Copy if !typing => {
                    input::key_down(editor, egui::Key::C, egui::Modifiers::COMMAND);
                }
                egui::Event::Cut if !typing => {
                    input::key_down(editor, egui::Key::X, egui::Modifiers::COMMAND);
                }
                egui::Event::Paste(_) if !typing && !self.host_watches_paste => {
                    input::key_down(editor, egui::Key::V, egui::Modifiers::COMMAND);
                }
                _ => {}
            }
        }
    }

    // ------------------------------------------------------------------ context menu

    fn context_menu(&mut self, ui: &mut Ui, colors: &Colors, editor: &mut Editor) {
        let id = Id::new("editor-context-menu");
        if let Some(p) = editor.input.context_menu.take() {
            let at = self
                .canvas
                .geometry
                .map(|g| g.to_screen(p))
                .or_else(|| ui.input(|i| i.pointer.latest_pos()))
                .unwrap_or(ui.max_rect().center());
            ui.ctx().data_mut(|d| d.insert_temp(id, at));
            egui::Popup::open_id(ui.ctx(), id);
            self.context_menu_open = true;
        }
        if !egui::Popup::is_id_open(ui.ctx(), id) {
            self.context_menu_open = false;
            return;
        }
        let at = ui
            .ctx()
            .data(|d| d.get_temp::<egui::Pos2>(id))
            .unwrap_or(ui.max_rect().center());
        let has_selection = editor.has_selection();
        let has_annotations = editor.has_annotations();
        let toolbars = if editor.show_toolbars {
            t(s::MainViewModel_HideToolbars)
        } else {
            t(s::MainViewModel_ShowToolbars)
        };
        let list = vec![
            Entry::item(t(s::EditorView_Undo), icons::ACTION_UNDO, Some("Ctrl+Z"))
                .enabled(editor.core.can_undo()),
            Entry::item(t(s::EditorView_Redo), icons::ACTION_REDO, Some("Ctrl+Y"))
                .enabled(editor.core.can_redo()),
            Entry::Separator,
            Entry::item(
                t(s::EditorView_Delete),
                icons::ACTION_DELETE,
                Some("Delete"),
            )
            .enabled(has_selection),
            Entry::item(
                t(s::EditorView_Deleteall),
                icons::ACTION_CLEAR_ALL,
                Some("Shift+Delete"),
            )
            .enabled(has_annotations),
            Entry::item(
                t(s::EditorView_Flatten),
                icons::LAYER_FLATTEN,
                Some("Ctrl+Shift+F"),
            )
            .enabled(has_annotations),
            Entry::Separator,
            Entry::item(t(s::EditorView_Cut), icons::TOOL_CUT_OUT, Some("Ctrl+X"))
                .enabled(has_selection),
            Entry::item(
                t(s::EditorView_Copy),
                icons::ACTION_COPY,
                Some("Ctrl+Shift+C"),
            )
            .enabled(has_selection),
            Entry::item(t(s::EditorView_Paste), icons::ACTION_PASTE, Some("Ctrl+V")),
            Entry::item(
                t(s::EditorView_Duplicate),
                icons::ACTION_DUPLICATE,
                Some("Ctrl+D"),
            )
            .enabled(has_selection),
            Entry::Separator,
            Entry::item(
                t(s::EditorView_Bringtofront),
                icons::ACTION_BRING_TO_FRONT,
                Some("Home"),
            )
            .enabled(has_selection),
            Entry::item(
                t(s::EditorView_Bringforward),
                icons::ACTION_BRING_FORWARD,
                Some("PageUp"),
            )
            .enabled(has_selection),
            Entry::item(
                t(s::EditorView_Sendbackward),
                icons::ACTION_SEND_BACKWARD,
                Some("PageDown"),
            )
            .enabled(has_selection),
            Entry::item(
                t(s::EditorView_Sendtoback),
                icons::ACTION_SEND_TO_BACK,
                Some("End"),
            )
            .enabled(has_selection),
            Entry::Separator,
            Entry::item(toolbars, icons::ACTION_TOGGLE_TOOLBARS, Some("Ctrl+H")),
        ];
        let picked = egui::Popup::new(
            id,
            ui.ctx().clone(),
            egui::PopupAnchor::Position(at),
            ui.layer_id(),
        )
        .open_memory(None)
        .align(egui::RectAlign::BOTTOM_START)
        .frame(
            egui::Frame::new()
                .fill(colors.background_menu())
                .stroke(Stroke::new(1.0, colors.border()))
                .corner_radius(egui::CornerRadius::same(6))
                .inner_margin(egui::Margin::symmetric(0, 2))
                .shadow(egui::Shadow {
                    offset: [0, 4],
                    blur: 16,
                    spread: 0,
                    color: Color32::from_black_alpha(0x40),
                }),
        )
        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
        .show(|ui| menu::entries(ui, colors, &list, 6.0))
        .and_then(|r| r.inner);
        if let Some(path) = picked {
            egui::Popup::close_id(ui.ctx(), id);
            match path[0] {
                0 => editor.undo(),
                1 => editor.redo(),
                3 => editor.delete_selected(),
                4 => editor.clear_annotations(),
                5 => editor.flatten(),
                7 => editor.cut_annotation(),
                8 => editor.copy_annotation(),
                9 => editor.request(Request::Paste),
                10 => editor.duplicate_selected(),
                12 => editor.bring_to_front(),
                13 => editor.bring_forward(),
                14 => editor.send_backward(),
                15 => editor.send_to_back(),
                17 => editor.show_toolbars = !editor.show_toolbars,
                _ => {}
            }
            editor.invalidate();
        }
    }

    // ------------------------------------------------------------------ toast

    /// `app-toast`: bottom centre, 68 above the edge; a click dismisses it, hovering
    /// keeps it.
    fn toast(&mut self, ui: &mut Ui, colors: &Colors, editor: &mut Editor, view: Rect) {
        let now = Instant::now();
        if !editor.tick_notification(now) {
            return;
        }
        let Some(n) = editor.notification.clone() else {
            return;
        };
        let max_width = (view.width() - 32.0).max(80.0);
        let icon_w = if n.icon.is_some() { 18.0 + 10.0 } else { 0.0 };
        let g = style::galley_wrapped(
            ui,
            &n.message,
            style::FONT_SIZE,
            Weight::SemiBold,
            colors.text(),
            max_width - 28.0 - icon_w,
        );
        let size = vec2(g.size().x + 28.0 + icon_w, g.size().y.max(18.0) + 20.0);
        let rect = Rect::from_min_size(
            pos2(
                view.center().x - size.x / 2.0,
                view.bottom() - 68.0 - size.y,
            ),
            size,
        );
        // Closing: fade and slide down 8 points over 220 ms.
        let (opacity, offset) = match n.closing_since {
            Some(since) => {
                let p = (now.duration_since(since).as_secs_f32()
                    / crate::editor::NOTIFICATION_HIDE.as_secs_f32())
                .clamp(0.0, 1.0);
                (1.0 - p, 8.0 * p)
            }
            None => (1.0, 0.0),
        };
        let rect = rect.translate(vec2(0.0, offset));
        let resp = ui.interact(rect, Id::new("editor-toast"), Sense::click());
        if let Some(note) = editor.notification.as_mut() {
            note.hovered = resp.hovered();
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let painter = ui.painter();
        style::paint_shadow(
            painter,
            rect.translate(vec2(0.0, 14.0)).shrink(16.0),
            10.0,
            egui::Vec2::ZERO,
            28.0,
            Color32::from_black_alpha((0x70 as f32 * opacity) as u8),
        );
        painter.rect_filled(rect, 10.0, style::fade(colors.background_popup(), opacity));
        painter.rect_stroke(
            rect,
            10.0,
            Stroke::new(1.0, style::fade(colors.border(), opacity)),
            egui::StrokeKind::Inside,
        );
        let mut x = rect.left() + 14.0;
        if let Some(icon) = n.icon {
            style::paint_icon(
                ui,
                painter,
                pos2(x + 9.0, rect.center().y),
                icon,
                18.0,
                style::fade(colors.accent_start(), opacity),
            );
            x += 18.0 + 10.0;
        }
        painter.galley(
            pos2(x, rect.center().y - g.size().y / 2.0),
            g,
            style::fade(colors.text(), opacity),
        );
        if resp.clicked() {
            editor.dismiss_notification();
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }
}
