//! The drop-downs of `Presentation/Controls`: colour (`ColorPickerDropdown` with
//! `ColorPickerPanel`, Avalonia's `ColorView` on its palette tab plus "Pick colour from
//! screen"), width, border style, corner radius, font family, text alignment, arrow
//! style, cursor type, font size, step type, number and zoom, and the strength slider.
//!
//! Each is a `Button` 40 tall with its preview and the chevron, and a popup
//! (`popup-border popup-surface`) below it that closes on a click outside.

use std::sync::Arc;

use egui::{
    Align2, Color32, Id, Painter, Pos2, Rect, RectAlign, Response, Sense, Shape, Stroke, Ui, Vec2,
    pos2, vec2,
};

use super::style::{self, ButtonKind, ButtonPaint, Colors, Weight};
use crate::color::Color;
use crate::model::{ArrowStyle, BorderStyle, CursorType, StepType, TextAlign};
use modlight_core::l10n::{editor as s, t};

pub const WIDTH_OPTIONS: [i32; 10] = [2, 4, 6, 8, 10, 12, 14, 16, 18, 20];
pub const CORNER_RADIUS_OPTIONS: [i32; 10] = [0, 2, 4, 6, 8, 10, 15, 20, 25, 30];
pub const FONT_SIZE_OPTIONS: [f32; 12] = [
    10.0, 13.0, 16.0, 20.0, 24.0, 30.0, 48.0, 72.0, 96.0, 144.0, 216.0, 288.0,
];
pub const ZOOM_OPTIONS: [f64; 9] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0];
pub const ARROW_STYLES: [ArrowStyle; 5] = [
    ArrowStyle::Classic,
    ArrowStyle::Double,
    ArrowStyle::Modern,
    ArrowStyle::Basic,
    ArrowStyle::Line,
];
pub const BORDER_STYLES: [BorderStyle; 5] = [
    BorderStyle::Solid,
    BorderStyle::Dash,
    BorderStyle::Dot,
    BorderStyle::DashDot,
    BorderStyle::DashDotDot,
];
pub const STEP_TYPES: [StepType; 5] = [
    StepType::Numeric,
    StepType::UppercaseLetter,
    StepType::LowercaseLetter,
    StepType::UppercaseRoman,
    StepType::LowercaseRoman,
];
pub const CURSOR_TYPES: [CursorType; 28] = [
    CursorType::AppStarting,
    CursorType::Arrow,
    CursorType::Cross,
    CursorType::Default,
    CursorType::Hand,
    CursorType::Help,
    CursorType::HSplit,
    CursorType::IBeam,
    CursorType::No,
    CursorType::NoMove2D,
    CursorType::NoMoveHoriz,
    CursorType::NoMoveVert,
    CursorType::PanEast,
    CursorType::PanNE,
    CursorType::PanNorth,
    CursorType::PanNW,
    CursorType::PanSE,
    CursorType::PanSouth,
    CursorType::PanSW,
    CursorType::PanWest,
    CursorType::SizeAll,
    CursorType::SizeNESW,
    CursorType::SizeNS,
    CursorType::SizeNWSE,
    CursorType::SizeWE,
    CursorType::UpArrow,
    CursorType::VSplit,
    CursorType::WaitCursor,
];

// ---------------------------------------------------------------------------- frame

/// The `DropdownButton` of a picker: allocates it, paints the button and the chevron
/// at the right edge, and says whether its popup is open.
fn dropdown_button(
    ui: &mut Ui,
    colors: &Colors,
    id: Id,
    rect: Rect,
    enabled: bool,
    tooltip: &str,
) -> (Response, ButtonPaint, bool) {
    let response = ui.interact(
        rect,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let paint = style::paint_button(
        ui,
        colors,
        &response,
        rect,
        ButtonKind::Standard,
        10.0,
        false,
        enabled,
    );
    let popup_id = egui::Popup::default_response_id(&response);
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let response = if tooltip.is_empty() || open {
        response
    } else {
        response.on_hover_text(tooltip)
    };
    (response, paint, open)
}

/// Shows `content` in the picker's popup under `response`.
fn popup<R>(
    colors: &Colors,
    response: &Response,
    padding: i8,
    align: RectAlign,
    content: impl FnOnce(&mut Ui) -> Option<R>,
) -> Option<R> {
    egui::Popup::from_toggle_button_response(response)
        .align(align)
        .gap(4.0)
        .frame(style::popup_frame(colors, padding))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            content(ui)
        })
        .and_then(|r| r.inner)
}

fn paint_chevron(ui: &Ui, colors: &Colors, paint: &ButtonPaint, open: bool, x: f32) {
    style::paint_chevron(
        ui,
        ui.painter(),
        pos2(x, paint.rect.center().y),
        open,
        style::fade(colors.text_secondary(), paint.opacity),
    );
}

/// A `picker-button` in a popup list: 32 tall, transparent, hover tint.
fn picker_item(ui: &mut Ui, colors: &Colors, size: Vec2, active: bool) -> (Response, ButtonPaint) {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let paint = style::paint_button(
        ui,
        colors,
        &response,
        rect,
        ButtonKind::Picker,
        10.0,
        active,
        true,
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    (response, paint)
}

/// The width a text needs at a size and weight.
fn text_width(ui: &Ui, text: &str, size: f32, weight: Weight) -> f32 {
    style::galley(ui, text, size, weight, Color32::WHITE)
        .size()
        .x
}

/// The chevron glyph's width (10 points of Lucide).
const CHEVRON_WIDTH: f32 = 10.0;

// ---------------------------------------------------------------------------- colour

/// What a colour picker reported.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColorEvent {
    Picked(Color),
    /// "Pick colour from screen" was pressed (the popup closed).
    PickScreen,
}

pub fn to_color32(c: Color) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

/// `ColorPickerDropdown`: a 24-point swatch (with a hole for border colours, an "A" for
/// text colours) and the chevron.
#[allow(clippy::too_many_arguments)]
pub fn color_dropdown(
    ui: &mut Ui,
    colors: &Colors,
    id: Id,
    rect: Rect,
    value: Color,
    border_hole: bool,
    text_label: bool,
    enabled: bool,
    tooltip: &str,
) -> Option<ColorEvent> {
    let (response, paint, open) = dropdown_button(ui, colors, id, rect, enabled, tooltip);
    let painter = ui.painter().clone();
    let swatch = Rect::from_min_size(
        pos2(paint.rect.left() + 10.0, paint.rect.center().y - 12.0),
        vec2(24.0, 24.0),
    );
    style::paint_checkered_small(ui, &painter, colors, swatch, 4.0);
    painter.rect_filled(swatch, 4.0, style::fade(to_color32(value), paint.opacity));
    if border_hole {
        style::paint_checkered_small(ui, &painter, colors, swatch.shrink(5.0), 2.0);
    }
    if text_label {
        let light = value.a < 128
            || (0.299 * value.r as f32 + 0.587 * value.g as f32 + 0.114 * value.b as f32) > 128.0;
        let fg = if light {
            Color32::from_rgb(78, 78, 78)
        } else {
            Color32::from_rgb(222, 224, 225)
        };
        style::paint_text(
            ui,
            &painter,
            swatch,
            Align2::CENTER_CENTER,
            "A",
            14.0,
            Weight::SemiBold,
            style::fade(fg, paint.opacity),
        );
    }
    paint_chevron(
        ui,
        colors,
        &paint,
        open,
        paint.rect.right() - 10.0 - CHEVRON_WIDTH / 2.0,
    );
    popup(colors, &response, 4, RectAlign::BOTTOM, |ui| {
        let event = color_panel(ui, colors, id.with("panel"), value);
        if matches!(event, Some(ColorEvent::PickScreen)) {
            ui.close();
        }
        event
    })
}

