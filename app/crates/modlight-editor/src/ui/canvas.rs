//! `EditorView`'s canvas: the scroll viewer with the zoomed frame (checkered border,
//! the preview frame, the image, the spotlight layer, the annotations), the overlay
//! canvas (selection handles, hover outlines, crop chrome, the cut-out preview, the
//! text box) and the pointer input of `EditorInputController`, `EditorZoomController`
//! and `EditorSelectionController`.
//!
//! The image is a GPU texture (pre-scaled below 100 %, as Skia's mipmaps would); the
//! annotations are rasterised with tiny-skia for the visible part of the frame at the
//! window's pixel density, so they stay vectors at any zoom. While the Background panel
//! is open the whole frame (background, card, shadow, rounded corners) is composited on
//! the CPU. The chrome is drawn with egui in the overlay's coordinates (image units
//! scaled by the zoom, like upstream's `OverlayCanvas` with its `ScaleTransform`).

use std::sync::Arc;

use egui::{
    Align2, Color32, CursorIcon, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use image::RgbaImage;
use tiny_skia::{Pixmap, Transform};

use super::style::{self, Colors, Weight};
use crate::editor::input::{self, Button, CropHandle, HandleTag, Modifiers};
use crate::editor::{Editor, TextEditing};
use crate::frame;
use crate::geom::Point;
use crate::model::{self, Annotation, EditorTool, Kind};

/// The minimum time between two Cmd+wheel zoom steps (`ZoomThrottleMilliseconds`).
const ZOOM_THROTTLE: f64 = 0.150;
/// `CanvasHost` margin.
pub const HOST_MARGIN: f32 = 8.0;

/// Where the frame is on screen this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    /// The scroll viewer's viewport.
    pub host: Rect,
    /// The frame's top-left corner in points.
    pub origin: Pos2,
    /// Points per image pixel (`EffectiveZoom`).
    pub z: f32,
    /// Physical pixels per point.
    pub ppp: f32,
    /// The image's top-left inside the frame, in image pixels.
    pub image_offset: Vec2,
    /// The frame's size in image pixels.
    pub frame: Vec2,
}

impl Geometry {
    pub fn to_image(&self, p: Pos2) -> Point {
        let x = (p.x - self.origin.x) / self.z - self.image_offset.x;
        let y = (p.y - self.origin.y) / self.z - self.image_offset.y;
        Point::new(x, y)
    }

    pub fn to_screen(&self, p: Point) -> Pos2 {
        pos2(
            self.origin.x + (p.x + self.image_offset.x) * self.z,
            self.origin.y + (p.y + self.image_offset.y) * self.z,
        )
    }

    /// The frame on screen.
    pub fn frame_rect(&self) -> Rect {
        Rect::from_min_size(self.origin, self.frame * self.z)
    }

