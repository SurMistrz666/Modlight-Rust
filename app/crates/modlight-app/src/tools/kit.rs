//! The pieces every tool window is built from, after `ToolStyles.axaml` and the global
//! control styles: the window fill, `Border.tool-panel`, `TextBlock.section-heading` and
//! `row-label`, the 40 px buttons with an icon and a label, text boxes, drop-downs,
//! number boxes, check boxes and pictures, all placed into rectangles the way Avalonia
//! grids place them.

use egui::{
    Align2, Color32, FontFamily, FontId, Rect, Response, Sense, Stroke, StrokeKind, TextureHandle,
    Ui, Vec2, pos2, vec2,
};

use crate::theme::{Colors, Weight};
use crate::widgets::{self, Content, Look, layout, paint_icon, trimmed, wrapped};

/// A track of a grid: fixed pixels, a share of what is left, or what the content needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Track {
    Px(f32),
    Star(f32),
}

/// Split `total` into tracks with `spacing` between them (`ColumnDefinitions`).
fn split(total: f32, tracks: &[Track], spacing: f32) -> Vec<f32> {
    let fixed: f32 = tracks
        .iter()
        .map(|t| match t {
            Track::Px(px) => *px,
            Track::Star(_) => 0.0,
        })
        .sum::<f32>()
        + spacing * (tracks.len().saturating_sub(1)) as f32;
    let stars: f32 = tracks
        .iter()
        .map(|t| match t {
            Track::Star(s) => *s,
            Track::Px(_) => 0.0,
        })
        .sum();
    let free = (total - fixed).max(0.0);
    tracks
        .iter()
        .map(|t| match t {
            Track::Px(px) => *px,
            Track::Star(s) => {
                if stars > 0.0 {
                    free * s / stars
                } else {
                    0.0
                }
            }
        })
        .collect()
}

/// The columns of `rect`.
pub fn columns(rect: Rect, tracks: &[Track], spacing: f32) -> Vec<Rect> {
    let widths = split(rect.width(), tracks, spacing);
    let mut x = rect.left();
    widths
        .into_iter()
        .map(|w| {
            let r = Rect::from_min_size(pos2(x, rect.top()), vec2(w, rect.height()));
            x += w + spacing;
            r
        })
        .collect()
}

/// The rows of `rect`.
pub fn rows(rect: Rect, tracks: &[Track], spacing: f32) -> Vec<Rect> {
    let heights = split(rect.height(), tracks, spacing);
    let mut y = rect.top();
    heights
        .into_iter()
        .map(|h| {
            let r = Rect::from_min_size(pos2(rect.left(), y), vec2(rect.width(), h));
            y += h + spacing;
            r
        })
        .collect()
}

/// `Margin`: the rectangle inside.
pub fn inset(rect: Rect, margin: f32) -> Rect {
    rect.shrink(margin)
}

/// Fill the window with `ShareX.Brush.Background.Main`.
pub fn fill(ui: &Ui, colors: &Colors) {
    ui.painter()
        .rect_filled(ui.max_rect().expand(1.0), 0.0, colors.background_main());
}

/// `Border.tool-panel`: the panel colour, a 1 px border, 4 px corners.
pub fn panel(ui: &Ui, colors: &Colors, rect: Rect) {
    ui.painter()
        .rect_filled(rect, 4.0, colors.background_panel());
    ui.painter().rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
}

/// `TextBlock.section-heading`: 15 px SemiBold in Text, centred on `rect`'s height.
pub fn heading(ui: &Ui, colors: &Colors, rect: Rect, text: &str) -> Rect {
    widgets::paint_text(
        ui,
        ui.painter(),
        rect,
        Align2::LEFT_CENTER,
        text,
        15.0,
        Weight::SemiBold,
        colors.text(),
    )
}

/// The height of a section heading line.
pub const HEADING_HEIGHT: f32 = 20.0;

/// `TextBlock.row-label` (and `FontWeight="Normal"` labels): Normal, Text.Secondary.
pub fn label(ui: &Ui, colors: &Colors, rect: Rect, text: &str) -> Rect {
    widgets::paint_text(
        ui,
        ui.painter(),
        rect,
        Align2::LEFT_CENTER,
        text,
        13.0,
        Weight::Normal,
        colors.text_secondary(),
    )
}

/// A label in its own alignment, colour and weight.
#[allow(clippy::too_many_arguments)]
pub fn text_at(
    ui: &Ui,
    rect: Rect,
    align: Align2,
    text: &str,
    size: f32,
    weight: Weight,
    color: Color32,
) -> Rect {
    widgets::paint_text(ui, ui.painter(), rect, align, text, size, weight, color)
}