/// The panel's own state (tab and the HSV the colour came from, so hue survives grey).
#[derive(Clone, Debug, PartialEq)]
struct PanelState {
    tab: usize,
    hsv: [f32; 3],
    alpha: f32,
    color: Color,
    hex: String,
}

fn rgb_to_hsv(c: Color) -> [f32; 3] {
    let r = c.r as f32 / 255.0;
    let g = c.g as f32 / 255.0;
    let b = c.b as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max == 0.0 { 0.0 } else { d / max };
    [h, s, max]
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let c = v * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    ]
}

/// Avalonia's `FluentColorPalette` (8 colours by 6 shades), with `ColorPickerPanel`'s
/// three replacements: #EBEBEB, #141414 and transparent.
pub fn palette_colors() -> Vec<Color> {
    const CHART: [[u32; 8]; 6] = [
        [
            0xFF4343, 0xD13438, 0xEF6950, 0xDA3B01, 0xCA5010, 0xF7630C, 0xFF8C00, 0xFFB900,
        ],
        [
            0xE74856, 0xE81123, 0xEA005E, 0xC30052, 0xE3008C, 0xBF0077, 0xC239B3, 0x9A0089,
        ],
        [
            0x0078D7, 0x0063B1, 0x8E8CD8, 0x6B69D6, 0x8764B8, 0x744DA9, 0xB146C2, 0x881798,
        ],
        [
            0x0099BC, 0x2D7D9A, 0x00B7C3, 0x038387, 0x00B294, 0x018574, 0x00CC6A, 0x10893E,
        ],
        [
            0x7A7574, 0x5D5A58, 0x68768A, 0x515C6B, 0x567C73, 0x486860, 0x498205, 0x107C10,
        ],
        [
            0x767676, 0x4C4A48, 0x69797E, 0x4A5459, 0x647C64, 0x525E54, 0x847545, 0x7E735F,
        ],
    ];
    let mut list: Vec<Color> = CHART
        .iter()
        .flatten()
        .map(|v| Color::rgb((v >> 16) as u8, (v >> 8) as u8, *v as u8))
        .collect();
    let n = list.len();
    list[n - 13] = Color::rgb(235, 235, 235);
    list[n - 7] = Color::rgb(20, 20, 20);
    list[n - 1] = Color::TRANSPARENT;
    list
}

const PANEL_WIDTH: f32 = 300.0;

