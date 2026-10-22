//! The modal dialogs of `EditorView` (`ModalContent` over `modal-scrim`): the exit
//! confirmation, New image, Insert image, the emoji picker, the start screen and the
//! toolbar customisation.

use egui::{
    Align2, Color32, CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};

use super::pickers::{self, ColorEvent};
use super::style::{self, ButtonKind, Colors, Weight};
use crate::editor::toolbar::{self, ToolbarItem};
use crate::editor::{
    Editor, EmojiPickerState, InsertPlacement, Modal, NewImageState, Request, StartScreenState,
};
use crate::emoji;
use modlight_core::l10n::{editor as s, t, tf};

/// What the dialogs keep between frames.
#[derive(Default)]
pub struct Dialogs {
    /// The toolbar customisation row whose hotkey box listens for a shortcut.
    hotkey_row: Option<usize>,
    focus_requested: bool,
}

/// The dialog chrome: a panel-coloured card with a 1 px border, radius 4 and a shadow.
fn card(colors: &Colors) -> egui::Frame {
    egui::Frame::new()
        .fill(colors.background_panel())
        .stroke(Stroke::new(1.0, colors.border()))
        .corner_radius(CornerRadius::same(4))
        .shadow(egui::Shadow {
            offset: [0, 4],
            blur: 10,
            spread: 0,
            color: Color32::from_black_alpha(0x40),
        })
}

