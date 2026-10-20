//! The skin and the controls of `ControlStyles.axaml` and `EditorStyles.axaml` the
//! editor draws: the palette by the names the styles use, the text runs (Inter, default
//! `TextBlock` SemiBold 13 in the secondary colour), the button classes (`Button`,
//! `toolbar-button`, `picker-button`, `ratio-button`, `transparent-button`,
//! `task-mode-button`, `effect-item`), Fluent's slider and toggle switch, the capsule,
//! the popup surface and the checkered brushes.
//!
//! The host (the app) installs the fonts and egui's global style; the editor only paints
//! with the families it names (`inter-semibold`, `lucide`). [`install_fonts`] puts the
//! same families into a context of its own (tests, QA shots).

use std::sync::Arc;

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Galley, Painter,
    Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use modlight_core::theme::{Palette, ResolvedTheme, Rgba};

/// `ShareX.FontSize.Default`.
pub const FONT_SIZE: f32 = 13.0;

pub const FAMILY_SEMIBOLD: &str = "inter-semibold";
pub const FAMILY_ICON: &str = "lucide";

pub fn semibold() -> FontFamily {
    FontFamily::Name(FAMILY_SEMIBOLD.into())
}

pub fn icon_family() -> FontFamily {
    FontFamily::Name(FAMILY_ICON.into())
}

/// A text weight as the styles name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    Normal,
    SemiBold,
}

impl Weight {
    pub fn family(self) -> FontFamily {
        match self {
            Weight::Normal => FontFamily::Proportional,
            Weight::SemiBold => semibold(),
        }
    }
}

pub fn rgba(color: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), color.a())
}

/// The skin of the moment, with the colours by the names the styles use.
#[derive(Clone, Debug, PartialEq)]
pub struct Colors {
    pub dark: bool,
    pub palette: Palette,
}

macro_rules! color {
    ($($name:ident),* $(,)?) => {
        $(pub fn $name(&self) -> Color32 { rgba(self.palette.$name) })*
    };
}

impl Colors {
    pub fn new(theme: &ResolvedTheme) -> Colors {
        Colors {
            dark: theme.dark,
            palette: theme.palette,
        }
    }

    /// The default skin in light or dark mode (tests and QA).
    pub fn default_skin(dark: bool) -> Colors {
        let options = modlight_core::theme::ThemeOptions {
            theme: if dark { "Dark".into() } else { "Light".into() },
            ..Default::default()
        };
        Colors::new(&modlight_core::theme::resolve(&options, None, None))
    }

    color!(
        accent_start,
        accent_end,
        accent_foreground,
        text,
        text_secondary,
        focus,
        background_main,
        background_panel,
        background_popup,
        background_menu,
        background_toolbar,
        border,
        control_background,
        control_background_hover,
        control_background_disabled,
        separator,
        cancel_background,
        cancel_border,
        continue_background,
        continue_background_hover,
        continue_border,
        task_foreground,
        checker_a,
        checker_b,
    );

    /// `ShareX.Brush.Overlay.Soft`.
    pub fn overlay_soft(&self) -> Color32 {
        Color32::from_black_alpha(0x30)
    }

    /// `ShareX.Brush.Overlay.Modal`.
    pub fn overlay_modal(&self) -> Color32 {
        Color32::from_black_alpha(0x80)
    }
}

/// The families the editor paints with, for a context the app did not set up.
pub fn font_definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let faces: [(&str, &'static [u8]); 5] = [
        ("inter-regular-latin", crate::text::INTER_REGULAR_LATIN),
        ("inter-regular-ext", crate::text::INTER_REGULAR_EXT),
        ("inter-semibold-latin", crate::text::INTER_SEMIBOLD_LATIN),
        ("inter-semibold-ext", crate::text::INTER_SEMIBOLD_EXT),
        ("lucide", modlight_core::icons::FONT),
    ];
    for (name, bytes) in faces {
        fonts
            .font_data
            .insert(name.to_string(), Arc::new(FontData::from_static(bytes)));
    }
    let fallback: Vec<String> = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let chain = |first: &str, second: &str| {
        let mut list = vec![first.to_string(), second.to_string()];
        list.extend(fallback.iter().cloned());
        list
    };
    fonts.families.insert(
        FontFamily::Proportional,
        chain("inter-regular-latin", "inter-regular-ext"),
    );
    fonts.families.insert(
        semibold(),
        chain("inter-semibold-latin", "inter-semibold-ext"),
    );
    fonts.families.insert(
        FontFamily::Name("inter-medium".into()),
        chain("inter-semibold-latin", "inter-semibold-ext"),
    );
    fonts
        .families
        .insert(icon_family(), vec!["lucide".to_string()]);
    fonts
}

