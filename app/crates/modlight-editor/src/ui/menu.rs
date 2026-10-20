//! The editor's menus: the canvas `ContextMenu` (`EditorContextMenu`), the file menu
//! flyout of the toolbar and the favourite-effects menu. `ContextMenu MenuItem` and
//! `MenuFlyoutPresenter MenuItem`: 13 points, padding 12 by 8, radius 6, the accent
//! behind the item under the pointer; the icon column, the header and the input gesture
//! in the secondary colour on the right.

use egui::{Align2, Color32, Rect, Sense, Ui, pos2, vec2};

use super::style::{self, Colors, Weight};

/// One line of a menu.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Item {
        label: String,
        icon: Option<char>,
        gesture: Option<String>,
        enabled: bool,
        children: Vec<Entry>,
    },
    Separator,
}

impl Entry {
    pub fn item(label: &str, icon: char, gesture: Option<&str>) -> Entry {
        Entry::Item {
            label: label.to_string(),
            icon: Some(icon),
            gesture: gesture.map(crate::editor::toolbar::display_hotkey),
            enabled: true,
            children: Vec::new(),
        }
    }

    pub fn enabled(mut self, value: bool) -> Entry {
        if let Entry::Item { enabled, .. } = &mut self {
            *enabled = value;
        }
        self
    }

    pub fn with_children(mut self, list: Vec<Entry>) -> Entry {
        if let Entry::Item { children, .. } = &mut self {
            *children = list;
        }
        self
    }
}

const ITEM_HEIGHT: f32 = 32.0;
const PAD_X: f32 = 12.0;
const ICON_COLUMN: f32 = 16.0 + 12.0;

/// The width the entries need.
pub fn natural_width(ui: &Ui, entries: &[Entry]) -> f32 {
    let mut width: f32 = 120.0;
    for e in entries {
        if let Entry::Item {
            label,
            gesture,
            children,
            ..
        } = e
        {
            let l = style::galley(ui, label, style::FONT_SIZE, Weight::Normal, Color32::WHITE)
                .size()
                .x;
            let g = gesture
                .as_ref()
                .map(|g| {
                    24.0 + style::galley(ui, g, style::FONT_SIZE, Weight::Normal, Color32::WHITE)
                        .size()
                        .x
                })
                .unwrap_or(0.0);
            let chevron = if children.is_empty() { 0.0 } else { 24.0 };
            width = width.max(PAD_X * 2.0 + ICON_COLUMN + l + g + chevron);
        }
    }
    width
}

/// Draws the entries; returns the path of the item clicked (index, then the index in
/// its submenu).
pub fn entries(ui: &mut Ui, colors: &Colors, list: &[Entry], inset: f32) -> Option<Vec<usize>> {
    let width = natural_width(ui, list);
    ui.set_min_width(width + inset * 2.0);
    let mut picked = None;
    for (index, entry) in list.iter().enumerate() {
        match entry {
            Entry::Separator => {
                let (r, _) = ui.allocate_exact_size(vec2(width + inset * 2.0, 9.0), Sense::hover());
                ui.painter().rect_filled(
                    Rect::from_min_max(
                        pos2(r.left() + inset, r.center().y),
                        pos2(r.right() - inset, r.center().y + 1.0),
                    ),
                    0.0,
                    colors.separator(),
                );
            }
            Entry::Item {
                label,
                icon,
                gesture,
                enabled,
                children,
            } => {
                let (outer, response) = ui.allocate_exact_size(
                    vec2(width + inset * 2.0, ITEM_HEIGHT + 4.0),
                    if *enabled {
                        Sense::click()
                    } else {
                        Sense::hover()
                    },
                );
                let r = Rect::from_min_max(
                    pos2(outer.left() + inset, outer.top() + 2.0),
                    pos2(outer.right() - inset, outer.bottom() - 2.0),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, *enabled, label.as_str())
                });
                let hovered = *enabled && response.hovered();
                let opacity = if *enabled { 1.0 } else { 0.35 };
                let fg = if hovered {
                    colors.accent_foreground()
                } else {
                    style::fade(colors.text(), opacity)
                };
                let secondary = if hovered {
                    colors.accent_foreground()
                } else {
                    style::fade(colors.text_secondary(), opacity)
                };
                let painter = ui.painter().clone();
                if hovered {
                    style::paint_accent(&painter, colors, r, 6.0, 1.0);
                }
                if let Some(icon) = icon {
                    style::paint_icon(
                        ui,
                        &painter,
                        pos2(r.left() + PAD_X + 8.0, r.center().y),
                        *icon,
                        16.0,
                        fg,
                    );
                }
                let text_left = r.left() + PAD_X + ICON_COLUMN;
                style::paint_text(
                    ui,
                    &painter,
                    Rect::from_min_max(
                        pos2(text_left, r.top()),
                        pos2(r.right() - PAD_X, r.bottom()),
                    ),
                    Align2::LEFT_CENTER,
                    label,
                    style::FONT_SIZE,
                    Weight::Normal,
                    fg,
                );
                if let Some(g) = gesture {
                    style::paint_text(
                        ui,
                        &painter,
                        Rect::from_min_max(
                            pos2(text_left, r.top()),
                            pos2(r.right() - PAD_X, r.bottom()),
                        ),
                        Align2::RIGHT_CENTER,
                        g,
                        style::FONT_SIZE,
                        Weight::Normal,
                        secondary,
                    );
                }
                if !children.is_empty() {
                    style::paint_icon(
                        ui,
                        &painter,
                        pos2(r.right() - PAD_X - 4.0, r.center().y),
                        modlight_core::icons::CHEVRON_RIGHT,
                        12.0,
                        secondary,
                    );
                    let sub = egui::Popup::menu(&response)
                        .id(ui.id().with(("submenu", index)))
                        .align(egui::RectAlign::RIGHT_START)
                        .gap(2.0)
                        .frame(style::menu_frame(colors))
                        .open_memory(if response.hovered() {
                            Some(egui::SetOpenCommand::Bool(true))
                        } else {
                            None
                        })
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| entries(ui, colors, children, 6.0));
                    if let Some(inner) = sub
                        && let Some(path) = inner.inner
                    {
                        let mut full = vec![index];
                        full.extend(path);
                        picked = Some(full);
                    }
                } else if response.clicked() {
                    picked = Some(vec![index]);
                }
            }
        }
    }
    picked
}
