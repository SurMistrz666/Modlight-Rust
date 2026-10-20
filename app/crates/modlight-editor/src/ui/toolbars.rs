//! `EditorBuiltInToolbars`: the `AnnotationToolbar` at the top (the customisable tool
//! buttons with the file menu, then the tool-options row under it) and the bottom bar
//! (zoom, the task buttons, copy, save, save as, pin, print, upload).

use egui::{Align2, Id, Rect, Sense, Ui, Vec2, pos2, vec2};

use super::canvas::Canvas;
use super::menu::{self, Entry};
use super::pickers::{self, ColorEvent, ZoomEvent};
use super::style::{self, ButtonKind, Colors, Weight};
use crate::color::Color;
use crate::editor::{ColorTarget, Editor, Request};
use crate::model::EditorTool;
use crate::ui::icons;
use modlight_core::l10n::{editor as s, t};

const ITEM: f32 = 40.0;
const SPACING: f32 = 4.0;

/// The pieces of the options row, in upstream's order.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Opt {
    ActiveTool,
    StrokeColor,
    FillColor,
    TextColor,
    Width,
    BorderStyle,
    CornerRadius,
    FontFamily,
    TextAlign,
    ArrowStyle,
    CursorType,
    FontSize,
    Bold,
    Italic,
    StepType,
    StepStart,
    Strength,
    SpotlightIcon,
    SpotlightBlur,
    Ellipse,
    Tail,
    Shadow,
}

fn hex_color(hex: &str) -> Color {
    Color::parse(hex).unwrap_or(Color::TRANSPARENT)
}

/// The top toolbar; returns the rectangle it covers.
pub fn top_toolbar(ui: &mut Ui, colors: &Colors, editor: &mut Editor, view: Rect) -> Rect {
    let items: Vec<_> = editor.visible_toolbar_items().cloned().collect();
    // Widths: a separator (1 + spacing) before group starts, the file menu at least 54.
    let mut widths = Vec::new();
    for item in &items {
        let sep = if item.separator_visible() {
            1.0 + SPACING
        } else {
            0.0
        };
        let w = if item.is_file_menu() { 54.0 } else { ITEM };
        widths.push((sep, w));
    }
    let inner: f32 = widths.iter().map(|(s, w)| s + w).sum::<f32>()
        + SPACING * (items.len().saturating_sub(1)) as f32;
    let width = (inner + 8.0 + 2.0).min(view.width());
    let bar = Rect::from_min_size(
        pos2(view.center().x - width / 2.0, view.top()),
        vec2(width, ITEM + 8.0 + 1.0),
    );
    // The bar is hit-test visible: clicks on it never reach the canvas.
    ui.interact(bar, Id::new("editor-top-toolbar"), Sense::click());
    let painter = ui.painter().clone();
    painter.rect_filled(
        bar,
        egui::CornerRadius {
            nw: 0,
            ne: 0,
            sw: 4,
            se: 4,
        },
        colors.background_toolbar(),
    );
    painter.rect_stroke(
        bar,
        egui::CornerRadius {
            nw: 0,
            ne: 0,
            sw: 4,
            se: 4,
        },
        egui::Stroke::new(1.0, colors.border()),
        egui::StrokeKind::Inside,
    );
    let mut x = bar.left() + 5.0;
    let top = bar.top() + 4.0;
    let mut file_menu_response = None;
    for (item, (sep, w)) in items.iter().zip(widths) {
        if sep > 0.0 {
            style::vertical_separator(&painter, colors, x, top, top + ITEM);
            x += sep;
        }
        let rect = Rect::from_min_size(pos2(x, top), vec2(w, ITEM));
        if item.is_file_menu() {
            let (resp, paint) =
                style::button_at(ui, colors, rect, ButtonKind::Standard, 10.0, false, true);
            style::paint_icon(
                ui,
                ui.painter(),
                pos2(paint.rect.left() + 10.0 + 9.0, paint.rect.center().y),
                item.icon,
                18.0,
                paint.foreground,
            );
            let open = egui::Popup::is_id_open(ui.ctx(), Id::new("editor-file-menu"));
            style::paint_chevron(
                ui,
                ui.painter(),
                pos2(paint.rect.right() - 10.0 - 5.0, paint.rect.center().y),
                open,
                colors.text_secondary(),
            );
            let resp = if open {
                resp
            } else {
                resp.on_hover_text(item.tooltip())
            };
            if editor.input.file_menu_requested {
                editor.input.file_menu_requested = false;
                egui::Popup::open_id(ui.ctx(), Id::new("editor-file-menu"));
            }
            file_menu_response = Some(resp);
        } else {
            let resp = style::toolbar_button(ui, colors, rect, item.icon, item.is_active, true)
                .on_hover_text(item.tooltip());
            if resp.clicked() {
                editor.execute_toolbar_item(item);
            }
            if item.id == crate::editor::toolbar::IMAGE_EFFECTS_ITEM {
                favorites_menu(ui, colors, editor, &resp);
            }
        }
        x += w + SPACING;
    }
    if let Some(resp) = file_menu_response {
        file_menu(ui, colors, editor, &resp);
    }

    let mut covered = bar;
    if editor.visibility().any() {
        let row = options_row(ui, colors, editor, view, bar.bottom() - 1.0);
        covered = covered.union(row);
    }
    covered
}

