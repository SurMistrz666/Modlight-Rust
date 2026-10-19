//! The drag-and-drop target (`DragDropUploadWindow`): a small undecorated topmost
//! square in the accent colour, at a corner of the screen, that uploads whatever is
//! dropped on it. Dragging it moves it; a right click closes it. Its size, corner,
//! offset and two opacities come from the application settings (`Drop*`).

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{s, t};
use modlight_core::tasks::settings::ContentAlignment;

use crate::theme::{Colors, Weight};
use crate::widgets::{gradient_rect, layout, paint_icon, wrapped};

#[derive(Clone, Debug, PartialEq)]
pub struct DropWindow {
    pub size: f32,
    pub offset: f32,
    pub alignment: ContentAlignment,
    pub opacity: f32,
    pub hover_opacity: f32,
    pub hovered: bool,
}

/// What happened this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum DropOutcome {
    Close,
    Dropped(Vec<std::path::PathBuf>),
    StartDrag,
}

impl DropWindow {
    pub fn new(
        size: i64,
        offset: i64,
        alignment: ContentAlignment,
        opacity: i64,
        hover_opacity: i64,
    ) -> DropWindow {
        DropWindow {
            size: size.clamp(10, 300) as f32,
            offset: offset.max(0) as f32,
            alignment,
            opacity: opacity.clamp(1, 255) as f32 / 255.0,
            hover_opacity: hover_opacity.clamp(1, 255) as f32 / 255.0,
            hovered: false,
        }
    }

    pub fn window_size(&self) -> Vec2 {
        Vec2::splat(self.size)
    }

    /// Where the window goes in the screen's working area.
    pub fn position(&self, area: Rect) -> egui::Pos2 {
        use ContentAlignment::*;
        let size = self.size;
        let x = match self.alignment {
            TopLeft | MiddleLeft | BottomLeft => area.left() + self.offset,
            TopCenter | MiddleCenter | BottomCenter => area.left() + (area.width() - size) / 2.0,
            _ => area.right() - size - self.offset,
        };
        let y = match self.alignment {
            TopLeft | TopCenter | TopRight => area.top() + self.offset,
            MiddleLeft | MiddleCenter | MiddleRight => area.top() + (area.height() - size) / 2.0,
            _ => area.bottom() - size - self.offset,
        };
        pos2(x, y)
    }

    /// Draw the square filling `ui`; `hovering_files` says whether a drag is over it.
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        colors: &Colors,
        hovering_files: bool,
        dropped: Vec<std::path::PathBuf>,
    ) -> Option<DropOutcome> {
        self.hovered = hovering_files;
        ui.multiply_opacity(if self.hovered {
            self.hover_opacity
        } else {
            self.opacity
        });
        let rect = ui.max_rect();
        let painter = ui.painter();
        gradient_rect(
            painter,
            rect,
            10.0,
            colors.accent_start(),
            colors.accent_end(),
        );
        painter.rect_stroke(
            rect,
            10.0,
            Stroke::new(
                if self.hovered { 3.0 } else { 2.0 },
                Color32::from_rgba_unmultiplied(255, 255, 255, 0xEF),
            ),
            StrokeKind::Inside,
        );
        let icon_size = (self.size / 3.0).clamp(16.0, 42.0);
        let text_size = (self.size / 7.0).clamp(10.0, 20.0);
        let show_icon = self.size >= 35.0;
        let show_text = self.size >= 55.0;
        let text = show_text.then(|| {
            let mut job = wrapped(
                t(s::DragDropUploadWindow_DropHere),
                text_size,
                Weight::SemiBold,
                colors.accent_foreground(),
                self.size - 12.0,
            );
            job.halign = egui::Align::Center;
            layout(ui, job)
        });
        let total = if show_icon { icon_size } else { 0.0 }
            + if show_icon && show_text { 5.0 } else { 0.0 }
            + text.as_ref().map(|g| g.size().y).unwrap_or(0.0);
        let mut y = rect.center().y - total / 2.0;
        if show_icon {
            paint_icon(
                ui,
                painter,
                pos2(rect.center().x, y + icon_size / 2.0),
                icons::UPLOAD,
                icon_size,
                colors.accent_foreground(),
            );
            y += icon_size + 5.0;
        }
        if let Some(text) = text {
            painter.galley(pos2(rect.center().x, y), text, colors.accent_foreground());
        }
        let response = ui.interact(rect, egui::Id::new("drop-window"), Sense::click_and_drag());
        if !dropped.is_empty() {
            return Some(DropOutcome::Dropped(dropped));
        }
        if response.secondary_clicked() {
            return Some(DropOutcome::Close);
        }
        if response.drag_started() {
            return Some(DropOutcome::StartDrag);
        }
        let _ = vec2(0.0, 0.0);
        None
    }
}