/// Installs [`font_definitions`] and a style close to the app's (tests, QA shots).
pub fn install(ctx: &egui::Context, colors: &Colors) {
    ctx.set_fonts(font_definitions());
    let theme = if colors.dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    ctx.set_theme(theme);
    let mut visuals = if colors.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.panel_fill = colors.background_main();
    visuals.window_fill = colors.background_popup();
    visuals.window_stroke = Stroke::new(1.0, colors.border());
    visuals.extreme_bg_color = colors.control_background();
    visuals.text_edit_bg_color = Some(colors.control_background());
    visuals.selection.bg_fill = colors.accent_start().gamma_multiply(0.45);
    visuals.text_cursor.stroke = Stroke::new(1.5, colors.text());
    visuals.widgets.inactive.bg_fill = colors.control_background();
    visuals.widgets.inactive.weak_bg_fill = colors.control_background();
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, colors.border());
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, colors.text());
    visuals.widgets.hovered.bg_fill = colors.control_background_hover();
    visuals.widgets.hovered.weak_bg_fill = colors.control_background_hover();
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, colors.text());
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, colors.text());
    ctx.style_mut_of(theme, |style| style.visuals = visuals);
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = vec2(0.0, 0.0);
        style.spacing.button_padding = vec2(10.0, 6.0);
        style.spacing.scroll = egui::style::ScrollStyle::thin();
        style.spacing.scroll.floating = true;
        style.interaction.selectable_labels = false;
        style.text_styles.insert(
            egui::TextStyle::Body,
            FontId::new(FONT_SIZE, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            FontId::new(FONT_SIZE, FontFamily::Proportional),
        );
    });
}

// ---------------------------------------------------------------------------- text

pub fn job(text: &str, size: f32, weight: Weight, color: Color32) -> LayoutJob {
    LayoutJob::single_section(
        text.to_string(),
        TextFormat {
            font_id: FontId::new(size, weight.family()),
            color,
            ..Default::default()
        },
    )
}

pub fn galley(ui: &Ui, text: &str, size: f32, weight: Weight, color: Color32) -> Arc<Galley> {
    ui.fonts_mut(|f| f.layout_job(job(text, size, weight, color)))
}

/// A single line cut with an ellipsis (`TextTrimming="CharacterEllipsis"`).
pub fn galley_trimmed(
    ui: &Ui,
    text: &str,
    size: f32,
    weight: Weight,
    color: Color32,
    max_width: f32,
) -> Arc<Galley> {
    let mut job = job(text, size, weight, color);
    job.wrap = TextWrapping {
        max_width: max_width.max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    ui.fonts_mut(|f| f.layout_job(job))
}

/// Text wrapping at `max_width` (`TextWrapping="Wrap"`).
pub fn galley_wrapped(
    ui: &Ui,
    text: &str,
    size: f32,
    weight: Weight,
    color: Color32,
    max_width: f32,
) -> Arc<Galley> {
    let mut job = job(text, size, weight, color);
    job.wrap = TextWrapping {
        max_width: max_width.max(1.0),
        max_rows: usize::MAX,
        break_anywhere: false,
        overflow_character: None,
    };
    ui.fonts_mut(|f| f.layout_job(job))
}

/// A path cut from the start (`TextTrimming="LeadingCharacterEllipsis"`).
pub fn leading_trimmed(
    ui: &Ui,
    text: &str,
    size: f32,
    weight: Weight,
    color: Color32,
    max_width: f32,
) -> Arc<Galley> {
    let full = galley(ui, text, size, weight, color);
    if full.size().x <= max_width {
        return full;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut start = 0;
    while start < chars.len() {
        start += 1;
        let candidate: String = std::iter::once('…')
            .chain(chars[start..].iter().copied())
            .collect();
        let g = galley(ui, &candidate, size, weight, color);
        if g.size().x <= max_width {
            return g;
        }
    }
    galley(ui, "…", size, weight, color)
}

pub fn icon_galley(ui: &Ui, glyph: char, size: f32, color: Color32) -> Arc<Galley> {
    ui.fonts_mut(|f| f.layout_no_wrap(glyph.to_string(), FontId::new(size, icon_family()), color))
}

/// A Lucide glyph centred on `center`.
pub fn paint_icon(
    ui: &Ui,
    painter: &Painter,
    center: Pos2,
    glyph: char,
    size: f32,
    color: Color32,
) {
    let g = icon_galley(ui, glyph, size, color);
    painter.galley(center - g.size() / 2.0, g, color);
}

/// A text run placed in `rect` by `align`; returns where it went.
#[allow(clippy::too_many_arguments)]
pub fn paint_text(
    ui: &Ui,
    painter: &Painter,
    rect: Rect,
    align: Align2,
    text: &str,
    size: f32,
    weight: Weight,
    color: Color32,
) -> Rect {
    let g = galley_trimmed(ui, text, size, weight, color, rect.width());
    let pos = align.align_size_within_rect(g.size(), rect).min;
    let out = Rect::from_min_size(pos, g.size());
    painter.galley(pos, g, color);
    out
}

/// Multiplies a colour's alpha (`Opacity`).
pub fn fade(color: Color32, opacity: f32) -> Color32 {
    if opacity >= 1.0 {
        return color;
    }
    color.gamma_multiply(opacity.clamp(0.0, 1.0))
}

// ---------------------------------------------------------------------------- surfaces

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        l(a.r(), b.r()),
        l(a.g(), b.g()),
        l(a.b(), b.b()),
        l(a.a(), b.a()),
    )
}