    pub fn rect_to_screen(&self, r: &crate::geom::Rect) -> Rect {
        Rect::from_min_max(
            self.to_screen(Point::new(r.left, r.top)),
            self.to_screen(Point::new(r.right, r.bottom)),
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
struct OverlayKey {
    revision: u64,
    zoom_bits: u32,
    rect: [i32; 4],
    origin: [i32; 2],
    background: bool,
}

struct ImageTexture {
    key: (usize, u32, u32, u32),
    texture: egui::TextureHandle,
}

/// The canvas's view state (scroll offset, textures, the pointer being held).
#[derive(Default)]
pub struct Canvas {
    image: Option<ImageTexture>,
    image_pixmap: Option<(usize, Arc<Pixmap>)>,
    overlay: Option<(OverlayKey, egui::TextureHandle)>,
    /// The scroll viewer's offset in points.
    pub scroll: Vec2,
    panning: Option<(Pos2, Vec2)>,
    last_zoom_time: Option<f64>,
    zoom_anchor: Option<(Pos2, Vec2)>,
    pinch: f32,
    pressed: Option<Button>,
    last_press: Option<(f64, Pos2, Button, u32)>,
    /// The geometry of the last frame (tests, the host's coordinate questions).
    pub geometry: Option<Geometry>,
    text_box_focus: Option<crate::model::AnnotationId>,
    text_box_guard: bool,
    /// The overlay was re-rendered this frame (tests).
    pub overlay_renders: u64,
}

impl Canvas {
    fn geometry(&mut self, editor: &Editor, host: Rect, ppp: f32) -> Geometry {
        let state = editor.background_state();
        let layout = frame::layout(&state, editor.core.canvas_size());
        let z = editor.effective_zoom() as f32;
        let frame = vec2(layout.width.max(0.0), layout.height.max(0.0));
        let extent = frame * z;
        let max = vec2(
            (extent.x - host.width()).max(0.0),
            (extent.y - host.height()).max(0.0),
        );
        self.scroll = vec2(
            self.scroll.x.clamp(0.0, max.x),
            self.scroll.y.clamp(0.0, max.y),
        );
        let x = if extent.x <= host.width() {
            host.left() + (host.width() - extent.x) / 2.0
        } else {
            host.left() - self.scroll.x
        };
        let y = if extent.y <= host.height() {
            host.top() + (host.height() - extent.y) / 2.0
        } else {
            host.top() - self.scroll.y
        };
        // Whole physical pixels, so the image and the overlay meet the screen's grid.
        let origin = pos2((x * ppp).round() / ppp, (y * ppp).round() / ppp);
        let t = layout.image_transform();
        Geometry {
            host,
            origin,
            z,
            ppp,
            image_offset: vec2(t.tx, t.ty),
            frame,
        }
    }

    fn max_scroll(&self, g: &Geometry) -> Vec2 {
        let extent = g.frame * g.z;
        vec2(
            (extent.x - g.host.width()).max(0.0),
            (extent.y - g.host.height()).max(0.0),
        )
    }

    /// Shows the canvas in `area` (the column between the sidebars). `input_blocked`
    /// is set while a modal or the effect browser covers it.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        editor: &mut Editor,
        colors: &Colors,
        area: Rect,
        input_blocked: bool,
    ) -> Geometry {
        let ppp = ui.ctx().pixels_per_point();
        if (editor.dpi_scale as f32 - ppp).abs() > f32::EPSILON {
            editor.dpi_scale = ppp as f64;
            editor.invalidate();
        }
        let host = area.shrink(HOST_MARGIN);
        if editor.zoom_to_fit_requested && editor.has_image() {
            editor.zoom_to_fit_requested = false;
            editor.zoom_to_fit((host.width() as f64, host.height() as f64));
        }
        let mut g = self.geometry(editor, host, ppp);
        if let Some((anchor, logical)) = self.zoom_anchor.take() {
            let target = logical * g.z - (anchor - host.min);
            let max = self.max_scroll(&g);
            self.scroll = vec2(target.x.clamp(0.0, max.x), target.y.clamp(0.0, max.y));
            g = self.geometry(editor, host, ppp);
        }
        if editor.input.center_on_zoom {
            editor.input.center_on_zoom = false;
            self.scroll = self.max_scroll(&g) / 2.0;
            g = self.geometry(editor, host, ppp);
        }

        let response = ui.interact(host, ui.id().with("editor-canvas"), Sense::click_and_drag());
        if !input_blocked && editor.has_image() {
            self.handle_events(ui, editor, &g, response.hovered());
            g = self.geometry(editor, host, ppp);
        } else if input_blocked {
            self.pressed = None;
            self.panning = None;
        }

        // Visible centre for inserts (`GetVisibleCanvasCenter`).
        if editor.has_image() {
            let visible = host.intersect(g.frame_rect());
            if visible.is_positive() {
                editor.set_visible_center(Some(g.to_image(visible.center())));
            }
        }

        let painter = ui.painter().with_clip_rect(area);
        if editor.has_image() {
            self.paint_frame(ui, &painter, editor, colors, &g);
            paint_chrome(ui, &painter, editor, &g);
            self.crop_button(ui, editor, colors, &g);
            self.text_box(ui, editor, &g);
            self.update_cursor(ui, editor, &g, response.hovered());
        }
        self.geometry = Some(g);
        g
    }

    // ------------------------------------------------------------------ input

    fn handle_events(&mut self, ui: &Ui, editor: &mut Editor, g: &Geometry, hovered: bool) {
        let (events, time, modifiers) = ui.input(|i| (i.events.clone(), i.time, i.modifiers));
        let mut g = *g;
        for event in events {
            match event {
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                } => {
                    let Some(button) = map_button(button) else {
                        continue;
                    };
                    let m = Modifiers::from_egui(modifiers);
                    if pressed {
                        if !hovered || !g.host.contains(pos) || self.pressed.is_some() {
                            continue;
                        }
                        if button == Button::Middle {
                            // `OnScrollViewerPointerPressed`: the middle button pans.
                            self.panning = Some((pos, self.scroll));
                            continue;
                        }
                        let clicks = self.click_count(time, pos, button);
                        self.pressed = Some(button);
                        // A press on the canvas takes the focus from an open text box
                        // (`LostFocus` commits it) before the tool sees the press.
                        if editor.text_editing.is_some() && button == Button::Left {
                            input::commit_text_edit(editor);
                            self.text_box_focus = None;
                        }
                        input::pointer_pressed(editor, g.to_image(pos), button, m, clicks);
                        // A box this press opened keeps the focus when it is released.
                        self.text_box_guard = editor.text_editing.is_some();
                    } else {
                        if button == Button::Middle && self.panning.is_some() {
                            self.panning = None;
                            continue;
                        }
                        if self.pressed == Some(button) {
                            self.pressed = None;
                            input::pointer_released(editor, g.to_image(pos), button, m);
                        }
                    }
                }
                egui::Event::PointerMoved(pos) => {
                    if let Some((start, origin)) = self.panning {
                        let delta = pos - start;
                        let max = self.max_scroll(&g);
                        self.scroll = vec2(
                            (origin.x - delta.x).clamp(0.0, max.x),
                            (origin.y - delta.y).clamp(0.0, max.y),
                        );
                        g = self.geometry(editor, g.host, g.ppp);
                        continue;
                    }
                    if self.pressed.is_some() || (hovered && g.host.contains(pos)) {
                        let right = self.pressed == Some(Button::Right);
                        input::pointer_moved(
                            editor,
                            g.to_image(pos),
                            right,
                            Modifiers::from_egui(modifiers),
                        );
                    }
                }
                egui::Event::PointerGone => {
                    if let Some(button) = self.pressed.take()
                        && let Some(pos) = ui.input(|i| i.pointer.latest_pos())
                    {
                        input::pointer_released(
                            editor,
                            g.to_image(pos),
                            button,
                            Modifiers::from_egui(modifiers),
                        );
                    }
                }
                egui::Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                    ..
                } => {
                    if !hovered {
                        continue;
                    }
                    let pointer = ui
                        .input(|i| i.pointer.latest_pos())
                        .unwrap_or(g.host.center());
                    if modifiers.command || modifiers.ctrl {
                        // `OnPreviewPointerWheelChanged`: one zoom level per notch.
                        let direction = delta.y.signum() as i32;
                        if direction != 0 {
                            self.step_zoom(editor, &g, pointer, direction, time);
                            g = self.geometry(editor, g.host, g.ppp);
                        }
                        continue;
                    }
                    let factor = match unit {
                        egui::MouseWheelUnit::Point => 1.0,
                        egui::MouseWheelUnit::Line => 50.0,
                        egui::MouseWheelUnit::Page => g.host.height(),
                    };
                    let mut d = delta * factor;
                    if modifiers.shift && d.x == 0.0 {
                        d = vec2(d.y, 0.0);
                    }
                    let max = self.max_scroll(&g);
                    self.scroll = vec2(
                        (self.scroll.x - d.x).clamp(0.0, max.x),
                        (self.scroll.y - d.y).clamp(0.0, max.y),
                    );
                    g = self.geometry(editor, g.host, g.ppp);
                }
                egui::Event::Zoom(factor) => {
                    // A pinch: one zoom level per 15 % of pinch, throttled like the wheel.
                    if !hovered {
                        continue;
                    }
                    self.pinch += factor.ln();
                    if self.pinch.abs() >= 0.14 {
                        let direction = self.pinch.signum() as i32;
                        self.pinch = 0.0;
                        let pointer = ui
                            .input(|i| i.pointer.latest_pos())
                            .unwrap_or(g.host.center());
                        self.step_zoom(editor, &g, pointer, direction, time);
                        g = self.geometry(editor, g.host, g.ppp);
                    }
                }
                _ => {}
            }
        }
    }

