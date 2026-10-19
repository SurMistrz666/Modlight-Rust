//! The controls of `ControlStyles.axaml` and the window styles, drawn with egui.
//!
//! Every Avalonia control the screens use has a look here with its numbers: heights,
//! paddings, corner radii and which palette colour fills what in which state. The
//! global `Button` style is 40 tall with 10 px corners; the classes (`tab`,
//! `strip-button`, `quiet-link`, `crumb`, ...) override what they override.

use std::sync::Arc;

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align2, Color32, CornerRadius, FontFamily, FontId, Galley, Painter, Pos2, Rect, Response,
    Sense, Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};

use crate::theme::{Colors, Weight, icon_family};

/// A text run: size, weight and colour.
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

/// A text run trimmed to `max_width` on at most `max_rows` lines, with an ellipsis.
pub fn trimmed(
    text: &str,
    size: f32,
    weight: Weight,
    color: Color32,
    max_width: f32,
    max_rows: usize,
) -> LayoutJob {
    let mut job = job(text, size, weight, color);
    job.wrap = TextWrapping {
        max_width,
        max_rows,
        break_anywhere: max_rows == 1,
        overflow_character: Some('…'),
    };
    job
}

/// A text run wrapping at `max_width` with no line limit.
pub fn wrapped(text: &str, size: f32, weight: Weight, color: Color32, max_width: f32) -> LayoutJob {
    let mut job = job(text, size, weight, color);
    job.wrap = TextWrapping {
        max_width,
        max_rows: usize::MAX,
        break_anywhere: false,
        overflow_character: None,
    };
    job
}

pub fn layout(ui: &Ui, job: LayoutJob) -> Arc<Galley> {
    ui.fonts_mut(|f| f.layout_job(job))
}

pub fn icon_galley(ui: &Ui, glyph: char, size: f32, color: Color32) -> Arc<Galley> {
    ui.fonts_mut(|f| f.layout_no_wrap(glyph.to_string(), FontId::new(size, icon_family()), color))
}

/// Paint a Lucide glyph centred on `center`.
pub fn paint_icon(
    ui: &Ui,
    painter: &Painter,
    center: Pos2,
    glyph: char,
    size: f32,
    color: Color32,
) {
    let galley = icon_galley(ui, glyph, size, color);
    let pos = center - galley.size() / 2.0;
    painter.galley(pos, galley, color);
}

/// Paint a galley with its left edge at `left`, centred vertically on `center_y`.
pub fn paint_text_left(
    painter: &Painter,
    left: f32,
    center_y: f32,
    galley: Arc<Galley>,
    color: Color32,
) -> Rect {
    let rect = Rect::from_min_size(pos2(left, center_y - galley.size().y / 2.0), galley.size());
    painter.galley(rect.min, galley, color);
    rect
}

/// Allocate and paint a label (an Avalonia TextBlock).
pub fn text(ui: &mut Ui, text: &str, size: f32, weight: Weight, color: Color32) -> Response {
    let galley = layout(ui, job(text, size, weight, color));
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter().galley(rect.min, galley, color);
    response
}

/// A label wrapping to the available width.
pub fn text_wrapped(
    ui: &mut Ui,
    text: &str,
    size: f32,
    weight: Weight,
    color: Color32,
) -> Response {
    let width = ui.available_width().max(1.0);
    let galley = layout(ui, wrapped(text, size, weight, color, width));
    let (rect, response) = ui.allocate_exact_size(
        vec2(width.min(galley.size().x.max(1.0)), galley.size().y),
        Sense::hover(),
    );
    ui.painter().galley(rect.min, galley, color);
    response
}

/// A label trimmed to one line of the available width.
pub fn text_trimmed(
    ui: &mut Ui,
    text: &str,
    size: f32,
    weight: Weight,
    color: Color32,
) -> Response {
    let width = ui.available_width().max(1.0);
    let galley = layout(ui, trimmed(text, size, weight, color, width, 1));
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter().galley(rect.min, galley, color);
    response
}

