//! Moving cards onto a folder by dragging them (`MainWindow.MySpace.DragDrop.cs`): the
//! press that may become a drag, the folder under the pointer, and the ghost that
//! follows it with a picture, a count and a sentence saying what a drop would do.

use egui::{Color32, Modifiers, Order, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use modlight_core::l10n::{Arg, format, s, t};
use modlight_core::library::CloudFolder;

use crate::state::AppState;
use crate::theme::Weight;
use crate::widgets::{layout, paint_icon, trimmed};

/// How far a press may travel and still be a click (`FolderClickSlop`).
pub const CLICK_SLOP: f32 = 6.0;
/// The ghost's distance from the pointer.
pub const GHOST_OFFSET: f32 = 16.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Press {
    pub id: String,
    pub at: Pos2,
    pub modifiers: Modifiers,
}

/// A folder the pointer is over while cards are dragged.
#[derive(Clone, Debug, PartialEq)]
pub struct Over {
    /// The folder's id in the listing (the unfiled shelf has its own).
    pub folder: String,
    pub accepts: bool,
    pub hint: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drag {
    pub press: Option<Press>,
    /// The ids being carried, once the press travelled.
    pub moving: Vec<String>,
    pub over: Option<Over>,
    /// The over-target found this frame (folders report themselves while drawing).
    pub over_next: Option<Over>,
}

impl Drag {
    pub fn is_active(&self) -> bool {
        !self.moving.is_empty()
    }

    pub fn clear(&mut self) {
        self.press = None;
        self.moving.clear();
        self.over = None;
        self.over_next = None;
    }
}

impl AppState {
    /// Whether the carried files would actually move into `folder` (`CanDropCloudFiles`).
    pub fn can_drop_on(&self, folder: &CloudFolder) -> bool {
        let target = folder.is_real_folder().then(|| folder.id.clone());
        self.drag
            .moving
            .iter()
            .filter_map(|id| self.space.file(id))
            .any(|file| file.folder_id != target)
    }

    /// A folder tile or row says the pointer is over it during a drag.
    pub fn note_drag_over(&mut self, folder: &CloudFolder) {
        if !self.drag.is_active() {
            return;
        }
        let accepts = self.can_drop_on(folder);
        let hint = if !accepts {
            t(s::MySpace_DragMoveAlreadyHere).to_string()
        } else if folder.is_real_folder() {
            format(t(s::MySpace_DragMoveTo), &[Arg::from(folder.name.as_str())])
        } else {
            t(s::MySpace_DragMoveOut).to_string()
        };
        self.drag.over_next = Some(Over {
            folder: folder.id.clone(),
            accepts,
            hint,
        });
    }

    pub fn is_drop_target(&self, folder_id: &str) -> bool {
        self.drag
            .over
            .as_ref()
            .is_some_and(|o| o.folder == folder_id && o.accepts)
    }

    /// Per frame, after everything that can be dropped on has been drawn: start a drag
    /// that travelled far enough, finish one whose button came up.
    pub fn update_drag(&mut self, ctx: &egui::Context) {
        let (pointer, down, released) = ctx.input(|i| {
            (
                i.pointer.interact_pos(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
            )
        });
        self.drag.over = self.drag.over_next.take();

        if !self.drag.is_active() {
            let Some(press) = &self.drag.press else {
                return;
            };
            if !down {
                self.drag.press = None;
                return;
            }
            let Some(at) = pointer else { return };
            if (at.x - press.at.x).abs() <= CLICK_SLOP && (at.y - press.at.y).abs() <= CLICK_SLOP {
                return;
            }
            if self.space.trashed {
                self.drag.press = None;
                return;
            }
            let moving = self.space.selected_ids();
            self.drag.press = None;
            if moving.is_empty() {
                return;
            }
            self.details.pending_open = None;
            self.drag.moving = moving;
            ctx.request_repaint();
            return;
        }

        if released || !down {
            let moving = std::mem::take(&mut self.drag.moving);
            let over = self.drag.over.take();
            self.drag.clear();
            if let Some(over) = over.filter(|o| o.accepts) {
                let target = self
                    .space
                    .find(&over.folder)
                    .filter(|f| f.is_real_folder())
                    .map(|f| f.id.clone());
                self.push(crate::commands::Command::MoveTo(moving, target));
            }
            return;
        }
        ctx.set_cursor_icon(if self.drag.over.as_ref().is_some_and(|o| o.accepts) {
            egui::CursorIcon::Move
        } else {
            egui::CursorIcon::Grabbing
        });
        ctx.request_repaint();
    }

    /// Paint the ghost next to the pointer (`ShowDragGhost`).
    pub fn paint_drag_ghost(&self, ctx: &egui::Context) {
        if !self.drag.is_active() {
            return;
        }
        let Some(pointer) = ctx.input(|i| i.pointer.hover_pos()) else {
            return;
        };
        let colors = &self.colors;
        let hint = self
            .drag
            .over
            .as_ref()
            .map(|o| o.hint.clone())
            .unwrap_or_else(|| t(s::MySpace_DragMoveHint).to_string());
        let first = self.drag.moving.first().and_then(|id| self.space.file(id));
        let count = self.drag.moving.len();
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::new("drag-ghost"))
            .order(Order::Tooltip)
            .interactable(false)
            .fixed_pos(pos2(0.0, 0.0))
            .show(ctx, |ui| {
                let galley = layout(
                    ui,
                    trimmed(&hint, 13.0, Weight::SemiBold, colors.text(), 260.0, 1),
                );
                let size = vec2(9.0 + 40.0 + 10.0 + galley.size().x + 9.0, 9.0 + 40.0 + 9.0);
                let x = (pointer.x + GHOST_OFFSET)
                    .min(screen.right() - size.x - 4.0)
                    .max(4.0);
                let y = (pointer.y + GHOST_OFFSET)
                    .min(screen.bottom() - size.y - 4.0)
                    .max(4.0);
                let rect = Rect::from_min_size(pos2(x, y), size);
                let mut painter = ui.painter().clone();
                painter.set_opacity(0.96);
                painter.rect_filled(rect, 10.0, colors.background_popup());
                painter.rect_stroke(
                    rect,
                    10.0,
                    Stroke::new(1.0, colors.accent_start()),
                    StrokeKind::Inside,
                );
                let picture = Rect::from_min_size(rect.min + vec2(9.0, 9.0), vec2(40.0, 40.0));
                painter.rect_filled(picture, 8.0, colors.control_background());
                if let Some(file) = first {
                    if let Some(texture) = self.media.thumbs.get(&file.id) {
                        paint_cover(&painter, picture, texture, 8.0);
                    } else {
                        paint_icon(
                            ui,
                            &painter,
                            picture.center(),
                            file.icon,
                            17.0,
                            colors.text_secondary(),
                        );
                    }
                }
                if count > 1 {
                    let label = count.to_string();
                    let text = layout(
                        ui,
                        crate::widgets::job(
                            &label,
                            11.0,
                            Weight::SemiBold,
                            colors.accent_foreground(),
                        ),
                    );
                    let w = (text.size().x + 10.0).max(20.0);
                    let badge = Rect::from_min_size(
                        pos2(picture.right() + 6.0 - w, picture.bottom() + 5.0 - 18.0),
                        vec2(w, 18.0),
                    );
                    painter.rect_filled(badge, 9.0, colors.accent_start());
                    painter.galley(
                        badge.center() - text.size() / 2.0,
                        text,
                        colors.accent_foreground(),
                    );
                }
                painter.galley(
                    pos2(
                        picture.right() + 10.0,
                        rect.center().y - galley.size().y / 2.0,
                    ),
                    galley,
                    colors.text(),
                );
            });
    }
}

/// Paint a texture filling `rect` with the edges cut off (`Stretch.UniformToFill`),
/// corners rounded by `radius` (top-left, top-right, bottom-right, bottom-left).
pub fn paint_cover(
    painter: &egui::Painter,
    rect: Rect,
    texture: &egui::TextureHandle,
    radius: impl Into<egui::CornerRadius>,
) {
    let size = texture.size_vec2();
    if size.x <= 0.0 || size.y <= 0.0 {
        return;
    }
    let scale = (rect.width() / size.x).max(rect.height() / size.y);
    let shown = size * scale;
    let uv_w = (rect.width() / shown.x).min(1.0);
    let uv_h = (rect.height() / shown.y).min(1.0);
    let uv = Rect::from_min_size(
        pos2((1.0 - uv_w) / 2.0, (1.0 - uv_h) / 2.0),
        vec2(uv_w, uv_h),
    );
    let shape = egui::epaint::RectShape::filled(rect, radius, Color32::WHITE)
        .with_texture(texture.id(), uv);
    painter.add(shape);
}

/// Paint a texture inside `rect` keeping all of it (`Stretch.Uniform`), centred.
pub fn paint_contain(painter: &egui::Painter, rect: Rect, texture: &egui::TextureHandle) -> Rect {
    let size = texture.size_vec2();
    if size.x <= 0.0 || size.y <= 0.0 {
        return rect;
    }
    let scale = (rect.width() / size.x).min(rect.height() / size.y);
    let shown = Rect::from_center_size(rect.center(), size * scale);
    painter.image(
        texture.id(),
        shown,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    shown
}