fn rounded_points(rect: Rect, radius: f32) -> Vec<Pos2> {
    let r = radius
        .min(rect.width() / 2.0)
        .min(rect.height() / 2.0)
        .max(0.0);
    let mut points = Vec::new();
    let corners = [
        (pos2(rect.right() - r, rect.top() + r), -90.0f32),
        (pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (pos2(rect.left() + r, rect.bottom() - r), 90.0),
        (pos2(rect.left() + r, rect.top() + r), 180.0),
    ];
    for (center, start) in corners {
        for step in 0..=6 {
            let angle = (start + step as f32 * 15.0).to_radians();
            points.push(pos2(center.x + r * angle.cos(), center.y + r * angle.sin()));
        }
    }
    points
}

/// A rounded rectangle with a diagonal gradient (`ShareX.Brush.Accent`).
pub fn gradient_rect(painter: &Painter, rect: Rect, radius: f32, start: Color32, end: Color32) {
    if start == end {
        painter.rect_filled(rect, radius, start);
        return;
    }
    let points = rounded_points(rect, radius);
    let mut mesh = Mesh::default();
    let center = rect.center();
    let span = (rect.width() + rect.height()).max(1.0);
    let color_at = |p: Pos2| {
        let t = (((p.x - rect.left()) + (p.y - rect.top())) / span).clamp(0.0, 1.0);
        lerp_color(start, end, t)
    };
    mesh.vertices.push(Vertex {
        pos: center,
        uv: WHITE_UV,
        color: color_at(center),
    });
    for p in &points {
        mesh.vertices.push(Vertex {
            pos: *p,
            uv: WHITE_UV,
            color: color_at(*p),
        });
    }
    let n = points.len() as u32;
    for i in 0..n {
        mesh.indices.extend_from_slice(&[0, 1 + i, 1 + (i + 1) % n]);
    }
    painter.add(Shape::mesh(mesh));
    painter.add(Shape::closed_line(
        points,
        Stroke::new(0.6, lerp_color(start, end, 0.5)),
    ));
}

/// `ShareX.Brush.Accent` at an opacity.
pub fn paint_accent(painter: &Painter, colors: &Colors, rect: Rect, radius: f32, opacity: f32) {
    gradient_rect(
        painter,
        rect,
        radius,
        fade(colors.accent_start(), opacity),
        fade(colors.accent_end(), opacity),
    );
}

/// A soft shadow under a surface (`BoxShadow`).
pub fn paint_shadow(
    painter: &Painter,
    rect: Rect,
    radius: f32,
    offset: Vec2,
    blur: f32,
    color: Color32,
) {
    let shadow = egui::Shadow {
        offset: [offset.x as i8, offset.y as i8],
        blur: blur.clamp(0.0, 255.0) as u8,
        spread: 0,
        color,
    };
    painter.add(shadow.as_shape(rect, CornerRadius::same(radius.clamp(0.0, 255.0) as u8)));
}

/// `Border.popup-border.popup-surface.open` with the background the editor gives it
/// (`ShareX.Brush.Background.Panel`, radius 8).
pub fn popup_frame(colors: &Colors, padding: i8) -> egui::Frame {
    egui::Frame::new()
        .fill(colors.background_panel())
        .stroke(Stroke::new(1.0, colors.border()))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(padding))
        .shadow(egui::Shadow {
            offset: [0, 14],
            blur: 28,
            spread: 0,
            color: Color32::from_black_alpha(0x40),
        })
}

/// The menu surface (`ContextMenu`, `MenuFlyoutPresenter`).
pub fn menu_frame(colors: &Colors) -> egui::Frame {
    egui::Frame::new()
        .fill(colors.background_menu())
        .stroke(Stroke::new(1.0, colors.border()))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(0, 2))
        .shadow(egui::Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha(0x40),
        })
}

