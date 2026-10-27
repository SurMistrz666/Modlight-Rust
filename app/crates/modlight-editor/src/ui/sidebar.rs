//! The sidebars of `EditorView`: the Background panel on the left (margin, padding,
//! smart padding, rounded corner, shadow, the output ratio, the background kind with
//! its gradient presets, colour or picture) and the right one with the effect dialog
//! (`SchemaDrivenEffectDialog`) or the editor's options (`EditorOptionsPanel`).

use egui::{Align2, Color32, Id, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};

use super::pickers::{self, ColorEvent};
use super::style::{self, ButtonKind, Colors, Weight};
use crate::color::Color;
use crate::editor::background::{BackgroundSettings, GRADIENT_PRESETS, OUTPUT_RATIOS};
use crate::editor::{ColorTarget, Editor, Request, SidePanel};
use crate::effects::{ParamKind, ParamValue};
use crate::frame::BackgroundMode;
use crate::ui::icons;
use modlight_core::l10n::{editor as s, t};

/// A change to the Background panel's values, applied after the panel is drawn.
type BackgroundChange = Box<dyn FnOnce(&mut BackgroundSettings)>;

/// One toggle of the options panel: its label, its value and how to set it.
type OptionRow = (
    &'static str,
    bool,
    fn(&mut crate::options::ImageEditorOptions, bool),
);

/// `editor-sidebar.open`: 282 wide.
pub const SIDEBAR_WIDTH: f32 = 282.0;
const CONTENT_WIDTH: f32 = 250.0;

/// The heading of a panel: 20 points, normal weight, the accent, 16 below.
fn heading(ui: &mut Ui, colors: &Colors, icon: char, title: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(CONTENT_WIDTH, 24.0), Sense::hover());
    let painter = ui.painter().clone();
    style::paint_icon(
        ui,
        &painter,
        pos2(r.left() + 10.0, r.center().y + 1.0),
        icon,
        20.0,
        colors.accent_start(),
    );
    style::paint_text(
        ui,
        &painter,
        Rect::from_min_max(pos2(r.left() + 28.0, r.top()), r.max),
        Align2::LEFT_CENTER,
        title,
        20.0,
        Weight::Normal,
        colors.accent_start(),
    );
    ui.add_space(16.0 + 8.0);
}

fn label(ui: &mut Ui, colors: &Colors, text: &str) {
    let g = style::galley_wrapped(
        ui,
        text,
        style::FONT_SIZE,
        Weight::SemiBold,
        colors.text_secondary(),
        CONTENT_WIDTH,
    );
    let (r, _) = ui.allocate_exact_size(vec2(CONTENT_WIDTH, g.size().y), Sense::hover());
    ui.painter().galley(r.min, g, colors.text_secondary());
}

/// Lays a sidebar out at `rect` and runs `content` in its scroll viewer.
fn sidebar_frame(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    left: bool,
    id: &str,
    content: impl FnOnce(&mut Ui),
) {
    ui.interact(rect, Id::new((id, "blocker")), Sense::click());
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, colors.background_panel());
    let x = if left {
        rect.right() - 0.5
    } else {
        rect.left() + 0.5
    };
    painter.line_segment(
        [pos2(x, rect.top()), pos2(x, rect.bottom())],
        Stroke::new(1.0, colors.border()),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(rect);
    egui::ScrollArea::vertical()
        .id_salt(id)
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            egui::Frame::new()
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    ui.set_width(CONTENT_WIDTH);
                    content(ui);
                });
        });
}

fn mode_name(mode: BackgroundMode) -> &'static str {
    t(match mode {
        BackgroundMode::Gradient => s::MainViewModel_BackgroundModeGradient,
        BackgroundMode::Color => s::MainViewModel_BackgroundModeColor,
        BackgroundMode::Transparent => s::MainViewModel_BackgroundModeTransparent,
        BackgroundMode::Image => s::MainViewModel_BackgroundModeImage,
        BackgroundMode::Wallpaper => s::MainViewModel_BackgroundModeWallpaper,
    })
}