/// An icon as its own element (`TextBlock.icon`).
pub fn icon(ui: &mut Ui, glyph: char, size: f32, color: Color32) -> Response {
    let galley = icon_galley(ui, glyph, size, color);
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter().galley(rect.min, galley, color);
    response
}

/// How a surface is filled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fill {
    None,
    Solid(Color32),
    /// `ShareX.Brush.Accent`: the accent's start to end, top-left to bottom-right.
    Accent,
}

/// Fill a rounded rectangle with a diagonal two-colour gradient.
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
    // A hairline in the average colour softens the edge the fan cannot antialias.
    painter.add(Shape::closed_line(
        points,
        Stroke::new(0.6, lerp_color(start, end, 0.5)),
    ));
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

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        l(a.r(), b.r()),
        l(a.g(), b.g()),
        l(a.b(), b.b()),
        l(a.a(), b.a()),
    )
}

/// Paint a surface: fill, then a 1 px border inside the rect.
pub fn paint_surface(
    painter: &Painter,
    colors: &Colors,
    rect: Rect,
    radius: f32,
    fill: Fill,
    border: Option<Color32>,
) {
    match fill {
        Fill::None => {}
        Fill::Solid(color) => {
            painter.rect_filled(rect, radius, color);
        }
        Fill::Accent => gradient_rect(
            painter,
            rect,
            radius,
            colors.accent_start(),
            colors.accent_end(),
        ),
    }
    if let Some(border) = border {
        painter.rect_stroke(rect, radius, Stroke::new(1.0, border), StrokeKind::Inside);
    }
}

/// A button's look, per Avalonia class.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub height: f32,
    pub width: Option<f32>,
    pub min_width: f32,
    pub padding_x: f32,
    pub radius: f32,
    pub fill: Fill,
    pub hover_fill: Fill,
    pub pressed_fill: Fill,
    pub border: Option<Color32>,
    pub hover_border: Option<Color32>,
    pub text: Color32,
    pub hover_text: Color32,
    /// The colour of a label that is the button's own string `Content` (no TextBlock
    /// of the caller's, so the global TextBlock style does not reach it): the button's
    /// Foreground. `None` keeps `text`.
    pub string_text: Option<Color32>,
    pub icon_color: Option<Color32>,
    pub font_size: f32,
    pub weight: Weight,
    pub icon_size: f32,
    pub spacing: f32,
    pub align_left: bool,
    pub opacity: f32,
    pub hover_opacity: f32,
    /// The pressed button shrinks to 97% (`Button:pressed`).
    pub press_scale: bool,
}

impl Look {
    /// The global `Button` style.
    pub fn button(c: &Colors) -> Look {
        Look {
            height: 40.0,
            width: None,
            min_width: 0.0,
            padding_x: 10.0,
            radius: 10.0,
            fill: Fill::Solid(c.control_background()),
            hover_fill: Fill::Solid(c.control_background_hover()),
            pressed_fill: Fill::Solid(c.control_background()),
            border: Some(c.border()),
            hover_border: Some(c.border()),
            // The global TextBlock style colours every label Text.Secondary, inside a
            // button too; only `.active` buttons repaint theirs.
            text: c.text_secondary(),
            hover_text: c.text_secondary(),
            string_text: None,
            icon_color: None,
            font_size: 13.0,
            weight: Weight::Normal,
            icon_size: 15.0,
            spacing: 8.0,
            align_left: false,
            opacity: 1.0,
            hover_opacity: 1.0,
            press_scale: true,
        }
    }

    /// `Button.compact` (dialog footers): at least 96 wide, 14 px padding, still 40 tall.
    pub fn compact(c: &Colors) -> Look {
        Look {
            min_width: 96.0,
            padding_x: 14.0,
            // Dialog footers say `Content="..."`: the label is the button's Text.
            string_text: Some(c.text()),
            ..Look::button(c)
        }
    }

    /// `Button.compact.primary` / `Button.active`: the accent gradient.
    pub fn primary(c: &Colors) -> Look {
        Look {
            fill: Fill::Accent,
            hover_fill: Fill::Accent,
            pressed_fill: Fill::Accent,
            text: c.accent_foreground(),
            hover_text: c.accent_foreground(),
            string_text: Some(c.accent_foreground()),
            ..Look::compact(c)
        }
    }

