//! "Auto capture" (`AutoCaptureWindow`): 460 by 430, not resizable, centred on the
//! screen. The area (a custom region or the whole desktop), the repeat time, "minimise to
//! the tray" and "wait for uploads", Start/Stop, and a status bar with the countdown.
//! The timer itself lives in [`AutoCaptureState`] so the hotkeys can start and stop it
//! with the window closed, as upstream's `AutoCaptureWindowIntegration` does.

use egui::{Rect, Stroke, StrokeKind, Ui, pos2, vec2};
use modlight_core::capture::geometry::IRect;
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, s, t};
use modlight_core::tasks::settings::TaskSettings;

use super::background;
use crate::controls::{number_box_at, radio_row};
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Look, button_at, checkbox_weighted, layout, paint_icon, paint_progress, trimmed,
};

pub const WIDTH: f32 = 460.0;
pub const HEIGHT: f32 = 430.0;

/// The timer and its settings (`Program.Settings.AutoCapture*`).
#[derive(Default)]
pub struct AutoCaptureState {
    pub window_open: bool,
    pub running: bool,
    pub settings: Option<TaskSettings>,
    /// UI seconds of the next shot.
    pub next_at: f64,
    /// When the current wait began.
    pub waited_from: f64,
    pub delay: f64,
    pub count: u64,
    pub wait_uploads: bool,
    /// "Custom region" or "Fullscreen".
    pub fullscreen: bool,
    /// The custom region kept while the full screen is chosen.
    pub custom_region: IRect,
    /// The window is hidden in the tray while running.
    pub hidden: bool,
}

/// What the window asks the app to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AutoCaptureAction {
    SelectRegion,
    Execute,
    Fullscreen,
    CustomRegion,
    RepeatTime(f64),
    AutoMinimize(bool),
    WaitUploads(bool),
}

/// What the window shows (read from the settings each frame).
pub struct AutoCaptureView {
    pub region: IRect,
    pub repeat_time: f64,
    pub minimize: bool,
    pub wait_uploads: bool,
}

