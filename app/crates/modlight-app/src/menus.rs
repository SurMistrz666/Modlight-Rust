//! Context and strip menus (`MainMenuEntry` drawn through `BuildMenuControls`): the
//! compact items of `ControlStyles.axaml`, 28 tall, the accent icon, check marks for
//! toggles, separators, disabled headers and submenus.
//!
//! A menu is data: a list of [`Entry`] built when the menu opens, each carrying the
//! [`Command`] it runs. The command is handed back to the caller instead of being run
//! inside the menu, so nothing in a menu ever holds the app state.

use egui::{Color32, CursorIcon, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::commands::Command;
use crate::theme::{Colors, Weight};
use crate::widgets::{icon_galley, job, layout, paint_icon};

/// Whether a menu item toggles, and how its mark is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    None,
    Check(bool),
    Radio(bool),
}

/// One line of a menu.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Item {
        label: String,
        icon: Option<char>,
        enabled: bool,
        toggle: Toggle,
        command: Option<Command>,
    },
    Submenu {
        label: String,
        icon: Option<char>,
        enabled: bool,
        children: Vec<Entry>,
    },
    Separator,
}

impl Entry {
    pub fn item(label: impl Into<String>, icon: char, command: Command) -> Entry {
        Entry::Item {
            label: label.into(),
            icon: Some(icon),
            enabled: true,
            toggle: Toggle::None,
            command: Some(command),
        }
    }

    pub fn item_if(label: impl Into<String>, icon: char, command: Command, enabled: bool) -> Entry {
        Entry::Item {
            label: label.into(),
            icon: Some(icon),
            enabled,
            toggle: Toggle::None,
            command: Some(command),
        }
    }

    /// A line that only says something (`isEnabled: false`, no action).
    pub fn header(label: impl Into<String>, icon: Option<char>) -> Entry {
        Entry::Item {
            label: label.into(),
            icon,
            enabled: false,
            toggle: Toggle::None,
            command: None,
        }
    }

    pub fn check(label: impl Into<String>, icon: char, checked: bool, command: Command) -> Entry {
        Entry::Item {
            label: label.into(),
            icon: Some(icon),
            enabled: true,
            toggle: Toggle::Check(checked),
            command: Some(command),
        }
    }

    pub fn submenu(
        label: impl Into<String>,
        icon: char,
        enabled: bool,
        children: Vec<Entry>,
    ) -> Entry {
        Entry::Submenu {
            label: label.into(),
            icon: Some(icon),
            enabled,
            children,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Entry::Item { label, .. } | Entry::Submenu { label, .. } => label,
            Entry::Separator => "",
        }
    }
}

/// The menu surface: `MenuFlyoutPresenterBackground`, a 1 px border, 10 px corners,
/// 6,2 padding.
pub fn frame(colors: &Colors) -> egui::Frame {
    egui::Frame::new()
        .fill(colors.background_menu())
        .stroke(Stroke::new(1.0, colors.border()))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(6, 4))
        .shadow(egui::Shadow {
            offset: [0, 6],
            blur: 18,
            spread: 0,
            color: Color32::from_black_alpha(0x48),
        })
}

pub const ITEM_HEIGHT: f32 = 28.0;
const PAD_X: f32 = 7.0;
const ICON_COLUMN: f32 = 16.0 + 12.0;
const CHEVRON: f32 = 22.0;

/// The width the widest line needs.
pub fn natural_width(ui: &Ui, entries: &[Entry]) -> f32 {
    let toggles = entries
        .iter()
        .any(|e| matches!(e, Entry::Item { toggle, .. } if *toggle != Toggle::None));
    let mut widest: f32 = 120.0;
    for entry in entries {
        if let Entry::Separator = entry {
            continue;
        }
        let text = layout(ui, job(entry.label(), 13.0, Weight::Normal, Color32::WHITE))
            .size()
            .x;
        let mut width = PAD_X * 2.0 + ICON_COLUMN + text + CHEVRON;
        if toggles {
            width += ICON_COLUMN;
        }
        widest = widest.max(width);
    }
    widest.ceil()
}

/// Draw the entries into the current (menu) Ui. Returns the command picked, if any.
pub fn entries(ui: &mut Ui, colors: &Colors, entries: &[Entry]) -> Option<Command> {
    let width = natural_width(ui, entries);
    ui.set_min_width(width);
    ui.set_max_width(width.max(ui.min_rect().width()));
    let toggles = entries
        .iter()
        .any(|e| matches!(e, Entry::Item { toggle, .. } if *toggle != Toggle::None));
    let mut picked = None;
    let mut previous_separator = true;
    for (index, entry) in entries.iter().enumerate() {
        match entry {
            Entry::Separator => {
                // Never two rules in a row, never one first or last.
                if previous_separator || index + 1 == entries.len() {
                    continue;
                }
                let (rect, _) = ui.allocate_exact_size(vec2(width, 5.0), Sense::hover());
                ui.painter().hline(
                    (rect.left() + 6.0)..=(rect.right() - 6.0),
                    rect.center().y,
                    Stroke::new(1.0, colors.separator()),
                );
                previous_separator = true;
            }
            Entry::Item {
                label,
                icon,
                enabled,
                toggle,
                command,
            } => {
                previous_separator = false;
                let response = item_row(
                    ui, colors, width, label, *icon, *enabled, *toggle, toggles, false,
                );
                if *enabled && response.clicked() {
                    picked = command.clone();
                    ui.close();
                }
            }
            Entry::Submenu {
                label,
                icon,
                enabled,
                children,
            } => {
                previous_separator = false;
                let response = item_row(
                    ui,
                    colors,
                    width,
                    label,
                    *icon,
                    *enabled,
                    Toggle::None,
                    toggles,
                    true,
                );
                if *enabled {
                    let inner = egui::containers::menu::SubMenu::new().show(ui, &response, |ui| {
                        ui.style_mut().visuals.window_fill = colors.background_menu();
                        self::entries(ui, colors, children)
                    });
                    if let Some(command) = inner.and_then(|r| r.inner) {
                        picked = Some(command);
                        ui.close();
                    }
                }
            }
        }
    }
    picked
}