/// A checkered brush texture (two colours, 2 by 2 cells, repeating, nearest).
fn checker_texture(ctx: &egui::Context, a: Color32, b: Color32) -> egui::TextureHandle {
    let id = egui::Id::new(("editor-checker", a.to_array(), b.to_array()));
    if let Some(t) = ctx.memory_mut(|m| m.data.get_temp::<egui::TextureHandle>(id)) {
        return t;
    }
    let image = egui::ColorImage::new([2, 2], vec![b, a, a, b]);
    let options = egui::TextureOptions {
        wrap_mode: egui::TextureWrapMode::Repeat,
        ..egui::TextureOptions::NEAREST
    };
    let texture = ctx.load_texture("editor-checker", image, options);
    ctx.memory_mut(|m| m.data.insert_temp(id, texture.clone()));
    texture
}

/// `ShareX.Brush.Checkered` (15-unit cells) or `.Small` (4-unit cells), with the cell
/// size in points, anchored at `origin`.
pub fn paint_checkered(
    ui: &Ui,
    painter: &Painter,
    colors: &Colors,
    rect: Rect,
    cell: f32,
    origin: Pos2,
) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 || cell <= 0.0 {
        return;
    }
    let texture = checker_texture(ui.ctx(), colors.checker_a(), colors.checker_b());
    let period = cell * 2.0;
    let uv = Rect::from_min_max(
        pos2(
            (rect.left() - origin.x) / period,
            (rect.top() - origin.y) / period,
        ),
        pos2(
            (rect.right() - origin.x) / period,
            (rect.bottom() - origin.y) / period,
        ),
    );
    painter.image(texture.id(), rect, uv, Color32::WHITE);
}

/// `ShareX.Brush.Checkered.Small` clipped to a rounded rectangle.
pub fn paint_checkered_small(ui: &Ui, painter: &Painter, colors: &Colors, rect: Rect, radius: f32) {
    if radius <= 0.0 {
        paint_checkered(ui, painter, colors, rect, 4.0, rect.min);
        return;
    }
    let texture = checker_texture(ui.ctx(), colors.checker_a(), colors.checker_b());
    let uv = Rect::from_min_max(
        pos2(0.0, 0.0),
        pos2(rect.width() / 8.0, rect.height() / 8.0),
    );
    painter.add(
        egui::epaint::RectShape::filled(rect, radius, Color32::WHITE)
            .with_texture(texture.id(), uv),
    );
}

// ---------------------------------------------------------------------------- buttons

/// The button classes the editor uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    /// `Button`: control background, 1 px border, radius 10, pressed 97 %.
    Standard,
    /// `picker-button`: transparent, hover `Control.Background.Disabled`.
    Picker,
    /// `transparent-button`: no fill in any state.
    Transparent,
    /// `effect-item`: control background, no border, radius 8.
    EffectItem,
    /// `task-mode-button danger`.
    Danger,
    /// `task-mode-button success`.
    Success,
}

/// What a painted button looks like now.
#[derive(Clone, Copy, Debug)]
pub struct ButtonPaint {
    /// The content's colour for a string `Content` (the button's `Foreground`).
    pub foreground: Color32,
    /// The content's colour for an explicit `TextBlock` (secondary unless active).
    pub text_block: Color32,
    /// The rect the content goes in (shrunk while pressed).
    pub rect: Rect,
    pub opacity: f32,
}

/// Allocates `rect` as a button and paints its background for the state.
pub fn button_at(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    kind: ButtonKind,
    radius: f32,
    active: bool,
    enabled: bool,
) -> (Response, ButtonPaint) {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.allocate_rect(rect, sense);
    let paint = paint_button(ui, colors, &response, rect, kind, radius, active, enabled);
    if enabled && response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    (response, paint)
}