/// `ColorPickerPanel`: Avalonia's `ColorView` (spectrum, palette and components tabs,
/// the palette shown first, alpha on, the previewer with its accent shades) and the
/// "Pick colour from screen" button.
pub fn color_panel(ui: &mut Ui, colors: &Colors, id: Id, value: Color) -> Option<ColorEvent> {
    let mut state = ui
        .ctx()
        .data(|d| d.get_temp::<PanelState>(id))
        .filter(|s| s.color == value)
        .unwrap_or_else(|| {
            let previous_tab = ui
                .ctx()
                .data(|d| d.get_temp::<PanelState>(id))
                .map(|s| s.tab)
                .unwrap_or(1);
            PanelState {
                tab: previous_tab,
                hsv: rgb_to_hsv(value),
                alpha: value.a as f32 / 255.0,
                color: value,
                hex: format_hex(value),
            }
        });
    let mut picked: Option<Color> = None;
    let mut event = None;

    // The tab strip.
    let (strip, _) = ui.allocate_exact_size(vec2(PANEL_WIDTH, 40.0), Sense::hover());
    let tabs = [
        modlight_core::icons::RAINBOW,
        modlight_core::icons::GRID_3X3,
        modlight_core::icons::SLIDERS_HORIZONTAL,
    ];
    for (i, glyph) in tabs.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(strip.left() + i as f32 * PANEL_WIDTH / 3.0, strip.top()),
            vec2(PANEL_WIDTH / 3.0, 40.0),
        )
        .shrink2(vec2(2.0, 2.0));
        let resp = ui.interact(r, id.with(("tab", i)), Sense::click());
        let selected = state.tab == i;
        if selected {
            ui.painter()
                .rect_filled(r, 6.0, colors.control_background_hover());
            ui.painter().rect_filled(
                Rect::from_min_size(pos2(r.center().x - 8.0, r.bottom() - 3.0), vec2(16.0, 3.0)),
                1.5,
                colors.accent_start(),
            );
        } else if resp.hovered() {
            ui.painter()
                .rect_filled(r, 6.0, colors.control_background_disabled());
        }
        style::paint_icon(ui, ui.painter(), r.center(), *glyph, 18.0, colors.text());
        if resp.clicked() {
            state.tab = i;
        }
    }
    ui.add_space(8.0);

    match state.tab {
        0 => {
            // Spectrum: saturation/value square, the hue and alpha sliders beside it.
            let (area, _) = ui.allocate_exact_size(vec2(PANEL_WIDTH, 220.0), Sense::hover());
            let square = Rect::from_min_size(area.min, vec2(220.0, 220.0));
            let hue =
                Rect::from_min_size(pos2(square.right() + 16.0, area.top()), vec2(24.0, 220.0));
            let alpha =
                Rect::from_min_size(pos2(hue.right() + 16.0, area.top()), vec2(24.0, 220.0));
            paint_sv_square(ui.painter(), square, state.hsv[0]);
            let sv = ui.interact(square, id.with("sv"), Sense::click_and_drag());
            if let Some(p) = sv
                .interact_pointer_pos()
                .filter(|_| sv.dragged() || sv.clicked())
            {
                state.hsv[1] = ((p.x - square.left()) / square.width()).clamp(0.0, 1.0);
                state.hsv[2] = 1.0 - ((p.y - square.top()) / square.height()).clamp(0.0, 1.0);
                picked = Some(from_hsv(&state));
            }
            let marker = pos2(
                square.left() + state.hsv[1] * square.width(),
                square.top() + (1.0 - state.hsv[2]) * square.height(),
            );
            ui.painter()
                .circle_stroke(marker, 6.0, Stroke::new(2.0, Color32::WHITE));
            ui.painter()
                .circle_stroke(marker, 7.0, Stroke::new(1.0, Color32::BLACK));
            paint_hue_bar(ui.painter(), hue);
            let hr = ui.interact(hue, id.with("hue"), Sense::click_and_drag());
            if let Some(p) = hr
                .interact_pointer_pos()
                .filter(|_| hr.dragged() || hr.clicked())
            {
                state.hsv[0] = ((p.y - hue.top()) / hue.height()).clamp(0.0, 0.9999) * 360.0;
                picked = Some(from_hsv(&state));
            }
            let hy = hue.top() + state.hsv[0] / 360.0 * hue.height();
            slider_marker(ui.painter(), hue, hy);
            style::paint_checkered(ui, ui.painter(), colors, alpha, 4.0, alpha.min);
            let opaque = hsv_to_rgb(state.hsv[0], state.hsv[1], state.hsv[2]);
            paint_vertical_gradient(
                ui.painter(),
                alpha,
                Color32::from_rgb(opaque[0], opaque[1], opaque[2]),
                Color32::from_rgba_unmultiplied(opaque[0], opaque[1], opaque[2], 0),
            );
            let ar = ui.interact(alpha, id.with("alpha"), Sense::click_and_drag());
            if let Some(p) = ar
                .interact_pointer_pos()
                .filter(|_| ar.dragged() || ar.clicked())
            {
                state.alpha = 1.0 - ((p.y - alpha.top()) / alpha.height()).clamp(0.0, 1.0);
                picked = Some(from_hsv(&state));
            }
            slider_marker(
                ui.painter(),
                alpha,
                alpha.top() + (1.0 - state.alpha) * alpha.height(),
            );
        }
        1 => {
            // Palette: 8 columns.
            let list = palette_colors();
            let cell = PANEL_WIDTH / 8.0;
            for row in list.chunks(8) {
                let (line, _) = ui.allocate_exact_size(vec2(PANEL_WIDTH, cell), Sense::hover());
                for (i, c) in row.iter().enumerate() {
                    let r = Rect::from_min_size(
                        pos2(line.left() + i as f32 * cell, line.top()),
                        vec2(cell, cell),
                    )
                    .shrink(1.0);
                    let resp =
                        ui.interact(r, id.with(("swatch", line.top() as i32, i)), Sense::click());
                    if c.a < 255 {
                        style::paint_checkered(ui, ui.painter(), colors, r, 4.0, r.min);
                    }
                    ui.painter().rect_filled(r, 0.0, to_color32(*c));
                    let selected = *c == value;
                    if selected || resp.hovered() {
                        ui.painter().rect_stroke(
                            r.shrink(1.0),
                            0.0,
                            Stroke::new(
                                2.0,
                                if selected {
                                    colors.text()
                                } else {
                                    colors.text_secondary()
                                },
                            ),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if resp.clicked() {
                        state.hsv = rgb_to_hsv(*c);
                        state.alpha = c.a as f32 / 255.0;
                        picked = Some(*c);
                    }
                }
            }
        }
        _ => {
            // Components: hex and the four channels.
            let (row, _) = ui.allocate_exact_size(vec2(PANEL_WIDTH, 40.0), Sense::hover());
            let mut hex = state.hex.clone();
            let resp = style::text_box(ui, colors, row, &mut hex, "#", true, id.with("hex"));
            state.hex = hex.clone();
            if (resp.lost_focus() || resp.changed())
                && let Some(c) = parse_hex(&hex)
                && c != value
            {
                state.hsv = rgb_to_hsv(c);
                state.alpha = c.a as f32 / 255.0;
                picked = Some(c);
            }
            ui.add_space(8.0);
            let current = value;
            let channels = [
                ("R", current.r),
                ("G", current.g),
                ("B", current.b),
                ("A", current.a),
            ];
            for (i, (name, v)) in channels.iter().enumerate() {
                let (line, _) = ui.allocate_exact_size(vec2(PANEL_WIDTH, 32.0), Sense::hover());
                style::paint_text(
                    ui,
                    ui.painter(),
                    Rect::from_min_size(line.min, vec2(20.0, 32.0)),
                    Align2::LEFT_CENTER,
                    name,
                    style::FONT_SIZE,
                    Weight::SemiBold,
                    colors.text_secondary(),
                );
                style::paint_text(
                    ui,
                    ui.painter(),
                    Rect::from_min_max(pos2(line.right() - 36.0, line.top()), line.max),
                    Align2::RIGHT_CENTER,
                    &v.to_string(),
                    style::FONT_SIZE,
                    Weight::Normal,
                    colors.text(),
                );
                let slider_rect = Rect::from_min_max(
                    pos2(line.left() + 20.0, line.top()),
                    pos2(line.right() - 40.0, line.bottom()),
                );
                if let Some(nv) = style::slider(
                    ui,
                    colors,
                    id.with(("channel", i)),
                    slider_rect,
                    *v as f64,
                    0.0,
                    255.0,
                    Some(1.0),
                    true,
                ) {
                    let mut c = current;
                    match i {
                        0 => c.r = nv as u8,
                        1 => c.g = nv as u8,
                        2 => c.b = nv as u8,
                        _ => c.a = nv as u8,
                    }
                    state.hsv = rgb_to_hsv(c);
                    state.alpha = c.a as f32 / 255.0;
                    picked = Some(c);
                }
            }
        }
    }

    // The previewer: the colour with two lighter and two darker accents.
    ui.add_space(12.0);
    let (bar, _) = ui.allocate_exact_size(vec2(PANEL_WIDTH, 40.0), Sense::hover());
    let shown = picked.unwrap_or(value);
    let accent_w = 40.0;
    let center = Rect::from_min_max(
        pos2(bar.left() + accent_w * 2.0, bar.top()),
        pos2(bar.right() - accent_w * 2.0, bar.bottom()),
    );
    style::paint_checkered(ui, ui.painter(), colors, center, 4.0, center.min);
    ui.painter().rect_filled(center, 4.0, to_color32(shown));
    let base = rgb_to_hsv(shown);
    for (i, delta) in [-0.3f32, -0.15, 0.15, 0.3].iter().enumerate() {
        let x = if i < 2 {
            bar.left() + i as f32 * accent_w
        } else {
            center.right() + (i - 2) as f32 * accent_w
        };
        let r = Rect::from_min_size(pos2(x, bar.top() + 4.0), vec2(accent_w, 32.0))
            .shrink2(vec2(1.0, 0.0));
        let v = (base[2] + delta).clamp(0.0, 1.0);
        let rgb = hsv_to_rgb(base[0], base[1], v);
        let c = Color::rgba(rgb[0], rgb[1], rgb[2], shown.a);
        let resp = ui.interact(r, id.with(("accent", i)), Sense::click());
        style::paint_checkered(ui, ui.painter(), colors, r, 4.0, r.min);
        ui.painter().rect_filled(r, 2.0, to_color32(c));
        if resp.clicked() {
            state.hsv = rgb_to_hsv(c);
            state.alpha = c.a as f32 / 255.0;
            picked = Some(c);
        }
    }

    // "Pick colour from screen".
    ui.add_space(4.0);
    let label = t(s::ColorPickerPanel_Pickcolorfromscreen);
    let label_w = text_width(ui, label, 16.0, Weight::Normal);
    let bw = 10.0 + 16.0 + 8.0 + label_w + 10.0 + 2.0;
    let (row, _) = ui.allocate_exact_size(vec2(PANEL_WIDTH, 48.0), Sense::hover());
    let br = Rect::from_center_size(row.center(), vec2(bw, 40.0));
    let (resp, paint) = style::button_at(ui, colors, br, ButtonKind::Standard, 10.0, false, true);
    style::paint_icon(
        ui,
        ui.painter(),
        pos2(paint.rect.left() + 10.0 + 8.0, paint.rect.center().y),
        crate::ui::icons::SCREEN_COLOR_PICKER,
        16.0,
        paint.text_block,
    );
    style::paint_text(
        ui,
        ui.painter(),
        Rect::from_min_max(
            pos2(paint.rect.left() + 10.0 + 16.0 + 8.0, paint.rect.top()),
            paint.rect.max,
        ),
        Align2::LEFT_CENTER,
        label,
        16.0,
        Weight::Normal,
        paint.text_block,
    );
    if resp.clicked() {
        event = Some(ColorEvent::PickScreen);
    }

    if let Some(c) = picked {
        state.color = c;
        state.hex = format_hex(c);
        event = Some(ColorEvent::Picked(c));
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, state));
    event
}

fn from_hsv(state: &PanelState) -> Color {
    let rgb = hsv_to_rgb(state.hsv[0], state.hsv[1], state.hsv[2]);
    Color::rgba(rgb[0], rgb[1], rgb[2], (state.alpha * 255.0).round() as u8)
}

fn format_hex(c: Color) -> String {
    format!("{:02X}{:02X}{:02X}{:02X}", c.a, c.r, c.g, c.b)
}