/// A wrapped paragraph from the top left of `rect`; returns its height.
pub fn paragraph(
    ui: &Ui,
    rect: Rect,
    text: &str,
    size: f32,
    weight: Weight,
    color: Color32,
) -> f32 {
    let galley = layout(
        ui,
        wrapped(text, size, weight, color, rect.width().max(1.0)),
    );
    let height = galley.size().y;
    ui.painter().galley(rect.min, galley, color);
    height
}

/// The width a label needs.
pub fn text_width(ui: &Ui, text: &str, size: f32, weight: Weight) -> f32 {
    layout(ui, widgets::job(text, size, weight, Color32::WHITE))
        .size()
        .x
        .ceil()
}

/// The global `Button` (40 tall, 10 px padding) with an optional icon.
pub fn button_look(colors: &Colors) -> Look {
    Look::button(colors)
}

/// The natural width of a button.
pub fn button_width(ui: &Ui, colors: &Colors, icon: Option<char>, label: Option<&str>) -> f32 {
    widgets::button_width(ui, &button_look(colors), Content { icon, label })
}

/// A button filling `rect`.
pub fn button_at(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    icon: Option<char>,
    label: Option<&str>,
    enabled: bool,
) -> Response {
    let look = button_look(colors);
    let mut child = child(ui, rect);
    widgets::button_at(
        &mut child,
        colors,
        &look,
        Content { icon, label },
        enabled,
        rect,
    )
}

/// The accent button (`Classes="primary"` / `IsDefault`).
pub fn primary_at(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    icon: Option<char>,
    label: Option<&str>,
    enabled: bool,
) -> Response {
    let look = Look::primary(colors)
        .width(rect.width())
        .height(rect.height());
    let mut child = child(ui, rect);
    widgets::button_at(
        &mut child,
        colors,
        &look,
        Content { icon, label },
        enabled,
        rect,
    )
}

/// Buttons laid right to left from `rect`'s right edge, 8 apart; returns their rects
/// in the order given.
pub fn right_aligned(
    ui: &Ui,
    colors: &Colors,
    rect: Rect,
    items: &[(Option<char>, Option<&str>)],
    spacing: f32,
) -> Vec<Rect> {
    let widths: Vec<f32> = items
        .iter()
        .map(|(icon, label)| button_width(ui, colors, *icon, *label))
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + spacing * (items.len().saturating_sub(1)) as f32;
    let mut x = rect.right() - total;
    widths
        .into_iter()
        .map(|w| {
            let r = Rect::from_min_size(pos2(x, rect.center().y - 20.0), vec2(w, 40.0));
            x += w + spacing;
            r
        })
        .collect()
}

/// Buttons laid left to right from `rect`'s left edge.
pub fn left_aligned(
    ui: &Ui,
    colors: &Colors,
    rect: Rect,
    items: &[(Option<char>, Option<&str>)],
    spacing: f32,
) -> Vec<Rect> {
    let mut x = rect.left();
    items
        .iter()
        .map(|(icon, label)| {
            let w = button_width(ui, colors, *icon, *label);
            let r = Rect::from_min_size(pos2(x, rect.center().y - 20.0), vec2(w, 40.0));
            x += w + spacing;
            r
        })
        .collect()
}

/// A child `Ui` confined to `rect`.
pub fn child(ui: &mut Ui, rect: Rect) -> Ui {
    ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    )
}

/// A child `Ui` laid out top to bottom.
pub fn column(ui: &mut Ui, rect: Rect) -> Ui {
    ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    )
}

/// The text box frame of the global style: control fill, 8 px corners, a 2 px focus
/// border.
fn text_frame(ui: &Ui, colors: &Colors, rect: Rect, focused: bool, enabled: bool) {
    let hovered = ui.rect_contains_pointer(rect);
    let fill = if !enabled {
        colors.control_background_disabled()
    } else if hovered && !focused {
        colors.control_background_hover()
    } else {
        colors.control_background()
    };
    ui.painter().rect_filled(rect, 8.0, fill);
    ui.painter().rect_stroke(
        rect,
        8.0,
        if focused {
            Stroke::new(2.0, colors.focus())
        } else {
            Stroke::new(1.0, colors.border())
        },
        StrokeKind::Inside,
    );
}

