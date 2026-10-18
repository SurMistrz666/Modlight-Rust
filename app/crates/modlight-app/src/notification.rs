//! The desktop notice (`NotificationWindow`): an undecorated, transparent, topmost
//! card in a corner of the screen with a badge, a title, the text, the action buttons
//! the task settings ask for, and a close button. It stays for its duration, then
//! fades; while the pointer is on it, it neither counts down nor fades.

use egui::{Color32, Id, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use modlight_core::config::ToastSettings;
use modlight_core::icons;
use modlight_core::l10n::{helpers, s, t};

use crate::theme::{Colors, Weight};
use crate::widgets::{icon_galley, layout, paint_icon, trimmed};

const SHADOW_MARGIN: f32 = 14.0;
const SURFACE_RADIUS: f32 = 10.0;
const CARD_PADDING: f32 = 16.0;
const ACTIONS_PADDING: f32 = 8.0;
const MIN_TEXT_WIDTH: f32 = 240.0;
const MAX_TEXT_WIDTH: f32 = 520.0;
const CLOSE_SIZE: f32 = 24.0;
const CLOSE_INSET: f32 = 8.0;
const ICON_SIZES: [f32; 4] = [14.0, 16.0, 18.0, 22.0];

/// A `ToastClickAction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastAction {
    AnnotateImage,
    CopyImageToClipboard,
    CopyFile,
    CopyFilePath,
    CopyUrl,
    OpenFile,
    OpenFolder,
    OpenUrl,
    Upload,
    PinToScreen,
    DeleteFile,
    CloseNotification,
}

impl ToastAction {
    pub fn parse(name: &str) -> Option<ToastAction> {
        Some(match name {
            "AnnotateImage" => ToastAction::AnnotateImage,
            "CopyImageToClipboard" => ToastAction::CopyImageToClipboard,
            "CopyFile" => ToastAction::CopyFile,
            "CopyFilePath" => ToastAction::CopyFilePath,
            "CopyUrl" => ToastAction::CopyUrl,
            "OpenFile" => ToastAction::OpenFile,
            "OpenFolder" => ToastAction::OpenFolder,
            "OpenUrl" => ToastAction::OpenUrl,
            "Upload" => ToastAction::Upload,
            "PinToScreen" => ToastAction::PinToScreen,
            "DeleteFile" => ToastAction::DeleteFile,
            "CloseNotification" => ToastAction::CloseNotification,
            _ => return None,
        })
    }

    /// `NotificationActionButton.GetDefaultLabel`.
    pub fn label(self) -> &'static str {
        match self {
            ToastAction::CopyImageToClipboard => t(s::MainWindow_Copy),
            ToastAction::CopyUrl => t(s::Download_ToastCopyLink),
            ToastAction::AnnotateImage => t(helpers::ToastClickAction_AnnotateImage),
            ToastAction::CopyFile => t(helpers::ToastClickAction_CopyFile),
            ToastAction::CopyFilePath => t(helpers::ToastClickAction_CopyFilePath),
            ToastAction::OpenFile => t(helpers::ToastClickAction_OpenFile),
            ToastAction::OpenFolder => t(helpers::ToastClickAction_OpenFolder),
            ToastAction::OpenUrl => t(helpers::ToastClickAction_OpenUrl),
            ToastAction::Upload => t(helpers::ToastClickAction_Upload),
            ToastAction::PinToScreen => t(helpers::ToastClickAction_PinToScreen),
            ToastAction::DeleteFile => t(helpers::ToastClickAction_DeleteFile),
            ToastAction::CloseNotification => t(helpers::ToastClickAction_CloseNotification),
        }
    }

    /// `NotificationActionButton.GetDefaultIcon`.
    pub fn icon(self) -> char {
        match self {
            ToastAction::AnnotateImage => icons::PEN_LINE,
            ToastAction::CopyImageToClipboard => icons::COPY,
            ToastAction::CopyFile => icons::FILES,
            ToastAction::CopyFilePath => icons::CLIPBOARD,
            ToastAction::CopyUrl => icons::LINK,
            ToastAction::OpenFile => icons::EXTERNAL_LINK,
            ToastAction::OpenFolder => icons::FOLDER_OPEN,
            ToastAction::OpenUrl => icons::EXTERNAL_LINK,
            ToastAction::Upload => icons::UPLOAD,
            ToastAction::PinToScreen => icons::PIN,
            ToastAction::DeleteFile => icons::TRASH_2,
            ToastAction::CloseNotification => icons::X,
        }
    }
}