fn parse_hex(text: &str) -> Option<Color> {
    let t = text.trim().trim_start_matches('#');
    match t.len() {
        6 | 8 => Color::parse(&format!("#{t}")),
        _ => None,
    }
}

fn paint_sv_square(painter: &Painter, rect: Rect, hue: f32) {
    let hue_rgb = hsv_to_rgb(hue, 1.0, 1.0);
    let hue_color = Color32::from_rgb(hue_rgb[0], hue_rgb[1], hue_rgb[2]);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), Color32::WHITE);
    mesh.colored_vertex(rect.right_top(), hue_color);
    mesh.colored_vertex(rect.right_bottom(), hue_color);
    mesh.colored_vertex(rect.left_bottom(), Color32::WHITE);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
    paint_vertical_gradient(painter, rect, Color32::TRANSPARENT, Color32::BLACK);
}

fn paint_vertical_gradient(painter: &Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
}

fn paint_hue_bar(painter: &Painter, rect: Rect) {
    let mut mesh = egui::Mesh::default();
    let steps = 12;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let rgb = hsv_to_rgb(t * 360.0, 1.0, 1.0);
        let c = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
        let y = rect.top() + t * rect.height();
        mesh.colored_vertex(pos2(rect.left(), y), c);
        mesh.colored_vertex(pos2(rect.right(), y), c);
    }
    for i in 0..steps as u32 {
        let a = i * 2;
        mesh.add_triangle(a, a + 1, a + 3);
        mesh.add_triangle(a, a + 3, a + 2);
    }
    painter.add(Shape::mesh(mesh));
}

fn slider_marker(painter: &Painter, rect: Rect, y: f32) {
    let r = Rect::from_min_max(
        pos2(rect.left() - 2.0, y - 3.0),
        pos2(rect.right() + 2.0, y + 3.0),
    );
    painter.rect_stroke(
        r,
        2.0,
        Stroke::new(2.0, Color32::WHITE),
        egui::StrokeKind::Middle,
    );
    painter.rect_stroke(
        r.expand(1.5),
        2.0,
        Stroke::new(1.0, Color32::BLACK),
        egui::StrokeKind::Middle,
    );
}

// ---------------------------------------------------------------------------- width

pub fn width_dropdown(
    ui: &mut Ui,
    colors: &Colors,
    id: Id,
    rect: Rect,
    value: i32,
    tooltip: &str,
) -> Option<i32> {
    let (response, paint, open) = dropdown_button(ui, colors, id, rect, true, tooltip);
    let center = pos2(paint.rect.left() + 10.0 + 12.0, paint.rect.center().y);
    ui.painter()
        .circle_filled(center, value as f32 / 2.0, colors.text_secondary());
    paint_chevron(
        ui,
        colors,
        &paint,
        open,
        paint.rect.right() - 10.0 - CHEVRON_WIDTH / 2.0,
    );
    popup(colors, &response, 8, RectAlign::BOTTOM, |ui| {
        let mut out = None;
        for row in WIDTH_OPTIONS.chunks(5) {
            ui.horizontal(|ui| {
                for w in row {
                    let (resp, paint) = picker_item(ui, colors, vec2(32.0, 32.0), false);
                    ui.painter().circle_filled(
                        paint.rect.center(),
                        *w as f32 / 2.0,
                        colors.text_secondary(),
                    );
                    let resp = resp.on_hover_text(format!("{w} px"));
                    if resp.clicked() {
                        out = Some(*w);
                        ui.close();
                    }
                }
            });
        }
        out
    })
}

// ---------------------------------------------------------------------------- corner radius

fn corner_preview(painter: &Painter, center: Pos2, radius: i32, color: Color32) {
    // `CornerRadiusPreviewGeometryConverter` at 20: an L with the corner rounded.
    let size = 20.0;
    let origin = center - vec2(size / 2.0, size / 2.0);
    let r = (radius as f32).clamp(0.0, size);
    let mut points = vec![pos2(0.0, size)];
    if r <= 0.01 {
        points.push(pos2(0.0, 0.0));
    } else {
        points.push(pos2(0.0, r));
        for i in 1..=12 {
            let a = std::f32::consts::PI + i as f32 / 12.0 * std::f32::consts::FRAC_PI_2;
            points.push(pos2(r + r * a.cos(), r + r * a.sin()));
        }
    }
    points.push(pos2(size, 0.0));
    let points: Vec<Pos2> = points.into_iter().map(|p| origin + p.to_vec2()).collect();
    painter.add(Shape::line(points, Stroke::new(2.5, color)));
}

pub fn corner_radius_dropdown(
    ui: &mut Ui,
    colors: &Colors,
    id: Id,
    rect: Rect,
    value: i32,
    tooltip: &str,
) -> Option<i32> {
    let (response, paint, open) = dropdown_button(ui, colors, id, rect, true, tooltip);
    corner_preview(
        ui.painter(),
        pos2(paint.rect.left() + 10.0 + 12.0, paint.rect.center().y),
        value,
        colors.text_secondary(),
    );
    paint_chevron(
        ui,
        colors,
        &paint,
        open,
        paint.rect.right() - 10.0 - CHEVRON_WIDTH / 2.0,
    );
    popup(colors, &response, 8, RectAlign::BOTTOM, |ui| {
        let mut out = None;
        for row in CORNER_RADIUS_OPTIONS.chunks(5) {
            ui.horizontal(|ui| {
                for v in row {
                    let (resp, paint) = picker_item(ui, colors, vec2(36.0, 36.0), false);
                    corner_preview(
                        ui.painter(),
                        paint.rect.center(),
                        *v,
                        colors.text_secondary(),
                    );
                    let resp = resp.on_hover_text(format!("{v} px"));
                    if resp.clicked() {
                        out = Some(*v);
                        ui.close();
                    }
                }
            });
        }
        out
    })
}

// ---------------------------------------------------------------------------- styles

pub fn border_style_name(style: BorderStyle) -> &'static str {
    t(match style {
        BorderStyle::Solid => s::BorderStyleDisplayConverter_Solid,
        BorderStyle::Dash => s::BorderStyleDisplayConverter_Dash,
        BorderStyle::Dot => s::BorderStyleDisplayConverter_Dot,
        BorderStyle::DashDot => s::BorderStyleDisplayConverter_DashDot,
        BorderStyle::DashDotDot => s::BorderStyleDisplayConverter_DashDotDot,
    })
}

pub fn arrow_style_name(style: ArrowStyle) -> &'static str {
    t(match style {
        ArrowStyle::Classic => s::ArrowStyleDisplayNameConverter_Classic,
        ArrowStyle::Modern => s::ArrowStyleDisplayNameConverter_Modern,
        ArrowStyle::Double => s::ArrowStyleDisplayNameConverter_Double,
        ArrowStyle::Basic => s::ArrowStyleDisplayNameConverter_Basic,
        ArrowStyle::Line => s::ArrowStyleDisplayNameConverter_Line,
    })
}