fn favorites_menu(ui: &mut Ui, colors: &Colors, editor: &mut Editor, resp: &egui::Response) {
    let id = Id::new("editor-favorite-effects");
    if resp.secondary_clicked() {
        egui::Popup::open_id(ui.ctx(), id);
    }
    let favorites = editor.favorite_effects();
    let registry = crate::effects::registry();
    let entries: Vec<(String, Entry)> = favorites
        .iter()
        .filter_map(|fid| {
            let effect = registry.get(fid)?;
            Some((
                fid.clone(),
                Entry::Item {
                    label: crate::effects::browser_label(effect.as_ref()),
                    icon: Some(effect.icon()),
                    gesture: None,
                    enabled: true,
                    children: Vec::new(),
                },
            ))
        })
        .collect();
    if entries.is_empty() {
        return;
    }
    let list: Vec<Entry> = entries.iter().map(|(_, e)| e.clone()).collect();
    let picked = egui::Popup::from_response(resp)
        .open_memory(None)
        .id(id)
        .align(egui::RectAlign::BOTTOM_START)
        .gap(4.0)
        .frame(style::menu_frame(colors))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
        .show(|ui| menu::entries(ui, colors, &list, 6.0))
        .and_then(|r| r.inner);
    if let Some(path) = picked
        && let Some((fid, _)) = entries.get(path[0])
    {
        editor.run_effect(fid);
    }
}

fn file_menu(_ui: &mut Ui, colors: &Colors, editor: &mut Editor, resp: &egui::Response) {
    let recent = editor.options.recent_image_files.clone();
    let mut list = vec![
        Entry::item(
            t(s::AnnotationToolbar_NewEllipsis),
            icons::FILE_NEW,
            Some("Ctrl+N"),
        ),
        Entry::item(
            t(s::AnnotationToolbar_OpenEllipsis),
            icons::FILE_OPEN,
            Some("Ctrl+O"),
        ),
    ];
    let has_recent = !recent.is_empty();
    if has_recent {
        list.push(
            Entry::item(t(s::AnnotationToolbar_Openrecent), icons::HISTORY, None).with_children(
                recent
                    .iter()
                    .map(|p| Entry::Item {
                        label: p.clone(),
                        icon: None,
                        gesture: None,
                        enabled: true,
                        children: Vec::new(),
                    })
                    .collect(),
            ),
        );
    }
    list.push(Entry::Separator);
    list.push(Entry::item(
        t(s::AnnotationToolbar_Save),
        icons::FILE_SAVE,
        Some("Ctrl+S"),
    ));
    list.push(Entry::item(
        t(s::AnnotationToolbar_SaveasEllipsis),
        icons::FILE_SAVE_AS,
        Some("Ctrl+Shift+S"),
    ));
    list.push(Entry::Separator);
    if editor.config.show_options_button {
        list.push(Entry::item(
            t(s::AnnotationToolbar_Options),
            icons::SETTINGS,
            None,
        ));
        list.push(Entry::Separator);
    }
    list.push(Entry::item(
        t(s::AnnotationToolbar_Exit),
        icons::FILE_EXIT,
        Some("Escape"),
    ));
    let picked = egui::Popup::menu(resp)
        .id(Id::new("editor-file-menu"))
        .align(egui::RectAlign::BOTTOM_START)
        .gap(4.0)
        .frame(style::menu_frame(colors))
        .show(|ui| menu::entries(ui, colors, &list, 6.0))
        .and_then(|r| r.inner);
    let Some(path) = picked else {
        return;
    };
    let offset = if has_recent { 1 } else { 0 };
    match path[0] {
        0 => editor.modal_new_image(),
        1 => editor.request(Request::OpenImage),
        2 if has_recent => {
            if let Some(file) = path.get(1).and_then(|i| recent.get(*i)) {
                editor.request(Request::OpenRecent(file.clone()));
            }
        }
        i if i == 3 + offset => editor.request(Request::Save),
        i if i == 4 + offset => editor.request(Request::SaveAs),
        i if editor.config.show_options_button && i == 6 + offset => editor.toggle_options_panel(),
        _ => editor.request_close(false),
    }
}