    /// `Button.chip`: 11 px padding, 6 px corners, 12 px text.
    pub fn chip(c: &Colors) -> Look {
        Look {
            padding_x: 11.0,
            radius: 6.0,
            font_size: 12.0,
            ..Look::button(c)
        }
    }

    /// `Button.strip-button`: 32 tall, transparent until hovered.
    pub fn strip(c: &Colors) -> Look {
        Look {
            height: 32.0,
            padding_x: 9.0,
            radius: 6.0,
            fill: Fill::None,
            hover_fill: Fill::Solid(c.control_background_hover()),
            pressed_fill: Fill::Solid(c.control_background_hover()),
            border: None,
            hover_border: None,
            spacing: 7.0,
            ..Look::button(c)
        }
    }

    /// `Button.quiet-link`: reads as a link, a surface only under the pointer.
    pub fn quiet_link(c: &Colors) -> Look {
        Look {
            height: 32.0,
            padding_x: 10.0,
            radius: 8.0,
            fill: Fill::None,
            hover_fill: Fill::Solid(c.control_background()),
            pressed_fill: Fill::Solid(c.control_background()),
            border: None,
            hover_border: None,
            text: c.text_secondary(),
            hover_text: c.text_secondary(),
            ..Look::button(c)
        }
    }

    /// `Button.dd-icon`, `Button.toast-action` and friends: a square of `size`.
    pub fn icon_button(c: &Colors, size: f32, radius: f32, hover: Color32) -> Look {
        Look {
            height: size,
            width: Some(size),
            padding_x: 0.0,
            radius,
            fill: Fill::None,
            hover_fill: Fill::Solid(hover),
            pressed_fill: Fill::Solid(c.control_background_hover()),
            border: None,
            hover_border: None,
            ..Look::button(c)
        }
    }

    /// `Button.dd-danger`.
    pub fn danger(self, c: &Colors) -> Look {
        Look {
            fill: Fill::Solid(c.cancel_background()),
            hover_fill: Fill::Solid(c.cancel_border()),
            pressed_fill: Fill::Solid(c.cancel_border()),
            border: Some(c.cancel_border()),
            hover_border: Some(c.cancel_border()),
            text: c.task_foreground(),
            hover_text: c.task_foreground(),
            ..self
        }
    }

    pub fn height(mut self, height: f32) -> Look {
        self.height = height;
        self
    }

    pub fn width(mut self, width: f32) -> Look {
        self.width = Some(width);
        self
    }

    pub fn padding(mut self, padding_x: f32) -> Look {
        self.padding_x = padding_x;
        self
    }

    pub fn radius(mut self, radius: f32) -> Look {
        self.radius = radius;
        self
    }

    pub fn font(mut self, size: f32, weight: Weight) -> Look {
        self.font_size = size;
        self.weight = weight;
        self
    }

    pub fn icon(mut self, size: f32, color: Option<Color32>) -> Look {
        self.icon_size = size;
        self.icon_color = color;
        self
    }

    pub fn spacing(mut self, spacing: f32) -> Look {
        self.spacing = spacing;
        self
    }

    pub fn text_color(mut self, color: Color32) -> Look {
        self.text = color;
        self.hover_text = color;
        self
    }
}

/// The content of a button: an icon, a label, or both.
#[derive(Clone, Copy, Debug)]
pub struct Content<'a> {
    pub icon: Option<char>,
    pub label: Option<&'a str>,
}