pub fn cursor_type_name(c: CursorType) -> &'static str {
    use CursorType::*;
    t(match c {
        AppStarting => s::CursorTypeDisplayNameConverter_AppStarting,
        Arrow => s::CursorTypeDisplayNameConverter_Arrow,
        Cross => s::CursorTypeDisplayNameConverter_Cross,
        Default => s::CursorTypeDisplayNameConverter_Default,
        Hand => s::CursorTypeDisplayNameConverter_Hand,
        Help => s::CursorTypeDisplayNameConverter_Help,
        HSplit => s::CursorTypeDisplayNameConverter_HSplit,
        IBeam => s::CursorTypeDisplayNameConverter_IBeam,
        No => s::CursorTypeDisplayNameConverter_No,
        NoMove2D => s::CursorTypeDisplayNameConverter_NoMove2D,
        NoMoveHoriz => s::CursorTypeDisplayNameConverter_NoMoveHoriz,
        NoMoveVert => s::CursorTypeDisplayNameConverter_NoMoveVert,
        PanEast => s::CursorTypeDisplayNameConverter_PanEast,
        PanNE => s::CursorTypeDisplayNameConverter_PanNE,
        PanNorth => s::CursorTypeDisplayNameConverter_PanNorth,
        PanNW => s::CursorTypeDisplayNameConverter_PanNW,
        PanSE => s::CursorTypeDisplayNameConverter_PanSE,
        PanSouth => s::CursorTypeDisplayNameConverter_PanSouth,
        PanSW => s::CursorTypeDisplayNameConverter_PanSW,
        PanWest => s::CursorTypeDisplayNameConverter_PanWest,
        SizeAll => s::CursorTypeDisplayNameConverter_SizeAll,
        SizeNESW => s::CursorTypeDisplayNameConverter_SizeNESW,
        SizeNS => s::CursorTypeDisplayNameConverter_SizeNS,
        SizeNWSE => s::CursorTypeDisplayNameConverter_SizeNWSE,
        SizeWE => s::CursorTypeDisplayNameConverter_SizeWE,
        UpArrow => s::CursorTypeDisplayNameConverter_UpArrow,
        VSplit => s::CursorTypeDisplayNameConverter_VSplit,
        WaitCursor => s::CursorTypeDisplayNameConverter_WaitCursor,
    })
}

pub fn align_name(a: TextAlign) -> &'static str {
    t(match a {
        TextAlign::Left => s::TextHorizontalAlignmentHelper_Left,
        TextAlign::Center => s::TextHorizontalAlignmentHelper_Center,
        TextAlign::Right => s::TextHorizontalAlignmentHelper_Right,
    })
}

pub fn align_icon(a: TextAlign) -> char {
    match a {
        TextAlign::Left => modlight_core::icons::TEXT_ALIGN_START,
        TextAlign::Center => modlight_core::icons::TEXT_ALIGN_CENTER,
        TextAlign::Right => modlight_core::icons::TEXT_ALIGN_END,
    }
}

/// The 36 by 14 dash preview (`M 2,7 L 34,7`, 1.75 thick, flat caps).
fn border_preview(painter: &Painter, left: Pos2, style: BorderStyle, color: Color32) {
    let a = left + vec2(2.0, 7.0);
    let b = left + vec2(34.0, 7.0);
    let stroke = Stroke::new(1.75, color);
    let dashes = style.dash_array();
    if dashes.is_empty() {
        painter.line_segment([a, b], stroke);
        return;
    }
    // Avalonia multiplies the dash array by the stroke thickness.
    let mut x = a.x;
    let mut i = 0;
    let mut on = true;
    while x < b.x {
        let len = dashes[i % dashes.len()] * 1.75;
        let end = (x + len).min(b.x);
        if on {
            painter.line_segment([pos2(x, a.y), pos2(end, a.y)], stroke);
        }
        x = end;
        on = !on;
        i += 1;
    }
}

/// The 36 by 14 arrow preview (`ArrowStylePreviewGeometryConverter`), rasterised.
fn arrow_preview(ui: &Ui, painter: &Painter, left: Pos2, style: ArrowStyle, color: Color32) {
    let ppp = ui.ctx().pixels_per_point();
    let key = format!("arrow-{}-{:?}-{}", style.name(), color.to_array(), ppp);
    let image = cached_image(ui, &key, || {
        let w = (36.0 * ppp).ceil() as u32;
        let h = (14.0 * ppp).ceil() as u32;
        let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
        let mut a = crate::model::Annotation::arrow();
        a.stroke_width = 1.75;
        if let crate::model::Kind::Arrow { style: s, .. } = &mut a.kind {
            *s = style;
        }
        a.start = crate::geom::Point::new(2.0, 7.0);
        a.end = crate::geom::Point::new(34.0, 7.0);
        let c = crate::color::Color::rgba(color.r(), color.g(), color.b(), color.a());
        a.stroke_color = c.to_hex_argb();
        let geometry = crate::render::arrow_geometry(&a);
        let transform = tiny_skia::Transform::from_scale(ppp, ppp);
        if let Some(fill) = &geometry.fill {
            crate::render::fill_path(&mut pixmap, fill, c, transform, None);
        }
        if let Some(stroke) = &geometry.stroke {
            let mut pen = crate::render::Pen::new(c, 1.75);
            pen.cap = tiny_skia::LineCap::Round;
            pen.join = tiny_skia::LineJoin::Round;
            crate::render::stroke_path(&mut pixmap, stroke, &pen, transform, None);
        }
        Some(crate::raster::from_pixmap(&pixmap))
    });
    if let Some(image) = image {
        style::paint_image(
            ui,
            painter,
            &key,
            &image,
            Rect::from_min_size(left, vec2(36.0, 14.0)),
            Color32::WHITE,
        );
    }
}

/// A picture made once per key and kept in the context's memory.
fn cached_image(
    ui: &Ui,
    key: &str,
    make: impl FnOnce() -> Option<image::RgbaImage>,
) -> Option<Arc<image::RgbaImage>> {
    let id = Id::new(("editor-cached-image", key));
    if let Some(v) = ui
        .ctx()
        .data(|d| d.get_temp::<Option<Arc<image::RgbaImage>>>(id))
    {
        return v;
    }
    let v = make().map(Arc::new);
    ui.ctx().data_mut(|d| d.insert_temp(id, v.clone()));
    v
}

/// The natural width of a label-and-preview drop-down, clamped to its min and max.
fn label_width(ui: &Ui, label: &str, extra: f32, min: f32, max: f32) -> f32 {
    (text_width(ui, label, 12.0, Weight::Normal) + 6.0 + extra + CHEVRON_WIDTH + 20.0 + 2.0)
        .clamp(min, max)
}

pub fn border_style_width(ui: &Ui, value: BorderStyle) -> f32 {
    label_width(ui, border_style_name(value), 52.0, 92.0, 168.0)
}

pub fn border_style_dropdown(
    ui: &mut Ui,
    colors: &Colors,
    id: Id,
    rect: Rect,
    value: BorderStyle,
    tooltip: &str,
) -> Option<BorderStyle> {
    let (response, paint, open) = dropdown_button(ui, colors, id, rect, true, tooltip);
    let right = paint.rect.right() - 10.0;
    paint_chevron(ui, colors, &paint, open, right - CHEVRON_WIDTH / 2.0);
    let preview_left = right - CHEVRON_WIDTH - 8.0 - 36.0;
    border_preview(
        ui.painter(),
        pos2(preview_left, paint.rect.center().y - 7.0),
        value,
        colors.text_secondary(),
    );
    style::paint_text(
        ui,
        ui.painter(),
        Rect::from_min_max(
            pos2(paint.rect.left() + 10.0, paint.rect.top()),
            pos2(preview_left - 8.0 - 6.0, paint.rect.bottom()),
        ),
        Align2::LEFT_CENTER,
        border_style_name(value),
        12.0,
        Weight::Normal,
        colors.text_secondary(),
    );
    popup(colors, &response, 8, RectAlign::BOTTOM, |ui| {
        let mut out = None;
        let width = BORDER_STYLES
            .iter()
            .map(|b| {
                text_width(ui, border_style_name(*b), 14.0, Weight::Normal) + 16.0 + 36.0 + 16.0
            })
            .fold(132.0f32, f32::max);
        for b in BORDER_STYLES {
            let (resp, paint) = picker_item(ui, colors, vec2(width, 32.0), false);
            let r = paint.rect.shrink2(vec2(8.0, 0.0));
            style::paint_text(
                ui,
                ui.painter(),
                r,
                Align2::LEFT_CENTER,
                border_style_name(b),
                14.0,
                Weight::Normal,
                colors.text(),
            );
            border_preview(
                ui.painter(),
                pos2(r.right() - 36.0, r.center().y - 7.0),
                b,
                colors.text(),
            );
            if resp.on_hover_text(border_style_name(b)).clicked() {
                out = Some(b);
                ui.close();
            }
        }
        out
    })
}