    fn click_count(&mut self, time: f64, pos: Pos2, button: Button) -> u32 {
        let count = match self.last_press {
            Some((t, p, b, n)) if b == button && time - t < 0.5 && p.distance(pos) < 6.0 => n + 1,
            _ => 1,
        };
        self.last_press = Some((time, pos, button, count));
        count
    }

    /// One `ZoomLevels` step around the pointer (`AdjustZoomToAnchor`).
    fn step_zoom(
        &mut self,
        editor: &mut Editor,
        g: &Geometry,
        pointer: Pos2,
        direction: i32,
        time: f64,
    ) {
        if let Some(last) = self.last_zoom_time
            && time - last < ZOOM_THROTTLE
        {
            return;
        }
        let extent = g.frame * g.z;
        let mut offset = self.scroll;
        if extent.x <= g.host.width() {
            offset.x = 0.0;
        }
        if extent.y <= g.host.height() {
            offset.y = 0.0;
        }
        let anchor = pointer - g.host.min;
        let logical = (offset + anchor) / g.z;
        if editor.zoom_step(direction) {
            self.last_zoom_time = Some(time);
            let z = editor.effective_zoom() as f32;
            let target = logical * z - anchor;
            let ng = self.geometry(editor, g.host, g.ppp);
            let max = self.max_scroll(&ng);
            self.scroll = vec2(target.x.clamp(0.0, max.x), target.y.clamp(0.0, max.y));
        }
    }

    /// Zoom from the zoom picker, kept around the viewport's centre.
    pub fn zoom_around_center(&mut self, editor: &mut Editor, zoom: f64) {
        let Some(g) = self.geometry else {
            editor.set_zoom(zoom);
            return;
        };
        let anchor = g.host.center();
        let extent = g.frame * g.z;
        let mut offset = self.scroll;
        if extent.x <= g.host.width() {
            offset.x = 0.0;
        }
        if extent.y <= g.host.height() {
            offset.y = 0.0;
        }
        let logical = (offset + (anchor - g.host.min)) / g.z;
        editor.set_zoom(zoom);
        self.zoom_anchor = Some((anchor, logical));
    }

    // ------------------------------------------------------------------ painting

    fn paint_frame(
        &mut self,
        ui: &Ui,
        painter: &egui::Painter,
        editor: &mut Editor,
        colors: &Colors,
        g: &Geometry,
    ) {
        let frame_rect = g.frame_rect();
        // The checkered `Border` under the preview frame, 15-unit cells and a 1-unit
        // border, both scaled with the zoom.
        style::paint_checkered(ui, painter, colors, frame_rect, 15.0 * g.z, frame_rect.min);
        painter.rect_stroke(
            frame_rect,
            0.0,
            Stroke::new(g.z.max(0.5), colors.border()),
            StrokeKind::Inside,
        );
        let background = editor.are_background_effects_active();
        if !background {
            self.paint_image(ui, painter, editor, g);
        }
        self.paint_overlay(ui, painter, editor, g, background);
    }