fn opt_width(ui: &Ui, editor: &Editor, opt: Opt) -> f32 {
    let tools = &editor.tools;
    match opt {
        Opt::ActiveTool
        | Opt::Bold
        | Opt::Italic
        | Opt::Ellipse
        | Opt::Tail
        | Opt::Shadow
        | Opt::SpotlightIcon => ITEM,
        Opt::StrokeColor | Opt::FillColor | Opt::TextColor | Opt::Width | Opt::CornerRadius => 60.0,
        Opt::BorderStyle => pickers::border_style_width(ui, tools.border_style),
        Opt::FontFamily => pickers::font_family_width(ui, &tools.font_family),
        Opt::TextAlign => 58.0,
        Opt::ArrowStyle => pickers::arrow_style_width(ui, tools.arrow_style),
        Opt::CursorType => pickers::cursor_type_width(ui, tools.cursor_type),
        Opt::FontSize => pickers::font_size_width(ui, tools.font_size),
        Opt::StepType => pickers::step_type_width(ui, tools.step_type),
        Opt::StepStart => pickers::number_width(ui, tools.step_start_number),
        Opt::Strength | Opt::SpotlightBlur => pickers::STRENGTH_WIDTH,
    }
}

fn visible_options(editor: &Editor) -> Vec<Opt> {
    let v = editor.visibility();
    let mut list = vec![Opt::ActiveTool];
    let flags = [
        (v.border_color, Opt::StrokeColor),
        (v.fill_color, Opt::FillColor),
        (v.text_color, Opt::TextColor),
        (v.thickness, Opt::Width),
        (v.border_style, Opt::BorderStyle),
        (v.corner_radius, Opt::CornerRadius),
        (v.font_family, Opt::FontFamily),
        (v.text_align, Opt::TextAlign),
        (v.arrow_style, Opt::ArrowStyle),
        (v.cursor_type, Opt::CursorType),
        (v.font_size, Opt::FontSize),
        (v.text_style, Opt::Bold),
        (v.text_italic, Opt::Italic),
        (v.step_type, Opt::StepType),
        (v.step_start_number, Opt::StepStart),
        (v.strength, Opt::Strength),
        (v.spotlight_blur, Opt::SpotlightIcon),
        (v.spotlight_blur, Opt::SpotlightBlur),
        (v.effect_ellipse, Opt::Ellipse),
        (v.speech_balloon_tail, Opt::Tail),
        (v.shadow, Opt::Shadow),
    ];
    list.extend(flags.into_iter().filter(|(on, _)| *on).map(|(_, o)| o));
    list
}