/// One line of text in a box filling `rect`.
#[allow(clippy::too_many_arguments)]
pub fn text_box_at(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    id: egui::Id,
    text: &mut String,
    hint: &str,
    enabled: bool,
    centered: bool,
) -> Response {
    let focused = ui.memory(|m| m.has_focus(id));
    text_frame(ui, colors, rect, focused, enabled);
    let inner = rect.shrink2(vec2(8.0, 0.0));
    let mut child = child(ui, inner);
    let hint = egui::RichText::new(hint)
        .font(FontId::new(13.0, FontFamily::Proportional))
        .color(colors.text_secondary().gamma_multiply(0.75));
    let mut edit = egui::TextEdit::singleline(text)
        .id(id)
        .frame(egui::Frame::NONE)
        .font(FontId::new(13.0, FontFamily::Proportional))
        .text_color(if enabled {
            colors.text()
        } else {
            colors.text_secondary()
        })
        .hint_text(hint)
        .vertical_align(egui::Align::Center)
        .margin(egui::Margin::ZERO)
        .desired_width(inner.width());
    if centered {
        edit = edit.horizontal_align(egui::Align::Center);
    }
    child.add_enabled(enabled, edit)
}

/// A read-only line of text in a box (`IsReadOnly="True"`).
pub fn read_only_box_at(ui: &mut Ui, colors: &Colors, rect: Rect, text: &str, centered: bool) {
    text_frame(ui, colors, rect, false, true);
    let inner = rect.shrink2(vec2(8.0, 0.0));
    widgets::paint_text(
        ui,
        ui.painter(),
        inner,
        if centered {
            Align2::CENTER_CENTER
        } else {
            Align2::LEFT_CENTER
        },
        text,
        13.0,
        Weight::Normal,
        colors.text(),
    );
}

/// A multi-line text box filling `rect`, scrolling inside (`AcceptsReturn`,
/// `TextWrapping="Wrap"`).
#[allow(clippy::too_many_arguments)]
pub fn text_area_at(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    id: egui::Id,
    text: &mut String,
    hint: &str,
    enabled: bool,
    read_only: bool,
    monospace: bool,
) -> Response {
    let focused = ui.memory(|m| m.has_focus(id));
    text_frame(ui, colors, rect, focused, enabled);
    let inner = rect.shrink2(vec2(8.0, 6.0));
    let mut child = column(ui, inner);
    child.set_clip_rect(inner);
    let font = if monospace {
        FontId::new(12.0, FontFamily::Monospace)
    } else {
        FontId::new(13.0, FontFamily::Proportional)
    };
    let hint = egui::RichText::new(hint)
        .font(font.clone())
        .color(colors.text_secondary().gamma_multiply(0.75));
    let mut response = None;
    egui::ScrollArea::vertical()
        .id_salt(id.with("scroll"))
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            let edit = egui::TextEdit::multiline(text)
                .id(id)
                .frame(egui::Frame::NONE)
                .font(font)
                .text_color(if enabled {
                    colors.text()
                } else {
                    colors.text_secondary()
                })
                .hint_text(hint)
                .margin(egui::Margin::ZERO)
                .interactive(enabled && !read_only)
                .desired_width(inner.width() - 6.0)
                .desired_rows(1);
            response = Some(ui.add_enabled(enabled, edit));
        });
    response.unwrap_or_else(|| ui.interact(rect, id.with("area"), Sense::hover()))
}

/// A drop-down filling `rect` (the global ComboBox: 40 tall, 8 px corners).
pub fn combo_at<T: Clone + PartialEq>(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    id: &str,
    current: &T,
    options: &[(T, String)],
    enabled: bool,
) -> Option<T> {
    let mut child = child(ui, rect);
    if !enabled {
        child.disable();
    }
    widgets::combo_rounded(
        &mut child,
        colors,
        id,
        rect.width(),
        rect.height(),
        8.0,
        current,
        options,
    )
}

/// `NumericUpDown` filling `rect`.
#[allow(clippy::too_many_arguments)]
pub fn number_at(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    id: &str,
    value: &mut f64,
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
    enabled: bool,
) -> bool {
    crate::controls::number_box_at(
        ui, colors, id, rect, value, min, max, step, decimals, enabled,
    )
}

/// `NumericUpDown` with integers.
#[allow(clippy::too_many_arguments)]
pub fn int_at(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    id: &str,
    value: &mut i64,
    min: i64,
    max: i64,
    step: i64,
    enabled: bool,
) -> bool {
    let mut v = *value as f64;
    let changed = number_at(
        ui,
        colors,
        rect,
        id,
        &mut v,
        min as f64,
        max as f64,
        step as f64,
        0,
        enabled,
    );
    if changed {
        *value = v.round() as i64;
    }
    changed
}