    fn paint_image(&mut self, ui: &Ui, painter: &egui::Painter, editor: &Editor, g: &Geometry) {
        let Some(image) = editor.display_image() else {
            return;
        };
        let zoom_px = g.z * g.ppp;
        let scaled = zoom_px < 0.999;
        let (tw, th) = if scaled {
            (
                ((image.width() as f32 * zoom_px).round() as u32).max(1),
                ((image.height() as f32 * zoom_px).round() as u32).max(1),
            )
        } else {
            (image.width(), image.height())
        };
        let key = (
            Arc::as_ptr(&image) as usize,
            tw,
            th,
            editor.core.generation() as u32,
        );
        if self.image.as_ref().map(|t| t.key) != Some(key) {
            let pixels: RgbaImage = if scaled {
                image::imageops::resize(
                    image.as_ref(),
                    tw,
                    th,
                    image::imageops::FilterType::Triangle,
                )
            } else {
                image.as_ref().clone()
            };
            let color = egui::ColorImage::from_rgba_unmultiplied(
                [pixels.width() as usize, pixels.height() as usize],
                pixels.as_raw(),
            );
            let options = egui::TextureOptions {
                magnification: egui::TextureFilter::Linear,
                minification: egui::TextureFilter::Linear,
                wrap_mode: egui::TextureWrapMode::ClampToEdge,
                mipmap_mode: None,
            };
            match &mut self.image {
                Some(t) => {
                    t.texture.set(color, options);
                    t.key = key;
                }
                None => {
                    let texture = ui.ctx().load_texture("editor-image", color, options);
                    self.image = Some(ImageTexture { key, texture });
                }
            }
        }
        if let Some(t) = &self.image {
            let rect = Rect::from_min_size(
                g.to_screen(Point::new(0.0, 0.0)),
                vec2(image.width() as f32, image.height() as f32) * g.z,
            );
            painter.image(
                t.texture.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
    }

    fn source_pixmap(&mut self, editor: &Editor) -> Option<Arc<Pixmap>> {
        let image = editor.display_image()?;
        let key =
            Arc::as_ptr(&image) as usize ^ (editor.core.generation() as usize).rotate_left(17);
        if let Some((k, p)) = &self.image_pixmap
            && *k == key
        {
            return Some(p.clone());
        }
        let p = Arc::new(crate::raster::to_pixmap(&image));
        self.image_pixmap = Some((key, p.clone()));
        Some(p)
    }

    fn paint_overlay(
        &mut self,
        ui: &Ui,
        painter: &egui::Painter,
        editor: &mut Editor,
        g: &Geometry,
        background: bool,
    ) {
        let frame_rect = g.frame_rect();
        let visible = frame_rect.intersect(g.host.expand(HOST_MARGIN));
        if !visible.is_positive() {
            return;
        }
        let has_content =
            background || !editor.core.annotations().is_empty() || editor.input.pending.is_some();
        if !has_content {
            self.overlay = None;
            return;
        }
        let px = [
            (visible.left() * g.ppp).floor() as i32,
            (visible.top() * g.ppp).floor() as i32,
            (visible.right() * g.ppp).ceil() as i32,
            (visible.bottom() * g.ppp).ceil() as i32,
        ];
        let origin_px = [
            (g.origin.x * g.ppp).round() as i32,
            (g.origin.y * g.ppp).round() as i32,
        ];
        let key = OverlayKey {
            revision: editor.revision,
            zoom_bits: (g.z * g.ppp).to_bits(),
            rect: px,
            origin: origin_px,
            background,
        };
        let stale = self
            .overlay
            .as_ref()
            .map(|(k, _)| k != &key)
            .unwrap_or(true);
        if stale {
            let w = (px[2] - px[0]).max(1) as u32;
            let h = (px[3] - px[1]).max(1) as u32;
            let Some(mut target) = Pixmap::new(w, h) else {
                return;
            };
            let zoom_px = g.z * g.ppp;
            let transform = Transform::from_row(
                zoom_px,
                0.0,
                0.0,
                zoom_px,
                (origin_px[0] - px[0]) as f32,
                (origin_px[1] - px[1]) as f32,
            );
            let state = editor.background_state();
            let layout = frame::layout(&state, editor.core.canvas_size());
            let blur = editor.spotlight_blur_source();
            let pixmap = if background {
                self.source_pixmap(editor)
            } else {
                None
            };
            let top_left = if background {
                editor.display_image().and_then(|i| {
                    i.get_pixel_checked(0, 0)
                        .map(|p| crate::color::Color::rgba(p.0[0], p.0[1], p.0[2], p.0[3]))
                })
            } else {
                None
            };
            let (annotations, hidden) = editor.scene_annotations();
            frame::draw(
                &mut target,
                transform,
                &state,
                &layout,
                &frame::Content {
                    image: pixmap.as_deref(),
                    canvas_size: editor.core.canvas_size(),
                    annotations: &annotations,
                    hidden,
                    spotlight_blur: blur.as_deref(),
                    draw_image: background,
                },
                top_left,
            );
            let color =
                egui::ColorImage::from_rgba_premultiplied([w as usize, h as usize], target.data());
            match &mut self.overlay {
                Some((k, t)) => {
                    t.set(color, egui::TextureOptions::NEAREST);
                    *k = key.clone();
                }
                None => {
                    let t = ui.ctx().load_texture(
                        "editor-overlay",
                        color,
                        egui::TextureOptions::NEAREST,
                    );
                    self.overlay = Some((key.clone(), t));
                }
            }
            self.overlay_renders += 1;
        }
        if let Some((_, t)) = &self.overlay {
            let rect = Rect::from_min_max(
                pos2(px[0] as f32 / g.ppp, px[1] as f32 / g.ppp),
                pos2(px[2] as f32 / g.ppp, px[3] as f32 / g.ppp),
            );
            painter.image(
                t.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
    }

    /// The crop confirmation button (`ShowCropConfirmButton`), 80 by 40 overlay units.
    fn crop_button(&mut self, ui: &mut Ui, editor: &mut Editor, colors: &Colors, g: &Geometry) {
        if editor.input.crop.is_dragging() {
            return;
        }
        let Some(r) = input::crop_button_rect(editor, (80.0, 40.0)) else {
            return;
        };
        let rect = g.rect_to_screen(&r);
        let response = ui.interact(rect, ui.id().with("crop-confirm"), Sense::click());
        let paint = style::paint_button(
            ui,
            colors,
            &response,
            rect,
            style::ButtonKind::Standard,
            10.0 * g.z,
            false,
            true,
        );
        let text = modlight_core::l10n::t(modlight_core::l10n::editor::EditorView_Crop);
        let galley = style::galley(
            ui,
            text,
            style::FONT_SIZE * g.z,
            Weight::Normal,
            paint.foreground,
        );
        let pos = Align2::CENTER_CENTER
            .align_size_within_rect(galley.size(), paint.rect)
            .min;
        ui.painter().galley(pos, galley, paint.foreground);
        if response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if response.clicked() {
            input::confirm_crop(editor);
        }
    }

    /// The text box over a text annotation or a balloon being written.
    fn text_box(&mut self, ui: &mut Ui, editor: &mut Editor, g: &Geometry) {
        let Some(edit) = editor.text_editing.clone() else {
            self.text_box_focus = None;
            return;
        };
        let Some(a) = editor.annotation(edit.id).cloned() else {
            editor.text_editing = None;
            return;
        };
        let Some(data) = a.text_data().cloned() else {
            return;
        };
        let bounds = a.bounds();
        let family = if data.bold {
            style::semibold()
        } else {
            egui::FontFamily::Proportional
        };
        let font = egui::FontId::new((data.font_size * g.z).max(1.0), family.clone());
        let (rect, padding, fill, border) = if edit.balloon {
            let fill = crate::color::Color::parse(&a.fill_color)
                .filter(|c| c.a > 0)
                .map(|c| Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a))
                .unwrap_or(Color32::TRANSPARENT);
            (g.rect_to_screen(&bounds), 12.0, fill, None)
        } else if edit.creating {
            // The creation box grows with its text (no wrapping, white border).
            let sample = if edit.text.is_empty() {
                " "
            } else {
                edit.text.as_str()
            };
            let galley = ui
                .fonts_mut(|f| f.layout_no_wrap(sample.to_string(), font.clone(), Color32::WHITE));
            let pad = 4.0 * g.z;
            let size = vec2(
                galley.size().x + pad * 2.0 + 4.0,
                (galley.size().y + pad * 2.0 + 2.0).max(32.0 * g.z),
            );
            (
                Rect::from_min_size(g.to_screen(a.start), size),
                4.0,
                Color32::TRANSPARENT,
                Some(Color32::WHITE),
            )
        } else {
            let r = crate::geom::Rect::from_xywh(
                bounds.left,
                bounds.top,
                bounds.width().max(20.0),
                bounds.height().max(20.0),
            );
            (
                g.rect_to_screen(&r),
                4.0,
                Color32::TRANSPARENT,
                Some(Color32::GRAY),
            )
        };
        let text_color = crate::color::Color::parse(&text_color_for(&a, &data))
            .map(|c| Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a.max(1)))
            .unwrap_or(Color32::WHITE);
        let painter = ui.painter().clone();
        if fill.a() > 0 {
            painter.rect_filled(rect, 0.0, fill);
        }
        if let Some(border) = border {
            painter.rect_stroke(rect, 0.0, Stroke::new(1.0, border), StrokeKind::Inside);
        }
        let id = ui.id().with(("text-box", edit.id));
        // Enter (or Escape) finishes; Cmd+Enter breaks the line.
        let finish = ui.memory(|m| m.has_focus(id))
            && ui.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                    || i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
            });
        let mut text = edit.text.clone();
        let align = match data.align {
            model::TextAlign::Left => egui::Align::LEFT,
            model::TextAlign::Center => egui::Align::Center,
            model::TextAlign::Right => egui::Align::RIGHT,
        };
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        let output = egui::TextEdit::multiline(&mut text)
            .id(id)
            .frame(egui::Frame::NONE)
            .text_color(text_color)
            .font(font)
            .margin(egui::Margin::same((padding * g.z).round() as i8))
            .horizontal_align(align)
            .vertical_align(egui::Align::Center)
            .desired_width(rect.width())
            .min_size(rect.size())
            .return_key(egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::Enter,
            ))
            .show(&mut child);
        if self.text_box_focus != Some(edit.id) {
            self.text_box_focus = Some(edit.id);
            output.response.request_focus();
        }
        if let Some(e) = editor.text_editing.as_mut() {
            e.text = text;
        }
        let mut lost = output.response.lost_focus();
        if self.text_box_guard && ui.input(|i| i.pointer.any_released()) {
            self.text_box_guard = false;
            if lost {
                output.response.request_focus();
                lost = false;
            }
        }
        if finish || lost {
            self.text_box_focus = None;
            input::commit_text_edit(editor);
        }
    }

    fn update_cursor(&self, ui: &Ui, editor: &Editor, g: &Geometry, hovered: bool) {
        if self.panning.is_some() {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            return;
        }
        if !hovered && self.pressed.is_none() {
            return;
        }
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos())
            && let Some(edit) = &editor.text_editing
            && let Some(a) = editor.annotation(edit.id)
            && g.rect_to_screen(&a.bounds()).contains(pos)
        {
            ui.ctx().set_cursor_icon(CursorIcon::Text);
            return;
        }
        let Some(pos) = ui.input(|i| i.pointer.hover_pos()) else {
            return;
        };
        let p = g.to_image(pos);
        if editor.input.sel.dragging_handle.is_some() || editor.input.sel.dragging_shape {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            return;
        }
        if let Some(h) = editor
            .input
            .crop
            .rect
            .filter(|_| editor.input.crop.active)
            .and_then(|_| input::hit_crop_handle(editor, p))
        {
            ui.ctx().set_cursor_icon(crop_cursor(h));
            return;
        }
        let handle_radius = input::HANDLE_SIZE / 2.0;
        if input::selection_handles(editor)
            .iter()
            .any(|(_, c)| c.distance(p) <= handle_radius)
        {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
            return;
        }
        let tool = editor.tools.active_tool;
        let over_shape = editor.input.sel.hovered.is_some()
            && !matches!(tool, EditorTool::Crop | EditorTool::CutOut);
        let icon = if tool == EditorTool::Select || over_shape {
            CursorIcon::Default
        } else {
            CursorIcon::Crosshair
        };
        ui.ctx().set_cursor_icon(icon);
    }
}