/// The title bar (`Background.Toolbar`, a border under it, padding 14 by 8). Returns
/// the close button's click when it has one.
fn title_bar(
    ui: &mut Ui,
    colors: &Colors,
    width: f32,
    title: &str,
    close: bool,
    title_color: Color32,
) -> bool {
    let height = if close { 48.0 } else { 15.0 * 1.25 + 16.0 };
    let (r, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(
        r,
        CornerRadius {
            nw: 4,
            ne: 4,
            sw: 0,
            se: 0,
        },
        colors.background_toolbar(),
    );
    painter.line_segment(
        [
            pos2(r.left(), r.bottom() - 0.5),
            pos2(r.right(), r.bottom() - 0.5),
        ],
        Stroke::new(1.0, colors.border()),
    );
    style::paint_text(
        ui,
        &painter,
        r.shrink2(vec2(14.0, 0.0)),
        Align2::LEFT_CENTER,
        title,
        15.0,
        Weight::SemiBold,
        title_color,
    );
    if close {
        let b = Rect::from_min_size(
            pos2(r.right() - 14.0 - 32.0, r.center().y - 16.0),
            vec2(32.0, 32.0),
        );
        let (resp, paint) =
            style::button_at(ui, colors, b, ButtonKind::Standard, 10.0, false, true);
        style::paint_icon(
            ui,
            ui.painter(),
            paint.rect.center(),
            modlight_core::icons::X,
            16.0,
            paint.text_block,
        );
        return resp.clicked();
    }
    false
}

/// The footer (`Background.Toolbar`, a border above it, padding 10) with buttons on
/// the right (padding 20, spacing 5). Returns the index clicked.
fn footer(
    ui: &mut Ui,
    colors: &Colors,
    width: f32,
    buttons: &[&str],
    message: Option<&str>,
) -> Option<usize> {
    let (r, _) = ui.allocate_exact_size(vec2(width, 60.0), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(
        r,
        CornerRadius {
            nw: 0,
            ne: 0,
            sw: 4,
            se: 4,
        },
        colors.background_toolbar(),
    );
    painter.line_segment(
        [
            pos2(r.left(), r.top() + 0.5),
            pos2(r.right(), r.top() + 0.5),
        ],
        Stroke::new(1.0, colors.border()),
    );
    let mut x = r.right() - 10.0;
    let mut clicked = None;
    for (i, label) in buttons.iter().enumerate().rev() {
        let w = style::text_button_width(ui, label, 20.0);
        let b = Rect::from_min_size(pos2(x - w, r.top() + 10.0), vec2(w, 40.0));
        if style::text_button(ui, colors, b, label, false, true).clicked() {
            clicked = Some(i);
        }
        x -= w + 5.0;
    }
    if let Some(m) = message {
        let g = style::galley_wrapped(
            ui,
            m,
            style::FONT_SIZE,
            Weight::SemiBold,
            colors.cancel_border(),
            x - r.left() - 20.0,
        );
        painter.galley(
            pos2(r.left() + 14.0, r.center().y - g.size().y / 2.0),
            g,
            colors.cancel_border(),
        );
    }
    clicked
}

fn label_galley(
    ui: &Ui,
    text: &str,
    color: Color32,
    width: f32,
    weight: Weight,
) -> std::sync::Arc<egui::Galley> {
    style::galley_wrapped(ui, text, style::FONT_SIZE, weight, color, width)
}

impl Dialogs {
    /// Draws the scrim and the open dialog over `view`.
    pub fn show(&mut self, ui: &mut Ui, colors: &Colors, editor: &mut Editor, view: Rect) {
        let Some(modal) = editor.modal.clone() else {
            self.hotkey_row = None;
            self.focus_requested = false;
            return;
        };
        let scrim = ui.interact(view, Id::new("editor-modal-scrim"), Sense::click());
        ui.painter().rect_filled(view, 0.0, colors.overlay_modal());
        let mut close_from_scrim = scrim.clicked();
        let area = egui::Area::new(Id::new("editor-modal"))
            .order(egui::Order::Foreground)
            .pivot(Align2::CENTER_CENTER)
            .fixed_pos(view.center())
            .show(ui.ctx(), |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                match modal {
                    Modal::Confirmation => self.confirmation(ui, colors, editor),
                    Modal::NewImage(state) => self.new_image(ui, colors, editor, state),
                    Modal::InsertImage { .. } => self.insert_image(ui, colors, editor),
                    Modal::EmojiPicker(state) => self.emoji_picker(ui, colors, editor, state, view),
                    Modal::StartScreen(state) => self.start_screen(ui, colors, editor, state),
                    Modal::ToolbarCustomization(items) => {
                        self.toolbar_customization(ui, colors, editor, items, view)
                    }
                }
            });
        if area.response.contains_pointer() {
            close_from_scrim = false;
        }
        if close_from_scrim && editor.modal.is_some() {
            editor.cancel_effect_preview();
            editor.close_modal();
        }
    }

    fn confirmation(&mut self, ui: &mut Ui, colors: &Colors, editor: &mut Editor) {
        let width = 400.0;
        card(colors).show(ui, |ui| {
            ui.set_width(width);
            title_bar(
                ui,
                colors,
                width,
                t(s::ConfirmationDialogView_Title),
                false,
                colors.text_secondary(),
            );
            // Body: the warning sign (40 points) and the message, padding 25.
            let text_w = width - 50.0 - 40.0 - 25.0;
            let g = label_galley(
                ui,
                t(s::ConfirmationDialogView_Message),
                colors.text_secondary(),
                text_w,
                Weight::Normal,
            );
            let h = g.size().y.max(48.0) + 50.0;
            let (r, _) = ui.allocate_exact_size(vec2(width, h), Sense::hover());
            let icon =
                Rect::from_min_size(pos2(r.left() + 25.0, r.center().y - 24.0), vec2(48.0, 48.0));
            paint_emoji(ui, "26A0 FE0F", icon);
            ui.painter().galley(
                pos2(icon.right() + 25.0 - 8.0, r.center().y - g.size().y / 2.0),
                g,
                colors.text_secondary(),
            );
            match footer(
                ui,
                colors,
                width,
                &[
                    t(s::ConfirmationDialogView_Yes),
                    t(s::ConfirmationDialogView_No),
                    t(s::ConfirmationDialogView_Cancel),
                ],
                None,
            ) {
                Some(0) => editor.confirmation_answer(Some(true)),
                Some(1) => editor.confirmation_answer(Some(false)),
                Some(_) => editor.confirmation_answer(None),
                None => {}
            }
        });
    }

    fn new_image(
        &mut self,
        ui: &mut Ui,
        colors: &Colors,
        editor: &mut Editor,
        mut state: NewImageState,
    ) {
        let width = 340.0;
        let mut ok = false;
        let mut cancel = false;
        card(colors).show(ui, |ui| {
            ui.set_width(width);
            title_bar(
                ui,
                colors,
                width,
                t(s::NewImageDialogView_Title),
                false,
                colors.text_secondary(),
            );
            ui.add_space(20.0);
            let inner = width - 40.0;
            let label_w = [
                t(s::NewImageDialogView_Width),
                t(s::NewImageDialogView_Height),
            ]
            .iter()
            .map(|l| {
                style::galley(ui, l, style::FONT_SIZE, Weight::Normal, Color32::WHITE)
                    .size()
                    .x
            })
            .fold(0.0f32, f32::max);
            let px_w = style::galley(
                ui,
                t(s::NewImageDialogView_Pixels),
                style::FONT_SIZE,
                Weight::Normal,
                Color32::WHITE,
            )
            .size()
            .x;
            for (i, (label, value)) in [
                (t(s::NewImageDialogView_Width), state.width),
                (t(s::NewImageDialogView_Height), state.height),
            ]
            .into_iter()
            .enumerate()
            {
                let (row, _) = ui.allocate_exact_size(vec2(width, 40.0), Sense::hover());
                let left = row.left() + 20.0;
                style::paint_text(
                    ui,
                    ui.painter(),
                    Rect::from_min_size(pos2(left, row.top()), vec2(label_w, 40.0)),
                    Align2::LEFT_CENTER,
                    label,
                    style::FONT_SIZE,
                    Weight::Normal,
                    colors.text_secondary(),
                );
                let field = Rect::from_min_max(
                    pos2(left + label_w + 8.0, row.top()),
                    pos2(row.right() - 20.0 - px_w - 8.0, row.bottom()),
                );
                if let Some(Some(v)) = style::numeric_box(
                    ui,
                    colors,
                    Id::new(("new-image", i)),
                    field,
                    Some(value as f64),
                    1.0,
                    10000.0,
                    1.0,
                    0,
                ) {
                    if i == 0 {
                        state.width = v.round() as i32;
                    } else {
                        state.height = v.round() as i32;
                    }
                }
                style::paint_text(
                    ui,
                    ui.painter(),
                    Rect::from_min_max(
                        pos2(field.right() + 8.0, row.top()),
                        pos2(row.right() - 20.0, row.bottom()),
                    ),
                    Align2::LEFT_CENTER,
                    t(s::NewImageDialogView_Pixels),
                    style::FONT_SIZE,
                    Weight::Normal,
                    colors.text_secondary(),
                );
                if i == 0 {
                    ui.add_space(8.0);
                }
            }
            ui.add_space(12.0);
            let g = label_galley(
                ui,
                t(s::NewImageDialogView_Background),
                colors.text_secondary(),
                inner,
                Weight::Normal,
            );
            let (r, _) = ui.allocate_exact_size(vec2(width, g.size().y), Sense::hover());
            ui.painter()
                .galley(pos2(r.left() + 20.0, r.top()), g, colors.text_secondary());
            ui.add_space(8.0);
            let (row, _) = ui.allocate_exact_size(vec2(width, 40.0), Sense::hover());
            let mut x = row.left() + 20.0;
            if radio(
                ui,
                colors,
                Id::new("new-image-transparent"),
                pos2(x, row.center().y),
                t(s::NewImageDialogView_Transparent),
                state.transparent,
                &mut x,
            ) {
                state.transparent = true;
            }
            x += 16.0;
            if radio(
                ui,
                colors,
                Id::new("new-image-solid"),
                pos2(x, row.center().y),
                t(s::NewImageDialogView_SolidColor),
                !state.transparent,
                &mut x,
            ) {
                state.transparent = false;
            }
            x += 8.0;
            let picker = Rect::from_min_size(pos2(x, row.top()), vec2(72.0, 40.0));
            if let Some(ColorEvent::Picked(c)) = pickers::color_dropdown(
                ui,
                colors,
                Id::new("new-image-color"),
                picker,
                state.background,
                false,
                false,
                !state.transparent,
                "",
            ) {
                state.background = c;
            }
            ui.add_space(20.0);
            match footer(
                ui,
                colors,
                width,
                &[t(s::NewImageDialogView_OK), t(s::NewImageDialogView_Cancel)],
                None,
            ) {
                Some(0) => ok = true,
                Some(_) => cancel = true,
                None => {}
            }
        });
        if ok {
            editor.modal = None;
            editor.create_new_image(&state);
        } else if cancel {
            editor.close_modal();
        } else if let Some(Modal::NewImage(s)) = &mut editor.modal {
            *s = state;
        }
    }

    fn insert_image(&mut self, ui: &mut Ui, colors: &Colors, editor: &mut Editor) {
        let width = 380.0;
        let mut choice: Option<Option<InsertPlacement>> = None;
        egui::Frame::new()
            .fill(colors.background_panel())
            .stroke(Stroke::new(1.0, colors.border()))
            .corner_radius(CornerRadius::same(4))
            .shadow(egui::Shadow {
                offset: [0, 8],
                blur: 24,
                spread: 0,
                color: Color32::from_black_alpha(0x50),
            })
            .show(ui, |ui| {
                ui.set_width(width);
                if title_bar(
                    ui,
                    colors,
                    width,
                    t(s::InsertImageDialogView_Title),
                    true,
                    colors.text(),
                ) {
                    choice = Some(None);
                }
                ui.add_space(20.0);
                let rows = [
                    (
                        modlight_core::icons::LOCATE_FIXED,
                        t(s::InsertImageDialogView_Center),
                        t(s::InsertImageDialogView_CenterDescription),
                        InsertPlacement::Center,
                    ),
                    (
                        modlight_core::icons::ARROW_DOWN_TO_LINE,
                        t(s::InsertImageDialogView_Below),
                        t(s::InsertImageDialogView_BelowDescription),
                        InsertPlacement::CanvasExpandDown,
                    ),
                    (
                        modlight_core::icons::ARROW_RIGHT_TO_LINE,
                        t(s::InsertImageDialogView_Right),
                        t(s::InsertImageDialogView_RightDescription),
                        InsertPlacement::CanvasExpandRight,
                    ),
                ];
                for (i, (icon, title, description, placement)) in rows.into_iter().enumerate() {
                    if i > 0 {
                        ui.add_space(12.0);
                    }
                    let inner_w = width - 40.0 - 28.0 - 18.0 - 12.0;
                    let tg =
                        label_galley(ui, title, colors.text_secondary(), inner_w, Weight::Normal);
                    let dg = style::galley_wrapped(
                        ui,
                        description,
                        11.0,
                        Weight::Normal,
                        colors.text_secondary(),
                        inner_w,
                    );
                    let h = (tg.size().y + 2.0 + dg.size().y + 20.0).max(58.0);
                    let (row, _) = ui.allocate_exact_size(vec2(width, h), Sense::hover());
                    let b = Rect::from_min_max(
                        pos2(row.left() + 20.0, row.top()),
                        pos2(row.right() - 20.0, row.bottom()),
                    );
                    let (resp, paint) =
                        style::button_at(ui, colors, b, ButtonKind::Standard, 10.0, false, true);
                    style::paint_icon(
                        ui,
                        ui.painter(),
                        pos2(paint.rect.left() + 14.0 + 9.0, paint.rect.center().y),
                        icon,
                        18.0,
                        paint.text_block,
                    );
                    let text_left = paint.rect.left() + 14.0 + 18.0 + 12.0;
                    let total = tg.size().y + 2.0 + dg.size().y;
                    let top = paint.rect.center().y - total / 2.0;
                    let th = tg.size().y;
                    ui.painter()
                        .galley(pos2(text_left, top), tg, colors.text_secondary());
                    ui.painter().galley(
                        pos2(text_left, top + th + 2.0),
                        dg,
                        colors.text_secondary(),
                    );
                    if resp.clicked() {
                        choice = Some(Some(placement));
                    }
                }
                ui.add_space(20.0);
            });
        match choice {
            Some(Some(p)) => editor.insert_image_choice(Some(p)),
            Some(None) => editor.close_modal(),
            None => {}
        }
    }

    fn emoji_picker(
        &mut self,
        ui: &mut Ui,
        colors: &Colors,
        editor: &mut Editor,
        mut state: EmojiPickerState,
        view: Rect,
    ) {
        let width = 760.0f32.min(view.width() - 16.0);
        let height = 640.0f32.min(view.height() - 16.0);
        let groups = emoji::groups();
        let group = state
            .group
            .clone()
            .or_else(|| groups.first().cloned())
            .unwrap_or_default();
        let catalog = emoji::catalog();
        let category: Vec<&emoji::EmojiEntry> =
            catalog.iter().filter(|e| e.group == group).collect();
        let search = state.search.trim().to_string();
        let entries: Vec<&emoji::EmojiEntry> = if search.is_empty() {
            category.clone()
        } else {
            let mut scored: Vec<(i32, &emoji::EmojiEntry)> = category
                .iter()
                .map(|e| (e.search_score(&search), *e))
                .filter(|(s, _)| *s != i32::MAX)
                .collect();
            scored.sort_by_key(|(s, _)| *s);
            scored.into_iter().map(|(_, e)| e).collect()
        };
        let mut picked: Option<(String, String)> = None;
        let mut cancel = false;
        egui::Frame::new()
            .fill(colors.background_panel())
            .stroke(Stroke::new(1.0, colors.border()))
            .corner_radius(CornerRadius::same(4))
            .shadow(egui::Shadow {
                offset: [0, 6],
                blur: 18,
                spread: 0,
                color: Color32::from_black_alpha(0x40),
            })
            .show(ui, |ui| {
                ui.set_width(width);
                ui.set_height(height);
                ui.add_space(12.0);
                let (row, _) = ui.allocate_exact_size(vec2(width, 40.0), Sense::hover());
                let combo = Rect::from_min_max(
                    pos2(row.right() - 12.0 - 220.0, row.top()),
                    pos2(row.right() - 12.0, row.bottom()),
                );
                let field = Rect::from_min_max(
                    pos2(row.left() + 12.0, row.top()),
                    pos2(combo.left() - 12.0, row.bottom()),
                );
                let hint = tf(
                    s::EmojiPickerDialogView_SearchEmojisCount,
                    &[category.len().into()],
                );
                let resp = style::text_box(
                    ui,
                    colors,
                    field,
                    &mut state.search,
                    &hint,
                    true,
                    Id::new("emoji-search"),
                );
                if !self.focus_requested {
                    self.focus_requested = true;
                    resp.request_focus();
                }
                let selected = groups.iter().position(|g| *g == group).unwrap_or(0);
                if let Some(i) = style::combo_box(
                    ui,
                    colors,
                    Id::new("emoji-group"),
                    combo,
                    &groups,
                    selected,
                    true,
                ) {
                    state.group = Some(groups[i].clone());
                }
                ui.add_space(8.0);
                let tile = 56.0;
                let columns = (((width - 24.0) / tile).floor() as usize).max(1);
                if entries.is_empty() {
                    let (r, _) = ui.allocate_exact_size(vec2(width, height - 72.0), Sense::hover());
                    let g1 = style::galley(
                        ui,
                        t(s::EmojiPickerDialogView_NoMatches),
                        style::FONT_SIZE,
                        Weight::SemiBold,
                        colors.text_secondary(),
                    );
                    let g2 = style::galley(
                        ui,
                        t(s::EmojiPickerDialogView_TryDifferentKeyword),
                        11.0,
                        Weight::SemiBold,
                        colors.text_secondary(),
                    );
                    let total = g1.size().y + 8.0 + g2.size().y + 8.0 + 40.0;
                    let top = r.center().y - total / 2.0;
                    let g1h = g1.size().y;
                    let g2h = g2.size().y;
                    ui.painter().galley(
                        pos2(r.center().x - g1.size().x / 2.0, top),
                        g1,
                        colors.text_secondary(),
                    );
                    ui.painter().galley(
                        pos2(r.center().x - g2.size().x / 2.0, top + g1h + 8.0),
                        g2,
                        colors.text_secondary(),
                    );
                    let label = t(s::EmojiPickerDialogView_Cancel);
                    let w = style::text_button_width(ui, label, 20.0);
                    let b = Rect::from_min_size(
                        pos2(r.center().x - w / 2.0, top + g1h + g2h + 16.0),
                        vec2(w, 40.0),
                    );
                    if style::text_button(ui, colors, b, label, false, true).clicked() {
                        cancel = true;
                    }
                    return;
                }
                let rows = entries.len().div_ceil(columns);
                let ppp = ui.ctx().pixels_per_point();
                egui::ScrollArea::vertical()
                    .id_salt(("emoji-scroll", &group))
                    .max_height(height - 72.0)
                    .auto_shrink([false, false])
                    .show_rows(ui, tile, rows, |ui, range| {
                        for row in range {
                            let (line, _) =
                                ui.allocate_exact_size(vec2(width, tile), Sense::hover());
                            for col in 0..columns {
                                let Some(entry) = entries.get(row * columns + col) else {
                                    break;
                                };
                                let r = Rect::from_min_size(
                                    pos2(
                                        line.left() + 12.0 + col as f32 * tile + 2.0,
                                        line.top() + 2.0,
                                    ),
                                    vec2(52.0, 52.0),
                                );
                                let (resp, paint) = style::button_at(
                                    ui,
                                    colors,
                                    r,
                                    ButtonKind::EffectItem,
                                    6.0,
                                    false,
                                    true,
                                );
                                let size = (48.0 * ppp).round() as u32;
                                if let Some(image) = emoji::preview(&entry.unicode, size) {
                                    style::paint_image(
                                        ui,
                                        ui.painter(),
                                        &format!("emoji-{}-{size}", entry.unicode),
                                        &image,
                                        Rect::from_center_size(
                                            paint.rect.center(),
                                            vec2(48.0, 48.0),
                                        ),
                                        Color32::WHITE,
                                    );
                                }
                                if resp.on_hover_text(entry.name.as_str()).clicked() {
                                    picked = Some((entry.unicode.clone(), entry.name.clone()));
                                }
                            }
                        }
                    });
            });
        if let Some((unicode, name)) = picked {
            editor.emoji_picked(&unicode, &name);
        } else if cancel {
            editor.close_modal();
        } else if let Some(Modal::EmojiPicker(s)) = &mut editor.modal {
            *s = state;
        }
    }

    fn start_screen(
        &mut self,
        ui: &mut Ui,
        colors: &Colors,
        editor: &mut Editor,
        mut state: StartScreenState,
    ) {
        let width = 750.0;
        let mut close = false;
        let mut exit = false;
        let mut action: Option<Request> = None;
        let mut new_image = false;
        egui::Frame::new()
            .fill(colors.background_panel())
            .stroke(Stroke::new(1.0, colors.border()))
            .corner_radius(CornerRadius::same(4))
            .shadow(egui::Shadow {
                offset: [0, 8],
                blur: 24,
                spread: 0,
                color: Color32::from_black_alpha(0x50),
            })
            .show(ui, |ui| {
                ui.set_width(width);
                if title_bar(
                    ui,
                    colors,
                    width,
                    t(s::StartScreenDialogView_Title),
                    true,
                    colors.text(),
                ) {
                    close = true;
                }
                let left_w = 350.0;
                let top = ui.cursor().top();
                // Left column.
                let mut left = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(Rect::from_min_size(
                            pos2(ui.min_rect().left() + 14.0, top + 14.0),
                            vec2(left_w - 28.0, 600.0),
                        ))
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                left.spacing_mut().item_spacing = vec2(0.0, 6.0);
                let inner = left_w - 28.0;
                if let Some(status) = &state.status {
                    let g = style::galley_wrapped(
                        &left,
                        status,
                        style::FONT_SIZE,
                        Weight::Normal,
                        colors.text(),
                        inner - 24.0 - 30.0,
                    );
                    let (r, _) = left.allocate_exact_size(
                        vec2(inner, g.size().y.max(18.0) + 24.0),
                        Sense::hover(),
                    );
                    left.painter()
                        .rect_filled(r, 4.0, colors.cancel_background());
                    left.painter().rect_stroke(
                        r,
                        4.0,
                        Stroke::new(1.0, colors.cancel_border()),
                        StrokeKind::Inside,
                    );
                    style::paint_icon(
                        &left,
                        left.painter(),
                        pos2(r.left() + 12.0 + 9.0, r.center().y),
                        modlight_core::icons::CIRCLE_X,
                        18.0,
                        colors.text(),
                    );
                    left.painter().galley(
                        pos2(
                            r.left() + 12.0 + 18.0 + 12.0,
                            r.center().y - g.size().y / 2.0,
                        ),
                        g,
                        colors.text(),
                    );
                    left.add_space(4.0);
                }
                let buttons = [
                    (
                        modlight_core::icons::IMAGE_PLUS,
                        t(s::StartScreenDialogView_CreateNewImage),
                    ),
                    (
                        modlight_core::icons::FOLDER_OPEN,
                        t(s::StartScreenDialogView_OpenImageFile),
                    ),
                    (
                        modlight_core::icons::CLIPBOARD_PASTE,
                        t(s::StartScreenDialogView_LoadImageFromClipboard),
                    ),
                    (
                        modlight_core::icons::LINK,
                        t(s::StartScreenDialogView_LoadImageFromURL),
                    ),
                ];
                for (i, (icon, label)) in buttons.iter().enumerate() {
                    if start_button(&mut left, colors, inner, *icon, label) {
                        match i {
                            0 => new_image = true,
                            1 => action = Some(Request::OpenImage),
                            2 => action = Some(Request::LoadFromClipboard),
                            _ => {
                                state.url_input_visible = true;
                                state.status = None;
                            }
                        }
                    }
                }
                if state.url_input_visible {
                    left.add_space(8.0);
                    let (r, _) = left.allocate_exact_size(vec2(inner, 40.0), Sense::hover());
                    let resp = style::text_box(
                        &mut left,
                        colors,
                        r,
                        &mut state.url,
                        "https://example.com/image.png",
                        !state.url_loading,
                        Id::new("start-url"),
                    );
                    let enter =
                        resp.lost_focus() && left.input(|i| i.key_pressed(egui::Key::Enter));
                    if state.url_loading {
                        let (bar, _) = left.allocate_exact_size(vec2(inner, 4.0), Sense::hover());
                        left.painter()
                            .rect_filled(bar, 2.0, colors.control_background_disabled());
                        let t = (left.input(|i| i.time) as f32 * 0.8).fract();
                        let seg = Rect::from_min_size(
                            pos2(bar.left() + (bar.width() - 60.0) * t, bar.top()),
                            vec2(60.0, 4.0),
                        );
                        left.painter().rect_filled(seg, 2.0, colors.accent_start());
                        left.ctx().request_repaint();
                    }
                    let (row, _) = left.allocate_exact_size(vec2(inner, 40.0), Sense::hover());
                    let cancel_label = t(s::StartScreenDialogView_Cancel);
                    let load_label = t(s::StartScreenDialogView_Load);
                    let cw = style::text_button_width(&left, cancel_label, 18.0);
                    let lw = style::text_button_width(&left, load_label, 18.0);
                    let cancel_rect =
                        Rect::from_min_size(pos2(row.right() - cw, row.top()), vec2(cw, 40.0));
                    let load_rect = Rect::from_min_size(
                        pos2(cancel_rect.left() - 6.0 - lw, row.top()),
                        vec2(lw, 40.0),
                    );
                    let can_submit = !state.url_loading && !state.url.trim().is_empty();
                    let submit = style::text_button(
                        &mut left, colors, load_rect, load_label, false, can_submit,
                    )
                    .clicked()
                        || (enter && can_submit);
                    if submit {
                        match validate_url(&state.url) {
                            Some(url) => {
                                state.status = None;
                                state.url_loading = true;
                                action = Some(Request::LoadFromUrl(url));
                            }
                            None => {
                                state.status =
                                    Some(t(s::StartScreenDialogView_InvalidURL).to_string())
                            }
                        }
                    }
                    if style::text_button(&mut left, colors, cancel_rect, cancel_label, false, true)
                        .clicked()
                    {
                        state.url_input_visible = false;
                        state.url.clear();
                    }
                }
                if start_button(
                    &mut left,
                    colors,
                    inner,
                    modlight_core::icons::LOG_OUT,
                    t(s::StartScreenDialogView_Exit),
                ) {
                    exit = true;
                }
                let left_bottom = left.min_rect().bottom() + 14.0;
                // Right column: recent files.
                let right_rect = Rect::from_min_max(
                    pos2(ui.min_rect().left() + left_w, top),
                    pos2(ui.min_rect().left() + width, left_bottom),
                );
                ui.painter().line_segment(
                    [right_rect.left_top(), right_rect.left_bottom()],
                    Stroke::new(1.0, colors.border()),
                );
                let mut right = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(right_rect.shrink(14.0))
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                right.spacing_mut().item_spacing = Vec2::ZERO;
                let rw = right_rect.width() - 28.0;
                let (head, _) = right.allocate_exact_size(vec2(rw, 18.0), Sense::hover());
                style::paint_icon(
                    &right,
                    right.painter(),
                    pos2(head.left() + 7.0, head.center().y),
                    modlight_core::icons::HISTORY,
                    14.0,
                    colors.text_secondary(),
                );
                style::paint_text(
                    &right,
                    right.painter(),
                    Rect::from_min_max(pos2(head.left() + 22.0, head.top()), head.max),
                    Align2::LEFT_CENTER,
                    t(s::StartScreenDialogView_RecentFiles),
                    12.0,
                    Weight::Normal,
                    colors.text_secondary(),
                );
                right.add_space(10.0);
                if state.recent_files.is_empty() {
                    let g = style::galley_wrapped(
                        &right,
                        t(s::StartScreenDialogView_NoRecentImageFiles),
                        style::FONT_SIZE,
                        Weight::Normal,
                        colors.text_secondary(),
                        rw - 24.0,
                    );
                    let (r, _) =
                        right.allocate_exact_size(vec2(rw, g.size().y + 24.0), Sense::hover());
                    right
                        .painter()
                        .rect_filled(r, 4.0, colors.background_popup());
                    right.painter().rect_stroke(
                        r,
                        4.0,
                        Stroke::new(1.0, colors.border()),
                        StrokeKind::Inside,
                    );
                    right
                        .painter()
                        .galley(r.min + vec2(12.0, 12.0), g, colors.text_secondary());
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("start-recent")
                        .max_height(right_rect.height() - 28.0 - 28.0)
                        .show(&mut right, |ui| {
                            ui.spacing_mut().item_spacing = vec2(0.0, 4.0);
                            for path in &state.recent_files {
                                let (r, _) = ui.allocate_exact_size(vec2(rw, 36.0), Sense::hover());
                                let (resp, paint) = style::button_at(
                                    ui,
                                    colors,
                                    r,
                                    ButtonKind::Standard,
                                    10.0,
                                    false,
                                    true,
                                );
                                let g = style::leading_trimmed(
                                    ui,
                                    path,
                                    12.0,
                                    Weight::Normal,
                                    paint.text_block,
                                    paint.rect.width() - 24.0,
                                );
                                ui.painter().galley(
                                    pos2(
                                        paint.rect.left() + 12.0,
                                        paint.rect.center().y - g.size().y / 2.0,
                                    ),
                                    g,
                                    paint.text_block,
                                );
                                if resp.on_hover_text(path.as_str()).clicked() {
                                    action = Some(Request::OpenRecent(path.clone()));
                                }
                            }
                        });
                }
                ui.allocate_rect(
                    Rect::from_min_max(
                        pos2(ui.min_rect().left(), top),
                        pos2(ui.min_rect().left() + width, right_rect.bottom()),
                    ),
                    Sense::hover(),
                );
            });
        if new_image {
            editor.close_modal();
            editor.modal_new_image();
            return;
        }
        if close {
            editor.close_modal();
            return;
        }
        if exit {
            editor.close_modal();
            editor.request_close(true);
            return;
        }
        if matches!(action, Some(Request::OpenImage)) {
            editor.close_modal();
        } else if let Some(Modal::StartScreen(s)) = &mut editor.modal {
            *s = state;
        }
        if let Some(a) = action {
            editor.request(a);
        }
    }

    fn toolbar_customization(
        &mut self,
        ui: &mut Ui,
        colors: &Colors,
        editor: &mut Editor,
        mut items: Vec<ToolbarItem>,
        view: Rect,
    ) {
        let width = 720.0f32.min(view.width() - 16.0);
        let max_height = 620.0f32.min(view.height() - 16.0);
        let mut ok = false;
        let mut cancel = false;
        let validation = validate(&items);
        // A shortcut typed into the listening box.
        if let Some(row) = self.hotkey_row {
            let events = ui.input(|i| i.events.clone());
            for e in events {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = e
                {
                    if let Some(item) = items.get_mut(row).filter(|i| i.hotkey_editable) {
                        if matches!(key, egui::Key::Backspace | egui::Key::Delete) {
                            item.hotkey.clear();
                        } else {
                            item.hotkey = toolbar::format_hotkey(&toolbar::Hotkey {
                                key,
                                ctrl: modifiers.command || modifiers.ctrl,
                                shift: modifiers.shift,
                                alt: modifiers.alt,
                                meta: false,
                            });
                        }
                    }
                    self.hotkey_row = None;
                }
            }
        }
        card(colors).show(ui, |ui| {
            ui.set_width(width);
            // Title with "Reset toolbar to default".
            let (r, _) = ui.allocate_exact_size(vec2(width, 48.0), Sense::hover());
            let painter = ui.painter().clone();
            painter.rect_filled(r, CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, colors.background_toolbar());
            painter.line_segment([pos2(r.left(), r.bottom() - 0.5), pos2(r.right(), r.bottom() - 0.5)], Stroke::new(1.0, colors.border()));
            style::paint_text(ui, &painter, r.shrink2(vec2(14.0, 0.0)), Align2::LEFT_CENTER, t(s::ToolbarCustomizationDialogView_CustomizeToolbar), 15.0, Weight::SemiBold, colors.text_secondary());
            let reset = t(s::ToolbarCustomizationDialogView_Resettoolbartodefault);
            let rw = style::text_button_width(ui, reset, 10.0);
            let rb = Rect::from_min_size(pos2(r.right() - 14.0 - rw, r.center().y - 16.0), vec2(rw, 32.0));
            if style::text_button(ui, colors, rb, reset, false, true).clicked() {
                items = toolbar::default_items();
                self.hotkey_row = None;
            }
            ui.add_space(14.0);
            let list_h = (max_height - 48.0 - 28.0 - 60.0).max(120.0);
            let (frame, _) = ui.allocate_exact_size(vec2(width, list_h), Sense::hover());
            let list = Rect::from_min_max(pos2(frame.left() + 14.0, frame.top()), pos2(frame.right() - 14.0, frame.bottom()));
            ui.painter().rect_stroke(list, 4.0, Stroke::new(1.0, colors.border()), StrokeKind::Inside);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list.shrink(1.0)).layout(egui::Layout::top_down(egui::Align::Min)));
            child.set_clip_rect(list.shrink(1.0));
            let row_w = list.width() - 2.0 - 8.0;
            let count = items.len();
            let mut move_up = None;
            let mut move_down = None;
            egui::ScrollArea::vertical()
                .id_salt("toolbar-customization")
                .auto_shrink([false, false])
                .show(&mut child, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    for (i, item) in items.iter_mut().enumerate() {
                        let (row, _) = ui.allocate_exact_size(vec2(row_w, 56.0), Sense::hover());
                        ui.painter().line_segment([pos2(row.left(), row.bottom() - 0.5), pos2(row.right(), row.bottom() - 0.5)], Stroke::new(1.0, colors.border()));
                        let y = row.center().y;
                        let mut x = row.left() + 8.0;
                        let up = Rect::from_min_size(pos2(x, y - 16.0), vec2(32.0, 32.0));
                        let (resp, paint) = style::button_at(ui, colors, up, ButtonKind::Standard, 10.0, false, i > 0);
                        style::paint_icon(ui, ui.painter(), paint.rect.center(), modlight_core::icons::CHEVRON_UP, 13.0, paint.text_block);
                        if resp.on_hover_text(t(s::ToolbarCustomizationDialogView_Moveup)).clicked() {
                            move_up = Some(i);
                        }
                        x += 42.0;
                        let down = Rect::from_min_size(pos2(x, y - 16.0), vec2(32.0, 32.0));
                        let (resp, paint) = style::button_at(ui, colors, down, ButtonKind::Standard, 10.0, false, i + 1 < count);
                        style::paint_icon(ui, ui.painter(), paint.rect.center(), modlight_core::icons::CHEVRON_DOWN, 13.0, paint.text_block);
                        if resp.on_hover_text(t(s::ToolbarCustomizationDialogView_Movedown)).clicked() {
                            move_down = Some(i);
                        }
                        x += 42.0;
                        let group = Rect::from_min_size(pos2(x, y - 16.0), vec2(32.0, 32.0));
                        let (resp, paint) = style::button_at(ui, colors, group, ButtonKind::Standard, 10.0, item.begin_group, true);
                        style::paint_icon(ui, ui.painter(), paint.rect.center(), modlight_core::icons::ARROW_RIGHT_FROM_LINE, 13.0, paint.text_block);
                        if resp.on_hover_text(t(s::ToolbarCustomizationDialogView_Begingroup)).clicked() {
                            item.begin_group = !item.begin_group;
                        }
                        x += 42.0;
                        let switch = Rect::from_min_size(pos2(x + 2.0, y - 10.0), vec2(40.0, 20.0));
                        let sresp = ui.interact(switch, Id::new(("toolbar-visible", i)), if item.visibility_editable { Sense::click() } else { Sense::hover() });
                        style::paint_toggle(ui.painter(), colors, switch, item.is_visible, sresp.hovered() && item.visibility_editable, item.visibility_editable);
                        if sresp.on_hover_text(t(s::ToolbarCustomizationDialogView_ShowHide)).clicked() {
                            item.is_visible = !item.is_visible;
                        }
                        x += 44.0 + 10.0;
                        style::paint_icon(ui, ui.painter(), pos2(x + 16.0, y), item.icon, 18.0, colors.text());
                        x += 32.0 + 10.0;
                        let hotkey_box = Rect::from_min_size(pos2(row.right() - 8.0 - 150.0, y - 20.0), vec2(150.0, 40.0));
                        style::paint_text(ui, ui.painter(), Rect::from_min_max(pos2(x, row.top()), pos2(hotkey_box.left() - 10.0, row.bottom())), Align2::LEFT_CENTER, item.name(), style::FONT_SIZE, Weight::SemiBold, colors.text());
                        // The read-only box that listens for a shortcut once clicked.
                        let listening = self.hotkey_row == Some(i);
                        let hresp = ui.interact(hotkey_box, Id::new(("toolbar-hotkey", i)), if item.hotkey_editable { Sense::click() } else { Sense::hover() });
                        let fill = if !item.hotkey_editable {
                            colors.control_background_disabled()
                        } else if hresp.hovered() {
                            colors.control_background_hover()
                        } else {
                            colors.control_background()
                        };
                        ui.painter().rect_filled(hotkey_box, 8.0, fill);
                        if listening {
                            ui.painter().rect_stroke(hotkey_box, 8.0, Stroke::new(2.0, colors.focus()), StrokeKind::Inside);
                        } else {
                            ui.painter().rect_stroke(hotkey_box, 8.0, Stroke::new(1.0, colors.border()), StrokeKind::Inside);
                        }
                        let text_color = if item.hotkey_editable { colors.text() } else { colors.text_secondary() };
                        style::paint_text(ui, ui.painter(), hotkey_box.shrink2(vec2(8.0, 0.0)), Align2::CENTER_CENTER, &toolbar::display_hotkey(&item.hotkey), style::FONT_SIZE, Weight::Normal, text_color);
                        if hresp.on_hover_text(t(s::ToolbarCustomizationDialogView_ClickandpressashortcutBackspaceorDeleteclearsit)).clicked() {
                            self.hotkey_row = Some(i);
                        }
                    }
                });
            if let Some(i) = move_up.filter(|i| *i > 0) {
                items.swap(i, i - 1);
            }
            if let Some(i) = move_down.filter(|i| i + 1 < items.len()) {
                items.swap(i, i + 1);
            }
            ui.add_space(14.0);
            match footer(
                ui,
                colors,
                width,
                &[t(s::ToolbarCustomizationDialogView_OK), t(s::ToolbarCustomizationDialogView_Cancel)],
                validation.as_deref(),
            ) {
                Some(0) => ok = validation.is_none(),
                Some(_) => cancel = true,
                None => {}
            }
        });
        if ok {
            for item in items
                .iter_mut()
                .filter(|i| i.hotkey_editable && !i.hotkey.trim().is_empty())
            {
                if let Some(h) = toolbar::parse_hotkey(&item.hotkey) {
                    item.hotkey = toolbar::format_hotkey(&h);
                }
            }
            editor.modal = None;
            editor.apply_toolbar_items(items);
            self.hotkey_row = None;
        } else if cancel {
            editor.close_modal();
            self.hotkey_row = None;
        } else if let Some(Modal::ToolbarCustomization(list)) = &mut editor.modal {
            *list = items;
        }
    }
}