/// One notice and where its life is.
#[derive(Clone, Debug)]
pub struct Notification {
    pub title: String,
    pub text: String,
    pub url: String,
    pub file_path: String,
    pub has_image: bool,
    pub actions: Vec<ToastAction>,
    pub button_size: f32,
    pub size: (f32, f32),
    pub placement: String,
    pub offset: f32,
    pub duration: f64,
    pub fade: f64,
    pub shown_at: f64,
    pub duration_ended: bool,
    pub fade_started: Option<f64>,
    pub pointer_inside: bool,
    /// The card's size last frame, which the window is sized to.
    pub card_size: Vec2,
}

/// What a frame of the notice asked for.
#[derive(Clone, Debug, PartialEq)]
pub enum NotificationOutcome {
    Close,
    Run(ToastAction),
}

impl Notification {
    /// `TaskHelpers.ShowNotificationTip`: text only, with the task's toast settings.
    pub fn tip(
        text: &str,
        title: &str,
        duration_ms: Option<f64>,
        settings: &ToastSettings,
        now: f64,
    ) -> Notification {
        Notification {
            title: title.to_string(),
            text: text.to_string(),
            url: String::new(),
            file_path: String::new(),
            has_image: false,
            actions: settings
                .buttons
                .iter()
                .filter_map(|b| ToastAction::parse(b))
                .collect(),
            button_size: settings.button_size as f32,
            size: (settings.size.0 as f32, settings.size.1 as f32),
            placement: settings.placement.clone(),
            offset: 5.0,
            duration: duration_ms
                .map(|ms| ms / 1000.0)
                .unwrap_or(settings.duration_seconds),
            fade: settings.fade_seconds,
            shown_at: now,
            duration_ended: false,
            fade_started: None,
            pointer_inside: false,
            card_size: vec2(400.0, 100.0),
        }
    }

    /// `CanExecute`: which configured buttons make sense for this notice.
    pub fn can_execute(&self, action: ToastAction) -> bool {
        let has_file = !self.file_path.trim().is_empty();
        let has_target = has_file || !self.url.trim().is_empty();
        match action {
            ToastAction::CopyImageToClipboard => self.has_image,
            ToastAction::AnnotateImage | ToastAction::PinToScreen => has_file && self.has_image,
            ToastAction::CopyFile
            | ToastAction::CopyFilePath
            | ToastAction::OpenFile
            | ToastAction::OpenFolder => has_file,
            ToastAction::Upload | ToastAction::DeleteFile => has_file,
            ToastAction::CopyUrl | ToastAction::OpenUrl => has_target,
            ToastAction::CloseNotification => true,
        }
    }

    pub fn visible_actions(&self) -> Vec<ToastAction> {
        self.actions
            .iter()
            .copied()
            .filter(|a| self.can_execute(*a))
            .collect()
    }

    /// The window around the card: the card plus the shadow gutter.
    pub fn window_size(&self) -> Vec2 {
        self.card_size + Vec2::splat(SHADOW_MARGIN * 2.0)
    }

    /// The opacity now, and whether the notice has run its course.
    pub fn tick(&mut self, now: f64) -> (f32, bool) {
        if !self.duration_ended && (self.duration <= 0.0 || now - self.shown_at >= self.duration) {
            self.duration_ended = true;
        }
        if self.pointer_inside {
            self.fade_started = None;
            return (1.0, false);
        }
        if self.duration_ended && self.fade_started.is_none() {
            if self.fade <= 0.0 {
                return (1.0, true);
            }
            self.fade_started = Some(now);
        }
        match self.fade_started {
            Some(started) => {
                let progress = ((now - started) / self.fade.max(0.001)) as f32;
                if progress >= 1.0 {
                    (0.0, true)
                } else {
                    (1.0 - progress, false)
                }
            }
            None => (1.0, false),
        }
    }

    /// Where the window goes in a screen area (x, y, w, h), `ContentAlignment` style.
    pub fn position(&self, area: Rect) -> egui::Pos2 {
        let size = self.window_size();
        let offset = self.offset.max(0.0) - SHADOW_MARGIN;
        let x = match self.placement.as_str() {
            "TopLeft" | "MiddleLeft" | "BottomLeft" => area.left() + offset,
            "TopCenter" | "MiddleCenter" | "BottomCenter" => {
                area.left() + (area.width() - size.x) / 2.0
            }
            _ => area.right() - size.x - offset,
        };
        let y = match self.placement.as_str() {
            "TopLeft" | "TopCenter" | "TopRight" => area.top() + offset,
            "MiddleLeft" | "MiddleCenter" | "MiddleRight" => {
                area.top() + (area.height() - size.y) / 2.0
            }
            _ => area.bottom() - size.y - offset,
        };
        pos2(x, y)
    }

    /// `BadgeIconFor`.
    fn badge_icon(&self) -> char {
        if !self.file_path.trim().is_empty() {
            return if self.has_image {
                icons::IMAGE
            } else {
                icons::FILE
            };
        }
        if self.url.trim().is_empty() {
            icons::INFO
        } else {
            icons::LINK
        }
    }