fn crop_cursor(h: CropHandle) -> CursorIcon {
    match h {
        CropHandle::TopLeft | CropHandle::BottomRight => CursorIcon::ResizeNwSe,
        CropHandle::TopRight | CropHandle::BottomLeft => CursorIcon::ResizeNeSw,
        CropHandle::TopCenter | CropHandle::BottomCenter => CursorIcon::ResizeVertical,
        CropHandle::LeftCenter | CropHandle::RightCenter => CursorIcon::ResizeHorizontal,
        CropHandle::Move => CursorIcon::Move,
    }
}

/// The text box's foreground (`ShowTextEditor`: the text colour, the stroke colour
/// when there is none).
fn text_color_for(a: &Annotation, data: &model::TextData) -> String {
    let c = crate::color::Color::parse(&data.text_color);
    match c {
        Some(c) if c.a > 0 => data.text_color.clone(),
        _ => a.stroke_color.clone(),
    }
}

fn map_button(b: egui::PointerButton) -> Option<Button> {
    match b {
        egui::PointerButton::Primary => Some(Button::Left),
        egui::PointerButton::Secondary => Some(Button::Right),
        egui::PointerButton::Middle => Some(Button::Middle),
        _ => None,
    }
}

// ---------------------------------------------------------------------------- chrome