#[allow(clippy::too_many_arguments)]
pub fn paint_button(
    ui: &Ui,
    colors: &Colors,
    response: &Response,
    rect: Rect,
    kind: ButtonKind,
    radius: f32,
    active: bool,
    enabled: bool,
) -> ButtonPaint {
    let hovered = enabled && response.hovered();
    let pressed = enabled && response.is_pointer_button_down_on();
    let opacity = if enabled { 1.0 } else { 0.35 };
    let scale = matches!(kind, ButtonKind::Standard | ButtonKind::EffectItem) && pressed;
    let rect = if scale {
        Rect::from_center_size(rect.center(), rect.size() * 0.97)
    } else {
        rect
    };
    let painter = ui.painter();
    let mut foreground = colors.text();
    let mut text_block = colors.text_secondary();
    match kind {
        ButtonKind::Standard | ButtonKind::Picker if active => {
            paint_accent(painter, colors, rect, radius, opacity);
            foreground = colors.accent_foreground();
            text_block = colors.accent_foreground();
        }
        ButtonKind::Standard => {
            let fill = if hovered {
                colors.control_background_hover()
            } else {
                colors.control_background()
            };
            painter.rect_filled(rect, radius, fade(fill, opacity));
            painter.rect_stroke(
                rect,
                radius,
                Stroke::new(1.0, fade(colors.border(), opacity)),
                StrokeKind::Inside,
            );
        }
        ButtonKind::Picker => {
            if hovered || pressed {
                painter.rect_filled(rect, radius, colors.control_background_disabled());
            }
        }
        ButtonKind::Transparent => {}
        ButtonKind::EffectItem => {
            let fill = if hovered {
                colors.control_background_hover()
            } else {
                colors.control_background()
            };
            painter.rect_filled(rect, radius, fade(fill, opacity));
        }
        ButtonKind::Danger => {
            let fill = if hovered {
                colors.cancel_border()
            } else {
                colors.cancel_background()
            };
            painter.rect_filled(rect, radius, fade(fill, opacity));
            painter.rect_stroke(
                rect,
                radius,
                Stroke::new(1.0, fade(colors.cancel_border(), opacity)),
                StrokeKind::Inside,
            );
            foreground = colors.task_foreground();
            text_block = colors.task_foreground();
        }
        ButtonKind::Success => {
            let fill = if hovered {
                colors.continue_background_hover()
            } else {
                colors.continue_background()
            };
            painter.rect_filled(rect, radius, fade(fill, opacity));
            painter.rect_stroke(
                rect,
                radius,
                Stroke::new(1.0, fade(colors.continue_border(), opacity)),
                StrokeKind::Inside,
            );
            foreground = colors.task_foreground();
            text_block = colors.task_foreground();
        }
    }
    ButtonPaint {
        foreground: fade(foreground, opacity),
        text_block: fade(text_block, opacity),
        rect,
        opacity,
    }
}

/// A `Button` with a string `Content` at a size; returns whether it was clicked.
#[allow(clippy::too_many_arguments)]
pub fn text_button(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    label: &str,
    active: bool,
    enabled: bool,
) -> Response {
    let (response, paint) = button_at(
        ui,
        colors,
        rect,
        ButtonKind::Standard,
        10.0,
        active,
        enabled,
    );
    paint_text(
        ui,
        ui.painter(),
        paint.rect.shrink2(vec2(6.0, 0.0)),
        Align2::CENTER_CENTER,
        label,
        FONT_SIZE,
        Weight::Normal,
        paint.foreground,
    );
    response
}

/// The width a text button needs with a horizontal padding.
pub fn text_button_width(ui: &Ui, label: &str, padding: f32) -> f32 {
    galley(ui, label, FONT_SIZE, Weight::Normal, Color32::WHITE)
        .size()
        .x
        + padding * 2.0
        + 2.0
}

/// `toolbar-button`: 40 by 40, an icon glyph as `Content` (Text colour, 18).
pub fn toolbar_button(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    glyph: char,
    active: bool,
    enabled: bool,
) -> Response {
    let (response, paint) = button_at(
        ui,
        colors,
        rect,
        ButtonKind::Standard,
        10.0,
        active,
        enabled,
    );
    paint_icon(
        ui,
        ui.painter(),
        paint.rect.center(),
        glyph,
        18.0,
        paint.foreground,
    );
    response
}

/// The chevron of the drop-downs (`dropdown-chevron`, turned while open).
pub fn paint_chevron(ui: &Ui, painter: &Painter, center: Pos2, open: bool, color: Color32) {
    let g = icon_galley(ui, crate::ui::icons::CHEVRON_DOWN, 10.0, color);
    let pos = center - g.size() / 2.0;
    if open {
        let mut shape = egui::epaint::TextShape::new(pos + g.size(), g, color);
        shape.angle = std::f32::consts::PI;
        painter.add(shape);
    } else {
        painter.galley(pos, g, color);
    }
}

// ---------------------------------------------------------------------------- slider