/// `RefreshValidation`: an unreadable or a repeated shortcut.
pub fn validate(items: &[ToolbarItem]) -> Option<String> {
    let mut used: Vec<(String, &str)> = Vec::new();
    for item in items.iter().filter(|i| i.hotkey_editable) {
        let hotkey = item.hotkey.trim();
        if hotkey.is_empty() {
            continue;
        }
        let Some(parsed) = toolbar::parse_hotkey(hotkey) else {
            return Some(tf(
                s::ToolbarCustomizationDialogView_InvalidHotkey,
                &[item.name().into()],
            ));
        };
        let normalized = toolbar::format_hotkey(&parsed);
        if let Some((_, other)) = used
            .iter()
            .find(|(h, _)| h.eq_ignore_ascii_case(&normalized))
        {
            return Some(tf(
                s::ToolbarCustomizationDialogView_DuplicateHotkey,
                &[
                    item.name().into(),
                    (*other).into(),
                    toolbar::display_hotkey(&normalized).into(),
                ],
            ));
        }
        used.push((normalized, item.name()));
    }
    None
}

/// `SubmitUrl`'s check: an absolute http or https address.
pub fn validate_url(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))?;
    if rest.is_empty() || rest.starts_with('/') {
        return None;
    }
    Some(trimmed.to_string())
}

