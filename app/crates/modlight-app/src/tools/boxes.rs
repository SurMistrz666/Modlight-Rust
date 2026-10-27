//! The small windows of `ShareX.HelpersLib/Presentation`: `InputBoxWindow` (one line of
//! text, OK and Cancel), `OutputBoxWindow` (read-only text, copy all, close; ffmpeg's
//! output when a recording fails) and `ErrorWindow` (an unexpected failure: what
//! happened, the details, the log file; Continue or Quit).
//!
//! Upstream opens the error window for unhandled exceptions; here a panic on a background
//! thread (the UI thread's own panic ends the process) is shown in it.

use std::sync::Mutex;

use egui::{Align2, Id, Rect, Ui, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, helpers as h, t};

use super::kit::{self, Track};
use super::{ToolWindow, WindowSpec};
use crate::state::{AppState, Effect};
use crate::theme::Weight;

/// `Button.compact` of these windows: 32 tall (36 in the error window), at least 96 wide.
fn compact(
    ui: &mut Ui,
    colors: &crate::theme::Colors,
    rect: Rect,
    label: &str,
    primary: bool,
) -> bool {
    let mut child = kit::child(ui, rect);
    let look = if primary {
        crate::widgets::Look::primary(colors)
    } else {
        crate::widgets::Look::compact(colors)
    }
    .width(rect.width())
    .height(rect.height());
    crate::widgets::button_at(
        &mut child,
        colors,
        &look,
        crate::widgets::Content::label(label),
        true,
        rect,
    )
    .clicked()
}

fn compact_width(ui: &Ui, label: &str, padding: f32, min: f32) -> f32 {
    (kit::text_width(ui, label, 13.0, Weight::Normal) + padding * 2.0).max(min)
}

// ------------------------------------------------------------------ the input box

/// `InputBoxWindow`.
pub struct InputBox {
    pub key: u64,
    pub title: String,
    pub text: String,
    pub ok: String,
    pub cancel: String,
    /// `SubmittedText`.
    pub submitted: Option<String>,
    focused: bool,
}

impl InputBox {
    pub fn new(
        title: &str,
        text: Option<&str>,
        ok: Option<&str>,
        cancel: Option<&str>,
    ) -> InputBox {
        InputBox {
            key: 0,
            title: format(t(h::InputBoxWindow_Title), &[Arg::from(title)]),
            text: text.unwrap_or("").to_string(),
            ok: ok
                .filter(|s| !s.is_empty())
                .unwrap_or(t(h::MyMessageBox_MyMessageBox_OK))
                .to_string(),
            cancel: cancel
                .filter(|s| !s.is_empty())
                .unwrap_or(t(h::MyMessageBox_MyMessageBox_Cancel))
                .to_string(),
            submitted: None,
            focused: false,
        }
    }
}

impl ToolWindow for InputBox {
    fn spec(&self) -> WindowSpec {
        WindowSpec {
            topmost: true,
            ..WindowSpec::resizable(&self.title, vec2(520.0, 120.0), vec2(384.0, 120.0))
        }
    }