fn to_color32(hex: &str) -> Color32 {
    pickers::to_color32(Color::parse(hex).unwrap_or(Color::TRANSPARENT))
}

/// The Background panel.
pub fn background_panel(ui: &mut Ui, colors: &Colors, editor: &mut Editor, rect: Rect) {
    sidebar_frame(ui, colors, rect, true, "editor-background-panel", |ui| {
        heading(
            ui,
            colors,
            icons::PANEL_BACKGROUND,
            t(s::EditorView_Background),
        );
        let b = editor.background.clone();
        let mut change: Option<BackgroundChange> = None;
        let px = |v: f64| format!("{v:.0} px");
        if let Some(v) = style::slider_row(
            ui,
            colors,
            Id::new("bg-margin"),
            CONTENT_WIDTH,
            t(s::EditorView_Margin),
            &px(b.margin),
            b.margin,
            0.0,
            200.0,
            None,
        ) {
            change = Some(Box::new(move |s| s.margin = v));
        }
        if let Some(v) = style::slider_row(
            ui,
            colors,
            Id::new("bg-padding"),
            CONTENT_WIDTH,
            t(s::EditorView_Padding),
            &px(b.padding),
            b.padding,
            0.0,
            200.0,
            None,
        ) {
            change = Some(Box::new(move |s| s.padding = v));
        }
        let can_smart = b.can_use_smart_padding(editor.has_image());
        if let Some(v) = style::labeled_toggle(
            ui,
            colors,
            CONTENT_WIDTH,
            t(s::EditorView_SmartPadding),
            b.smart_padding,
            can_smart,
        ) {
            change = Some(Box::new(move |s| s.smart_padding = v));
        }
        ui.add_space(16.0);
        if let Some(v) = style::slider_row(
            ui,
            colors,
            Id::new("bg-corner"),
            CONTENT_WIDTH,
            t(s::EditorView_RoundedCorner),
            &px(b.rounded_corner),
            b.rounded_corner,
            0.0,
            50.0,
            None,
        ) {
            change = Some(Box::new(move |s| s.rounded_corner = v));
        }
        if let Some(v) = style::slider_row(
            ui,
            colors,
            Id::new("bg-shadow"),
            CONTENT_WIDTH,
            t(s::EditorView_ShadowRadius),
            &px(b.shadow_radius),
            b.shadow_radius,
            0.0,
            100.0,
            None,
        ) {
            change = Some(Box::new(move |s| s.shadow_radius = v));
        }
        label(ui, colors, t(s::EditorView_Ratio));
        ui.add_space(16.0);
        let cell = CONTENT_WIDTH / 3.0;
        for row in OUTPUT_RATIOS.chunks(3) {
            let (line, _) = ui.allocate_exact_size(vec2(CONTENT_WIDTH, 36.0), Sense::hover());
            for (i, ratio) in row.iter().enumerate() {
                let r = Rect::from_min_size(
                    pos2(line.left() + i as f32 * cell, line.top()),
                    vec2(cell, 36.0),
                )
                .shrink(2.0);
                let text = if *ratio == "Auto" {
                    t(s::EditorView_Auto)
                } else {
                    ratio
                };
                let active = b.selected_ratio.eq_ignore_ascii_case(ratio);
                if style::text_button(ui, colors, r, text, active, true).clicked() {
                    let value = ratio.to_string();
                    change = Some(Box::new(move |s| s.selected_ratio = value));
                }
            }
        }
        ui.add_space(16.0);
        label(ui, colors, t(s::EditorView_Background));
        ui.add_space(16.0);
        let modes = BackgroundSettings::modes();
        let labels: Vec<String> = modes.iter().map(|m| mode_name(*m).to_string()).collect();
        let selected = modes.iter().position(|m| *m == b.mode).unwrap_or(0);
        let (combo, _) = ui.allocate_exact_size(vec2(CONTENT_WIDTH, 40.0), Sense::hover());
        if let Some(i) = style::combo_box(
            ui,
            colors,
            Id::new("bg-mode"),
            combo,
            &labels,
            selected,
            true,
        ) {
            let mode = modes[i];
            change = Some(Box::new(move |s| s.mode = mode));
        }
        ui.add_space(12.0);
        match b.mode {
            BackgroundMode::Gradient => {
                let (preview, _) =
                    ui.allocate_exact_size(vec2(CONTENT_WIDTH, 40.0), Sense::hover());
                paint_gradient(
                    ui,
                    preview,
                    to_color32(&b.gradient1),
                    to_color32(&b.gradient2),
                    5.0,
                );
                ui.painter().rect_stroke(
                    preview,
                    5.0,
                    Stroke::new(1.0, colors.border()),
                    egui::StrokeKind::Inside,
                );
                for (half, target) in [(0, ColorTarget::Gradient1), (1, ColorTarget::Gradient2)] {
                    let r = Rect::from_min_size(
                        pos2(
                            preview.left() + half as f32 * preview.width() / 2.0,
                            preview.top(),
                        ),
                        vec2(preview.width() / 2.0, preview.height()),
                    );
                    let tip = if half == 0 {
                        t(s::EditorView_Editgradientcolor1)
                    } else {
                        t(s::EditorView_Editgradientcolor2)
                    };
                    let value = if half == 0 {
                        &b.gradient1
                    } else {
                        &b.gradient2
                    };
                    if let Some(ev) =
                        color_area(ui, colors, Id::new(("bg-gradient", half)), r, value, tip)
                    {
                        match ev {
                            ColorEvent::Picked(c) => {
                                let hex = c.to_hex_argb();
                                change = Some(Box::new(move |s| {
                                    if half == 0 {
                                        s.gradient1 = hex;
                                    } else {
                                        s.gradient2 = hex;
                                    }
                                }));
                            }
                            ColorEvent::PickScreen => {
                                editor.request(Request::PickScreenColor(target))
                            }
                        }
                    }
                }
                ui.add_space(12.0);
                for (row_index, row) in GRADIENT_PRESETS.chunks(4).enumerate() {
                    let (line, _) =
                        ui.allocate_exact_size(vec2(CONTENT_WIDTH, 40.0), Sense::hover());
                    let cell = CONTENT_WIDTH / 4.0;
                    for (i, (name, a, bb)) in row.iter().enumerate() {
                        let slot = Rect::from_min_size(
                            pos2(line.left() + i as f32 * cell, line.top()),
                            vec2(cell, 40.0),
                        );
                        let r = Rect::from_center_size(slot.center(), vec2(46.0, 36.0));
                        let resp =
                            ui.interact(r, Id::new(("bg-preset", row_index, i)), Sense::click());
                        paint_gradient(ui, r, to_color32(a), to_color32(bb), 5.0);
                        ui.painter().rect_stroke(
                            r,
                            5.0,
                            Stroke::new(1.0, colors.border()),
                            egui::StrokeKind::Inside,
                        );
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if resp.on_hover_text(*name).clicked() {
                            editor.apply_gradient_preset(row_index * 4 + i);
                        }
                    }
                }
            }
            BackgroundMode::Color => {
                let (preview, _) =
                    ui.allocate_exact_size(vec2(CONTENT_WIDTH, 40.0), Sense::hover());
                style::paint_checkered_small(ui, ui.painter(), colors, preview, 5.0);
                ui.painter().rect_filled(preview, 5.0, to_color32(&b.color));
                ui.painter().rect_stroke(
                    preview,
                    5.0,
                    Stroke::new(1.0, colors.border()),
                    egui::StrokeKind::Inside,
                );
                if let Some(ev) = color_area(
                    ui,
                    colors,
                    Id::new("bg-color"),
                    preview,
                    &b.color,
                    t(s::EditorView_Editbackgroundcolor),
                ) {
                    match ev {
                        ColorEvent::Picked(c) => {
                            let hex = c.to_hex_argb();
                            change = Some(Box::new(move |s| s.color = hex));
                        }
                        ColorEvent::PickScreen => {
                            editor.request(Request::PickScreenColor(ColorTarget::Background))
                        }
                    }
                }
            }
            BackgroundMode::Image => {
                let (r, _) = ui.allocate_exact_size(vec2(CONTENT_WIDTH, 40.0), Sense::hover());
                if style::text_button(
                    ui,
                    colors,
                    r,
                    t(s::EditorView_BrowsebackgroundimageEllipsis),
                    false,
                    true,
                )
                .clicked()
                {
                    editor.request(Request::BrowseBackgroundImage);
                }
                if let Some(path) = &b.image_path {
                    ui.add_space(6.0);
                    let g = style::galley(
                        ui,
                        t(s::EditorView_Backgroundimage),
                        11.0,
                        Weight::Normal,
                        colors.text_secondary(),
                    );
                    let (r, _) =
                        ui.allocate_exact_size(vec2(CONTENT_WIDTH, g.size().y), Sense::hover());
                    ui.painter().galley(r.min, g, colors.text_secondary());
                    ui.add_space(2.0);
                    let g = style::galley_wrapped(
                        ui,
                        path,
                        11.0,
                        Weight::Normal,
                        colors.text_secondary(),
                        CONTENT_WIDTH,
                    );
                    let (r, _) =
                        ui.allocate_exact_size(vec2(CONTENT_WIDTH, g.size().y), Sense::hover());
                    ui.painter().galley(r.min, g, colors.text_secondary());
                }
            }
            BackgroundMode::Transparent | BackgroundMode::Wallpaper => {}
        }
        if let Some(change) = change {
            editor.update_background(change);
        }
    });
}