const DASH: [f32; 1] = [3.0];

/// A black and a white dashed line over each other (`StrokeDashArray 3,3`, the white
/// one offset by 3), in overlay units.
fn ant_line(painter: &egui::Painter, points: &[Pos2], closed: bool, z: f32) {
    if points.len() < 2 {
        return;
    }
    let mut path = points.to_vec();
    if closed {
        path.push(points[0]);
    }
    let dash = [DASH[0] * z];
    let width = z.max(0.5);
    let mut shapes = Vec::new();
    Shape::dashed_line_many_with_offset(
        &path,
        Stroke::new(width, Color32::BLACK),
        &dash,
        &dash,
        0.0,
        &mut shapes,
    );
    Shape::dashed_line_many_with_offset(
        &path,
        Stroke::new(width, Color32::WHITE),
        &dash,
        &dash,
        dash[0],
        &mut shapes,
    );
    painter.extend(shapes);
}

fn ellipse_points(rect: Rect) -> Vec<Pos2> {
    let c = rect.center();
    let (rx, ry) = (rect.width() / 2.0, rect.height() / 2.0);
    (0..72)
        .map(|i| {
            let t = i as f32 / 72.0 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
            pos2(c.x + rx * t.cos(), c.y + ry * t.sin())
        })
        .collect()
}

