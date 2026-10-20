//! `ColorPickerWindow` (ShareX.HelpersLib): the colour box and slider, every colour model
//! in boxes that edit each other, the current and previous swatches, copy in six formats,
//! the recent and standard palettes, pick from the screen and from the clipboard. The
//! Tools menu opens it on its own; other windows can open it to choose a colour
//! (`selection_mode`: OK and Cancel, always on top).

use egui::{Align2, Color32, Id, Rect, Sense, Stroke, StrokeKind, TextureHandle, Ui, pos2, vec2};
use modlight_core::l10n::{helpers as h, t};
use modlight_tools::color::{self, Color, ColorFormat};
use modlight_tools::color_picker::{self as picker, DrawStyle};
use modlight_tools::settings::{ColorPickerOptions, ScreenColorPickerOptions};

use super::kit::{self, Track};
use super::{ToolWindow, WindowSpec};
use crate::state::AppState;
use crate::theme::Weight;

pub const WIDTH: f32 = 790.0;
const PICKER_W: f32 = 342.0;
const PICKER_H: f32 = 300.0;

/// What the window is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// The Tools menu's colour picker.
    Tool,
    /// Choose a colour (`PickColor`): the answer is `selected` when the window closes.
    Select,
}

pub struct ColorPickerWindow {
    pub key: u64,
    pub purpose: Purpose,
    pub current: Color,
    pub previous: Color,
    pub has_previous: bool,
    pub style: DrawStyle,
    pub options: ColorPickerOptions,
    pub screen_options: ScreenColorPickerOptions,
    pub recent: Vec<Color>,
    hex_text: String,
    dragging: Option<bool>,
    textures: Option<((DrawStyle, Color), TextureHandle, TextureHandle)>,
    opened: bool,
    /// The window minimises while the screen picker runs.
    pub minimized: bool,
    pub selected: Option<Color>,
    height: f32,
}

impl ColorPickerWindow {
    pub fn new(
        key: u64,
        purpose: Purpose,
        current: Color,
        options: ColorPickerOptions,
        screen_options: ScreenColorPickerOptions,
        recent: Vec<Color>,
    ) -> ColorPickerWindow {
        let selection = purpose != Purpose::Tool;
        ColorPickerWindow {
            key,
            purpose,
            current,
            previous: current,
            has_previous: selection,
            style: DrawStyle::Hue,
            options,
            screen_options,
            recent,
            hex_text: format!("#{}", color::to_hex(current, ColorFormat::Rgb)),
            dragging: None,
            textures: None,
            opened: false,
            minimized: false,
            selected: None,
            height: 526.0,
        }
    }

    fn selection_mode(&self) -> bool {
        self.purpose != Purpose::Tool
    }

    /// `SetColor`.
    pub fn set_color(&mut self, color: Color) {
        self.current = color;
        self.hex_text = format!("#{}", color::to_hex(color, ColorFormat::Rgb));
    }

    /// A colour from outside (the screen, the clipboard, a swatch): the old one becomes
    /// the previous colour.
    pub fn take_color(&mut self, color: Color) {
        self.previous = self.current;
        self.has_previous = true;
        self.set_color(color);
    }

    /// `OnPickScreenClick`'s ending: the picked colour, also into the recent list.
    pub fn screen_picked(&mut self, color: Color) {
        self.take_color(color);
        picker::add_recent(&mut self.recent, self.current);
    }