pub fn arrow_style_width(ui: &Ui, value: ArrowStyle) -> f32 {
    label_width(ui, arrow_style_name(value), 52.0, 80.0, 160.0)
}

pub fn arrow_style_dropdown(
    ui: &mut Ui,
    colors: &Colors,
    id: Id,
    rect: Rect,
    value: ArrowStyle,
    tooltip: &str,
) -> Option<ArrowStyle> {
    let (response, paint, open) = dropdown_button(ui, colors, id, rect, true, tooltip);
    let right = paint.rect.right() - 10.0;
    paint_chevron(ui, colors, &paint, open, right - CHEVRON_WIDTH / 2.0);
    let preview_left = right - CHEVRON_WIDTH - 8.0 - 36.0;
    let painter = ui.painter().clone();
    arrow_preview(
        ui,
        &painter,
        pos2(preview_left, paint.rect.center().y - 7.0),
        value,
        colors.text_secondary(),
    );
    style::paint_text(
        ui,
        &painter,
        Rect::from_min_max(
            pos2(paint.rect.left() + 10.0, paint.rect.top()),
            pos2(preview_left - 8.0 - 6.0, paint.rect.bottom()),
        ),
        Align2::LEFT_CENTER,
        arrow_style_name(value),
        12.0,
        Weight::Normal,
        colors.text_secondary(),
    );
    popup(colors, &response, 8, RectAlign::BOTTOM, |ui| {
        let mut out = None;
        let width = ARROW_STYLES
            .iter()
            .map(|a| text_width(ui, arrow_style_name(*a), 14.0, Weight::Normal) + 16.0 + 36.0 + 8.0)
            .fold(122.0f32, f32::max);
        for a in ARROW_STYLES {
            let (resp, paint) = picker_item(ui, colors, vec2(width, 32.0), false);
            let r = paint.rect.shrink2(vec2(8.0, 0.0));
            let painter = ui.painter().clone();
            style::paint_text(
                ui,
                &painter,
                r,
                Align2::LEFT_CENTER,
                arrow_style_name(a),
                14.0,
                Weight::Normal,
                colors.text(),
            );
            arrow_preview(
                ui,
                &painter,
                pos2(r.right() - 36.0, r.center().y - 7.0),
                a,
                colors.text(),
            );
            if resp.on_hover_text(arrow_style_name(a)).clicked() {
                out = Some(a);
                ui.close();
            }
        }
        out
    })
}

fn cursor_preview(ui: &Ui, painter: &Painter, rect: Rect, c: CursorType) {
    let ppp = ui.ctx().pixels_per_point();
    let size = (24.0 * ppp).round() as u32;
    let key = format!("cursor-{}-{size}", c.name());
    if let Some(image) = cached_image(ui, &key, || crate::cursor::preview_bitmap(c, size)) {
        style::paint_image(ui, painter, &key, &image, rect, Color32::WHITE);
    }
}

pub fn cursor_type_width(ui: &Ui, value: CursorType) -> f32 {
    label_width(ui, cursor_type_name(value), 40.0, 90.0, 180.0)
}

pub fn cursor_type_dropdown(
    ui: &mut Ui,
    colors: &Colors,
    id: Id,
    rect: Rect,
    value: CursorType,
    tooltip: &str,
) -> Option<CursorType> {
    let (response, paint, open) = dropdown_button(ui, colors, id, rect, true, tooltip);
    let right = paint.rect.right() - 10.0;
    paint_chevron(ui, colors, &paint, open, right - CHEVRON_WIDTH / 2.0);
    let preview_left = right - CHEVRON_WIDTH - 8.0 - 24.0;
    let painter = ui.painter().clone();
    cursor_preview(
        ui,
        &painter,
        Rect::from_min_size(
            pos2(preview_left, paint.rect.center().y - 12.0),
            vec2(24.0, 24.0),
        ),
        value,
    );
    style::paint_text(
        ui,
        &painter,
        Rect::from_min_max(
            pos2(paint.rect.left() + 10.0, paint.rect.top()),
            pos2(preview_left - 8.0 - 6.0, paint.rect.bottom()),
        ),
        Align2::LEFT_CENTER,
        cursor_type_name(value),
        12.0,
        Weight::Normal,
        colors.text_secondary(),
    );
    popup(colors, &response, 8, RectAlign::BOTTOM, |ui| {
        let mut out = None;
        let width = CURSOR_TYPES
            .iter()
            .map(|c| text_width(ui, cursor_type_name(*c), 14.0, Weight::Normal) + 16.0 + 32.0)
            .fold(136.0f32, f32::max);
        egui::ScrollArea::vertical()
            .max_height(ui.ctx().content_rect().height() * 0.6)
            .show(ui, |ui| {
                for c in CURSOR_TYPES {
                    let (resp, paint) = picker_item(ui, colors, vec2(width, 32.0), c == value);
                    let r = paint.rect.shrink2(vec2(8.0, 0.0));
                    let painter = ui.painter().clone();
                    let fg = if c == value {
                        colors.accent_foreground()
                    } else {
                        colors.text()
                    };
                    style::paint_text(
                        ui,
                        &painter,
                        r,
                        Align2::LEFT_CENTER,
                        cursor_type_name(c),
                        14.0,
                        Weight::Normal,
                        fg,
                    );
                    cursor_preview(
                        ui,
                        &painter,
                        Rect::from_min_size(
                            pos2(r.right() - 24.0, r.center().y - 12.0),
                            vec2(24.0, 24.0),
                        ),
                        c,
                    );
                    if resp.on_hover_text(cursor_type_name(c)).clicked() {
                        out = Some(c);
                        ui.close();
                    }
                }
            });
        out
    })
}

// ---------------------------------------------------------------------------- fonts

/// A font family's name in the family itself (`StringToFontFamilyConverter`), drawn
/// with the editor's text engine since egui knows only its own fonts.
fn family_name_image(
    ui: &Ui,
    family: &str,
    size: f32,
    color: Color32,
) -> Option<(String, Arc<image::RgbaImage>, Vec2)> {
    let ppp = ui.ctx().pixels_per_point();
    let key = format!("family-{family}-{size}-{:?}-{ppp}", color.to_array());
    let request = crate::text::FontRequest {
        family: family.to_string(),
        bold: false,
        italic: false,
    };
    let laid = crate::text::layout(family, &request, size, None, TextAlign::Left);
    let logical = vec2(laid.width.max(1.0), laid.height.max(1.0));
    let image = cached_image(ui, &key, || {
        let w = (logical.x * ppp).ceil() as u32 + 2;
        let h = (logical.y * ppp).ceil() as u32 + 2;
        let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
        let path = crate::text::build_geometry(&laid, 0.0, 0.0)?;
        let c = crate::color::Color::rgba(color.r(), color.g(), color.b(), color.a());
        crate::render::fill_path(
            &mut pixmap,
            &path,
            c,
            tiny_skia::Transform::from_scale(ppp, ppp),
            None,
        );
        Some(crate::raster::from_pixmap(&pixmap))
    })?;
    Some((key, image, logical))
}