/// A check box with its label, its box at `rect`'s left.
pub fn check_at(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    value: &mut bool,
    label: &str,
    enabled: bool,
) -> bool {
    let mut child = child(ui, rect);
    widgets::checkbox(&mut child, colors, value, Some(label), enabled).changed()
}

/// A check box whose label is Normal weight.
pub fn check_normal_at(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    value: &mut bool,
    label: &str,
    enabled: bool,
) -> bool {
    let mut child = child(ui, rect);
    widgets::checkbox_weighted(
        &mut child,
        colors,
        value,
        Some(label),
        enabled,
        Weight::Normal,
    )
    .changed()
}

/// A radio button with its label at `rect`'s left.
pub fn radio_at(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    checked: bool,
    label: &str,
    enabled: bool,
) -> bool {
    let mut child = child(ui, rect);
    crate::controls::radio_row(&mut child, colors, checked, label, enabled).clicked()
}

/// The width of a radio button and its label.
pub fn radio_width(ui: &Ui, label: &str) -> f32 {
    28.0 + text_width(ui, label, 13.0, Weight::SemiBold)
}

/// A picture fitted into `rect` (`Stretch="Uniform"`), centred; returns where it went.
pub fn image_uniform(ui: &Ui, texture: &TextureHandle, rect: Rect, upscale: bool) -> Rect {
    let size = texture.size_vec2();
    if size.x <= 0.0 || size.y <= 0.0 {
        return Rect::NOTHING;
    }
    let mut scale = (rect.width() / size.x).min(rect.height() / size.y);
    if !upscale {
        scale = scale.min(1.0);
    }
    let drawn = Rect::from_center_size(rect.center(), size * scale);
    ui.painter().image(
        texture.id(),
        drawn,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    drawn
}

/// An icon and a line under it, centred in `rect` (the empty states).
pub fn placeholder(ui: &Ui, colors: &Colors, rect: Rect, glyph: char, text: &str, icon_size: f32) {
    let galley = layout(
        ui,
        wrapped(
            text,
            13.0,
            Weight::Normal,
            colors.text_secondary(),
            (rect.width() - 24.0).max(40.0),
        ),
    );
    let total = icon_size + 8.0 + galley.size().y;
    let top = rect.center().y - total / 2.0;
    paint_icon(
        ui,
        ui.painter(),
        pos2(rect.center().x, top + icon_size / 2.0),
        glyph,
        icon_size,
        colors.text_secondary(),
    );
    let pos = pos2(
        rect.center().x - galley.size().x / 2.0,
        top + icon_size + 8.0,
    );
    ui.painter().galley(pos, galley, colors.text_secondary());
}

/// A path line trimmed with an ellipsis (`TextBlock.path-text`).
pub fn path_text(ui: &Ui, colors: &Colors, rect: Rect, text: &str) {
    let galley = layout(
        ui,
        trimmed(
            text,
            13.0,
            Weight::Normal,
            colors.text_secondary(),
            rect.width().max(1.0),
            1,
        ),
    );
    let pos = pos2(rect.left(), rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(pos, galley, colors.text_secondary());
}

/// A texture from an RGBA picture.
pub fn texture(
    ctx: &egui::Context,
    name: &str,
    image: &image::RgbaImage,
    nearest: bool,
) -> TextureHandle {
    let color = egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    );
    ctx.load_texture(
        name,
        color,
        if nearest {
            egui::TextureOptions::NEAREST
        } else {
            egui::TextureOptions::LINEAR
        },
    )
}

/// The size of a rectangle in whole pixels (dialogs ask the window for these).
pub fn size(width: f32, height: f32) -> Vec2 {
    vec2(width.round(), height.round())
}

/// `NumericUpDown ShowButtonSpinner="False"` with centred text: whole numbers between
/// `min` and `max`; returns the new value when the text holds one that differs.
#[allow(clippy::too_many_arguments)]
pub fn plain_number_at(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    id: egui::Id,
    value: i64,
    min: i64,
    max: i64,
    enabled: bool,
) -> Option<i64> {
    let focused = ui.memory(|m| m.has_focus(id));
    let mut text: String = ui
        .data(|d| d.get_temp::<String>(id.with("text")))
        .filter(|_| focused)
        .unwrap_or_else(|| value.to_string());
    let response = text_box_at(ui, colors, rect, id, &mut text, "", enabled, true);
    let mut out = None;
    if response.changed() {
        ui.data_mut(|d| d.insert_temp(id.with("text"), text.clone()));
        if let Ok(v) = text.trim().parse::<i64>() {
            let v = v.clamp(min, max);
            if v != value {
                out = Some(v);
            }
        }
    }
    if response.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(id.with("text")));
    }
    out
}

