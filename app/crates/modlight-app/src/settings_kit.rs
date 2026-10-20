//! The pieces every settings page is built from (`TaskSettingsPageBuilder`'s `Card`,
//! `Row`, `Check`, `Number`, `EnumCombo`, `Text`, `Hint`, and the application view's
//! rows), bound to the settings document by JSON paths so a page edits exactly the
//! members upstream's does and leaves every other member as it is.

use egui::{Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Map, Value, json};

use crate::theme::{Colors, Weight};
use crate::widgets::{Content, Field, Look, button, checkbox_weighted, combo, layout, wrapped};

/// A settings object a page reads and writes (the default task settings, a hotkey's
/// task settings, or the application settings), and whether anything changed.
pub struct Binder<'a> {
    pub root: &'a mut Map<String, Value>,
    pub changed: bool,
}

impl<'a> Binder<'a> {
    pub fn new(root: &'a mut Map<String, Value>) -> Binder<'a> {
        Binder {
            root,
            changed: false,
        }
    }

    pub fn get(&self, path: &[&str]) -> Option<&Value> {
        let (first, rest) = path.split_first()?;
        let mut value = self.root.get(*first)?;
        for key in rest {
            value = value.get(*key)?;
        }
        Some(value)
    }

    /// Write a member, creating the objects on the way (`null` ones too: a hotkey's
    /// task settings keep `null` for the groups they take from the defaults).
    pub fn set(&mut self, path: &[&str], value: Value) {
        let Some((last, parents)) = path.split_last() else {
            return;
        };
        let mut object = &mut *self.root;
        for key in parents {
            let entry = object
                .entry((*key).to_string())
                .or_insert_with(|| json!({}));
            if !entry.is_object() {
                *entry = json!({});
            }
            object = entry.as_object_mut().unwrap();
        }
        if object.get(*last) != Some(&value) {
            object.insert((*last).to_string(), value);
            self.changed = true;
        }
    }

    pub fn bool(&self, path: &[&str], default: bool) -> bool {
        self.get(path).and_then(Value::as_bool).unwrap_or(default)
    }

    pub fn f64(&self, path: &[&str], default: f64) -> f64 {
        self.get(path).and_then(Value::as_f64).unwrap_or(default)
    }

    pub fn str(&self, path: &[&str], default: &str) -> String {
        self.get(path)
            .and_then(Value::as_str)
            .unwrap_or(default)
            .to_string()
    }

    /// An enum member, read by name (or index, as Newtonsoft accepts).
    pub fn name<'n>(&self, path: &[&str], names: &[&'n str], default: &'n str) -> &'n str {
        match self.get(path) {
            Some(Value::String(text)) => names
                .iter()
                .copied()
                .find(|n| n.eq_ignore_ascii_case(text))
                .unwrap_or(default),
            Some(Value::Number(n)) => n
                .as_u64()
                .and_then(|i| names.get(i as usize).copied())
                .unwrap_or(default),
            _ => default,
        }
    }
}

/// `Border.section-card`: popup fill, 8 px corners, 16,14 padding, 12 below; a title
/// when one is given; dimmed and inert when `enabled` is false (`EnabledCard`).
pub fn card(ui: &mut Ui, colors: &Colors, title: &str, enabled: bool, add: impl FnOnce(&mut Ui)) {
    let width = ui.available_width();
    egui::Frame::new()
        .fill(colors.background_popup())
        .stroke(Stroke::new(1.0, colors.border()))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_width(width - 34.0);
            ui.spacing_mut().item_spacing.y = 4.0;
            if !enabled {
                ui.disable();
            }
            if !title.is_empty() {
                // `section-title`: 15 SemiBold Text, 4 above, 3 below.
                ui.add_space(4.0);
                crate::widgets::text(ui, title, 15.0, Weight::SemiBold, colors.text());
                ui.add_space(3.0);
            }
            add(ui);
        });
    ui.add_space(12.0);
}

/// `Border.override-card`: the panel colour, the accent border, one check box.
pub fn override_card(ui: &mut Ui, colors: &Colors, label: &str, value: &mut bool) -> bool {
    let width = ui.available_width();
    let mut changed = false;
    egui::Frame::new()
        .fill(colors.background_panel())
        .stroke(Stroke::new(1.0, colors.accent_start()))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(16, 10))
        .show(ui, |ui| {
            ui.set_width(width - 34.0);
            changed =
                checkbox_weighted(ui, colors, value, Some(label), true, Weight::Normal).changed();
        });
    ui.add_space(12.0);
    changed
}