fn paint_family_name(
    ui: &Ui,
    painter: &Painter,
    rect: Rect,
    family: &str,
    size: f32,
    color: Color32,
) {
    match family_name_image(ui, family, size, color) {
        Some((key, image, logical)) => {
            let ppp = ui.ctx().pixels_per_point();
            let shown = vec2(image.width() as f32 / ppp, image.height() as f32 / ppp);
            let top = rect.center().y - logical.y / 2.0;
            let clip = painter.clip_rect().intersect(rect);
            style::paint_image(
                ui,
                &painter.with_clip_rect(clip),
                &key,
                &image,
                Rect::from_min_size(pos2(rect.left(), top), shown),
                Color32::WHITE,
            );
        }
        None => {
            style::paint_text(
                ui,
                painter,
                rect,
                Align2::LEFT_CENTER,
                family,
                size,
                Weight::Normal,
                color,
            );
        }
    }
}

pub fn font_family_width(ui: &Ui, value: &str) -> f32 {
    let w = family_name_image(ui, value, 12.0, Color32::WHITE)
        .map(|(_, _, s)| s.x)
        .unwrap_or_else(|| text_width(ui, value, 12.0, Weight::Normal));
    (w + 6.0 + CHEVRON_WIDTH + 20.0 + 2.0).clamp(180.0, 240.0)
}

pub fn font_family_dropdown(
    ui: &mut Ui,
    colors: &Colors,
    id: Id,
    rect: Rect,
    value: &str,
    tooltip: &str,
) -> Option<String> {
    let (response, paint, open) = dropdown_button(ui, colors, id, rect, true, tooltip);
    let right = paint.rect.right() - 10.0;
    paint_chevron(ui, colors, &paint, open, right - CHEVRON_WIDTH / 2.0);
    let painter = ui.painter().clone();
    paint_family_name(
        ui,
        &painter,
        Rect::from_min_max(
            pos2(paint.rect.left() + 10.0, paint.rect.top()),
            pos2(right - CHEVRON_WIDTH - 6.0, paint.rect.bottom()),
        ),
        value,
        12.0,
        colors.text_secondary(),
    );
    popup(colors, &response, 8, RectAlign::BOTTOM, |ui| {
        let mut out = None;
        let families = crate::text::system_families();
        egui::ScrollArea::vertical().max_height(360.0).show_rows(
            ui,
            32.0,
            families.len(),
            |ui, range| {
                for family in &families[range] {
                    let (resp, paint) = picker_item(ui, colors, vec2(220.0, 32.0), false);
                    let painter = ui.painter().clone();
                    paint_family_name(
                        ui,
                        &painter,
                        paint.rect.shrink2(vec2(8.0, 0.0)),
                        family,
                        14.0,
                        colors.text(),
                    );
                    if resp.on_hover_text(family.as_str()).clicked() {
                        out = Some(family.clone());
                        ui.close();
                    }
                }
            },
        );
        out
    })
}

// ---------------------------------------------------------------------------- alignment

pub fn text_align_dropdown(
    ui: &mut Ui,
    colors: &Colors,
    id: Id,
    rect: Rect,
    value: TextAlign,
    tooltip: &str,
) -> Option<TextAlign> {
    let (response, paint, open) = dropdown_button(ui, colors, id, rect, true, tooltip);
    style::paint_icon(
        ui,
        ui.painter(),
        pos2(paint.rect.left() + 10.0 + 9.0, paint.rect.center().y),
        align_icon(value),
        18.0,
        colors.text(),
    );
    paint_chevron(
        ui,
        colors,
        &paint,
        open,
        paint.rect.left() + 10.0 + 18.0 + 8.0 + CHEVRON_WIDTH / 2.0,
    );
    popup(colors, &response, 8, RectAlign::BOTTOM, |ui| {
        let mut out = None;
        let width = TextAlign::ALL
            .iter()
            .map(|a| text_width(ui, align_name(*a), 14.0, Weight::Normal) + 16.0 + 24.0)
            .fold(132.0f32, f32::max);
        for a in TextAlign::ALL {
            let (resp, paint) = picker_item(ui, colors, vec2(width, 32.0), false);
            let r = paint.rect.shrink2(vec2(8.0, 0.0));
            style::paint_icon(
                ui,
                ui.painter(),
                pos2(r.left() + 8.0, r.center().y),
                align_icon(a),
                16.0,
                colors.text(),
            );
            style::paint_text(
                ui,
                ui.painter(),
                Rect::from_min_max(pos2(r.left() + 24.0, r.top()), r.max),
                Align2::LEFT_CENTER,
                align_name(a),
                14.0,
                Weight::Normal,
                colors.text(),
            );
            if resp.clicked() {
                out = Some(a);
                ui.close();
            }
        }
        out
    })
}

// ---------------------------------------------------------------------------- sizes and numbers

pub fn format_font_size(size: f32) -> String {
    if size.fract() == 0.0 {
        format!("{} pt", size as i64)
    } else {
        format!("{size} pt")
    }
}

pub fn font_size_width(ui: &Ui, value: f32) -> f32 {
    (text_width(ui, &format_font_size(value), 12.0, Weight::Normal)
        + 6.0
        + CHEVRON_WIDTH
        + 20.0
        + 2.0)
        .max(70.0)
}

pub fn font_size_dropdown(
    ui: &mut Ui,
    colors: &Colors,
    id: Id,
    rect: Rect,
    value: f32,
    tooltip: &str,
) -> Option<f32> {
    let (response, paint, open) = dropdown_button(ui, colors, id, rect, true, tooltip);
    style::paint_text(
        ui,
        ui.painter(),
        paint.rect.shrink2(vec2(10.0, 0.0)),
        Align2::LEFT_CENTER,
        &format_font_size(value),
        12.0,
        Weight::Normal,
        colors.text_secondary(),
    );
    paint_chevron(
        ui,
        colors,
        &paint,
        open,
        paint.rect.right() - 10.0 - CHEVRON_WIDTH / 2.0,
    );
    popup(colors, &response, 8, RectAlign::BOTTOM, |ui| {
        let mut out = None;
        let width = FONT_SIZE_OPTIONS
            .iter()
            .map(|v| text_width(ui, &format_font_size(*v), 12.0, Weight::Normal) + 16.0)
            .fold(50.0f32, f32::max);
        for v in FONT_SIZE_OPTIONS {
            let (resp, paint) = picker_item(ui, colors, vec2(width, 32.0), false);
            style::paint_text(
                ui,
                ui.painter(),
                paint.rect,
                Align2::CENTER_CENTER,
                &format_font_size(v),
                12.0,
                Weight::Normal,
                colors.text(),
            );
            if resp.clicked() {
                out = Some(v);
                ui.close();
            }
        }
        out
    })
}

pub fn step_type_width(ui: &Ui, value: StepType) -> f32 {
    (text_width(ui, value.preview_text(), 12.0, Weight::Normal) + 8.0 + CHEVRON_WIDTH + 20.0 + 2.0)
        .max(90.0)
}