pub fn ui(
    ui: &mut Ui,
    colors: &Colors,
    state: &AutoCaptureState,
    view: &AutoCaptureView,
    now: f64,
) -> Option<AutoCaptureAction> {
    background(ui, colors);
    let full = ui.max_rect();
    let painter = ui.painter().clone();
    let mut action = None;
    let status_h = 8.0 + 18.0 + 6.0 + 4.0 + 8.0;
    let status = Rect::from_min_max(pos2(full.left(), full.bottom() - status_h), full.max);
    let body = Rect::from_min_max(
        pos2(full.left() + 14.0, full.top() + 14.0),
        pos2(full.right() - 14.0, status.top() - 14.0),
    );

    // The area.
    let panel1 = Rect::from_min_size(
        body.min,
        vec2(body.width(), 14.0 + 32.0 + 10.0 + 34.0 + 14.0),
    );
    tool_panel(&painter, colors, panel1);
    let half = (panel1.width() - 28.0 - 8.0) / 2.0;
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(
        panel1.min + vec2(14.0, 14.0),
        vec2(half, 32.0),
    )));
    if radio_row(
        &mut row,
        colors,
        !state.fullscreen,
        t(s::AutoCaptureWindow_CustomRegion),
        true,
    )
    .clicked()
        && state.fullscreen
    {
        action = Some(AutoCaptureAction::CustomRegion);
    }
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(
        panel1.min + vec2(14.0 + half + 8.0, 14.0),
        vec2(half, 32.0),
    )));
    if radio_row(
        &mut row,
        colors,
        state.fullscreen,
        t(s::AutoCaptureWindow_Fullscreen),
        true,
    )
    .clicked()
        && !state.fullscreen
    {
        action = Some(AutoCaptureAction::Fullscreen);
    }
    let select_text = t(s::AutoCaptureWindow_SelectRegion);
    let look = Look::button(colors).height(34.0);
    let select_w = crate::widgets::button_width(ui, &look, Content::both(icons::SCAN, select_text));
    let line_top = panel1.top() + 14.0 + 32.0 + 10.0;
    let select_rect = Rect::from_min_size(
        pos2(panel1.right() - 14.0 - select_w, line_top),
        vec2(select_w, 34.0),
    );
    let region_box = Rect::from_min_max(
        pos2(panel1.left() + 14.0, line_top),
        pos2(select_rect.left() - 8.0, line_top + 34.0),
    );
    painter.rect_filled(region_box, 4.0, colors.control_background());
    painter.rect_stroke(
        region_box,
        4.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
    let region_text = if view.region.is_empty() {
        t(s::AutoCaptureWindow_NoRegion).to_string()
    } else {
        format(
            t(s::AutoCaptureWindow_Region),
            &[
                Arg::Int(view.region.x as i64),
                Arg::Int(view.region.y as i64),
                Arg::Int(view.region.width as i64),
                Arg::Int(view.region.height as i64),
            ],
        )
    };
    let galley = layout(
        ui,
        trimmed(
            &region_text,
            13.0,
            Weight::Normal,
            colors.text_secondary(),
            region_box.width() - 18.0,
            1,
        ),
    );
    painter.galley(
        pos2(
            region_box.left() + 9.0,
            region_box.center().y - galley.size().y / 2.0,
        ),
        galley,
        colors.text_secondary(),
    );
    if button_at(
        ui,
        colors,
        &look,
        Content::both(icons::SCAN, select_text),
        !state.fullscreen,
        select_rect,
    )
    .clicked()
    {
        action = Some(AutoCaptureAction::SelectRegion);
    }

    // The timing.
    let panel2 = Rect::from_min_size(
        pos2(body.left(), panel1.bottom() + 12.0),
        vec2(body.width(), 14.0 + 32.0 + 12.0 + 32.0 + 8.0 + 32.0 + 14.0),
    );
    tool_panel(&painter, colors, panel2);
    let label = layout(
        ui,
        trimmed(
            t(s::AutoCaptureWindow_RepeatTime),
            13.0,
            Weight::Normal,
            colors.text_secondary(),
            200.0,
            1,
        ),
    );
    let y = panel2.top() + 14.0;
    painter.galley(
        pos2(panel2.left() + 14.0, y + 16.0 - label.size().y / 2.0),
        label.clone(),
        colors.text_secondary(),
    );
    let number = Rect::from_min_size(
        pos2(panel2.left() + 14.0 + label.size().x + 8.0, y),
        vec2(120.0, 32.0),
    );
    let mut repeat = view.repeat_time;
    if number_box_at(
        ui,
        colors,
        "auto-capture-repeat",
        number,
        &mut repeat,
        1.0,
        86400.0,
        0.1,
        1,
        !state.running,
    ) {
        action = Some(AutoCaptureAction::RepeatTime(repeat));
    }
    let seconds = layout(
        ui,
        trimmed(
            t(s::AutoCaptureWindow_Seconds),
            13.0,
            Weight::Normal,
            colors.text_secondary(),
            200.0,
            1,
        ),
    );
    painter.galley(
        pos2(number.right() + 8.0, y + 16.0 - seconds.size().y / 2.0),
        seconds,
        colors.text_secondary(),
    );
    let mut checks = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(
        pos2(panel2.left() + 14.0, y + 32.0 + 12.0),
        vec2(panel2.width() - 28.0, 80.0),
    )));
    checks.spacing_mut().item_spacing.y = 8.0;
    let mut minimize = view.minimize;
    if checkbox_weighted(
        &mut checks,
        colors,
        &mut minimize,
        Some(t(s::AutoCaptureWindow_AutoMinimize)),
        true,
        Weight::SemiBold,
    )
    .changed()
    {
        action = Some(AutoCaptureAction::AutoMinimize(minimize));
    }
    let mut wait = view.wait_uploads;
    if checkbox_weighted(
        &mut checks,
        colors,
        &mut wait,
        Some(t(s::AutoCaptureWindow_WaitTasks)),
        true,
        Weight::SemiBold,
    )
    .changed()
    {
        action = Some(AutoCaptureAction::WaitUploads(wait));
    }

    // Start / Stop.
    let execute = Rect::from_center_size(
        pos2(body.center().x, panel2.bottom() + 12.0 + 21.0),
        vec2(240.0, 42.0),
    );
    let (glyph, text) = if state.running {
        (icons::SQUARE, t(s::AutoCaptureWindow_Stop))
    } else {
        (icons::PLAY, t(s::AutoCaptureWindow_Start))
    };
    let look = Look::button(colors).height(42.0).width(240.0);
    if button_at(
        ui,
        colors,
        &look,
        Content::both(glyph, text),
        !view.region.is_empty(),
        execute,
    )
    .clicked()
    {
        action = Some(AutoCaptureAction::Execute);
    }

    // The status bar.
    painter.rect_filled(status, 0.0, colors.background_panel());
    painter.hline(
        status.left()..=status.right(),
        status.top() + 0.5,
        Stroke::new(1.0, colors.border()),
    );
    let (text, fraction, icon_color) = if state.running {
        let left = (state.next_at - now).max(0.0);
        let percentage = if state.delay > 0.0 {
            (100.0 - left / state.delay * 100.0).clamp(0.0, 100.0) as i64
        } else {
            100
        };
        let seconds_left = modlight_core::culture::Culture::current().fixed(left, 1);
        (
            format(
                t(s::AutoCaptureWindow_Status),
                &[
                    Arg::from(seconds_left),
                    Arg::Int(percentage),
                    Arg::Int(state.count as i64),
                ],
            ),
            percentage as f32 / 100.0,
            egui::Color32::from_rgb(0x1E, 0x90, 0xFF),
        )
    } else {
        (
            t(s::AutoCaptureWindow_Ready).to_string(),
            0.0,
            colors.text_secondary(),
        )
    };
    let line = Rect::from_min_size(
        pos2(status.left() + 12.0, status.top() + 8.0),
        vec2(status.width() - 24.0, 18.0),
    );
    paint_icon(
        ui,
        &painter,
        pos2(line.left() + 8.0, line.center().y),
        icons::TIMER,
        16.0,
        icon_color,
    );
    let galley = layout(
        ui,
        trimmed(
            &text,
            13.0,
            Weight::Normal,
            colors.text_secondary(),
            line.width() - 23.0,
            1,
        ),
    );
    painter.galley(
        pos2(
            line.left() + 16.0 + 7.0,
            line.center().y - galley.size().y / 2.0,
        ),
        galley,
        colors.text_secondary(),
    );
    let bar = Rect::from_min_size(
        pos2(line.left(), line.bottom() + 6.0),
        vec2(line.width(), 4.0),
    );
    paint_progress(ui, colors, bar, Some(fraction));
    action
}

/// `Border.tool-panel`.
fn tool_panel(painter: &egui::Painter, colors: &Colors, rect: Rect) {
    painter.rect_filled(rect, 4.0, colors.background_panel());
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
}