impl<'a> Content<'a> {
    pub fn icon(glyph: char) -> Content<'a> {
        Content {
            icon: Some(glyph),
            label: None,
        }
    }

    pub fn label(text: &'a str) -> Content<'a> {
        Content {
            icon: None,
            label: Some(text),
        }
    }

    pub fn both(glyph: char, text: &'a str) -> Content<'a> {
        Content {
            icon: Some(glyph),
            label: Some(text),
        }
    }
}

/// The natural width of a button with this look and content.
pub fn button_width(ui: &Ui, look: &Look, content: Content) -> f32 {
    if let Some(width) = look.width {
        return width;
    }
    let mut width = look.padding_x * 2.0;
    if let Some(glyph) = content.icon {
        width += icon_galley(ui, glyph, look.icon_size, look.text).size().x;
    }
    if let Some(label) = content.label {
        if content.icon.is_some() {
            width += look.spacing;
        }
        width += layout(ui, job(label, look.font_size, look.weight, look.text))
            .size()
            .x;
    }
    width.max(look.min_width).ceil()
}

/// A button. Disabled buttons draw at 35% and do not react.
pub fn button(
    ui: &mut Ui,
    colors: &Colors,
    look: &Look,
    content: Content,
    enabled: bool,
) -> Response {
    let width = button_width(ui, look, content);
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, look.height),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    paint_button(ui, colors, look, content, enabled, rect, &response);
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// A button filling `rect` exactly.
pub fn button_at(
    ui: &mut Ui,
    colors: &Colors,
    look: &Look,
    content: Content,
    enabled: bool,
    rect: Rect,
) -> Response {
    let response = ui.allocate_rect(
        rect,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    paint_button(ui, colors, look, content, enabled, rect, &response);
    response
}

pub fn paint_button(
    ui: &Ui,
    colors: &Colors,
    look: &Look,
    content: Content,
    enabled: bool,
    rect: Rect,
    response: &Response,
) {
    let hovered = enabled && response.hovered();
    let pressed = enabled && response.is_pointer_button_down_on();
    let (fill, border, text_color) = if pressed {
        (look.pressed_fill, look.hover_border, look.hover_text)
    } else if hovered {
        (look.hover_fill, look.hover_border, look.hover_text)
    } else {
        (look.fill, look.border, look.text)
    };
    let opacity = if !enabled {
        0.35
    } else if hovered || pressed {
        look.hover_opacity
    } else {
        look.opacity
    };
    let rect = if pressed && look.press_scale {
        Rect::from_center_size(rect.center(), rect.size() * 0.97)
    } else {
        rect
    };

    let painter = ui.painter().clone();
    let painter = painter.with_clip_rect(painter.clip_rect());
    let mut layer = painter.clone();
    layer.set_opacity(opacity);
    paint_surface(&layer, colors, rect, look.radius, fill, border);

    let icon = content.icon.map(|glyph| {
        icon_galley(
            ui,
            glyph,
            look.icon_size,
            look.icon_color.unwrap_or(text_color),
        )
    });
    let label_color = match look.string_text {
        Some(color) if content.icon.is_none() => color,
        _ => text_color,
    };
    let label = content.label.map(|text| {
        layout(
            ui,
            trimmed(
                text,
                look.font_size,
                look.weight,
                label_color,
                (rect.width() - look.padding_x * 2.0).max(1.0),
                1,
            ),
        )
    });
    let mut total = 0.0;
    if let Some(icon) = &icon {
        total += icon.size().x;
    }
    if let Some(label) = &label {
        if icon.is_some() {
            total += look.spacing;
        }
        total += label.size().x;
    }
    let mut x = if look.align_left {
        rect.left() + look.padding_x
    } else {
        rect.center().x - total / 2.0
    };
    let cy = rect.center().y;
    if let Some(icon) = icon {
        let color = look.icon_color.unwrap_or(text_color);
        let size = icon.size();
        layer.galley(pos2(x, cy - size.y / 2.0), icon, color);
        x += size.x + look.spacing;
    }
    if let Some(label) = label {
        let size = label.size();
        layer.galley(pos2(x, cy - size.y / 2.0), label, text_color);
    }
}

/// `Border.card` and its relatives: a filled, bordered, rounded frame.
pub fn card(fill: Color32, border: Color32, radius: u8, padding: egui::Margin) -> egui::Frame {
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(radius))
        .inner_margin(padding)
}

/// A one-pixel horizontal rule spanning the available width.
pub fn rule(ui: &mut Ui, color: Color32, margin_x: f32, margin_y: f32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 1.0 + margin_y * 2.0), Sense::hover());
    ui.painter().hline(
        (rect.left() + margin_x)..=(rect.right() - margin_x),
        rect.center().y,
        Stroke::new(1.0, color),
    );
}