/// A start-screen button: 42 tall, padding 12 by 10, icon 18, normal text.
fn start_button(ui: &mut Ui, colors: &Colors, width: f32, icon: char, label: &str) -> bool {
    let (r, _) = ui.allocate_exact_size(vec2(width, 42.0), Sense::hover());
    let (resp, paint) = style::button_at(ui, colors, r, ButtonKind::Standard, 10.0, false, true);
    style::paint_icon(
        ui,
        ui.painter(),
        pos2(paint.rect.left() + 12.0 + 9.0, paint.rect.center().y),
        icon,
        18.0,
        paint.text_block,
    );
    style::paint_text(
        ui,
        ui.painter(),
        Rect::from_min_max(
            pos2(paint.rect.left() + 12.0 + 18.0 + 12.0, paint.rect.top()),
            paint.rect.max,
        ),
        Align2::LEFT_CENTER,
        label,
        style::FONT_SIZE,
        Weight::Normal,
        paint.text_block,
    );
    resp.clicked()
}

/// A Fluent radio button (20-point circle, the accent with a white dot when checked).
fn radio(
    ui: &mut Ui,
    colors: &Colors,
    id: Id,
    left_center: egui::Pos2,
    label: &str,
    checked: bool,
    x: &mut f32,
) -> bool {
    let g = style::galley(
        ui,
        label,
        style::FONT_SIZE,
        Weight::Normal,
        colors.text_secondary(),
    );
    let rect = Rect::from_min_size(
        pos2(left_center.x, left_center.y - 16.0),
        vec2(20.0 + 8.0 + g.size().x, 32.0),
    );
    let resp = ui.interact(rect, id, Sense::click());
    let c = pos2(rect.left() + 10.0, rect.center().y);
    if checked {
        ui.painter().circle_filled(c, 10.0, colors.accent_start());
        ui.painter()
            .circle_filled(c, 4.0, colors.accent_foreground());
    } else {
        ui.painter()
            .circle_stroke(c, 9.5, Stroke::new(1.0, colors.text_secondary()));
    }
    ui.painter().galley(
        pos2(rect.left() + 28.0, rect.center().y - g.size().y / 2.0),
        g,
        colors.text_secondary(),
    );
    *x = rect.right();
    resp.clicked()
}