/// A preview area that opens a colour panel under it (the background's colour and the
/// gradient's halves; hover lightens it, `color-preview-overlay`).
fn color_area(
    ui: &mut Ui,
    colors: &Colors,
    id: Id,
    rect: Rect,
    value: &str,
    tip: &str,
) -> Option<ColorEvent> {
    let resp = ui.interact(rect, id, Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(
            rect,
            5.0,
            Color32::from_rgba_unmultiplied(255, 255, 255, 0x66),
        );
        ui.painter().rect_stroke(
            rect,
            5.0,
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 0xCC)),
            egui::StrokeKind::Inside,
        );
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let popup_open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let resp = if popup_open {
        resp
    } else {
        resp.on_hover_text(tip)
    };
    let value = Color::parse(value).unwrap_or(Color::TRANSPARENT);
    egui::Popup::from_toggle_button_response(&resp)
        .align(egui::RectAlign::BOTTOM)
        .gap(4.0)
        .frame(style::popup_frame(colors, 4))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let ev = pickers::color_panel(ui, colors, id.with("panel"), value);
            if matches!(ev, Some(ColorEvent::PickScreen)) {
                ui.close();
            }
            ev
        })
        .and_then(|r| r.inner)
}

/// A horizontal two-colour gradient in a rounded rectangle.
fn paint_gradient(ui: &Ui, rect: Rect, a: Color32, b: Color32, radius: f32) {
    let painter = ui.painter();
    // Rounded ends in the end colours, the middle as a gradient mesh.
    painter.rect_filled(
        Rect::from_min_max(rect.min, pos2(rect.left() + radius * 2.0, rect.bottom())),
        radius,
        a,
    );
    painter.rect_filled(
        Rect::from_min_max(pos2(rect.right() - radius * 2.0, rect.top()), rect.max),
        radius,
        b,
    );
    let inner = Rect::from_min_max(
        pos2(rect.left() + radius, rect.top()),
        pos2(rect.right() - radius, rect.bottom()),
    );
    let mut mesh = egui::Mesh::default();
    let span = rect.width().max(1.0);
    let at = |x: f32| style::lerp_color(a, b, ((x - rect.left()) / span).clamp(0.0, 1.0));
    mesh.colored_vertex(inner.left_top(), at(inner.left()));
    mesh.colored_vertex(inner.right_top(), at(inner.right()));
    mesh.colored_vertex(inner.right_bottom(), at(inner.right()));
    mesh.colored_vertex(inner.left_bottom(), at(inner.left()));
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// The right sidebar: the effect dialog or the options.
pub fn right_panel(ui: &mut Ui, colors: &Colors, editor: &mut Editor, rect: Rect) {
    sidebar_frame(
        ui,
        colors,
        rect,
        false,
        "editor-right-panel",
        |ui| match editor.effects_panel.clone() {
            Some(SidePanel::Options) => options_panel(ui, colors, editor),
            Some(SidePanel::Effect(dialog)) => effect_dialog(ui, colors, editor, &dialog),
            None => {}
        },
    );
}

/// `EditorOptionsPanel`.
fn options_panel(ui: &mut Ui, colors: &Colors, editor: &mut Editor) {
    let (head, _) = ui.allocate_exact_size(vec2(CONTENT_WIDTH, 32.0), Sense::hover());
    let painter = ui.painter().clone();
    style::paint_icon(
        ui,
        &painter,
        pos2(head.left() + 10.0, head.center().y + 1.0),
        icons::SETTINGS,
        20.0,
        colors.accent_start(),
    );
    style::paint_text(
        ui,
        &painter,
        Rect::from_min_max(
            pos2(head.left() + 28.0, head.top()),
            pos2(head.right() - 40.0, head.bottom()),
        ),
        Align2::LEFT_CENTER,
        t(s::EditorOptionsPanel_Options),
        20.0,
        Weight::Normal,
        colors.accent_start(),
    );
    let close = Rect::from_min_size(pos2(head.right() - 32.0, head.top()), vec2(32.0, 32.0));
    let (resp, paint) =
        style::button_at(ui, colors, close, ButtonKind::Standard, 10.0, false, true);
    style::paint_icon(
        ui,
        ui.painter(),
        paint.rect.center(),
        icons::ACTION_CANCEL,
        13.0,
        paint.text_block,
    );
    if resp.on_hover_text(t(s::EditorOptionsPanel_Close)).clicked() {
        editor.close_effects_panel();
        return;
    }
    ui.add_space(16.0);
    let o = editor.options.clone();
    let rows: [OptionRow; 8] = [
        (
            t(s::EditorOptionsPanel_RememberWindowState),
            o.remember_window_state,
            |o, v| o.remember_window_state = v,
        ),
        (
            t(s::EditorOptionsPanel_ShowExitConfirmation),
            o.show_exit_confirmation,
            |o, v| o.show_exit_confirmation = v,
        ),
        (
            t(s::EditorOptionsPanel_ZoomToFitOnOpen),
            o.zoom_to_fit_on_open,
            |o, v| o.zoom_to_fit_on_open = v,
        ),
        (t(s::EditorOptionsPanel_QuickCrop), o.quick_crop, |o, v| {
            o.quick_crop = v
        }),
        (
            t(s::EditorOptionsPanel_AutoCloseEditorOnTask),
            o.auto_close_editor_on_task,
            |o, v| o.auto_close_editor_on_task = v,
        ),
        (
            t(s::EditorOptionsPanel_AutoCopyImageToClipboard),
            o.auto_copy_image_to_clipboard,
            |o, v| o.auto_copy_image_to_clipboard = v,
        ),
        (
            t(s::EditorOptionsPanel_ShowInsertImageDialog),
            o.show_insert_image_dialog,
            |o, v| o.show_insert_image_dialog = v,
        ),
        (
            t(s::EditorOptionsPanel_ShowNotifications),
            o.show_notifications,
            |o, v| o.show_notifications = v,
        ),
    ];
    for (text, value, set) in rows {
        if let Some(v) = style::labeled_toggle(ui, colors, CONTENT_WIDTH, text, value, true) {
            set(&mut editor.options, v);
        }
        ui.add_space(8.0);
    }
    let (r, _) = ui.allocate_exact_size(vec2(CONTENT_WIDTH, 40.0), Sense::hover());
    let (resp, paint) = style::button_at(ui, colors, r, ButtonKind::Standard, 10.0, false, true);
    style::paint_text(
        ui,
        ui.painter(),
        paint.rect.shrink2(vec2(10.0, 0.0)),
        Align2::LEFT_CENTER,
        t(s::EditorOptionsPanel_CustomizetoolbarEllipsis),
        style::FONT_SIZE,
        Weight::Normal,
        paint.foreground,
    );
    if resp.clicked() {
        editor.open_toolbar_customization();
    }
    ui.add_space(12.0);
}

/// `SchemaDrivenEffectDialog`.
fn effect_dialog(
    ui: &mut Ui,
    colors: &Colors,
    editor: &mut Editor,
    dialog: &crate::effects::DialogState,
) {
    let Some(effect) = crate::effects::registry().get(&dialog.effect_id) else {
        return;
    };
    heading(
        ui,
        colors,
        effect.icon(),
        &crate::effects::display_name(effect.as_ref()),
    );
    // The heading's own spacing is 8 below (`Margin 0,0,0,8`) plus the 16 padding.
    for p in &dialog.params {
        let id = Id::new(("effect-param", &dialog.effect_id, &p.key));
        let value = dialog.values.get(&p.key).cloned();
        match &p.kind {
            ParamKind::Slider {
                min,
                max,
                tick,
                snap,
                integer,
                ..
            } => {
                let v = match value {
                    Some(ParamValue::Number(n)) => n,
                    _ => *min,
                };
                let text = if *integer {
                    format!("{}", v.round() as i64)
                } else {
                    format_number(v)
                };
                let step = if *snap {
                    Some(*tick)
                } else if *integer {
                    Some(1.0)
                } else {
                    None
                };
                if let Some(nv) = style::slider_row(
                    ui,
                    colors,
                    id,
                    CONTENT_WIDTH,
                    &p.label,
                    &text,
                    v,
                    *min,
                    *max,
                    step,
                ) {
                    editor.set_effect_value(&p.key, ParamValue::Number(nv));
                }
                ui.add_space(10.0);
            }
            ParamKind::Checkbox { .. } => {
                let v = matches!(value, Some(ParamValue::Bool(true)));
                if let Some(nv) =
                    style::labeled_toggle(ui, colors, CONTENT_WIDTH, &p.label, v, true)
                {
                    editor.set_effect_value(&p.key, ParamValue::Bool(nv));
                }
                ui.add_space(10.0);
            }
            ParamKind::Enum { options, .. } => {
                label(ui, colors, &p.label);
                ui.add_space(4.0);
                let labels: Vec<String> = options.iter().map(|(l, _)| l.clone()).collect();
                let current = match &value {
                    Some(ParamValue::Choice(c)) => {
                        options.iter().position(|(_, v)| v == c).unwrap_or(0)
                    }
                    _ => 0,
                };
                let (r, _) = ui.allocate_exact_size(vec2(CONTENT_WIDTH, 40.0), Sense::hover());
                if let Some(i) = style::combo_box(ui, colors, id, r, &labels, current, true) {
                    editor.set_effect_value(&p.key, ParamValue::Choice(options[i].1.clone()));
                }
                ui.add_space(10.0);
            }
            ParamKind::Color { default } => {
                label(ui, colors, &p.label);
                ui.add_space(4.0);
                let c = match value {
                    Some(ParamValue::Color(c)) => c,
                    _ => *default,
                };
                let (r, _) = ui.allocate_exact_size(vec2(60.0, 40.0), Sense::hover());
                if let Some(ColorEvent::Picked(nc)) =
                    pickers::color_dropdown(ui, colors, id, r, c, false, false, true, "")
                {
                    editor.set_effect_value(&p.key, ParamValue::Color(nc));
                }
                ui.add_space(10.0);
            }
            ParamKind::Numeric {
                min,
                max,
                increment,
                integer,
                format,
                ..
            } => {
                label(ui, colors, &p.label);
                ui.add_space(4.0);
                let v = match value {
                    Some(ParamValue::Number(n)) => Some(n),
                    _ => None,
                };
                let decimals = if *integer { 0 } else { decimals_of(format) };
                let (r, _) = ui.allocate_exact_size(vec2(CONTENT_WIDTH, 40.0), Sense::hover());
                if let Some(nv) =
                    style::numeric_box(ui, colors, id, r, v, *min, *max, *increment, decimals)
                {
                    editor.set_effect_value(
                        &p.key,
                        nv.map(ParamValue::Number).unwrap_or(ParamValue::Empty),
                    );
                }
                ui.add_space(10.0);
            }
            ParamKind::Text { .. } => {
                label(ui, colors, &p.label);
                ui.add_space(4.0);
                let mut text = match value {
                    Some(ParamValue::Text(t)) => t,
                    _ => String::new(),
                };
                let (r, _) = ui.allocate_exact_size(vec2(CONTENT_WIDTH, 40.0), Sense::hover());
                if style::text_box(ui, colors, r, &mut text, "", true, id).changed() {
                    editor.set_effect_value(&p.key, ParamValue::Text(text));
                }
                ui.add_space(10.0);
            }
            ParamKind::FilePath { .. } => {
                label(ui, colors, &p.label);
                ui.add_space(4.0);
                let mut text = match value {
                    Some(ParamValue::Text(t)) => t,
                    _ => String::new(),
                };
                let browse = t(s::SchemaDrivenEffectDialog_BrowseEllipsis);
                let bw = style::text_button_width(ui, browse, 10.0);
                let (r, _) = ui.allocate_exact_size(vec2(CONTENT_WIDTH, 40.0), Sense::hover());
                let field = Rect::from_min_max(r.min, pos2(r.right() - bw - 8.0, r.bottom()));
                if style::text_box(ui, colors, field, &mut text, "", true, id).changed() {
                    editor.set_effect_value(&p.key, ParamValue::Text(text));
                }
                let br = Rect::from_min_max(pos2(r.right() - bw, r.top()), r.max);
                if style::text_button(ui, colors, br, browse, false, true).clicked() {
                    editor.request(Request::BrowseEffectFile(p.key.clone()));
                }
                ui.add_space(10.0);
            }
        }
    }
    ui.add_space(16.0);
    let (row, _) = ui.allocate_exact_size(vec2(CONTENT_WIDTH, 40.0), Sense::hover());
    let half = (CONTENT_WIDTH - 8.0) / 2.0;
    let cancel = Rect::from_min_size(row.min, vec2(half, 40.0));
    let apply = Rect::from_min_size(pos2(row.right() - half, row.top()), vec2(half, 40.0));
    if style::text_button(
        ui,
        colors,
        cancel,
        t(s::SchemaDrivenEffectDialog_Cancel),
        false,
        true,
    )
    .clicked()
    {
        editor.close_effects_panel();
    }
    if style::primary_text_button(
        ui,
        colors,
        apply,
        t(s::SchemaDrivenEffectDialog_Apply),
        true,
    )
    .clicked()
    {
        editor.apply_effect_dialog();
    }
}

/// `{0:0.##}`.
pub fn format_number(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

fn decimals_of(format: &str) -> usize {
    format
        .split_once('.')
        .map(|(_, d)| d.chars().filter(|c| *c == '0' || *c == '#').count())
        .unwrap_or(0)
}