/// The tool-options row under the toolbar (`ShowToolOptionsSeparator`).
/// The tool-options row, centred in `view` with its top at `top`.
pub fn options_row(
    ui: &mut Ui,
    colors: &Colors,
    editor: &mut Editor,
    view: Rect,
    top: f32,
) -> Rect {
    let opts = visible_options(editor);
    let widths: Vec<f32> = opts.iter().map(|o| opt_width(ui, editor, *o)).collect();
    let inner = widths.iter().sum::<f32>() + SPACING * (opts.len().saturating_sub(1)) as f32;
    let width = (inner + 8.0 + 2.0).min(view.width());
    let row = Rect::from_min_size(
        pos2(view.center().x - width / 2.0, top),
        vec2(width, ITEM + 4.0 + 1.0),
    );
    ui.interact(row, Id::new("editor-options-row"), Sense::click());
    let painter = ui.painter().clone();
    let radius = egui::CornerRadius {
        nw: 0,
        ne: 0,
        sw: 4,
        se: 4,
    };
    painter.rect_filled(row, radius, colors.background_toolbar());
    // Border on the left, right and bottom only.
    let stroke = egui::Stroke::new(1.0, colors.border());
    painter.line_segment(
        [
            pos2(row.left() + 0.5, row.top()),
            pos2(row.left() + 0.5, row.bottom() - 4.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            pos2(row.right() - 0.5, row.top()),
            pos2(row.right() - 0.5, row.bottom() - 4.0),
        ],
        stroke,
    );
    painter.rect_stroke(
        Rect::from_min_max(pos2(row.left(), row.bottom() - 8.0), row.max),
        radius,
        stroke,
        egui::StrokeKind::Inside,
    );
    painter.rect_filled(
        Rect::from_min_max(
            pos2(row.left() + 1.0, row.bottom() - 9.0),
            pos2(row.right() - 1.0, row.bottom() - 2.0),
        ),
        0.0,
        colors.background_toolbar(),
    );
    let mut x = row.left() + 5.0;
    let y = row.top();
    for (opt, w) in opts.into_iter().zip(widths) {
        let rect = Rect::from_min_size(pos2(x, y), vec2(w, ITEM));
        option_item(ui, colors, editor, opt, rect);
        x += w + SPACING;
    }
    row
}

fn option_item(ui: &mut Ui, colors: &Colors, editor: &mut Editor, opt: Opt, rect: Rect) {
    let id = |name: &str| Id::new(("editor-option", name));
    match opt {
        Opt::ActiveTool => {
            let tool = editor.effective_tool();
            let resp = ui.interact(rect, id("active-tool"), Sense::hover());
            style::paint_icon(
                ui,
                ui.painter(),
                rect.center(),
                icons::for_tool(tool),
                18.0,
                colors.accent_start(),
            );
            resp.on_hover_text(t(tool_name(tool)));
        }
        Opt::StrokeColor => {
            let value = hex_color(&editor.tools.selected_color);
            match pickers::color_dropdown(
                ui,
                colors,
                id("stroke"),
                rect,
                value,
                true,
                false,
                true,
                t(s::AnnotationToolbar_Bordercolor),
            ) {
                Some(ColorEvent::Picked(c)) => editor.set_selected_color(&c.to_hex_argb()),
                Some(ColorEvent::PickScreen) => {
                    editor.request(Request::PickScreenColor(ColorTarget::Stroke))
                }
                None => {}
            }
        }
        Opt::FillColor => {
            let value = hex_color(&editor.tools.fill_color);
            match pickers::color_dropdown(
                ui,
                colors,
                id("fill"),
                rect,
                value,
                false,
                false,
                true,
                t(s::AnnotationToolbar_Fillcolor),
            ) {
                Some(ColorEvent::Picked(c)) => editor.set_fill_color(&c.to_hex_argb()),
                Some(ColorEvent::PickScreen) => {
                    editor.request(Request::PickScreenColor(ColorTarget::Fill))
                }
                None => {}
            }
        }
        Opt::TextColor => {
            let value = hex_color(&editor.tools.text_color);
            match pickers::color_dropdown(
                ui,
                colors,
                id("text"),
                rect,
                value,
                false,
                true,
                true,
                t(s::AnnotationToolbar_Textcolor),
            ) {
                Some(ColorEvent::Picked(c)) => editor.set_text_color(&c.to_hex_argb()),
                Some(ColorEvent::PickScreen) => {
                    editor.request(Request::PickScreenColor(ColorTarget::Text))
                }
                None => {}
            }
        }
        Opt::Width => {
            if let Some(v) = pickers::width_dropdown(
                ui,
                colors,
                id("width"),
                rect,
                editor.tools.stroke_width,
                t(s::AnnotationToolbar_Thickness),
            ) {
                editor.set_stroke_width(v);
            }
        }
        Opt::BorderStyle => {
            if let Some(v) = pickers::border_style_dropdown(
                ui,
                colors,
                id("border"),
                rect,
                editor.tools.border_style,
                t(s::AnnotationToolbar_Borderstyle),
            ) {
                editor.set_border_style(v, true);
            }
        }
        Opt::CornerRadius => {
            if let Some(v) = pickers::corner_radius_dropdown(
                ui,
                colors,
                id("radius"),
                rect,
                editor.tools.corner_radius,
                t(s::AnnotationToolbar_Cornerradius),
            ) {
                editor.set_corner_radius(v);
            }
        }
        Opt::FontFamily => {
            let family = editor.tools.font_family.clone();
            if let Some(v) = pickers::font_family_dropdown(
                ui,
                colors,
                id("family"),
                rect,
                &family,
                t(s::AnnotationToolbar_Fontfamily),
            ) {
                editor.set_font_family(&v, true);
            }
        }
        Opt::TextAlign => {
            if let Some(v) = pickers::text_align_dropdown(
                ui,
                colors,
                id("align"),
                rect,
                editor.tools.text_align,
                t(s::AnnotationToolbar_Horizontalalignment),
            ) {
                editor.set_text_align(v);
            }
        }
        Opt::ArrowStyle => {
            if let Some(v) = pickers::arrow_style_dropdown(
                ui,
                colors,
                id("arrow"),
                rect,
                editor.tools.arrow_style,
                t(s::AnnotationToolbar_Arrowstyle),
            ) {
                editor.set_arrow_style(v, true);
            }
        }
        Opt::CursorType => {
            if let Some(v) = pickers::cursor_type_dropdown(
                ui,
                colors,
                id("cursor"),
                rect,
                editor.tools.cursor_type,
                t(s::AnnotationToolbar_Cursortype),
            ) {
                editor.set_cursor_type(v, true);
            }
        }
        Opt::FontSize => {
            if let Some(v) = pickers::font_size_dropdown(
                ui,
                colors,
                id("size"),
                rect,
                editor.tools.font_size,
                t(s::AnnotationToolbar_Fontsize),
            ) {
                editor.set_font_size(v, true);
            }
        }
        Opt::Bold => {
            let on = editor.tools.text_bold;
            if style::toolbar_button(ui, colors, rect, icons::FORMAT_BOLD, on, true)
                .on_hover_text(t(s::AnnotationToolbar_Togglebold))
                .clicked()
            {
                editor.set_text_bold(!on, true);
            }
        }
        Opt::Italic => {
            let on = editor.tools.text_italic;
            if style::toolbar_button(ui, colors, rect, icons::FORMAT_ITALIC, on, true)
                .on_hover_text(t(s::AnnotationToolbar_Toggleitalic))
                .clicked()
            {
                editor.set_text_italic(!on, true);
            }
        }
        Opt::StepType => {
            if let Some(v) = pickers::step_type_dropdown(
                ui,
                colors,
                id("step-type"),
                rect,
                editor.tools.step_type,
                t(s::AnnotationToolbar_Steptype),
            ) {
                editor.set_step_type(v);
            }
        }
        Opt::StepStart => {
            let options: Vec<i32> = (1..=10).collect();
            if let Some(v) = pickers::number_dropdown(
                ui,
                colors,
                id("step-start"),
                rect,
                editor.tools.step_start_number,
                &options,
                t(s::AnnotationToolbar_Startingnumber),
            ) {
                editor.set_step_start_number(v);
            }
        }
        Opt::Strength => {
            let max = editor.effect_strength_maximum();
            if let Some(v) = pickers::strength_slider(
                ui,
                colors,
                id("strength"),
                rect,
                editor.tools.effect_strength,
                crate::editor::tools::MIN_EFFECT_STRENGTH,
                max,
                t(s::AnnotationToolbar_Effectstrength),
            ) {
                editor.set_effect_strength(v, true);
            }
        }
        Opt::SpotlightIcon => {
            let resp = ui.interact(rect, id("spotlight-icon"), Sense::hover());
            style::paint_icon(
                ui,
                ui.painter(),
                rect.center(),
                icons::TOOL_BLUR,
                18.0,
                colors.text_secondary(),
            );
            resp.on_hover_text(t(s::AnnotationToolbar_Bluramount));
        }
        Opt::SpotlightBlur => {
            if let Some(v) = pickers::strength_slider(
                ui,
                colors,
                id("spotlight-blur"),
                rect,
                editor.tools.spotlight_blur,
                0.0,
                crate::editor::tools::MAX_BLUR_STRENGTH,
                t(s::AnnotationToolbar_Bluramount),
            ) {
                editor.set_spotlight_blur(v, true);
            }
        }
        Opt::Ellipse => {
            let on = editor.tools.effect_ellipse;
            if style::toolbar_button(ui, colors, rect, icons::TOGGLE_ELLIPSE_SHAPE, on, true)
                .on_hover_text(t(s::AnnotationToolbar_Toggleellipseshape))
                .clicked()
            {
                editor.set_effect_ellipse(!on, true);
            }
        }
        Opt::Tail => {
            let on = editor.tools.speech_balloon_tail;
            if style::toolbar_button(
                ui,
                colors,
                rect,
                icons::TOGGLE_SPEECH_BALLOON_TAIL,
                on,
                true,
            )
            .on_hover_text(t(s::AnnotationToolbar_Toggletail))
            .clicked()
            {
                editor.set_speech_balloon_tail(!on, true);
            }
        }
        Opt::Shadow => shadow_button(ui, colors, editor, rect),
    }
}

/// The shadow toggle; a right click opens its options (`ShadowOptionsPopup`).
fn shadow_button(ui: &mut Ui, colors: &Colors, editor: &mut Editor, rect: Rect) {
    let on = editor.tools.shadow_enabled;
    let resp = style::toolbar_button(ui, colors, rect, icons::MENU_THEME, on, true);
    let popup_id = Id::new("editor-shadow-options");
    if resp.secondary_clicked() {
        egui::Popup::open_id(ui.ctx(), popup_id);
    }
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let resp = if open {
        resp
    } else {
        resp.on_hover_text(t(s::AnnotationToolbar_ToggleshadowShadowoptionsRightclick))
    };
    if resp.clicked() {
        editor.set_shadow_enabled(!on, true);
    }
    let mut color = editor.tools.shadow_color.clone();
    let mut blur = editor.tools.shadow_blur_radius;
    let mut opacity = editor.tools.shadow_opacity;
    let mut ox = editor.tools.shadow_offset_x;
    let mut oy = editor.tools.shadow_offset_y;
    let mut screen_pick = false;
    let changed = egui::Popup::from_response(&resp)
        .open_memory(None)
        .id(popup_id)
        .align(egui::RectAlign::BOTTOM)
        .gap(4.0)
        .frame(style::popup_frame(colors, 10))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let width = 240.0;
            let mut changed = false;
            let title = style::galley(
                ui,
                t(s::AnnotationToolbar_Shadow),
                16.0,
                Weight::SemiBold,
                colors.accent_start(),
            );
            let (r, _) = ui.allocate_exact_size(vec2(width, title.size().y), Sense::hover());
            ui.painter().galley(r.min, title, colors.accent_start());
            ui.add_space(8.0);
            let (row, _) = ui.allocate_exact_size(vec2(width, 40.0), Sense::hover());
            style::paint_text(
                ui,
                ui.painter(),
                Rect::from_min_size(row.min, vec2(90.0, 40.0)),
                Align2::LEFT_CENTER,
                t(s::AnnotationToolbar_Color),
                style::FONT_SIZE,
                Weight::SemiBold,
                colors.text_secondary(),
            );
            let picker = Rect::from_min_size(pos2(row.right() - 60.0, row.top()), vec2(60.0, 40.0));
            match pickers::color_dropdown(
                ui,
                colors,
                Id::new("editor-shadow-color"),
                picker,
                hex_color(&color),
                false,
                false,
                true,
                t(s::AnnotationToolbar_Shadowcolor),
            ) {
                Some(ColorEvent::Picked(c)) => {
                    color = c.to_hex_argb();
                    changed = true;
                }
                Some(ColorEvent::PickScreen) => screen_pick = true,
                None => {}
            }
            ui.add_space(16.0);
            if let Some(v) = style::slider_row(
                ui,
                colors,
                Id::new("shadow-blur"),
                width,
                t(s::AnnotationToolbar_Blur),
                &format!("{blur:.0} px"),
                blur,
                0.0,
                200.0,
                None,
            ) {
                blur = v;
                changed = true;
            }
            ui.add_space(8.0);
            if let Some(v) = style::slider_row(
                ui,
                colors,
                Id::new("shadow-opacity"),
                width,
                t(s::AnnotationToolbar_Opacity),
                &format!("{}%", (opacity * 100.0).round() as i64),
                opacity,
                0.0,
                1.0,
                None,
            ) {
                opacity = v;
                changed = true;
            }
            ui.add_space(8.0);
            if let Some(v) = style::slider_row(
                ui,
                colors,
                Id::new("shadow-x"),
                width,
                t(s::AnnotationToolbar_OffsetX),
                &format!("{ox:.0} px"),
                ox,
                -200.0,
                200.0,
                None,
            ) {
                ox = v;
                changed = true;
            }
            ui.add_space(8.0);
            if let Some(v) = style::slider_row(
                ui,
                colors,
                Id::new("shadow-y"),
                width,
                t(s::AnnotationToolbar_OffsetY),
                &format!("{oy:.0} px"),
                oy,
                -200.0,
                200.0,
                None,
            ) {
                oy = v;
                changed = true;
            }
            changed
        })
        .map(|r| r.inner)
        .unwrap_or(false);
    if changed {
        editor.set_shadow_settings(&color, blur, opacity, ox, oy, true);
    }
    if screen_pick {
        editor.request(Request::PickScreenColor(ColorTarget::Shadow));
    }
}