/// Fluent's horizontal `Slider` (32 tall, 4-point track, 20-point thumb) with the
/// ShareX thumb (accent with a 4-point ring of the control background, 3 on hover).
/// Returns the new value when the user moved it.
#[allow(clippy::too_many_arguments)]
pub fn slider(
    ui: &mut Ui,
    colors: &Colors,
    id: egui::Id,
    rect: Rect,
    value: f64,
    min: f64,
    max: f64,
    step: Option<f64>,
    enabled: bool,
) -> Option<f64> {
    let sense = if enabled {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, id, sense);
    let span = (max - min).max(f64::EPSILON);
    let track_left = rect.left() + 10.0;
    let track_right = rect.right() - 10.0;
    let width = (track_right - track_left).max(1.0);
    let mut changed = None;
    if (response.dragged() || response.clicked() || response.drag_started())
        && let Some(p) = response.interact_pointer_pos()
    {
        let t = ((p.x - track_left) / width).clamp(0.0, 1.0) as f64;
        let mut v = min + t * span;
        if let Some(step) = step.filter(|s| *s > 0.0) {
            v = min + ((v - min) / step).round() * step;
        }
        let v = v.clamp(min, max);
        if (v - value).abs() > f64::EPSILON {
            changed = Some(v);
        }
    }
    let current = changed.unwrap_or(value).clamp(min, max);
    let t = ((current - min) / span) as f32;
    let opacity = if enabled { 1.0 } else { 0.35 };
    let painter = ui.painter();
    let cy = rect.center().y;
    let track = Rect::from_min_max(pos2(track_left, cy - 2.0), pos2(track_right, cy + 2.0));
    painter.rect_filled(
        track,
        2.0,
        fade(colors.control_background_disabled(), opacity),
    );
    let thumb_x = track_left + width * t;
    let filled = Rect::from_min_max(track.min, pos2(thumb_x, track.max.y));
    if filled.width() > 0.0 {
        painter.rect_filled(filled, 2.0, fade(colors.accent_start(), opacity));
    }
    let hovered = enabled && (response.hovered() || response.dragged());
    let ring = if hovered { 3.0 } else { 4.0 };
    let center = pos2(thumb_x, cy);
    painter.circle_filled(center, 10.0, fade(colors.control_background(), opacity));
    painter.circle_filled(center, 10.0 - ring, fade(colors.accent_start(), opacity));
    changed
}

// ---------------------------------------------------------------------------- toggle

/// Fluent's `ToggleSwitch` (40 by 20) with the ShareX colours.
pub fn paint_toggle(
    painter: &Painter,
    colors: &Colors,
    rect: Rect,
    on: bool,
    hovered: bool,
    enabled: bool,
) {
    let opacity = if enabled { 1.0 } else { 0.35 };
    let r = rect.height() / 2.0;
    let knob = if hovered { 7.0 } else { 6.0 };
    if on {
        paint_accent(painter, colors, rect, r, opacity);
        painter.circle_filled(
            pos2(rect.right() - r, rect.center().y),
            knob,
            fade(colors.accent_foreground(), opacity),
        );
    } else {
        let base = if colors.dark {
            Color32::WHITE
        } else {
            Color32::BLACK
        };
        painter.rect_stroke(
            rect,
            r,
            Stroke::new(1.0, fade(base.gamma_multiply(0.6), opacity)),
            StrokeKind::Inside,
        );
        painter.circle_filled(
            pos2(rect.left() + r, rect.center().y),
            knob,
            fade(base.gamma_multiply(0.8), opacity),
        );
    }
}

/// `LabeledToggleSwitch`: the label left, the switch right, the whole row clickable.
/// Returns the new value when it was toggled.
pub fn labeled_toggle(
    ui: &mut Ui,
    colors: &Colors,
    width: f32,
    label: &str,
    value: bool,
    enabled: bool,
) -> Option<bool> {
    let label_width = width - 40.0 - 12.0;
    let g = galley_wrapped(
        ui,
        label,
        FONT_SIZE,
        Weight::SemiBold,
        colors.text_secondary(),
        label_width,
    );
    let height = g.size().y.max(32.0);
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, height),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let opacity = if enabled { 1.0 } else { 0.35 };
    let painter = ui.painter();
    painter.galley(
        pos2(rect.left(), rect.center().y - g.size().y / 2.0),
        g,
        fade(colors.text_secondary(), opacity),
    );
    let switch = Rect::from_min_size(
        pos2(rect.right() - 40.0, rect.center().y - 10.0),
        vec2(40.0, 20.0),
    );
    paint_toggle(
        painter,
        colors,
        switch,
        value,
        enabled && response.hovered(),
        enabled,
    );
    if enabled && response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    (enabled && response.clicked()).then_some(!value)
}

/// `Border.capsule`: the accent pill with a value.
pub fn capsule(
    ui: &Ui,
    painter: &Painter,
    colors: &Colors,
    right: f32,
    center_y: f32,
    text: &str,
) -> Rect {
    let g = galley(
        ui,
        text,
        FONT_SIZE,
        Weight::SemiBold,
        colors.accent_foreground(),
    );
    let size = g.size() + vec2(12.0, 6.0);
    let rect = Rect::from_min_size(pos2(right - size.x, center_y - size.y / 2.0), size);
    paint_accent(painter, colors, rect, size.y / 2.0, 1.0);
    painter.galley(rect.min + vec2(6.0, 3.0), g, colors.accent_foreground());
    rect
}