#[allow(clippy::too_many_arguments)]
fn item_row(
    ui: &mut Ui,
    colors: &Colors,
    width: f32,
    label: &str,
    icon: Option<char>,
    enabled: bool,
    toggle: Toggle,
    toggles: bool,
    submenu: bool,
) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, ITEM_HEIGHT), sense);
    let hovered = enabled && (response.hovered() || (submenu && response.has_focus()));
    let painter = ui.painter().clone();
    if hovered {
        painter.rect_filled(rect, 6.0, colors.control_background_hover());
    }
    let alpha = if enabled { 1.0 } else { 0.45 };
    let text_color = colors.text().gamma_multiply(alpha);
    let icon_color = colors.accent_start().gamma_multiply(alpha);
    let mut x = rect.left() + PAD_X;
    let cy = rect.center().y;
    if toggles {
        match toggle {
            Toggle::Check(true) => {
                paint_icon(
                    ui,
                    &painter,
                    pos2(x + 8.0, cy),
                    modlight_core::icons::CHECK,
                    14.0,
                    text_color,
                );
            }
            Toggle::Radio(true) => {
                painter.circle_filled(pos2(x + 8.0, cy), 3.5, text_color);
            }
            _ => {}
        }
        x += ICON_COLUMN;
    }
    if let Some(glyph) = icon {
        let galley = icon_galley(ui, glyph, 16.0, icon_color);
        painter.galley(
            pos2(x + 8.0 - galley.size().x / 2.0, cy - galley.size().y / 2.0),
            galley,
            icon_color,
        );
    }
    x += ICON_COLUMN;
    let max_text = (rect.right() - PAD_X - if submenu { CHEVRON } else { 0.0 } - x).max(10.0);
    let galley = layout(
        ui,
        crate::widgets::trimmed(label, 13.0, Weight::Normal, text_color, max_text, 1),
    );
    painter.galley(pos2(x, cy - galley.size().y / 2.0), galley, text_color);
    if submenu {
        paint_icon(
            ui,
            &painter,
            pos2(rect.right() - PAD_X - 6.0, cy),
            modlight_core::icons::CHEVRON_RIGHT,
            13.0,
            text_color,
        );
    }
    if enabled {
        response.on_hover_cursor(CursorIcon::Default)
    } else {
        response
    }
}

/// Draw a whole menu as a panel at `at` (QA screens, which cannot open popups).
pub fn panel(ui: &mut Ui, colors: &Colors, at: egui::Pos2, list: &[Entry]) -> Rect {
    let width = natural_width(ui, list);
    let builder = egui::UiBuilder::new()
        .max_rect(Rect::from_min_size(at, vec2(width + 16.0, 2000.0)))
        .layout(egui::Layout::top_down(egui::Align::Min));
    let mut child = ui.new_child(builder);
    let response = frame(colors).show(&mut child, |ui| {
        let _ = entries(ui, colors, list);
    });
    response.response.rect
}

/// A menu that a click on `anchor` opens under it, right edges aligned
/// (`PlacementMode.BottomEdgeAlignedRight`); returns the command picked.
pub fn popup_below(
    colors: &Colors,
    anchor: &egui::Response,
    list: impl FnOnce() -> Vec<Entry>,
) -> Option<Command> {
    egui::Popup::menu(anchor)
        .frame(frame(colors))
        .align(egui::RectAlign::BOTTOM_END)
        .gap(4.0)
        .show(|ui| {
            let list = list();
            entries(ui, colors, &list)
        })
        .and_then(|r| r.inner)
}

/// A menu kept open by the caller (`open`), anchored under `anchor`.
pub fn popup_open(
    colors: &Colors,
    anchor: &egui::Response,
    open: &mut bool,
    list: impl FnOnce() -> Vec<Entry>,
) -> Option<Command> {
    let mut is_open = *open;
    let result = egui::Popup::from_response(anchor)
        .open_bool(&mut is_open)
        .kind(egui::PopupKind::Menu)
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .style(egui::containers::menu::menu_style)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .frame(frame(colors))
        .align(egui::RectAlign::BOTTOM_END)
        .gap(4.0)
        .show(|ui| {
            let list = list();
            entries(ui, colors, &list)
        })
        .and_then(|r| r.inner);
    *open = is_open;
    result
}

/// A context menu on secondary click of `response`.
pub fn context(
    colors: &Colors,
    response: &egui::Response,
    list: impl FnOnce() -> Vec<Entry>,
) -> Option<Command> {
    egui::Popup::context_menu(response)
        .frame(frame(colors))
        .show(|ui| {
            let list = list();
            entries(ui, colors, &list)
        })
        .and_then(|r| r.inner)
}