/// The Fluent progress bar: a thin track with an accent fill, or a moving segment.
pub fn progress(
    ui: &mut Ui,
    colors: &Colors,
    width: f32,
    height: f32,
    fraction: Option<f32>,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    paint_progress(ui, colors, rect, fraction);
    response
}

pub fn paint_progress(ui: &Ui, colors: &Colors, rect: Rect, fraction: Option<f32>) {
    let painter = ui.painter();
    let radius = rect.height() / 2.0;
    let track = if colors.dark {
        Color32::from_white_alpha(0x33)
    } else {
        Color32::from_black_alpha(0x2B)
    };
    match fraction {
        Some(fraction) => {
            painter.rect_filled(rect, radius, track);
            let filled = Rect::from_min_size(
                rect.min,
                vec2(rect.width() * fraction.clamp(0.0, 1.0), rect.height()),
            );
            if filled.width() > 0.0 {
                painter.rect_filled(filled, radius, colors.accent_start());
            }
        }
        None => {
            // Indeterminate: a segment sweeping across, like Fluent's.
            let time = ui.input(|i| i.time) as f32;
            let t = (time % 1.6) / 1.6;
            let segment = rect.width() * 0.4;
            let left = rect.left() - segment + (rect.width() + segment) * t;
            let bar = Rect::from_min_max(
                pos2(left.max(rect.left()), rect.top()),
                pos2((left + segment).min(rect.right()), rect.bottom()),
            );
            if bar.width() > 0.0 {
                painter.rect_filled(bar, radius, colors.accent_start());
            }
            ui.ctx().request_repaint();
        }
    }
}

/// A checkbox (`ControlStyles` CheckBox): 20 px box, accent when checked.
pub fn checkbox(
    ui: &mut Ui,
    colors: &Colors,
    checked: &mut bool,
    label: Option<&str>,
    enabled: bool,
) -> Response {
    checkbox_weighted(ui, colors, checked, label, enabled, Weight::SemiBold)
}