/// `TextBlock.hint`: 12 px, Text.Secondary, wrapped, 4 above and below.
pub fn hint(ui: &mut Ui, colors: &Colors, text: &str) {
    ui.add_space(4.0);
    crate::widgets::text_wrapped(ui, text, 12.0, Weight::Normal, colors.text_secondary());
    ui.add_space(4.0);
}

/// A label column of `label_w` (wrapping, 13 Normal, Text.Secondary) and an editor.
pub fn row<R>(
    ui: &mut Ui,
    colors: &Colors,
    label_w: f32,
    label: &str,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let mut out = None;
    ui.horizontal(|ui| {
        let g = layout(
            ui,
            wrapped(
                label,
                13.0,
                Weight::Normal,
                colors.text_secondary(),
                label_w,
            ),
        );
        let h = g.size().y.max(32.0);
        let (r, _) = ui.allocate_exact_size(vec2(label_w, h), Sense::hover());
        let color = if ui.is_enabled() {
            colors.text_secondary()
        } else {
            colors.text_secondary().gamma_multiply(0.6)
        };
        ui.painter()
            .galley(pos2(r.left(), r.center().y - g.size().y / 2.0), g, color);
        ui.add_space(10.0);
        out = Some(add(ui));
    });
    out.unwrap()
}

/// `CheckBox.setting` bound to a member.
pub fn check(
    ui: &mut Ui,
    colors: &Colors,
    b: &mut Binder,
    path: &[&str],
    default: bool,
    label: &str,
) -> bool {
    let mut value = b.bool(path, default);
    let enabled = ui.is_enabled();
    if checkbox_weighted(ui, colors, &mut value, Some(label), enabled, Weight::Normal).changed() {
        b.set(path, json!(value));
    }
    value
}

/// A check box over an inverted member (`UseMultiRegionMode` is `!QuickCrop`).
pub fn check_inverted(
    ui: &mut Ui,
    colors: &Colors,
    b: &mut Binder,
    path: &[&str],
    default: bool,
    label: &str,
) {
    let mut value = !b.bool(path, default);
    let enabled = ui.is_enabled();
    if checkbox_weighted(ui, colors, &mut value, Some(label), enabled, Weight::Normal).changed() {
        b.set(path, json!(!value));
    }
}

/// `NumericUpDown.form-control` (130 wide) bound to a number member.
#[allow(clippy::too_many_arguments)]
pub fn number(
    ui: &mut Ui,
    colors: &Colors,
    b: &mut Binder,
    path: &[&str],
    default: f64,
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
) {
    let mut value = b.f64(path, default);
    let enabled = ui.is_enabled();
    let id = path.join(".");
    if crate::controls::number_box(
        ui, colors, id, 130.0, 32.0, &mut value, min, max, step, decimals, enabled,
    ) {
        if decimals == 0 {
            b.set(path, json!(value.round() as i64));
        } else {
            b.set(path, json!(value));
        }
    }
}

/// A number box over a plain value (not a member).
#[allow(clippy::too_many_arguments)]
pub fn number_value(
    ui: &mut Ui,
    colors: &Colors,
    id: &str,
    value: &mut f64,
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
) -> bool {
    let enabled = ui.is_enabled();
    crate::controls::number_box(
        ui, colors, id, 130.0, 32.0, value, min, max, step, decimals, enabled,
    )
}