fn rotated_rect_points(g: &Geometry, r: &crate::geom::Rect, angle: f32) -> Vec<Pos2> {
    let center = r.center();
    [
        Point::new(r.left, r.top),
        Point::new(r.right, r.top),
        Point::new(r.right, r.bottom),
        Point::new(r.left, r.bottom),
    ]
    .into_iter()
    .map(|p| g.to_screen(p.rotate_around(center, angle)))
    .collect()
}

/// `UpdateHoverOutline`.
fn hover_outline(painter: &egui::Painter, a: &Annotation, g: &Geometry) {
    match &a.kind {
        Kind::Line { .. } | Kind::Arrow { .. } => {
            let points: Vec<Pos2> = if model::has_curve(a) {
                let c = model::quadratic_control_point(a);
                (0..=32)
                    .map(|i| {
                        let t = i as f32 / 32.0;
                        let u = 1.0 - t;
                        Point::new(
                            u * u * a.start.x + 2.0 * u * t * c.x + t * t * a.end.x,
                            u * u * a.start.y + 2.0 * u * t * c.y + t * t * a.end.y,
                        )
                    })
                    .map(|p| g.to_screen(p))
                    .collect()
            } else {
                vec![g.to_screen(a.start), g.to_screen(a.end)]
            };
            ant_line(painter, &points, false, g.z);
            return;
        }
        Kind::Freehand { points, .. } => {
            let pts: Vec<Pos2> = points.iter().map(|p| g.to_screen(*p)).collect();
            ant_line(painter, &pts, false, g.z);
            return;
        }
        _ => {}
    }
    let bounds = match &a.kind {
        Kind::Number(_) if model::number_tail_visible(a) => model::interaction_bounds(a, 0.0),
        _ => input::logical_rect(a),
    };
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return;
    }
    let ellipse = match &a.kind {
        Kind::Ellipse { .. } => true,
        Kind::Effect(d) => d.is_ellipse && d.effect == Some(model::EffectKind::Magnify),
        Kind::Spotlight(d) => d.is_ellipse,
        Kind::Number(_) => !model::number_tail_visible(a),
        _ => false,
    };
    let inflated = bounds.inflate(2.0, 2.0);
    if ellipse {
        let rect = g.rect_to_screen(&inflated);
        let mut points = ellipse_points(rect);
        if a.rotation != 0.0 {
            let center = rect.center();
            let (s, c) = a.rotation.to_radians().sin_cos();
            for p in &mut points {
                let d = *p - center;
                *p = center + vec2(d.x * c - d.y * s, d.x * s + d.y * c);
            }
        }
        ant_line(painter, &points, true, g.z);
        return;
    }
    let angle = if input::is_rotatable(a) {
        a.rotation
    } else {
        0.0
    };
    let points = rotated_rect_points(g, &inflated, angle);
    ant_line(painter, &points, true, g.z);
}