pub fn step_type_dropdown(
    ui: &mut Ui,
    colors: &Colors,
    id: Id,
    rect: Rect,
    value: StepType,
    tooltip: &str,
) -> Option<StepType> {
    let (response, paint, open) = dropdown_button(ui, colors, id, rect, true, tooltip);
    style::paint_text(
        ui,
        ui.painter(),
        Rect::from_min_max(
            pos2(paint.rect.left() + 10.0, paint.rect.top()),
            pos2(
                paint.rect.right() - 10.0 - CHEVRON_WIDTH - 8.0,
                paint.rect.bottom(),
            ),
        ),
        Align2::LEFT_CENTER,
        value.preview_text(),
        12.0,
        Weight::Normal,
        colors.text(),
    );
    paint_chevron(
        ui,
        colors,
        &paint,
        open,
        paint.rect.right() - 10.0 - CHEVRON_WIDTH / 2.0,
    );
    popup(colors, &response, 8, RectAlign::BOTTOM, |ui| {
        let mut out = None;
        let width = STEP_TYPES
            .iter()
            .map(|v| text_width(ui, v.preview_text(), 12.0, Weight::Normal) + 16.0)
            .fold(118.0f32, f32::max);
        for v in STEP_TYPES {
            let (resp, paint) = picker_item(ui, colors, vec2(width, 32.0), false);
            style::paint_text(
                ui,
                ui.painter(),
                paint.rect.shrink2(vec2(8.0, 0.0)),
                Align2::LEFT_CENTER,
                v.preview_text(),
                12.0,
                Weight::Normal,
                colors.text(),
            );
            if resp.clicked() {
                out = Some(v);
                ui.close();
            }
        }
        out
    })
}

pub fn number_width(ui: &Ui, value: i32) -> f32 {
    (text_width(ui, &value.to_string(), 12.0, Weight::Normal)
        + 4.0
        + text_width(ui, "...", 12.0, Weight::Normal)
        + CHEVRON_WIDTH
        + 20.0
        + 4.0)
        .max(60.0)
}

pub fn number_dropdown(
    ui: &mut Ui,
    colors: &Colors,
    id: Id,
    rect: Rect,
    value: i32,
    options: &[i32],
    tooltip: &str,
) -> Option<i32> {
    let (response, paint, open) = dropdown_button(ui, colors, id, rect, true, tooltip);
    let painter = ui.painter().clone();
    let first = style::paint_text(
        ui,
        &painter,
        paint.rect.shrink2(vec2(10.0, 0.0)),
        Align2::LEFT_CENTER,
        &value.to_string(),
        12.0,
        Weight::Normal,
        colors.text(),
    );
    style::paint_text(
        ui,
        &painter,
        Rect::from_min_max(pos2(first.right() + 4.0, paint.rect.top()), paint.rect.max),
        Align2::LEFT_CENTER,
        "...",
        12.0,
        Weight::Normal,
        colors.text_secondary(),
    );
    paint_chevron(
        ui,
        colors,
        &paint,
        open,
        paint.rect.right() - 10.0 - CHEVRON_WIDTH / 2.0,
    );
    popup(colors, &response, 8, RectAlign::BOTTOM, |ui| {
        let mut out = None;
        for v in options {
            let (resp, paint) = picker_item(ui, colors, vec2(50.0, 32.0), false);
            style::paint_text(
                ui,
                ui.painter(),
                paint.rect,
                Align2::CENTER_CENTER,
                &v.to_string(),
                12.0,
                Weight::Normal,
                colors.text(),
            );
            if resp.clicked() {
                out = Some(*v);
                ui.close();
            }
        }
        out
    })
}

/// What the zoom picker asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZoomEvent {
    Zoom(f64),
    Fit,
}

pub fn format_percent(v: f64) -> String {
    format!("{}%", (v * 100.0).round() as i64)
}

pub fn zoom_width(ui: &Ui, value: f64) -> f32 {
    (10.0
        + 16.0
        + 6.0
        + text_width(ui, &format_percent(value), 12.0, Weight::SemiBold)
        + 8.0
        + CHEVRON_WIDTH
        + 4.0
        + 10.0
        + 2.0)
        .max(80.0)
}

/// `ZoomPickerDropdown`: opens upwards with the levels and "Zoom to fit".
pub fn zoom_dropdown(
    ui: &mut Ui,
    colors: &Colors,
    id: Id,
    rect: Rect,
    value: f64,
    tooltip: &str,
) -> Option<ZoomEvent> {
    let (response, paint, open) = dropdown_button(ui, colors, id, rect, true, tooltip);
    let painter = ui.painter().clone();
    style::paint_icon(
        ui,
        &painter,
        pos2(paint.rect.left() + 10.0 + 8.0, paint.rect.center().y),
        crate::ui::icons::ZOOM,
        16.0,
        colors.text_secondary(),
    );
    let chevron_x = paint.rect.right() - 10.0 - 4.0 - CHEVRON_WIDTH / 2.0;
    style::paint_text(
        ui,
        &painter,
        Rect::from_min_max(
            pos2(paint.rect.left() + 10.0 + 16.0 + 6.0, paint.rect.top()),
            pos2(chevron_x - CHEVRON_WIDTH / 2.0 - 8.0, paint.rect.bottom()),
        ),
        Align2::CENTER_CENTER,
        &format_percent(value),
        12.0,
        Weight::SemiBold,
        colors.text_secondary(),
    );
    paint_chevron(ui, colors, &paint, open, chevron_x);
    popup(colors, &response, 8, RectAlign::TOP, |ui| {
        let mut out = None;
        let width = rect.width().max(60.0);
        for v in ZOOM_OPTIONS {
            let (r, resp) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            style::paint_text(
                ui,
                ui.painter(),
                r,
                Align2::CENTER_CENTER,
                &format_percent(v),
                12.0,
                Weight::SemiBold,
                colors.text_secondary(),
            );
            if resp.clicked() {
                out = Some(ZoomEvent::Zoom(v));
                ui.close();
            }
        }
        let (sep, _) = ui.allocate_exact_size(vec2(width, 9.0), Sense::hover());
        ui.painter().rect_filled(
            Rect::from_min_max(
                pos2(sep.left() + 4.0, sep.center().y),
                pos2(sep.right() - 4.0, sep.center().y + 1.0),
            ),
            0.0,
            colors.separator(),
        );
        let (r, resp) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
        style::paint_text(
            ui,
            ui.painter(),
            r,
            Align2::CENTER_CENTER,
            t(s::ZoomPickerDropdown_ZoomtoFit),
            12.0,
            Weight::SemiBold,
            colors.text_secondary(),
        );
        if resp.clicked() {
            out = Some(ZoomEvent::Fit);
            ui.close();
        }
        ui.add_space(6.0);
        out
    })
}

// ---------------------------------------------------------------------------- strength

pub const STRENGTH_WIDTH: f32 = 24.0 + 5.0 + 120.0 + 4.0;

/// `StrengthSlider`: the value (24 wide, centred) and a 120-point slider, snapping to
/// whole numbers.
#[allow(clippy::too_many_arguments)]
pub fn strength_slider(
    ui: &mut Ui,
    colors: &Colors,
    id: Id,
    rect: Rect,
    value: f32,
    min: f32,
    max: f32,
    tooltip: &str,
) -> Option<f32> {
    let painter = ui.painter().clone();
    style::paint_text(
        ui,
        &painter,
        Rect::from_min_size(rect.min, vec2(24.0, rect.height())),
        Align2::CENTER_CENTER,
        &format!("{}", value.round() as i64),
        12.0,
        Weight::SemiBold,
        colors.text_secondary(),
    );
    let slider_rect = Rect::from_min_size(
        pos2(rect.left() + 29.0, rect.top()),
        vec2(120.0, rect.height()),
    );
    let hover = ui.interact(rect, id.with("hover"), Sense::hover());
    if !tooltip.is_empty() {
        hover.on_hover_text(tooltip);
    }
    style::slider(
        ui,
        colors,
        id,
        slider_rect,
        value as f64,
        min as f64,
        max as f64,
        Some(1.0),
        true,
    )
    .map(|v| v as f32)
}