/// `ComboBox.form-control` (at least 250 wide) over an enum member, by name.
pub fn enum_combo(
    ui: &mut Ui,
    colors: &Colors,
    b: &mut Binder,
    path: &[&str],
    options: &[(&'static str, String)],
    default: &'static str,
) {
    let names: Vec<&str> = options.iter().map(|(n, _)| *n).collect();
    let current = b.name(path, &names, default).to_string();
    let list: Vec<(String, String)> = options
        .iter()
        .map(|(n, l)| (n.to_string(), l.clone()))
        .collect();
    let width = combo_width(ui, &list);
    if ui.is_enabled() {
        if let Some(picked) = combo(ui, colors, &path.join("."), width, &current, &list) {
            b.set(path, json!(picked));
        }
    } else {
        let _ = combo(ui, colors, &path.join("."), width, &current, &[]);
    }
}

/// The widest label decides (`MinWidth="250"`, `HorizontalAlignment="Left"`).
pub fn combo_width(ui: &Ui, options: &[(String, String)]) -> f32 {
    let widest = options
        .iter()
        .map(|(_, l)| {
            layout(
                ui,
                crate::widgets::job(l, 13.0, Weight::Normal, egui::Color32::WHITE),
            )
            .size()
            .x
        })
        .fold(0.0_f32, f32::max);
    (widest + 48.0).max(250.0)
}

/// `TextBox.form-control` bound to a string member; returns whether it changed.
pub fn text(ui: &mut Ui, colors: &Colors, b: &mut Binder, path: &[&str], default: &str) -> bool {
    let mut value = b.str(path, default);
    let enabled = ui.is_enabled();
    let id = ui.make_persistent_id(("settings-text", path.join(".")));
    let response = Field {
        height: 32.0,
        padding_x: 8.0,
        enabled,
        id: Some(id),
        ..Field::new(&mut value)
    }
    .show(ui, colors);
    if response.changed() {
        b.set(path, json!(value));
        return true;
    }
    false
}

/// `Button.compact`: 32 tall, 10,4 padding, the label in Text.
pub fn compact_button(ui: &mut Ui, colors: &Colors, label: &str) -> egui::Response {
    let look = Look {
        string_text: Some(colors.text()),
        ..Look::button(colors).height(32.0).padding(10.0)
    };
    let enabled = ui.is_enabled();
    button(ui, colors, &look, Content::label(label), enabled)
}

/// A thin rule between groups of a card.
pub fn rule(ui: &mut Ui, colors: &Colors) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
    ui.painter().hline(
        r.left()..=r.right(),
        r.center().y,
        Stroke::new(1.0, colors.border()),
    );
}

/// The list frame of `ListBox.settings-list` (main background, 1 px border), with an
/// empty state drawn inside when it holds nothing (`WithEmptyState`).
pub fn list_frame(
    ui: &mut Ui,
    colors: &Colors,
    min_height: f32,
    empty: Option<(&str, &str)>,
    add: impl FnOnce(&mut Ui),
) {
    let width = ui.available_width();
    let response = egui::Frame::new()
        .fill(colors.background_main())
        .stroke(Stroke::new(1.0, colors.border()))
        .inner_margin(2)
        .show(ui, |ui| {
            ui.set_width(width - 6.0);
            ui.set_min_height(min_height - 6.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui);
        });
    if let Some((title, example)) = empty {
        let rect: Rect = response.response.rect;
        let max_w = 380.0_f32.min(rect.width() - 20.0);
        let t = layout(ui, {
            let mut job = wrapped(title, 13.0, Weight::Normal, colors.text_secondary(), max_w);
            job.halign = egui::Align::Center;
            job
        });
        let e = layout(ui, {
            let mut job = wrapped(
                example,
                12.0,
                Weight::Normal,
                colors.text_secondary(),
                max_w,
            );
            job.halign = egui::Align::Center;
            job
        });
        let total = t.size().y + 6.0 + e.size().y;
        let top = rect.center().y - total / 2.0;
        let th = t.size().y;
        ui.painter()
            .galley(pos2(rect.center().x, top), t, colors.text_secondary());
        ui.painter().galley(
            pos2(rect.center().x, top + th + 6.0),
            e,
            colors.text_secondary(),
        );
    }
}

/// One selectable row of a settings list; returns the click.
pub fn list_row(
    ui: &mut Ui,
    colors: &Colors,
    selected: bool,
    height: f32,
    add: impl FnOnce(&mut Ui, Rect),
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    if selected {
        ui.painter()
            .rect_filled(rect, 4.0, colors.control_background_hover());
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, 4.0, colors.control_background());
    }
    add(ui, rect);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_binder_writes_into_null_and_missing_groups() {
        let mut root = json!({ "CaptureSettings": null, "Keep": 1 })
            .as_object()
            .unwrap()
            .clone();
        let mut b = Binder::new(&mut root);
        assert!(b.bool(&["CaptureSettings", "ShowCursor"], true));
        b.set(
            &["CaptureSettings", "SurfaceOptions", "ShowMagnifier"],
            json!(false),
        );
        assert!(b.changed);
        assert_eq!(
            b.get(&["CaptureSettings", "SurfaceOptions", "ShowMagnifier"]),
            Some(&json!(false))
        );
        assert_eq!(
            b.name(&["ImageSettings", "ImageFormat"], &["PNG", "JPEG"], "PNG"),
            "PNG"
        );
        b.set(&["ImageSettings", "ImageFormat"], json!("JPEG"));
        assert_eq!(
            b.name(&["ImageSettings", "ImageFormat"], &["PNG", "JPEG"], "PNG"),
            "JPEG"
        );
        let mut b2 = Binder::new(&mut root);
        b2.set(&["Keep"], json!(1));
        assert!(!b2.changed, "writing the same value is not a change");
    }
}