    fn textures(&mut self, ui: &Ui, box_w: u32, h: u32) -> (TextureHandle, TextureHandle) {
        let key = (self.style, self.current);
        if let Some((k, a, b)) = &self.textures
            && *k == key
        {
            return (a.clone(), b.clone());
        }
        let make = |w: u32, slider: bool, name: &str| {
            let pixels = picker::bitmap(self.style, self.current, w, h, slider);
            let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &pixels);
            ui.ctx()
                .load_texture(name, image, egui::TextureOptions::NEAREST)
        };
        let a = make(box_w, false, &format!("color-picker-box-{}", self.key));
        let b = make(
            picker::SLIDER_WIDTH as u32,
            true,
            &format!("color-picker-slider-{}", self.key),
        );
        self.textures = Some((key, a.clone(), b.clone()));
        (a, b)
    }

    /// The control (`AdvancedColorPickerControl`) in `rect`.
    fn picker_ui(&mut self, ui: &mut Ui, rect: Rect) {
        let ((_, bw), (sx, sw)) = picker::regions(rect.width() as f64);
        let box_rect = Rect::from_min_size(rect.min, vec2(bw as f32, rect.height()));
        let slider_rect = Rect::from_min_size(
            pos2(rect.left() + sx as f32, rect.top()),
            vec2(sw as f32, rect.height()),
        );
        let response = ui.interact(
            rect,
            Id::new(("color-picker-control", self.key)),
            Sense::drag(),
        );
        if response.drag_started()
            && let Some(p) = response.interact_pointer_pos()
        {
            if box_rect.contains(p) {
                self.dragging = Some(false);
            } else if slider_rect.contains(p) {
                self.dragging = Some(true);
            }
        }
        if let (Some(on_slider), Some(p)) = (self.dragging, response.interact_pointer_pos())
            && (response.dragged() || response.drag_started() || response.drag_stopped())
        {
            let local = p - rect.min;
            let picked = picker::pick(
                self.style,
                self.current,
                rect.width() as f64,
                rect.height() as f64,
                local.x as f64,
                local.y as f64,
                on_slider,
            );
            self.set_color(picked);
        }
        if response.drag_stopped() {
            self.dragging = None;
        }
        let (a, b) = self.textures(
            ui,
            (bw.round() as u32).max(1),
            (rect.height().round() as u32).max(1),
        );
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let painter = ui.painter();
        painter.image(a.id(), box_rect, uv, Color32::WHITE);
        painter.image(b.id(), slider_rect, uv, Color32::WHITE);
        let border = Stroke::new(1.0, Color32::from_rgb(95, 95, 95));
        painter.rect_stroke(box_rect, 0.0, border, StrokeKind::Middle);
        painter.rect_stroke(slider_rect, 0.0, border, StrokeKind::Middle);
        let (px, py) = picker::box_point(self.style, self.current);
        let center = pos2(
            box_rect.left() + px as f32 * box_rect.width(),
            box_rect.top() + py as f32 * box_rect.height(),
        );
        painter.circle_stroke(center, 6.0, Stroke::new(3.0, Color32::BLACK));
        painter.circle_stroke(center, 6.0, Stroke::new(1.5, Color32::WHITE));
        let y = slider_rect.top()
            + picker::slider_y(self.style, self.current) as f32 * slider_rect.height();
        let marker = Rect::from_min_max(
            pos2(slider_rect.left() - 3.0, y - 5.0),
            pos2(slider_rect.right() + 3.0, y + 5.0),
        );
        painter.rect_stroke(
            marker,
            0.0,
            Stroke::new(3.0, Color32::BLACK),
            StrokeKind::Middle,
        );
        painter.rect_stroke(
            marker,
            0.0,
            Stroke::new(1.0, Color32::WHITE),
            StrokeKind::Middle,
        );
    }

    fn copy(&mut self, state: &mut AppState, text: String) {
        picker::add_recent(&mut self.recent, self.current);
        state.set_recent_colors(&self.recent);
        state.copy_text(&text);
    }
}

fn swatch(ui: &Ui, colors: &crate::theme::Colors, rect: Rect, color: Option<Color>) {
    crate::widgets::paint_checkered(ui, colors, rect, 4.0);
    if let Some(c) = color {
        ui.painter().rect_filled(
            rect,
            4.0,
            Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a),
        );
    }
    ui.painter().rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
}