pub fn tool_name(tool: EditorTool) -> modlight_core::l10n::Key {
    crate::editor::toolbar::default_items()
        .into_iter()
        .find(|i| i.tool == Some(tool))
        .map(|i| i.name)
        .unwrap_or(s::AnnotationToolbar_Shadow)
}

/// The bottom bar; returns the rectangle it covers.
pub fn bottom_toolbar(
    ui: &mut Ui,
    colors: &Colors,
    editor: &mut Editor,
    canvas: &mut Canvas,
    view: Rect,
) -> Rect {
    let show_bottom = editor.config.show_bottom_toolbar;
    let show_tasks = editor.config.show_task_buttons;
    let zoom_w = pickers::zoom_width(ui, editor.zoom);
    let mut inner = zoom_w;
    if show_bottom {
        inner += SPACING + 9.0;
    }
    if show_tasks {
        inner += SPACING + 4.0 + 90.0 + 10.0;
    }
    if show_bottom {
        inner += (SPACING + ITEM) * 6.0;
    }
    let width = (inner + 8.0 + 2.0).min(view.width());
    let bar = Rect::from_min_size(
        pos2(
            view.center().x - width / 2.0,
            view.bottom() - (ITEM + 8.0 + 1.0),
        ),
        vec2(width, ITEM + 8.0 + 1.0),
    );
    ui.interact(bar, Id::new("editor-bottom-toolbar"), Sense::click());
    let painter = ui.painter().clone();
    let radius = egui::CornerRadius {
        nw: 4,
        ne: 4,
        sw: 0,
        se: 0,
    };
    painter.rect_filled(bar, radius, colors.background_toolbar());
    painter.rect_stroke(
        bar,
        radius,
        egui::Stroke::new(1.0, colors.border()),
        egui::StrokeKind::Inside,
    );
    let top = bar.top() + 5.0;
    let mut x = bar.left() + 5.0;
    let zoom_rect = Rect::from_min_size(pos2(x, top), vec2(zoom_w, ITEM));
    match pickers::zoom_dropdown(
        ui,
        colors,
        Id::new("editor-zoom"),
        zoom_rect,
        editor.zoom,
        t(s::EditorView_ZoomCtrlWheel),
    ) {
        Some(ZoomEvent::Zoom(z)) => canvas.zoom_around_center(editor, z),
        Some(ZoomEvent::Fit) => editor.zoom_to_fit_requested = true,
        None => {}
    }
    x += zoom_w + SPACING;
    if show_bottom {
        painter.rect_filled(
            Rect::from_min_size(pos2(x + 4.0, top + 1.0), vec2(1.0, 38.0)),
            0.0,
            colors.separator(),
        );
        x += 9.0 + SPACING;
    }
    if show_tasks {
        let grid = Rect::from_min_size(pos2(x + 4.0, top), vec2(90.0, ITEM));
        // Cancel: 55 by 40 on the right, its icon pushed right by the round button.
        let cancel = Rect::from_min_max(pos2(grid.right() - 55.0, grid.top()), grid.max);
        let cancel_resp = ui.interact(cancel, Id::new("editor-task-cancel"), Sense::click());
        let cancel_radius = egui::CornerRadius {
            nw: 0,
            ne: 4,
            sw: 0,
            se: 4,
        };
        // Repaint with the right corners only.
        let fill = if cancel_resp.hovered() {
            colors.cancel_border()
        } else {
            colors.cancel_background()
        };
        painter.rect_filled(cancel, cancel_radius, fill);
        painter.rect_stroke(
            cancel,
            cancel_radius,
            egui::Stroke::new(1.0, colors.cancel_border()),
            egui::StrokeKind::Inside,
        );
        style::paint_icon(
            ui,
            &painter,
            pos2(
                cancel.left() + 27.0 + (55.0 - 27.0 - 10.0) / 2.0,
                cancel.center().y,
            ),
            icons::ACTION_CANCEL,
            18.0,
            colors.task_foreground(),
        );
        if cancel_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if cancel_resp
            .on_hover_text(t(s::EditorView_CancelEsc))
            .clicked()
        {
            editor.request_close(false);
        }
        // Continue: the 56-point circle, 8 below the bar's content.
        let cont = Rect::from_min_size(
            pos2(grid.left(), grid.bottom() + 8.0 - 56.0),
            vec2(56.0, 56.0),
        );
        let cont_resp = ui.interact(cont, Id::new("editor-task-continue"), Sense::click());
        let fill = if cont_resp.hovered() {
            colors.continue_background_hover()
        } else {
            colors.continue_background()
        };
        painter.circle_filled(cont.center(), 28.0, fill);
        painter.circle_stroke(
            cont.center(),
            27.5,
            egui::Stroke::new(1.0, colors.continue_border()),
        );
        style::paint_icon(
            ui,
            &painter,
            cont.center() + vec2(2.0, 0.0),
            icons::ACTION_CONTINUE,
            22.0,
            colors.task_foreground(),
        );
        if cont_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let tip = if editor.config.use_continue_workflow {
            t(s::MainViewModel_ContinueEnter)
        } else {
            t(s::MainViewModel_RunAfterCaptureTasksEnter)
        };
        if cont_resp.on_hover_text(tip).clicked() {
            editor.continue_task();
        }
        x += 4.0 + 90.0 + 10.0 + SPACING;
    }
    if show_bottom {
        let has_image = editor.has_image();
        let buttons: [(char, &str, bool, Request); 6] = [
            (
                icons::ACTION_COPY,
                t(s::EditorView_CopyimagetoclipboardCtrlC),
                true,
                Request::CopyImage,
            ),
            (
                icons::ACTION_SAVE,
                t(s::EditorView_SaveimageCtrlS),
                true,
                Request::Save,
            ),
            (
                icons::ACTION_SAVE_AS,
                t(s::EditorView_SaveimageasEllipsisCtrlShiftS),
                true,
                Request::SaveAs,
            ),
            (
                icons::ACTION_PIN_TO_SCREEN,
                t(s::EditorView_PinimagetoscreenCtrlP),
                has_image,
                Request::PinToScreen,
            ),
            (
                icons::ACTION_PRINT,
                t(s::EditorView_PrintimageEllipsisCtrlShiftP),
                has_image,
                Request::Print,
            ),
            (
                icons::ACTION_UPLOAD,
                t(s::EditorView_UploadimageCtrlU),
                has_image,
                Request::Upload,
            ),
        ];
        for (glyph, tip, enabled, request) in buttons {
            let rect = Rect::from_min_size(pos2(x, top), vec2(ITEM, ITEM));
            let (resp, paint) =
                style::button_at(ui, colors, rect, ButtonKind::Standard, 10.0, false, enabled);
            style::paint_icon(
                ui,
                ui.painter(),
                paint.rect.center(),
                glyph,
                18.0,
                paint.text_block,
            );
            if resp.on_hover_text(tip).clicked() {
                editor.request(request);
            }
            x += ITEM + SPACING;
        }
    }
    bar.union(Rect::from_min_max(
        pos2(bar.left(), bar.top() - 8.0),
        bar.max,
    ))
}