/// A small menu below `anchor`'s right edge (`MenuFlyout Placement="BottomEdgeAlignedRight"`);
/// returns the index picked.
pub fn flyout(
    ui: &mut Ui,
    colors: &Colors,
    response: &Response,
    id: &str,
    items: &[String],
) -> Option<usize> {
    let width = items
        .iter()
        .map(|i| text_width(ui, i, 13.0, Weight::Normal))
        .fold(120.0f32, f32::max)
        + 32.0;
    let mut picked = None;
    egui::Popup::menu(response)
        .id(egui::Id::new(("tool-flyout", id)))
        .frame(crate::menus::frame(colors))
        .align(egui::RectAlign::BOTTOM_END)
        .gap(2.0)
        .show(|ui| {
            ui.set_min_width(width);
            for (index, item) in items.iter().enumerate() {
                let (r, row) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
                if row.hovered() {
                    ui.painter()
                        .rect_filled(r, 4.0, colors.control_background_hover());
                }
                widgets::paint_text(
                    ui,
                    ui.painter(),
                    r.shrink2(vec2(10.0, 0.0)),
                    Align2::LEFT_CENTER,
                    item,
                    13.0,
                    Weight::Normal,
                    colors.text(),
                );
                if row.clicked() {
                    picked = Some(index);
                    ui.close();
                }
            }
        });
    picked
}

/// `ShareX.Brush.Checkered`: 15 px squares of the skin's two checker colours, aligned to
/// the window's top left.
pub fn paint_checkered_big(ui: &Ui, colors: &Colors, rect: Rect) {
    let (a, b) = (colors.checker_a(), colors.checker_b());
    let id = egui::Id::new(("checkered-big", a.to_array(), b.to_array()));
    let texture = ui
        .ctx()
        .memory_mut(|m| m.data.get_temp::<TextureHandle>(id));
    let texture = match texture {
        Some(texture) => texture,
        None => {
            let mut image = egui::ColorImage::filled([30, 30], a);
            for y in 0..30 {
                for x in 0..30 {
                    if (x < 15) == (y < 15) {
                        image.pixels[y * 30 + x] = b;
                    }
                }
            }
            let options = egui::TextureOptions {
                wrap_mode: egui::TextureWrapMode::Repeat,
                ..egui::TextureOptions::NEAREST
            };
            let texture = ui.ctx().load_texture("checkered-big", image, options);
            ui.ctx()
                .memory_mut(|m| m.data.insert_temp(id, texture.clone()));
            texture
        }
    };
    let origin = ui.max_rect().min;
    let uv = Rect::from_min_max(
        pos2(
            (rect.left() - origin.x) / 30.0,
            (rect.top() - origin.y) / 30.0,
        ),
        pos2(
            (rect.right() - origin.x) / 30.0,
            (rect.bottom() - origin.y) / 30.0,
        ),
    );
    ui.painter().image(texture.id(), rect, uv, Color32::WHITE);
}

/// `ToggleSwitch` (empty On/Off content): a 40 x 20 pill at the right of `rect`, the
/// accent when on; returns true when clicked.
pub fn toggle_at(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    id: egui::Id,
    value: &mut bool,
    enabled: bool,
) -> bool {
    let pill = Rect::from_min_size(
        pos2(rect.right() - 40.0, rect.center().y - 10.0),
        vec2(40.0, 20.0),
    );
    let response = ui.interact(
        pill,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let painter = ui.painter();
    let dim = if enabled { 1.0 } else { 0.35 };
    if *value {
        painter.rect_filled(pill, 10.0, colors.accent_start().gamma_multiply(dim));
        painter.circle_filled(
            pos2(pill.right() - 10.0, pill.center().y),
            5.0,
            colors.accent_foreground().gamma_multiply(dim),
        );
    } else {
        let fill = if response.hovered() {
            colors.control_background_hover()
        } else {
            colors.control_background()
        };
        painter.rect_filled(pill, 10.0, fill.gamma_multiply(dim));
        painter.rect_stroke(
            pill,
            10.0,
            Stroke::new(1.0, colors.text_secondary().gamma_multiply(dim)),
            StrokeKind::Inside,
        );
        painter.circle_filled(
            pos2(pill.left() + 10.0, pill.center().y),
            5.0,
            colors.text_secondary().gamma_multiply(dim),
        );
    }
    if response.clicked() {
        *value = !*value;
        return true;
    }
    false
}