/// A checkbox whose label has its own weight (`FontWeight="Normal"` in a window's XAML).
pub fn checkbox_weighted(
    ui: &mut Ui,
    colors: &Colors,
    checked: &mut bool,
    label: Option<&str>,
    enabled: bool,
    weight: Weight,
) -> Response {
    let box_size = 20.0;
    let label_galley = label.map(|text| {
        layout(
            ui,
            job(
                text,
                13.0,
                weight,
                if enabled {
                    colors.text()
                } else {
                    colors.text_secondary()
                },
            ),
        )
    });
    let width = box_size
        + label_galley
            .as_ref()
            .map(|g| 8.0 + g.size().x)
            .unwrap_or(0.0);
    let height = box_size
        .max(label_galley.as_ref().map(|g| g.size().y).unwrap_or(0.0))
        .max(32.0);
    let (rect, mut response) = ui.allocate_exact_size(
        vec2(width, height),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if enabled && response.clicked() {
        *checked = !*checked;
        response.mark_changed();
    }
    let box_rect = Rect::from_min_size(
        pos2(rect.left(), rect.center().y - box_size / 2.0),
        Vec2::splat(box_size),
    );
    let painter = ui.painter();
    if *checked {
        gradient_rect(
            painter,
            box_rect,
            4.0,
            colors.accent_start(),
            colors.accent_end(),
        );
        paint_icon(
            ui,
            painter,
            box_rect.center(),
            modlight_core::icons::CHECK,
            14.0,
            colors.accent_foreground(),
        );
    } else {
        let fill = if !enabled {
            colors.control_background_disabled()
        } else if response.hovered() {
            colors.control_background_hover()
        } else {
            colors.control_background()
        };
        painter.rect_filled(box_rect, 4.0, fill);
        painter.rect_stroke(
            box_rect,
            4.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
    }
    if let Some(galley) = label_galley {
        let color = if enabled {
            colors.text()
        } else {
            colors.text_secondary()
        };
        paint_text_left(
            painter,
            box_rect.right() + 8.0,
            rect.center().y,
            galley,
            color,
        );
    }
    response
}

/// The text box (`ControlStyles` TextBox): control fill, 1 px border, 8 px corners,
/// a 2 px focus-coloured border while typing. Returns the inner `TextEdit` response.
pub struct Field<'a> {
    pub text: &'a mut String,
    pub hint: &'a str,
    pub height: f32,
    pub width: Option<f32>,
    pub password: bool,
    pub enabled: bool,
    pub font_size: f32,
    pub padding_x: f32,
    pub char_limit: usize,
    pub id: Option<egui::Id>,
}

impl<'a> Field<'a> {
    pub fn new(text: &'a mut String) -> Field<'a> {
        Field {
            text,
            hint: "",
            height: 40.0,
            width: None,
            password: false,
            enabled: true,
            font_size: 13.0,
            padding_x: 8.0,
            char_limit: usize::MAX,
            id: None,
        }
    }

    pub fn show(self, ui: &mut Ui, colors: &Colors) -> Response {
        let width = self.width.unwrap_or_else(|| ui.available_width());
        let (rect, _) = ui.allocate_exact_size(vec2(width, self.height), Sense::hover());
        let id = self.id.unwrap_or_else(|| ui.next_auto_id());
        let focused = ui.memory(|m| m.has_focus(id));
        let hovered = ui.rect_contains_pointer(rect);
        let fill = if !self.enabled {
            colors.control_background_disabled()
        } else if hovered && !focused {
            colors.control_background_hover()
        } else {
            colors.control_background()
        };
        let painter = ui.painter();
        painter.rect_filled(rect, 8.0, fill);
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
        let inner = rect.shrink2(vec2(self.padding_x, 0.0));
        let text_color = if self.enabled {
            colors.text()
        } else {
            colors.text_secondary()
        };
        let hint = egui::RichText::new(self.hint)
            .font(FontId::new(self.font_size, FontFamily::Proportional))
            .color(colors.text_secondary().gamma_multiply(0.75));
        let edit = egui::TextEdit::singleline(self.text)
            .id(id)
            .frame(egui::Frame::NONE)
            .password(self.password)
            .char_limit(self.char_limit)
            .font(FontId::new(self.font_size, FontFamily::Proportional))
            .text_color(text_color)
            .hint_text(hint)
            .vertical_align(egui::Align::Center)
            .margin(egui::Margin::ZERO)
            .desired_width(inner.width());
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.add_enabled(self.enabled, edit)
    }
}

/// Where a popup or a tooltip is drawn: a rounded, bordered surface on the popup colour.
pub fn popup_frame(colors: &Colors) -> egui::Frame {
    egui::Frame::new()
        .fill(colors.background_menu())
        .stroke(Stroke::new(1.0, colors.border()))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(6, 4))
        .shadow(egui::Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha(0x40),
        })
}

/// A painted text anchored in a rect.
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
    let galley = layout(
        ui,
        trimmed(text, size, weight, color, rect.width().max(1.0), 1),
    );
    let pos = align.align_size_within_rect(galley.size(), rect).min;
    let out = Rect::from_min_size(pos, galley.size());
    painter.galley(pos, galley, color);
    out
}

/// A drop-down of `Button` look: the current choice and a chevron; a menu of options.
pub fn combo<T: Clone + PartialEq>(
    ui: &mut Ui,
    colors: &Colors,
    id: &str,
    width: f32,
    current: &T,
    options: &[(T, String)],
) -> Option<T> {
    combo_sized(ui, colors, id, width, 32.0, current, options)
}

/// [`combo`] at a given height (`MinHeight` of the window's own ComboBox).
pub fn combo_sized<T: Clone + PartialEq>(
    ui: &mut Ui,
    colors: &Colors,
    id: &str,
    width: f32,
    height: f32,
    current: &T,
    options: &[(T, String)],
) -> Option<T> {
    combo_rounded(ui, colors, id, width, height, 6.0, current, options)
}