    fn ui(&mut self, ui: &mut Ui, state: &mut AppState) -> bool {
        let colors = state.colors.clone();
        kit::fill(ui, &colors);
        let full = kit::inset(ui.max_rect(), 16.0);
        let rows = kit::rows(full, &[Track::Px(40.0), Track::Px(32.0)], 12.0);
        let id = Id::new(("input-box", self.key));
        let response = kit::text_box_at(ui, &colors, rows[0], id, &mut self.text, "", true, false);
        if !self.focused {
            // `OnOpened`: focus and select everything.
            self.focused = true;
            response.request_focus();
            if let Some(mut text_state) = egui::TextEdit::load_state(ui.ctx(), id) {
                let all = egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(self.text.chars().count()),
                );
                text_state.cursor.set_char_range(Some(all));
                text_state.store(ui.ctx(), id);
            }
        }
        let ok_w = compact_width(ui, &self.ok, 14.0, 96.0);
        let cancel_w = compact_width(ui, &self.cancel, 14.0, 96.0);
        let cancel = Rect::from_min_size(
            pos2(rows[1].right() - cancel_w, rows[1].top()),
            vec2(cancel_w, 32.0),
        );
        let ok = Rect::from_min_size(
            pos2(cancel.left() - 7.0 - ok_w, rows[1].top()),
            vec2(ok_w, 32.0),
        );
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        if compact(ui, &colors, ok, &self.ok.clone(), true) || enter {
            self.submitted = Some(self.text.clone());
            return true;
        }
        compact(ui, &colors, cancel, &self.cancel.clone(), false)
            || ui.input(|i| i.key_pressed(egui::Key::Escape))
    }

    fn closed(&mut self, state: &mut AppState) {
        state
            .tools
            .input_answers
            .push((self.key, self.submitted.clone()));
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ------------------------------------------------------------------ the output box

/// `OutputBoxWindow`.
pub struct OutputBox {
    pub key: u64,
    pub title: String,
    pub text: String,
    pub scroll_to_end: bool,
    scrolled: bool,
}

impl OutputBox {
    pub fn new(text: &str, title: &str, scroll_to_end: bool) -> OutputBox {
        OutputBox {
            key: 0,
            title: format(t(h::OutputBoxWindow_Title), &[Arg::from(title)]),
            text: text.to_string(),
            scroll_to_end,
            scrolled: false,
        }
    }
}

/// A read-only monospace text in a box (`TextWrapping="NoWrap"`, both scroll bars).
fn mono_box(
    ui: &mut Ui,
    colors: &crate::theme::Colors,
    rect: Rect,
    id: Id,
    text: &str,
    padding: f32,
    scroll_to_end: bool,
) {
    ui.painter()
        .rect_filled(rect, 8.0, colors.control_background());
    ui.painter().rect_stroke(
        rect,
        8.0,
        egui::Stroke::new(1.0, colors.border()),
        egui::StrokeKind::Inside,
    );
    let inner = rect.shrink(padding);
    let mut child = kit::column(ui, inner);
    child.set_clip_rect(inner);
    let mut area = egui::ScrollArea::both()
        .id_salt(id)
        .auto_shrink([false, false]);
    if scroll_to_end {
        area = area.stick_to_bottom(true);
    }
    area.show(&mut child, |ui| {
        let galley = ui.fonts_mut(|f| {
            f.layout_no_wrap(
                text.to_string(),
                egui::FontId::new(12.0, egui::FontFamily::Monospace),
                colors.text(),
            )
        });
        let (r, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
        ui.painter().galley(r.min, galley, colors.text());
    });
}

impl ToolWindow for OutputBox {
    fn spec(&self) -> WindowSpec {
        WindowSpec::resizable(&self.title, vec2(900.0, 650.0), vec2(520.0, 360.0))
    }

    fn ui(&mut self, ui: &mut Ui, state: &mut AppState) -> bool {
        let colors = state.colors.clone();
        kit::fill(ui, &colors);
        let full = kit::inset(ui.max_rect(), 12.0);
        let rows = kit::rows(full, &[Track::Star(1.0), Track::Px(32.0)], 10.0);
        mono_box(
            ui,
            &colors,
            rows[0],
            Id::new(("output-box", self.key)),
            &self.text,
            10.0,
            self.scroll_to_end && !self.scrolled,
        );
        self.scrolled = true;
        let copy_label = t(h::OutputBoxWindow_Copy_all);
        let close_label = t(h::OutputBoxWindow_Close);
        let copy = Rect::from_min_size(
            rows[1].min,
            vec2(compact_width(ui, copy_label, 14.0, 96.0), 32.0),
        );
        let close_w = compact_width(ui, close_label, 14.0, 96.0);
        let close = Rect::from_min_size(
            pos2(rows[1].right() - close_w, rows[1].top()),
            vec2(close_w, 32.0),
        );
        if compact(ui, &colors, copy, copy_label, false) && !self.text.is_empty() {
            state.effects.push(Effect::CopyText(self.text.clone()));
        }
        compact(ui, &colors, close, close_label, true)
            || ui.input(|i| i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Enter))
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ------------------------------------------------------------------ the error window

/// `ErrorWindow`.
pub struct ErrorWindow {
    pub key: u64,
    pub title: String,
    pub message: String,
    pub log_file: Option<std::path::PathBuf>,
    pub bug_report: Option<String>,
    pub unhandled: bool,
}

impl ToolWindow for ErrorWindow {
    fn spec(&self) -> WindowSpec {
        WindowSpec {
            topmost: true,
            ..WindowSpec::resizable(
                t(h::ErrorWindow_Title),
                vec2(760.0, 560.0),
                vec2(620.0, 440.0),
            )
        }
    }

    fn ui(&mut self, ui: &mut Ui, state: &mut AppState) -> bool {
        let colors = state.colors.clone();
        kit::fill(ui, &colors);
        let full = ui.max_rect();
        // The heading band.
        let band = Rect::from_min_size(full.min, vec2(full.width(), 12.0 * 2.0 + 44.0));
        ui.painter()
            .rect_filled(band, 0.0, colors.background_toolbar());
        let square = Rect::from_min_size(
            pos2(band.left() + 18.0, band.center().y - 20.0),
            vec2(40.0, 40.0),
        );
        ui.painter()
            .rect_filled(square, 6.0, colors.background_panel());
        crate::widgets::paint_icon(
            ui,
            ui.painter(),
            square.center(),
            icons::TRIANGLE_ALERT,
            20.0,
            colors.accent_start(),
        );
        let text_left = square.right() + 10.0;
        let heading = Rect::from_min_size(
            pos2(text_left, band.center().y - 22.0),
            vec2(full.width() - text_left - 18.0, 24.0),
        );
        kit::text_at(
            ui,
            heading,
            Align2::LEFT_CENTER,
            t(h::ErrorWindow_Heading),
            19.0,
            Weight::SemiBold,
            colors.heading(),
        );
        let description = Rect::from_min_size(
            pos2(text_left, heading.bottom() + 2.0),
            vec2(heading.width(), 18.0),
        );
        kit::text_at(
            ui,
            description,
            Align2::LEFT_CENTER,
            t(h::ErrorWindow_Description),
            13.0,
            Weight::SemiBold,
            colors.text_secondary(),
        );

        // The body.
        let body = Rect::from_min_max(
            pos2(full.left() + 18.0, band.bottom() + 14.0),
            pos2(full.right() - 18.0, full.bottom() - 18.0),
        );
        let title_galley = crate::widgets::layout(
            ui,
            crate::widgets::wrapped(
                &self.title,
                14.0,
                Weight::SemiBold,
                colors.text_secondary(),
                body.width() - 22.0,
            ),
        );
        let title_h = title_galley.size().y + 18.0;
        let rows = kit::rows(
            body,
            &[Track::Px(title_h), Track::Star(1.0), Track::Px(36.0)],
            10.0,
        );
        ui.painter()
            .rect_filled(rows[0], 6.0, colors.background_panel());
        ui.painter().rect_stroke(
            rows[0],
            6.0,
            egui::Stroke::new(1.0, colors.border()),
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(
            pos2(rows[0].left() + 11.0, rows[0].top() + 9.0),
            title_galley,
            colors.text_secondary(),
        );
        mono_box(
            ui,
            &colors,
            rows[1],
            Id::new(("error-window", self.key)),
            &self.message,
            8.0,
            true,
        );

        // The buttons.
        let mut close = false;
        let mut x = rows[2].left();
        if let Some(url) = self.bug_report.clone() {
            let label = t(h::ErrorWindow_Send_bug_report);
            let w = compact_width(ui, label, 12.0, 104.0);
            if compact(
                ui,
                &colors,
                Rect::from_min_size(pos2(x, rows[2].top()), vec2(w, 36.0)),
                label,
                false,
            ) {
                state.open_url(&url);
            }
            x += w + 7.0;
        }
        if let Some(log) = self.log_file.clone().filter(|p| p.is_file()) {
            let label = t(h::ErrorWindow_Open_log_file);
            let w = compact_width(ui, label, 12.0, 104.0);
            if compact(
                ui,
                &colors,
                Rect::from_min_size(pos2(x, rows[2].top()), vec2(w, 36.0)),
                label,
                false,
            ) {
                state.effects.push(Effect::OpenFile(log));
            }
        }
        let mut right = rows[2].right();
        let mut place = |ui: &Ui, label: &str| {
            let w = compact_width(ui, label, 12.0, 104.0);
            right -= w;
            let r = Rect::from_min_size(pos2(right, rows[2].top()), vec2(w, 36.0));
            right -= 7.0;
            r
        };
        if self.unhandled {
            let quit_label = t(h::ErrorWindow_Quit);
            let quit = place(ui, quit_label);
            let continue_label = t(h::ErrorWindow_Continue);
            let cont = place(ui, continue_label);
            if compact(ui, &colors, cont, continue_label, true)
                || ui.input(|i| i.key_pressed(egui::Key::Enter))
            {
                log::info!("Modlight continue.");
                close = true;
            }
            if compact(ui, &colors, quit, quit_label, false) {
                log::info!("Modlight closing. Reason: Unhandled exception.");
                state.quit_requested = true;
                close = true;
            }
        } else {
            let ok_label = t(h::ErrorWindow_OK);
            let ok = place(ui, ok_label);
            if compact(ui, &colors, ok, ok_label, true)
                || ui.input(|i| i.key_pressed(egui::Key::Enter))
            {
                close = true;
            }
        }
        close
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Panics seen on background threads, waiting for the error window.
static PANICS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// `Application.ThreadException` / `UnhandledException` on a Mac: a panic outside the UI
/// thread is logged and shown in the error window (the real app only).
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| t(h::ErrorWindow_Unexpected_error).to_string());
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let thread = std::thread::current();
        let details = format!(
            "{message}\n   at {location} (thread {})\n\n{}",
            thread.name().unwrap_or("unnamed"),
            modlight_core::title()
        );
        log::error!("Unhandled exception: {details}");
        if thread.name() != Some("main")
            && let Ok(mut panics) = PANICS.lock()
        {
            panics.push((message, details));
        }
        previous(info);
    }));
}

impl AppState {
    /// `InputBoxWindowIntegration.Show`: the answer comes back to `tools.input_answers`.
    pub fn open_input_box(
        &mut self,
        title: &str,
        text: Option<&str>,
        ok: Option<&str>,
        cancel: Option<&str>,
    ) -> u64 {
        let mut window = InputBox::new(title, text, ok, cancel);
        self.tools.open_keyed(|key| {
            window.key = key;
            Box::new(window)
        })
    }

    /// `OutputBoxWindowIntegration.Show`.
    pub fn open_output_box(&mut self, text: &str, title: &str, scroll_to_end: bool) {
        let mut window = OutputBox::new(text, title, scroll_to_end);
        self.tools.open_keyed(|key| {
            window.key = key;
            Box::new(window)
        });
    }

    /// `ErrorWindowIntegration.Show(title, message, logFilePath, bugReportPath, unhandled)`.
    /// There is no bug report address: upstream's points at ShareX's issue tracker.
    pub fn open_error_window(&mut self, title: &str, message: &str, unhandled: bool) {
        let window = ErrorWindow {
            key: 0,
            title: title.to_string(),
            message: message.to_string(),
            log_file: modlight_core::debug_log::file_path(),
            bug_report: None,
            unhandled,
        };
        let mut window = Some(window);
        self.tools.open_keyed(|key| {
            let mut w = window.take().expect("once");
            w.key = key;
            Box::new(w)
        });
    }

    /// Panics from other threads become error windows (every frame).
    pub(super) fn tick_panics(&mut self) {
        let panics: Vec<(String, String)> = PANICS
            .lock()
            .map(|mut p| std::mem::take(&mut *p))
            .unwrap_or_default();
        for (title, details) in panics {
            self.open_error_window(&title, &details, true);
        }
    }
}