/// The overlay: hover outline, selection handles, the rotation connector, the crop
/// chrome and the cut-out preview.
pub fn paint_chrome(ui: &Ui, painter: &egui::Painter, editor: &Editor, g: &Geometry) {
    let z = g.z;
    let interacting = editor.input.is_interacting();
    if !interacting
        && let Some(a) = editor
            .input
            .sel
            .hovered
            .and_then(|id| editor.annotation(id))
    {
        hover_outline(painter, a, g);
    }

    let handles = input::selection_handles(editor);
    if !handles.is_empty() {
        if let Some((_, rotate)) = handles.iter().find(|(t, _)| *t == HandleTag::Rotate)
            && let Some((_, top)) = handles.iter().find(|(t, _)| *t == HandleTag::TopCenter)
        {
            ant_line(
                painter,
                &[g.to_screen(*top), g.to_screen(*rotate)],
                false,
                z,
            );
        }
        let r = input::HANDLE_SIZE / 2.0 * z;
        for (_, p) in &handles {
            let c = g.to_screen(*p);
            let rect = Rect::from_center_size(c, vec2(r * 2.0, r * 2.0));
            style::paint_shadow(
                painter,
                rect,
                r,
                Vec2::ZERO,
                8.0 * z,
                Color32::from_black_alpha(100),
            );
            painter.circle_filled(c, r, Color32::WHITE);
        }
    }

    // Crop: shades, the outline, the thirds, the handles.
    if let Some(cr) = editor
        .input
        .crop
        .rect
        .filter(|r| r.width() > 0.0 && r.height() > 0.0)
    {
        let (cw, ch) = editor.core.canvas_size();
        let quick = editor.input.crop.quick && !editor.input.crop.active;
        let crop = g.rect_to_screen(&cr);
        if editor.input.crop.active {
            let image = g.rect_to_screen(&crate::geom::Rect::new(0.0, 0.0, cw, ch));
            let l = cr.left.clamp(0.0, cw);
            let t = cr.top.clamp(0.0, ch);
            let r = cr.right.clamp(0.0, cw);
            let b = cr.bottom.clamp(0.0, ch);
            let inner = g.rect_to_screen(&crate::geom::Rect::new(l, t, r, b));
            let shade = Color32::from_black_alpha(140);
            for s in [
                Rect::from_min_max(image.min, pos2(image.max.x, inner.top())),
                Rect::from_min_max(
                    pos2(image.min.x, inner.top()),
                    pos2(inner.left(), inner.bottom()),
                ),
                Rect::from_min_max(
                    pos2(inner.right(), inner.top()),
                    pos2(image.max.x, inner.bottom()),
                ),
                Rect::from_min_max(pos2(image.min.x, inner.bottom()), image.max),
            ] {
                if s.is_positive() {
                    painter.rect_filled(s, 0.0, shade);
                }
            }
            painter.rect_stroke(
                crop,
                0.0,
                Stroke::new(z, Color32::WHITE),
                StrokeKind::Inside,
            );
            if (r - l) >= 24.0 && (b - t) >= 24.0 {
                let guide = Color32::from_rgba_unmultiplied(255, 255, 255, 210);
                let dash = [3.0 * z];
                let mut shapes = Vec::new();
                for i in 1..3 {
                    let x = l + (r - l) * i as f32 / 3.0;
                    let y = t + (b - t) * i as f32 / 3.0;
                    Shape::dashed_line_many_with_offset(
                        &[g.to_screen(Point::new(x, t)), g.to_screen(Point::new(x, b))],
                        Stroke::new(z, guide),
                        &dash,
                        &dash,
                        0.0,
                        &mut shapes,
                    );
                    Shape::dashed_line_many_with_offset(
                        &[g.to_screen(Point::new(l, y)), g.to_screen(Point::new(r, y))],
                        Stroke::new(z, guide),
                        &dash,
                        &dash,
                        0.0,
                        &mut shapes,
                    );
                }
                painter.extend(shapes);
            }
            for h in CropHandle::HANDLES {
                paint_crop_handle(painter, g, h, &cr);
            }
        } else {
            // Quick crop / drawing the crop area: the dashed 2-unit outline (4,2) or a
            // plain 1-unit one.
            let (width, dash) = if quick {
                (2.0, Some([4.0, 2.0]))
            } else {
                (1.0, None)
            };
            match dash {
                Some([d, gap]) => {
                    let pts = [
                        crop.left_top(),
                        crop.right_top(),
                        crop.right_bottom(),
                        crop.left_bottom(),
                        crop.left_top(),
                    ];
                    let mut shapes = Vec::new();
                    Shape::dashed_line_many_with_offset(
                        &pts,
                        Stroke::new(width * z, Color32::WHITE),
                        &[d * width * z],
                        &[gap * width * z],
                        0.0,
                        &mut shapes,
                    );
                    painter.extend(shapes);
                }
                None => {
                    painter.rect_stroke(
                        crop,
                        0.0,
                        Stroke::new(width * z, Color32::WHITE),
                        StrokeKind::Inside,
                    );
                }
            }
        }
    }

    // Cut-out: the band that will go, darkened and outlined (5,3 dashes).
    if let Some(cut) = editor.input.cut_out.as_ref().and_then(|c| c.rect)
        && cut.width() > 0.0
        && cut.height() > 0.0
    {
        let rect = g.rect_to_screen(&cut);
        painter.rect_filled(rect, 0.0, Color32::from_black_alpha(150));
        let pts = [
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
            rect.left_top(),
        ];
        let mut shapes = Vec::new();
        Shape::dashed_line_many_with_offset(
            &pts,
            Stroke::new(z, Color32::WHITE),
            &[5.0 * z],
            &[3.0 * z],
            0.0,
            &mut shapes,
        );
        painter.extend(shapes);
    }
    let _ = ui;
}

/// `CreateCropHandle`: an L bracket on a corner, a bar on an edge (white, 5 thick).
fn paint_crop_handle(painter: &egui::Painter, g: &Geometry, h: CropHandle, r: &crate::geom::Rect) {
    let z = g.z;
    let c = g.to_screen(h.position(r));
    let thickness = 5.0 * z;
    if h.is_corner() {
        let arm = 24.0 * z;
        let left = matches!(h, CropHandle::TopRight | CropHandle::BottomRight);
        let up = matches!(h, CropHandle::BottomLeft | CropHandle::BottomRight);
        let hx = if left { c.x - arm } else { c.x + arm };
        let vy = if up { c.y - arm } else { c.y + arm };
        let stroke = Stroke::new(thickness, Color32::WHITE);
        painter.line_segment([pos2(hx, c.y), c], stroke);
        painter.line_segment([c, pos2(c.x, vy)], stroke);
        // Round caps and join.
        for p in [pos2(hx, c.y), c, pos2(c.x, vy)] {
            painter.circle_filled(p, thickness / 2.0, Color32::WHITE);
        }
    } else {
        let horizontal = h.is_horizontal_edge();
        let (w, hh) = if horizontal {
            (30.0 * z, thickness)
        } else {
            (thickness, 30.0 * z)
        };
        painter.rect_filled(
            Rect::from_center_size(c, vec2(w, hh)),
            thickness / 2.0,
            Color32::WHITE,
        );
    }
}

/// A text box's editing state for tests.
pub fn editing(editor: &Editor) -> Option<&TextEditing> {
    editor.text_editing.as_ref()
}