    /// Draw the card into `ui` (whose rect is the whole window). Returns what was asked.
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        colors: &Colors,
        opacity: f32,
    ) -> Option<NotificationOutcome> {
        ui.multiply_opacity(opacity);
        let full = ui.max_rect();
        let width = self.size.0.clamp(MIN_TEXT_WIDTH, MAX_TEXT_WIDTH);
        let actions = self.visible_actions();
        let has_actions = !actions.is_empty();
        let button_size = self.button_size.clamp(16.0, 128.0);
        let footer_h = button_size + ACTIONS_PADDING * 2.0;
        let close_reserve = CLOSE_SIZE + CLOSE_INSET;

        // Text: badge 36, 12 apart, title 14 SemiBold, 4, body 13 on 18 px lines (6 at most).
        let text_left_pad = CARD_PADDING;
        let text_w = width - 2.0 - text_left_pad - close_reserve - 36.0 - 12.0;
        let title = (!self.title.trim().is_empty()).then(|| {
            layout(
                ui,
                trimmed(
                    &self.title,
                    14.0,
                    Weight::SemiBold,
                    colors.text(),
                    text_w,
                    1,
                ),
            )
        });
        let body = (!self.text.trim().is_empty()).then(|| {
            let mut job = trimmed(
                &self.text,
                13.0,
                Weight::Normal,
                colors.text_secondary(),
                text_w,
                6,
            );
            job.wrap.break_anywhere = false;
            job.sections[0].format.line_height = Some(18.0);
            layout(ui, job)
        });
        let mut stack = 0.0;
        if let Some(g) = &title {
            stack += g.size().y;
        }
        if let Some(g) = &body {
            if title.is_some() {
                stack += 4.0;
            }
            stack += g.size().y;
        }
        let content_h = stack.max(36.0).min(self.size.1.max(96.0));
        let bottom_pad = if has_actions {
            CARD_PADDING + footer_h
        } else {
            CARD_PADDING
        };
        let card_h = 1.0 + CARD_PADDING + content_h + bottom_pad + 1.0;
        self.card_size = vec2(width, card_h);

        let card = Rect::from_min_size(full.min + Vec2::splat(SHADOW_MARGIN), self.card_size);
        let painter = ui.painter().clone();
        painter.add(
            egui::Shadow {
                // `0 8 24 -8 #73000000`: egui has no negative spread, so the blur is
                // narrowed and the shadow lightened to the same visible weight.
                offset: [0, 8],
                blur: 16,
                spread: 0,
                color: Color32::from_black_alpha(0x50),
            }
            .as_shape(card, SURFACE_RADIUS),
        );
        painter.rect_filled(card, SURFACE_RADIUS, colors.background_popup());

        let surface = ui.interact(card, Id::new("notification-card"), Sense::click());
        self.pointer_inside = ui.rect_contains_pointer(card);

        // Badge and text.
        let left = card.left() + 1.0 + text_left_pad;
        let top = card.top() + 1.0 + CARD_PADDING;
        let badge = Rect::from_min_size(pos2(left, top), vec2(36.0, 36.0));
        painter.rect_filled(badge, 10.0, colors.accent_start());
        paint_icon(
            ui,
            &painter,
            badge.center(),
            self.badge_icon(),
            18.0,
            colors.accent_foreground(),
        );
        let mut y = top;
        let tx = badge.right() + 12.0;
        if let Some(g) = title {
            let h = g.size().y;
            painter.galley(pos2(tx, y), g, colors.text());
            y += h + 4.0;
        }
        if let Some(g) = body {
            painter.galley(pos2(tx, y), g, colors.text_secondary());
        }

        let mut outcome = None;
        // The pinned footer with the actions (text notices have no picture to hover).
        if has_actions {
            let footer = Rect::from_min_max(
                pos2(card.left() + 1.0, card.bottom() - 1.0 - footer_h),
                pos2(card.right() - 1.0, card.bottom() - 1.0),
            );
            painter.rect_filled(
                footer,
                egui::CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: (SURFACE_RADIUS - 1.0) as u8,
                    se: (SURFACE_RADIUS - 1.0) as u8,
                },
                colors.background_panel(),
            );
            painter.hline(
                footer.x_range(),
                footer.top() + 0.5,
                Stroke::new(1.0, colors.separator()),
            );
            let icon_size = nearest_icon_size(button_size / 2.0);
            let total = actions.len() as f32 * button_size + (actions.len() as f32 - 1.0) * 4.0;
            let mut bx = footer.center().x - total / 2.0;
            let radius = (button_size / 4.0).clamp(6.0, SURFACE_RADIUS);
            for action in actions {
                let r = Rect::from_min_size(
                    pos2(bx, footer.center().y - button_size / 2.0),
                    Vec2::splat(button_size),
                );
                let response = ui.interact(
                    r,
                    Id::new(("notification-action", action as u8)),
                    Sense::click(),
                );
                let hot = response.hovered() || response.is_pointer_button_down_on();
                let shown = if response.is_pointer_button_down_on() {
                    Rect::from_center_size(r.center(), r.size() * 0.97)
                } else {
                    r
                };
                painter.rect_filled(
                    shown,
                    radius,
                    if hot {
                        colors.accent_start()
                    } else {
                        colors.control_background()
                    },
                );
                painter.rect_stroke(
                    shown,
                    radius,
                    Stroke::new(
                        1.0,
                        if hot {
                            colors.accent_start()
                        } else {
                            colors.control_border()
                        },
                    ),
                    StrokeKind::Inside,
                );
                let color = if hot {
                    colors.accent_foreground()
                } else {
                    colors.text()
                };
                let icon = icon_galley(ui, action.icon(), icon_size, color);
                painter.galley(shown.center() - icon.size() / 2.0, icon, color);
                if response.on_hover_text(action.label()).clicked() {
                    outcome = Some(NotificationOutcome::Run(action));
                }
                bx += button_size + 4.0;
            }
        }

        // Close, always there.
        let close = Rect::from_min_size(
            pos2(
                card.right() - CLOSE_INSET - CLOSE_SIZE,
                card.top() + CLOSE_INSET,
            ),
            Vec2::splat(CLOSE_SIZE),
        );
        let response = ui.interact(close, Id::new("notification-close"), Sense::click());
        let fill = if response.is_pointer_button_down_on() {
            colors.control_background()
        } else if response.hovered() {
            colors.control_background_hover()
        } else {
            colors.background_popup()
        };
        painter.rect_filled(close, 8.0, fill);
        painter.rect_stroke(
            close,
            8.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        let x_color = if response.hovered() {
            colors.text()
        } else {
            colors.text_secondary()
        };
        paint_icon(ui, &painter, close.center(), icons::X, 13.0, x_color);
        if response.on_hover_text(t(s::Notification_Close)).clicked() {
            outcome = Some(NotificationOutcome::Close);
        }

        painter.rect_stroke(
            card,
            SURFACE_RADIUS,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );

        // A click on the card runs the left-click action, which for a text notice has
        // nothing to act on, and dismisses it.
        if outcome.is_none()
            && (surface.clicked() || surface.secondary_clicked() || surface.middle_clicked())
        {
            outcome = Some(NotificationOutcome::Close);
        }
        outcome
    }
}