/// [`combo_sized`] with its own corner radius (the global ComboBox's is 8).
#[allow(clippy::too_many_arguments)]
pub fn combo_rounded<T: Clone + PartialEq>(
    ui: &mut Ui,
    colors: &Colors,
    id: &str,
    width: f32,
    height: f32,
    radius: f32,
    current: &T,
    options: &[(T, String)],
) -> Option<T> {
    let label = options
        .iter()
        .find(|(v, _)| v == current)
        .map(|(_, l)| l.clone())
        .unwrap_or_default();
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        radius,
        if response.hovered() {
            colors.control_background_hover()
        } else {
            colors.control_background()
        },
    );
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    let text = layout(
        ui,
        trimmed(&label, 13.0, Weight::Normal, colors.text(), width - 40.0, 1),
    );
    painter.galley(
        pos2(rect.left() + 8.0, rect.center().y - text.size().y / 2.0),
        text,
        colors.text(),
    );
    paint_icon(
        ui,
        painter,
        pos2(rect.right() - 16.0, rect.center().y),
        modlight_core::icons::CHEVRON_DOWN,
        13.0,
        colors.text_secondary(),
    );
    let mut picked = None;
    egui::Popup::menu(&response)
        .id(egui::Id::new(("settings-combo", id)))
        .frame(crate::menus::frame(colors))
        .align(egui::RectAlign::BOTTOM_START)
        .gap(2.0)
        .show(|ui| {
            ui.set_min_width(width - 12.0);
            for (value, label) in options {
                let selected = value == current;
                let (r, row) = ui.allocate_exact_size(vec2(width - 12.0, 30.0), Sense::click());
                if selected || row.hovered() {
                    ui.painter()
                        .rect_filled(r, 4.0, colors.control_background_hover());
                }
                let g = layout(
                    ui,
                    trimmed(
                        label,
                        13.0,
                        Weight::Normal,
                        colors.text(),
                        r.width() - 16.0,
                        1,
                    ),
                );
                ui.painter().galley(
                    pos2(r.left() + 8.0, r.center().y - g.size().y / 2.0),
                    g,
                    colors.text(),
                );
                if row.clicked() {
                    picked = Some(value.clone());
                    ui.close();
                }
            }
        });
    picked
}

/// `ShareX.Brush.Checkered.Small`: 8 px tiles of the skin's two checker colours, the
/// background under pictures that may be transparent. Drawn as one repeating texture.
pub fn paint_checkered(ui: &Ui, colors: &Colors, rect: Rect, radius: f32) {
    let (a, b) = (colors.checker_a(), colors.checker_b());
    let id = egui::Id::new(("checkered-small", a.to_array(), b.to_array()));
    let texture = ui
        .ctx()
        .memory_mut(|m| m.data.get_temp::<egui::TextureHandle>(id));
    let texture = match texture {
        Some(texture) => texture,
        None => {
            let mut image = egui::ColorImage::filled([8, 8], a);
            for y in 0..8 {
                for x in 0..8 {
                    if (x < 4) == (y < 4) {
                        image.pixels[y * 8 + x] = b;
                    }
                }
            }
            let options = egui::TextureOptions {
                wrap_mode: egui::TextureWrapMode::Repeat,
                ..egui::TextureOptions::NEAREST
            };
            let texture = ui.ctx().load_texture("checkered-small", image, options);
            ui.ctx()
                .memory_mut(|m| m.data.insert_temp(id, texture.clone()));
            texture
        }
    };
    let uv = Rect::from_min_max(
        pos2(0.0, 0.0),
        pos2(rect.width() / 8.0, rect.height() / 8.0),
    );
    if radius > 0.0 {
        // Round the corners by painting the plain colour first and the tiles inset.
        ui.painter().rect_filled(rect, radius, a);
        let inner = rect.shrink(radius * 0.3);
        let uv = Rect::from_min_max(
            pos2(0.0, 0.0),
            pos2(inner.width() / 8.0, inner.height() / 8.0),
        );
        ui.painter().image(texture.id(), inner, uv, Color32::WHITE);
    } else {
        ui.painter().image(texture.id(), rect, uv, Color32::WHITE);
    }
}