/// An emoji from the emoji font (`emoji::EMOJI_FAMILY`) into `rect` (falls back to
/// nothing).
fn paint_emoji(ui: &Ui, sequence: &str, rect: Rect) {
    let ppp = ui.ctx().pixels_per_point();
    let size = (rect.width() * ppp).round() as u32;
    if let Some(image) = emoji::preview(sequence, size) {
        style::paint_image(
            ui,
            ui.painter(),
            &format!("emoji-{sequence}-{size}"),
            &image,
            rect,
            Color32::WHITE,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_need_a_scheme_and_a_host() {
        assert_eq!(
            validate_url(" https://a.b/c.png "),
            Some("https://a.b/c.png".into())
        );
        assert!(validate_url("ftp://a.b").is_none());
        assert!(validate_url("https://").is_none());
        assert!(validate_url("example.com").is_none());
    }

    #[test]
    fn duplicate_hotkeys_are_reported() {
        let mut items = toolbar::default_items();
        let first = items.iter().position(|i| i.hotkey_editable).unwrap();
        let second = items
            .iter()
            .skip(first + 1)
            .position(|i| i.hotkey_editable)
            .unwrap()
            + first
            + 1;
        items[first].hotkey = "Ctrl+K".into();
        items[second].hotkey = "Ctrl+K".into();
        assert!(validate(&items).is_some());
        items[second].hotkey = "Ctrl+J".into();
        assert!(validate(&items).is_none());
    }
}