fn nearest_icon_size(target: f32) -> f32 {
    let mut best = ICON_SIZES[0];
    for size in ICON_SIZES {
        if (size - target).abs() < (best - target).abs() {
            best = size;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> ToastSettings {
        ToastSettings {
            duration_seconds: 3.0,
            fade_seconds: 1.0,
            placement: "BottomRight".into(),
            size: (400, 300),
            button_size: 40,
            buttons: vec!["CopyImageToClipboard".into(), "CopyUrl".into()],
        }
    }

    #[test]
    fn a_text_tip_has_no_buttons_it_cannot_honour() {
        let tip = Notification::tip("Hello", "Modlight", None, &settings(), 0.0);
        assert!(tip.visible_actions().is_empty(), "no picture, no link");
        let mut with_link = tip.clone();
        with_link.url = "https://x".into();
        assert_eq!(with_link.visible_actions(), vec![ToastAction::CopyUrl]);
    }

    #[test]
    fn it_waits_then_fades_unless_the_pointer_holds_it() {
        let mut tip = Notification::tip("Hello", "Modlight", Some(8000.0), &settings(), 0.0);
        assert_eq!(tip.tick(7.9), (1.0, false));
        assert_eq!(tip.tick(8.0), (1.0, false));
        let (opacity, done) = tip.tick(8.5);
        assert!((opacity - 0.5).abs() < 0.01 && !done);
        tip.pointer_inside = true;
        assert_eq!(tip.tick(8.6), (1.0, false), "the pointer stops the fade");
        tip.pointer_inside = false;
        assert!(!tip.tick(8.7).1);
        assert!(tip.tick(9.8).1);
        assert_eq!(nearest_icon_size(20.0), 18.0);
    }

    #[test]
    fn placement_puts_the_card_five_points_from_the_corner() {
        let tip = Notification::tip("Hello", "", None, &settings(), 0.0);
        let area = Rect::from_min_size(pos2(0.0, 25.0), vec2(1440.0, 850.0));
        let at = tip.position(area);
        let card_right = at.x + SHADOW_MARGIN + tip.card_size.x;
        let card_bottom = at.y + SHADOW_MARGIN + tip.card_size.y;
        assert_eq!(card_right, 1435.0);
        assert_eq!(card_bottom, 870.0);
    }
}