impl ToolWindow for ColorPickerWindow {
    fn spec(&self) -> WindowSpec {
        WindowSpec {
            title: t(h::ColorPickerWindow_Title).to_string(),
            size: vec2(WIDTH, self.height),
            min_size: None,
            topmost: self.selection_mode(),
            fit_height: true,
            screen: None,
        }
    }

    fn content_height(&self) -> Option<f32> {
        Some(self.height)
    }

    fn ui(&mut self, ui: &mut Ui, state: &mut AppState) -> bool {
        let colors = state.colors.clone();
        if !self.opened {
            self.opened = true;
            // `OnOpened`: a colour on the clipboard is taken at once.
            if let Some(text) = state.clipboard_text()
                && let Some(c) = color::parse_color(&text)
            {
                self.take_color(c);
            }
        }
        if self.minimized {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        kit::fill(ui, &colors);
        let full = kit::inset(ui.max_rect(), 16.0);
        let mut close = false;

        // Row 0: the picker panel and the values panel.
        let top_h = 12.0 + PICKER_H + 10.0 + 40.0 + 12.0;
        let top = Rect::from_min_size(full.min, vec2(full.width(), top_h));
        let cols = kit::columns(top, &[Track::Px(370.0), Track::Star(1.0)], 12.0);
        kit::panel(ui, &colors, cols[0]);
        let inner = cols[0].shrink(12.0);
        let picker_rect = Rect::from_min_size(
            pos2(inner.center().x - PICKER_W / 2.0, inner.top()),
            vec2(PICKER_W, PICKER_H),
        );
        self.picker_ui(ui, picker_rect);
        let mode_row = Rect::from_min_size(
            pos2(inner.left(), picker_rect.bottom() + 10.0),
            vec2(inner.width(), 40.0),
        );
        let label_w = kit::text_width(ui, t(h::ColorPickerWindow_Color_mode), 13.0, Weight::Normal);
        kit::text_at(
            ui,
            Rect::from_min_size(mode_row.min, vec2(label_w, 40.0)),
            Align2::LEFT_CENTER,
            t(h::ColorPickerWindow_Color_mode),
            13.0,
            Weight::Normal,
            colors.text_secondary(),
        );
        let combo = Rect::from_min_max(
            pos2(mode_row.left() + label_w + 10.0, mode_row.top()),
            mode_row.max,
        );
        let modes: Vec<(DrawStyle, String)> = DrawStyle::ALL
            .iter()
            .zip([
                h::ColorPickerWindow_Hue,
                h::ColorPickerWindow_Saturation,
                h::ColorPickerWindow_Brightness,
                h::ColorPickerWindow_Red,
                h::ColorPickerWindow_Green,
                h::ColorPickerWindow_Blue,
            ])
            .map(|(s, k)| (*s, t(k).to_string()))
            .collect();
        if let Some(style) = kit::combo_at(
            ui,
            &colors,
            combo,
            &format!("cp-mode-{}", self.key),
            &self.style,
            &modes,
            true,
        ) {
            self.style = style;
        }

        // The values panel: five rows of a label over a box, 9 apart.
        kit::panel(ui, &colors, cols[1]);
        let values = cols[1].shrink(12.0);
        let row_h = 16.0 + 4.0 + 40.0;
        let rows = kit::rows(
            Rect::from_min_size(values.min, vec2(values.width(), row_h * 5.0 + 9.0 * 4.0)),
            &[Track::Px(row_h); 5],
            9.0,
        );
        let label_center = |ui: &Ui, r: Rect, text: &str| {
            kit::text_at(
                ui,
                Rect::from_min_size(r.min, vec2(r.width(), 16.0)),
                Align2::CENTER_CENTER,
                text,
                13.0,
                Weight::Normal,
                colors.text_secondary(),
            );
        };
        let field =
            |r: Rect| Rect::from_min_size(pos2(r.left(), r.top() + 20.0), vec2(r.width(), 40.0));
        let c = self.current;
        let hsb = color::to_hsb(c);
        let cmyk = color::to_cmyk(c);

        // Name, hex, decimal.
        let r0 = kit::columns(rows[0], &[Track::Star(1.0); 3], 8.0);
        label_center(ui, r0[0], t(h::ColorPickerWindow_Color_name));
        kit::read_only_box_at(ui, &colors, field(r0[0]), &color::color_name(c), true);
        label_center(ui, r0[1], t(h::ColorPickerWindow_Hex));
        let hex_id = Id::new(("cp-hex", self.key));
        let mut hex = self.hex_text.clone();
        if kit::text_box_at(ui, &colors, field(r0[1]), hex_id, &mut hex, "", true, true).changed() {
            self.hex_text = hex.clone();
            // `OnHexTextChanged`: eight digits are RGBA.
            let text = hex.trim();
            let format = if text.trim_start_matches('#').len() == 8 {
                ColorFormat::Rgba
            } else {
                ColorFormat::Rgb
            };
            if let Some(parsed) = color::hex_to_color(text, format) {
                self.current = parsed;
            }
        }
        label_center(ui, r0[2], t(h::ColorPickerWindow_Decimal));
        if let Some(v) = kit::plain_number_at(
            ui,
            &colors,
            field(r0[2]),
            Id::new(("cp-dec", self.key)),
            color::to_decimal(c, ColorFormat::Rgb) as i64,
            0,
            16_777_215,
            true,
        ) {
            self.set_color(color::decimal_to_color(v, ColorFormat::Rgb));
        }

        // RGBA.
        let r1 = kit::columns(rows[1], &[Track::Star(1.0); 4], 8.0);
        let rgba = [
            (h::ColorPickerWindow_Red, c.r),
            (h::ColorPickerWindow_Green, c.g),
            (h::ColorPickerWindow_Blue, c.b),
            (h::ColorPickerWindow_Alpha, c.a),
        ];
        for (i, (key, v)) in rgba.iter().enumerate() {
            label_center(ui, r1[i], t(*key));
            if let Some(nv) = kit::plain_number_at(
                ui,
                &colors,
                field(r1[i]),
                Id::new(("cp-rgba", self.key, i)),
                *v as i64,
                0,
                255,
                true,
            ) {
                let mut parts = [c.a as i64, c.r as i64, c.g as i64, c.b as i64];
                let index = [1, 2, 3, 0][i];
                parts[index] = nv;
                self.set_color(Color::from_ints(parts[0], parts[1], parts[2], parts[3]));
            }
        }

        // CMYK (row 2), then HSB (row 3), as the grid places them.
        let r2 = kit::columns(rows[2], &[Track::Star(1.0); 4], 8.0);
        let cmyk_values = [
            (h::ColorPickerWindow_Cyan_percent, cmyk.cyan100()),
            (h::ColorPickerWindow_Magenta_percent, cmyk.magenta100()),
            (h::ColorPickerWindow_Yellow_percent, cmyk.yellow100()),
            (h::ColorPickerWindow_Key_percent, cmyk.key100()),
        ];
        let rounded: Vec<i64> = cmyk_values
            .iter()
            .map(|(_, v)| v.round_ties_even() as i64)
            .collect();
        for (i, (key, _)) in cmyk_values.iter().enumerate() {
            label_center(ui, r2[i], t(*key));
            if let Some(nv) = kit::plain_number_at(
                ui,
                &colors,
                field(r2[i]),
                Id::new(("cp-cmyk", self.key, i)),
                rounded[i],
                0,
                100,
                true,
            ) {
                let mut parts = rounded.clone();
                parts[i] = nv;
                self.set_color(
                    color::Cmyk::from_ints(parts[0], parts[1], parts[2], parts[3], c.a as i64)
                        .to_color(),
                );
            }
        }
        let r3 = kit::columns(rows[3], &[Track::Star(1.0); 3], 8.0);
        let hsb_values = [
            (h::ColorPickerWindow_Hue_degrees, hsb.hue360(), 360),
            (
                h::ColorPickerWindow_Saturation_percent,
                hsb.saturation100(),
                100,
            ),
            (
                h::ColorPickerWindow_Brightness_percent,
                hsb.brightness100(),
                100,
            ),
        ];
        let rounded: Vec<i64> = hsb_values
            .iter()
            .map(|(_, v, _)| v.round_ties_even() as i64)
            .collect();
        for (i, (key, _, max)) in hsb_values.iter().enumerate() {
            label_center(ui, r3[i], t(*key));
            if let Some(nv) = kit::plain_number_at(
                ui,
                &colors,
                field(r3[i]),
                Id::new(("cp-hsb", self.key, i)),
                rounded[i],
                0,
                *max,
                true,
            ) {
                let mut parts = rounded.clone();
                parts[i] = nv;
                self.set_color(
                    color::Hsb::from_ints(parts[0], parts[1], parts[2], c.a as i64).to_color(),
                );
            }
        }

        // Current, previous, copy.
        let r4 = kit::columns(rows[4], &[Track::Star(1.0); 3], 8.0);
        label_center(ui, r4[0], t(h::ColorPickerWindow_Current));
        swatch(ui, &colors, field(r4[0]), Some(self.current));
        label_center(ui, r4[1], t(h::ColorPickerWindow_Previous));
        swatch(
            ui,
            &colors,
            field(r4[1]),
            self.has_previous.then_some(self.previous),
        );
        let previous = ui.interact(field(r4[1]), Id::new(("cp-prev", self.key)), Sense::click());
        if previous.clicked() && self.has_previous {
            let old = self.current;
            self.set_color(self.previous);
            self.previous = old;
        }
        let copy_rect = field(r4[2]);
        let look = crate::widgets::Look::button(&colors).width(copy_rect.width());
        let copy = {
            let mut child = kit::child(ui, copy_rect);
            crate::widgets::button_at(
                &mut child,
                &colors,
                &look,
                crate::widgets::Content {
                    icon: None,
                    label: None,
                },
                true,
                copy_rect,
            )
        };
        // Icon, label and a small chevron, centred as one row (`StackPanel Spacing="8"`).
        {
            let label = t(h::ColorPickerWindow_Copy);
            let icon_w = crate::widgets::icon_galley(
                ui,
                modlight_core::icons::COPY,
                15.0,
                colors.text_secondary(),
            )
            .size()
            .x;
            let chevron_w = crate::widgets::icon_galley(
                ui,
                modlight_core::icons::CHEVRON_DOWN,
                11.0,
                colors.text_secondary(),
            )
            .size()
            .x;
            let label_w = kit::text_width(ui, label, 13.0, Weight::Normal);
            let total = icon_w + 8.0 + label_w + 8.0 + chevron_w;
            let mut x = copy_rect.center().x - total / 2.0;
            let cy = copy_rect.center().y;
            crate::widgets::paint_icon(
                ui,
                ui.painter(),
                pos2(x + icon_w / 2.0, cy),
                modlight_core::icons::COPY,
                15.0,
                colors.text_secondary(),
            );
            x += icon_w + 8.0;
            kit::text_at(
                ui,
                Rect::from_min_size(
                    pos2(x, copy_rect.top()),
                    vec2(label_w + 2.0, copy_rect.height()),
                ),
                Align2::LEFT_CENTER,
                label,
                13.0,
                Weight::Normal,
                colors.text_secondary(),
            );
            x += label_w + 8.0;
            crate::widgets::paint_icon(
                ui,
                ui.painter(),
                pos2(x + chevron_w / 2.0, cy),
                modlight_core::icons::CHEVRON_DOWN,
                11.0,
                colors.text_secondary(),
            );
        }
        let items = [
            h::ColorPickerWindow_Copy_all_details,
            h::ColorPickerWindow_Copy_RGB,
            h::ColorPickerWindow_Copy_hexadecimal,
            h::ColorPickerWindow_Copy_HSB,
            h::ColorPickerWindow_Copy_CMYK,
            h::ColorPickerWindow_Copy_decimal,
        ]
        .iter()
        .map(|k| t(*k).to_string())
        .collect::<Vec<_>>();
        if let Some(index) =
            kit::flyout(ui, &colors, &copy, &format!("cp-copy-{}", self.key), &items)
        {
            let c = self.current;
            let text = match index {
                0 => color::details_text(c),
                1 => format!("{}, {}, {}", c.r, c.g, c.b),
                2 => format!("#{}", color::to_hex(c, ColorFormat::Rgb)),
                3 => color::hsb_text(c),
                4 => color::cmyk_text(c),
                _ => color::to_decimal(c, ColorFormat::Rgb).to_string(),
            };
            self.copy(state, text);
        }

        // Row 1: the palette panel (370 wide) and the buttons.
        let bottom_top = top.bottom() + 12.0;
        let swatches_per_row = ((370.0 - 24.0) / 21.0f32).floor().max(1.0) as usize;
        let list: Vec<Color> = if self.options.recent_colors_selected {
            self.recent.clone()
        } else {
            color::STANDARD_COLORS.to_vec()
        };
        let list: Vec<Color> = list.into_iter().take(color::RECENT_COLORS_MAX).collect();
        let swatch_rows = list.len().div_ceil(swatches_per_row);
        let palette_h = 12.0 + 32.0 + 10.0 + swatch_rows as f32 * 21.0 + 12.0;
        let palette = Rect::from_min_size(pos2(full.left(), bottom_top), vec2(370.0, palette_h));
        kit::panel(ui, &colors, palette);
        let pin = palette.shrink(12.0);
        let recent_w = kit::radio_width(ui, t(h::ColorPickerWindow_Recent_colors));
        if kit::radio_at(
            ui,
            &colors,
            Rect::from_min_size(pin.min, vec2(recent_w, 32.0)),
            self.options.recent_colors_selected,
            t(h::ColorPickerWindow_Recent_colors),
            true,
        ) && !self.options.recent_colors_selected
        {
            self.options.recent_colors_selected = true;
            state.save_capture_options(&["SurfaceOptions", "ColorPickerOptions"], &self.options);
        }
        let standard_w = kit::radio_width(ui, t(h::ColorPickerWindow_Standard_colors));
        if kit::radio_at(
            ui,
            &colors,
            Rect::from_min_size(
                pos2(pin.left() + recent_w + 8.0, pin.top()),
                vec2(standard_w, 32.0),
            ),
            !self.options.recent_colors_selected,
            t(h::ColorPickerWindow_Standard_colors),
            true,
        ) && self.options.recent_colors_selected
        {
            self.options.recent_colors_selected = false;
            state.save_capture_options(&["SurfaceOptions", "ColorPickerOptions"], &self.options);
        }
        for (i, sw) in list.iter().enumerate() {
            let x = pin.left() + (i % swatches_per_row) as f32 * 21.0;
            let y = pin.top() + 42.0 + (i / swatches_per_row) as f32 * 21.0;
            let cell = Rect::from_min_size(pos2(x, y), vec2(21.0, 21.0));
            let response = ui.interact(cell, Id::new(("cp-swatch", self.key, i)), Sense::click());
            if response.hovered() {
                ui.painter()
                    .rect_filled(cell, 4.0, colors.control_background_hover());
            }
            ui.painter().rect_filled(
                cell.shrink(1.0),
                3.0,
                Color32::from_rgba_unmultiplied(sw.r, sw.g, sw.b, sw.a),
            );
            if response.clicked() {
                self.take_color(*sw);
            }
        }

        // The buttons, bottom right of the row.
        let mut x = full.right();
        let bottom = palette.bottom();
        if self.selection_mode() {
            let cancel = Rect::from_min_size(pos2(x - 96.0, bottom - 40.0), vec2(96.0, 40.0));
            x -= 96.0 + 8.0;
            let ok = Rect::from_min_size(pos2(x - 96.0, bottom - 40.0), vec2(96.0, 40.0));
            x -= 96.0 + 8.0;
            if kit::button_at(
                ui,
                &colors,
                cancel,
                None,
                Some(t(h::ColorPickerWindow_Cancel)),
                true,
            )
            .clicked()
            {
                close = true;
            }
            if kit::primary_at(
                ui,
                &colors,
                ok,
                None,
                Some(t(h::ColorPickerWindow_OK)),
                true,
            )
            .clicked()
                || ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && !ui.ctx().egui_wants_keyboard_input()
            {
                self.selected = Some(self.current);
                picker::add_recent(&mut self.recent, self.current);
                state.set_recent_colors(&self.recent);
                close = true;
            }
        }
        let clip = Rect::from_min_size(pos2(x - 38.0, bottom - 40.0), vec2(38.0, 40.0));
        x -= 38.0 + 8.0;
        let pipette = Rect::from_min_size(pos2(x - 38.0, bottom - 40.0), vec2(38.0, 40.0));
        if kit::button_at(
            ui,
            &colors,
            pipette,
            Some(modlight_core::icons::PIPETTE),
            None,
            true,
        )
        .on_hover_text(t(h::ColorPickerWindow_Pick_from_screen))
        .clicked()
        {
            self.minimized = true;
            state.start_screen_pick(
                super::screen_picker::PickTarget::ColorPicker(self.key),
                self.screen_options.clone(),
                0.25,
            );
        }
        if kit::button_at(
            ui,
            &colors,
            clip,
            Some(modlight_core::icons::CLIPBOARD_PASTE),
            None,
            true,
        )
        .on_hover_text(t(h::ColorPickerWindow_Pick_from_clipboard))
        .clicked()
            && let Some(text) = state.clipboard_text()
            && let Some(c) = color::parse_color(&text)
        {
            self.take_color(c);
        }

        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        self.height = (palette.bottom() + 16.0 - ui.max_rect().top()).round();
        close
    }

    fn closed(&mut self, state: &mut AppState) {
        if !self.selection_mode() {
            picker::add_recent(&mut self.recent, self.current);
            state.set_recent_colors(&self.recent);
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl AppState {
    /// `HelpersOptions.RecentColors` (`ApplicationConfig.RecentColors`).
    pub fn recent_colors(&self) -> Vec<Color> {
        self.config
            .get("RecentColors")
            .and_then(|v| v.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|v| v.as_str())
                    .filter_map(color::parse_dotnet_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn set_recent_colors(&mut self, colors: &[Color]) {
        let list: Vec<serde_json::Value> = colors
            .iter()
            .map(|c| serde_json::Value::String(color::to_dotnet_string(*c)))
            .collect();
        self.config
            .set("RecentColors", serde_json::Value::Array(list));
        self.save_config();
    }

    /// `ShowScreenColorPickerDialog`.
    pub fn open_color_picker(&mut self, settings: &modlight_core::tasks::settings::TaskSettings) {
        let options: ColorPickerOptions = modlight_tools::settings::read(
            self.config
                .get("DefaultTaskSettings")
                .and_then(|d| d.pointer("/CaptureSettings/SurfaceOptions/ColorPickerOptions")),
        );
        let screen: ScreenColorPickerOptions =
            super::jobs::tool_options(settings, "ScreenColorPickerOptions");
        let recent = self.recent_colors();
        self.tools.open_keyed(|key| {
            Box::new(ColorPickerWindow::new(
                key,
                Purpose::Tool,
                Color::RED,
                options,
                screen,
                recent,
            ))
        });
    }
}