/// A labelled slider row (`EffectSlider` and the sidebars): the label and the value
/// capsule on one line, the slider under it. Returns the new value.
#[allow(clippy::too_many_arguments)]
pub fn slider_row(
    ui: &mut Ui,
    colors: &Colors,
    id: egui::Id,
    width: f32,
    label: &str,
    value_text: &str,
    value: f64,
    min: f64,
    max: f64,
    step: Option<f64>,
) -> Option<f64> {
    let (head, _) = ui.allocate_exact_size(vec2(width, 22.0), Sense::hover());
    let painter = ui.painter().clone();
    let cap = capsule(
        ui,
        &painter,
        colors,
        head.right(),
        head.center().y,
        value_text,
    );
    paint_text(
        ui,
        &painter,
        Rect::from_min_max(head.min, pos2(cap.left() - 6.0, head.max.y)),
        Align2::LEFT_CENTER,
        label,
        FONT_SIZE,
        Weight::SemiBold,
        colors.text_secondary(),
    );
    let (rect, _) = ui.allocate_exact_size(vec2(width, 32.0), Sense::hover());
    slider(ui, colors, id, rect, value, min, max, step, true)
}

/// The separator between toolbar groups (`Border Width="1"`).
pub fn vertical_separator(painter: &Painter, colors: &Colors, x: f32, top: f32, bottom: f32) {
    painter.rect_filled(
        Rect::from_min_max(pos2(x, top), pos2(x + 1.0, bottom)),
        0.0,
        colors.separator(),
    );
}