/// The annotation toolbar of the region capture (`CaptureAnnotationToolbar`): the
/// visible tool buttons only (no crop, cut-out, file, background or effects), left edge
/// at `left_top`, border 0,1,1,1 and corner 0,0,4,0. `tools_active` is false while the
/// region tool is in use (no button lit). Returns the strip and the tool clicked.
pub fn tool_strip(
    ui: &mut Ui,
    colors: &Colors,
    editor: &Editor,
    left_top: egui::Pos2,
    tools_active: bool,
) -> (Rect, Option<EditorTool>) {
    let items: Vec<_> = editor
        .visible_toolbar_items()
        .filter(|i| {
            i.tool
                .is_some_and(|t| !matches!(t, EditorTool::Crop | EditorTool::CutOut))
        })
        .cloned()
        .collect();
    let mut widths = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let sep = if index > 0 && item.separator_visible() {
            1.0 + SPACING
        } else {
            0.0
        };
        widths.push(sep);
    }
    let inner: f32 = widths.iter().sum::<f32>()
        + ITEM * items.len() as f32
        + SPACING * (items.len().saturating_sub(1)) as f32;
    let bar = Rect::from_min_size(left_top, vec2(inner + 8.0 + 1.0, ITEM + 8.0 + 2.0));
    ui.interact(bar, Id::new("capture-tool-strip"), Sense::click());
    let painter = ui.painter().clone();
    let radius = egui::CornerRadius {
        nw: 0,
        ne: 0,
        sw: 0,
        se: 4,
    };
    painter.rect_filled(bar, radius, colors.background_toolbar());
    let stroke = egui::Stroke::new(1.0, colors.border());
    painter.line_segment(
        [
            pos2(bar.left(), bar.top() + 0.5),
            pos2(bar.right(), bar.top() + 0.5),
        ],
        stroke,
    );
    painter.line_segment(
        [
            pos2(bar.right() - 0.5, bar.top()),
            pos2(bar.right() - 0.5, bar.bottom() - 4.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            pos2(bar.left(), bar.bottom() - 0.5),
            pos2(bar.right() - 4.0, bar.bottom() - 0.5),
        ],
        stroke,
    );
    let mut x = bar.left() + 4.0;
    let top = bar.top() + 1.0 + 4.0;
    let mut clicked = None;
    for (item, sep) in items.iter().zip(widths) {
        if sep > 0.0 {
            style::vertical_separator(&painter, colors, x, top, top + ITEM);
            x += sep;
        }
        let rect = Rect::from_min_size(pos2(x, top), vec2(ITEM, ITEM));
        let active = tools_active && item.is_active;
        if style::toolbar_button(ui, colors, rect, item.icon, active, true)
            .on_hover_text(item.tooltip())
            .clicked()
        {
            clicked = item.tool;
        }
        x += ITEM + SPACING;
    }
    (bar, clicked)
}
