//! Controls the phase-3 windows add to `widgets`: the numeric box (`NumericUpDown`),
//! the radio row and the field row used by the settings pages.

use egui::{Align2, FontFamily, FontId, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::theme::{Colors, Weight};
use crate::widgets::{layout, paint_icon, trimmed};

/// `NumericUpDown`: a text box with up and down spinners on the right. The value is kept
/// between `min` and `max`, shown with `decimals` places in the current culture. Returns
/// whether the value changed.
#[allow(clippy::too_many_arguments)]
pub fn number_box(
    ui: &mut Ui,
    colors: &Colors,
    id: impl std::hash::Hash + std::fmt::Debug,
    width: f32,
    height: f32,
    value: &mut f64,
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
    enabled: bool,
) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    number_box_at(
        ui, colors, id, rect, value, min, max, step, decimals, enabled,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn number_box_at(
    ui: &mut Ui,
    colors: &Colors,
    id: impl std::hash::Hash + std::fmt::Debug,
    rect: Rect,
    value: &mut f64,
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
    enabled: bool,
) -> bool {
    let id = egui::Id::new("number-box").with(id);
    let culture = modlight_core::culture::Culture::current();
    let format = |v: f64| culture.fixed(v, decimals);
    let text_id = id.with("text");
    let focused = ui.memory(|m| m.has_focus(text_id));
    let mut text: String = ui
        .data(|d| d.get_temp::<String>(id))
        .filter(|_| focused)
        .unwrap_or_else(|| format(*value));
    let painter = ui.painter().clone();
    let fill = if !enabled {
        colors.control_background_disabled()
    } else if ui.rect_contains_pointer(rect) && !focused {
        colors.control_background_hover()
    } else {
        colors.control_background()
    };
    painter.rect_filled(rect, 8.0, fill);
    painter.rect_stroke(
        rect,
        8.0,
        if focused {
            Stroke::new(2.0, colors.focus())
        } else {
            Stroke::new(1.0, colors.border())
        },
        StrokeKind::Inside,
    );
    let spinner_w = 24.0;
    let inner = Rect::from_min_max(
        pos2(rect.left() + 8.0, rect.top()),
        pos2(rect.right() - spinner_w - 4.0, rect.bottom()),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let response = child.add_enabled(
        enabled,
        egui::TextEdit::singleline(&mut text)
            .id(text_id)
            .frame(egui::Frame::NONE)
            .font(FontId::new(13.0, FontFamily::Proportional))
            .text_color(if enabled {
                colors.text()
            } else {
                colors.text_secondary()
            })
            .vertical_align(egui::Align::Center)
            .margin(egui::Margin::ZERO)
            .desired_width(inner.width()),
    );
    let mut changed = false;
    let clamp = |v: f64| {
        let scale = 10f64.powi(decimals as i32);
        ((v * scale).round() / scale).clamp(min, max)
    };
    if response.changed() {
        ui.data_mut(|d| d.insert_temp(id, text.clone()));
        let parsed = text
            .trim()
            .replace(culture.decimal, ".")
            .replace(['\u{a0}', ' '], "");
        if let Ok(v) = parsed.parse::<f64>() {
            let v = clamp(v);
            if (v - *value).abs() > f64::EPSILON {
                *value = v;
                changed = true;
            }
        }
    }
    if response.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    // The spinners.
    let up = Rect::from_min_max(
        pos2(rect.right() - spinner_w - 2.0, rect.top() + 2.0),
        pos2(rect.right() - 2.0, rect.center().y),
    );
    let down = Rect::from_min_max(
        pos2(up.left(), rect.center().y),
        pos2(up.right(), rect.bottom() - 2.0),
    );
    for (r, glyph, delta) in [
        (up, modlight_core::icons::CHEVRON_UP, step),
        (down, modlight_core::icons::CHEVRON_DOWN, -step),
    ] {
        let response = ui.interact(
            r,
            id.with(glyph),
            if enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        if response.hovered() && enabled {
            painter.rect_filled(r, 4.0, colors.control_background_hover());
        }
        paint_icon(
            ui,
            &painter,
            r.center(),
            glyph,
            12.0,
            colors.text_secondary(),
        );
        if response.clicked() {
            let v = clamp(*value + delta);
            if (v - *value).abs() > f64::EPSILON {
                *value = v;
                changed = true;
            }
        }
    }
    if response.hovered() && enabled {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.5 {
            let v = clamp(*value + step * scroll.signum() as f64);
            if (v - *value).abs() > f64::EPSILON {
                *value = v;
                changed = true;
            }
        }
    }
    changed
}

/// A radio button with its label (`RadioButton`). Returns the click.
pub fn radio_row(
    ui: &mut Ui,
    colors: &Colors,
    checked: bool,
    label: &str,
    enabled: bool,
) -> Response {
    let galley = layout(
        ui,
        trimmed(
            label,
            13.0,
            Weight::SemiBold,
            if enabled {
                colors.text()
            } else {
                colors.text_secondary()
            },
            600.0,
            1,
        ),
    );
    let (rect, response) = ui.allocate_exact_size(
        vec2(20.0 + 8.0 + galley.size().x, 32.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let center = pos2(rect.left() + 10.0, rect.center().y);
    crate::dialogs::after_capture::radio(
        ui,
        colors,
        center,
        checked,
        response.hovered() && enabled,
    );
    ui.painter().galley(
        pos2(rect.left() + 28.0, rect.center().y - galley.size().y / 2.0),
        galley,
        if enabled {
            colors.text()
        } else {
            colors.text_secondary()
        },
    );
    response
}

/// A label in its own colour and weight, left-aligned in a rect.
pub fn label_at(ui: &Ui, rect: Rect, text: &str, weight: Weight, color: egui::Color32) {
    crate::widgets::paint_text(
        ui,
        ui.painter(),
        rect,
        Align2::LEFT_CENTER,
        text,
        13.0,
        weight,
        color,
    );
}