/// A text box in the `TextBox` style (control background, 1 px border, radius 8,
/// padding 8 by 6, 40 tall; a 2 px focus border).
pub fn text_box(
    ui: &mut Ui,
    colors: &Colors,
    rect: Rect,
    text: &mut String,
    hint: &str,
    enabled: bool,
    id: egui::Id,
) -> Response {
    let painter = ui.painter().clone();
    let focused = ui.memory(|m| m.has_focus(id));
    let fill = if enabled {
        colors.control_background()
    } else {
        colors.control_background_disabled()
    };
    painter.rect_filled(rect, 8.0, fill);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(8.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let edit = egui::TextEdit::singleline(text)
        .id(id)
        .frame(egui::Frame::NONE)
        .hint_text(egui::RichText::new(hint).color(colors.text_secondary()))
        .text_color(colors.text())
        .font(FontId::new(FONT_SIZE, FontFamily::Proportional))
        .desired_width(rect.width() - 16.0)
        .vertical_align(egui::Align::Center);
    let response = child.add_enabled(enabled, edit);
    if focused {
        painter.rect_stroke(
            rect,
            8.0,
            Stroke::new(2.0, colors.focus()),
            StrokeKind::Inside,
        );
    } else {
        painter.rect_stroke(
            rect,
            8.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
    }
    response
}

/// Paints an RGBA picture into `rect` through a cached texture.
pub fn paint_image(
    ui: &Ui,
    painter: &Painter,
    key: &str,
    image: &image::RgbaImage,
    rect: Rect,
    tint: Color32,
) {
    let id = egui::Id::new(("editor-image", key));
    let texture = ui
        .ctx()
        .memory_mut(|m| m.data.get_temp::<egui::TextureHandle>(id))
        .unwrap_or_else(|| {
            let color = egui::ColorImage::from_rgba_unmultiplied(
                [image.width() as usize, image.height() as usize],
                image.as_raw(),
            );
            let t = ui.ctx().load_texture(
                format!("editor-image-{key}"),
                color,
                egui::TextureOptions::LINEAR,
            );
            ui.ctx().memory_mut(|m| m.data.insert_temp(id, t.clone()));
            t
        });
    painter.image(
        texture.id(),
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        tint,
    );
}

// ---------------------------------------------------------------------------- combo box

/// The `ComboBox` style: control background, 1 px border, radius 8, padding 8 by 6, a
/// chevron; its list in a popup as wide as the box, the selected item on the accent.
/// Returns the index picked.
pub fn combo_box(
    ui: &mut Ui,
    colors: &Colors,
    id: egui::Id,
    rect: Rect,
    labels: &[String],
    selected: usize,
    enabled: bool,
) -> Option<usize> {
    let response = ui.interact(
        rect,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let painter = ui.painter().clone();
    let opacity = if enabled { 1.0 } else { 0.35 };
    let fill = if !enabled {
        colors.control_background_disabled()
    } else if response.hovered() {
        colors.control_background_hover()
    } else {
        colors.control_background()
    };
    painter.rect_filled(rect, 8.0, fill);
    painter.rect_stroke(
        rect,
        8.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    let label = labels.get(selected).map(String::as_str).unwrap_or("");
    paint_text(
        ui,
        &painter,
        Rect::from_min_max(
            pos2(rect.left() + 8.0, rect.top()),
            pos2(rect.right() - 32.0, rect.bottom()),
        ),
        Align2::LEFT_CENTER,
        label,
        FONT_SIZE,
        Weight::SemiBold,
        fade(colors.text(), opacity),
    );
    let popup_id = egui::Popup::default_response_id(&response);
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    paint_chevron(
        ui,
        &painter,
        pos2(rect.right() - 18.0, rect.center().y),
        open,
        fade(colors.text_secondary(), opacity),
    );
    let width = rect.width();
    egui::Popup::from_toggle_button_response(&response)
        .align(egui::RectAlign::BOTTOM_START)
        .gap(2.0)
        .frame(
            egui::Frame::new()
                .fill(colors.background_menu())
                .stroke(Stroke::new(1.0, colors.border()))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(egui::Margin::same(4)),
        )
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let mut picked = None;
            for (i, label) in labels.iter().enumerate() {
                let (r, row) = ui.allocate_exact_size(vec2(width - 8.0, 34.0), Sense::click());
                let is_selected = i == selected;
                let fg = if is_selected {
                    paint_accent(ui.painter(), colors, r, 6.0, 1.0);
                    colors.accent_foreground()
                } else {
                    if row.hovered() {
                        ui.painter()
                            .rect_filled(r, 6.0, colors.control_background_hover());
                    }
                    colors.text()
                };
                paint_text(
                    ui,
                    ui.painter(),
                    r.shrink2(vec2(10.0, 0.0)),
                    Align2::LEFT_CENTER,
                    label,
                    FONT_SIZE,
                    Weight::SemiBold,
                    fg,
                );
                if row.clicked() {
                    picked = Some(i);
                    ui.close();
                }
            }
            picked
        })
        .and_then(|r| r.inner)
}

/// `NumericUpDown`: a text box and the spinner's two buttons. Returns the new value
/// (`None` when unchanged; `Some(None)` when emptied).
#[allow(clippy::too_many_arguments)]
pub fn numeric_box(
    ui: &mut Ui,
    colors: &Colors,
    id: egui::Id,
    rect: Rect,
    value: Option<f64>,
    min: f64,
    max: f64,
    increment: f64,
    decimals: usize,
) -> Option<Option<f64>> {
    let format = |v: f64| format!("{v:.decimals$}");
    let text_id = id.with("text");
    let focused = ui.memory(|m| m.has_focus(text_id));
    let mut text = if focused {
        ui.data(|d| d.get_temp::<String>(id))
            .unwrap_or_else(|| value.map(format).unwrap_or_default())
    } else {
        value.map(format).unwrap_or_default()
    };
    let spin_w = 24.0;
    let box_rect = Rect::from_min_max(
        rect.min,
        pos2(rect.right() - spin_w * 2.0 - 4.0, rect.bottom()),
    );
    let response = text_box(ui, colors, box_rect, &mut text, "", true, text_id);
    let mut out = None;
    if response.changed() {
        ui.data_mut(|d| d.insert_temp(id, text.clone()));
        let trimmed = text.trim();
        if trimmed.is_empty() {
            out = Some(None);
        } else if let Ok(v) = trimmed.replace(',', ".").parse::<f64>() {
            out = Some(Some(v.clamp(min, max)));
        }
    }
    if response.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    let up = Rect::from_min_size(
        pos2(box_rect.right() + 4.0, rect.top()),
        vec2(spin_w, rect.height()),
    );
    let down = Rect::from_min_size(pos2(up.right(), rect.top()), vec2(spin_w, rect.height()));
    for (r, glyph, delta) in [
        (up, modlight_core::icons::CHEVRON_UP, increment),
        (down, modlight_core::icons::CHEVRON_DOWN, -increment),
    ] {
        let (resp, paint) = button_at(ui, colors, r, ButtonKind::Standard, 6.0, false, true);
        paint_icon(
            ui,
            ui.painter(),
            paint.rect.center(),
            glyph,
            12.0,
            paint.foreground,
        );
        if resp.clicked() {
            let v = (value.unwrap_or(min) + delta).clamp(min, max);
            out = Some(Some(v));
        }
    }
    out
}
