//! Screen recording on the window's side (`ScreenRecordManager`, `ScreenRecordWindow`,
//! `RecordingRegionBorder`): choosing the area, the dashed frame around it, the toolbar
//! under it (timer, Start/Stop, Pause/Resume, Restart, Abort, the abort confirmation),
//! and the finished file going through the after-capture window into a task.
//!
//! The frame and the toolbar are two windows: macOS cannot cut a hole into one, as
//! upstream's window region does. The frame lets the pointer through; this app's windows
//! are left out of the recording by ScreenCaptureKit's filter.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{Color32, Rect, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use modlight_core::capture::geometry::IRect;
use modlight_core::capture::region::{RegionKind, RegionSession};
use modlight_core::capture::source::even_size;
use modlight_core::icons;
use modlight_core::l10n::{capture as c, s, t};
use modlight_core::recording::session::{
    ControlState, FrameSource, FrameSourceFactory, RecorderControl, RecordingStatus,
    SessionOutcome, SessionParams, run_session,
};
use modlight_core::recording::{FFmpegVideoCodec, ScreenRecordOutput};
use modlight_core::tasks::host::{MessageIcon, NotificationSound};
use modlight_core::tasks::info::TaskMetadata;
use modlight_core::tasks::settings::TaskSettings;

use crate::capture_flow::{PendingImage, RecordStartMethod};
use crate::dialogs::message::MessageBox;
use crate::region_overlay::{OverlayPurpose, RegionOverlay};
use crate::state::AppState;
use crate::theme::{Colors, Weight};
use crate::widgets::{Content, Look, button_at, layout, trimmed};

pub const TOOLBAR_WIDTH: f32 = 461.0;
pub const TOOLBAR_HEIGHT: f32 = 42.0;
const TIMER_WIDTH: f32 = 112.0;
const ACTION_WIDTH: f32 = 86.0;
const COMPACT_ACTION_WIDTH: f32 = 40.0;
const GAP: f32 = 3.0;
const GOLDENROD: Color32 = Color32::from_rgb(0xDA, 0xA5, 0x20);

/// A recording in progress, as the window sees it.
pub struct Recording {
    pub control: Arc<RecorderControl>,
    pub settings: TaskSettings,
    pub metadata: TaskMetadata,
    pub show_timer: bool,
    pub show_labels: bool,
    pub ask_on_abort: bool,
    pub confirming_abort: bool,
    /// The region tool started it: the toolbar takes the focus (`ActivateWindow`).
    pub activate: bool,
    /// Dragging the timer moves the area before recording (`OnTimerPointerMoved`).
    drag_from: Option<(egui::Pos2, IRect)>,
}

/// A toolbar button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingAction {
    StartStop,
    PauseResume,
    Restart,
    Abort,
    CancelAbort,
    ConfirmAbort,
}

impl Recording {
    /// A recording as a QA screen shows it (no session behind it).
    pub fn for_qa(
        control: Arc<RecorderControl>,
        settings: TaskSettings,
        compact: bool,
    ) -> Recording {
        Recording {
            control,
            settings,
            metadata: TaskMetadata::default(),
            show_timer: true,
            show_labels: !compact,
            ask_on_abort: false,
            confirming_abort: false,
            activate: false,
            drag_from: None,
        }
    }

    fn toolbar_width(&self) -> f32 {
        if self.confirming_abort {
            return TOOLBAR_WIDTH;
        }
        let mut width =
            TOOLBAR_WIDTH - TIMER_WIDTH + if self.show_timer { TIMER_WIDTH } else { 0.0 };
        if !self.show_labels {
            width -= (ACTION_WIDTH - COMPACT_ACTION_WIDTH) * 4.0;
        }
        width
    }

    /// The frame window: the area plus one point of border all round.
    pub fn frame_rect(&self) -> Rect {
        let r = self.control.snapshot().region;
        Rect::from_min_size(
            pos2(r.x as f32 - 1.0, r.y as f32 - 1.0),
            vec2(r.width as f32 + 2.0, r.height as f32 + 2.0),
        )
    }

    /// The toolbar window: centred under the frame, three points below it.
    pub fn toolbar_rect(&self) -> Rect {
        let frame = self.frame_rect();
        let width = self.toolbar_width();
        Rect::from_min_size(
            pos2(frame.center().x - width / 2.0, frame.bottom() + GAP),
            vec2(width, TOOLBAR_HEIGHT),
        )
    }

    /// `RecordingRegionBorder`: a dark dashed line and the accent dashed over its gaps.
    pub fn frame_ui(&self, ui: &mut Ui) {
        let full = ui.max_rect();
        let painter = ui.painter();
        let border = full.shrink(0.5);
        let state = self.control.snapshot();
        let accent = accent(&state);
        let backdrop = Stroke::new(1.0, Color32::from_rgba_unmultiplied(20, 20, 20, 220));
        let accent = Stroke::new(1.0, accent);
        let corners = [
            border.left_top(),
            border.right_top(),
            border.right_bottom(),
            border.left_bottom(),
            border.left_top(),
        ];
        painter.extend(Shape::dashed_line_with_offset(
            &corners,
            backdrop,
            &[5.0],
            &[5.0],
            0.0,
        ));
        painter.extend(Shape::dashed_line_with_offset(
            &corners,
            accent,
            &[5.0],
            &[5.0],
            5.0,
        ));
    }

    /// The toolbar. Returns the button pressed.
    pub fn toolbar_ui(&mut self, ui: &mut Ui, colors: &Colors) -> Option<RecordingAction> {
        let full = ui.max_rect();
        let painter = ui.painter().clone();
        let state = self.control.snapshot();
        painter.rect_filled(full, 8.0, colors.background_toolbar());
        let mut action = None;

        if self.confirming_abort {
            // `AbortConfirmation`: the question, Cancel, Abort.
            let text = t(c::ScreenRecordWindow_Abort_this_recording);
            let abort_rect = Rect::from_min_size(
                pos2(full.right() - 1.0 - ACTION_WIDTH, full.top() + 1.0),
                vec2(ACTION_WIDTH, 40.0),
            );
            let cancel_rect = Rect::from_min_size(
                pos2(abort_rect.left() - 1.0 - ACTION_WIDTH, full.top() + 1.0),
                vec2(ACTION_WIDTH, 40.0),
            );
            let label = layout(
                ui,
                trimmed(
                    text,
                    13.0,
                    Weight::SemiBold,
                    colors.text_secondary(),
                    cancel_rect.left() - full.left() - 20.0,
                    1,
                ),
            );
            painter.galley(
                pos2(full.left() + 10.0, full.center().y - label.size().y / 2.0),
                label,
                colors.text_secondary(),
            );
            let look = action_look(colors).radius(0.0);
            if button_at(
                ui,
                colors,
                &look,
                Content::label(t(c::ScreenRecordWindow_Cancel)),
                true,
                cancel_rect,
            )
            .clicked()
            {
                action = Some(RecordingAction::CancelAbort);
            }
            if button_at(
                ui,
                colors,
                &look.radius(7.0),
                Content::label(t(c::ScreenRecordWindow_Abort)),
                true,
                abort_rect,
            )
            .clicked()
            {
                action = Some(RecordingAction::ConfirmAbort);
            }
        } else {
            let mut x = full.left() + 1.0;
            if self.show_timer {
                let timer = Rect::from_min_size(pos2(x, full.top() + 1.0), vec2(TIMER_WIDTH, 40.0));
                painter.rect_filled(
                    timer,
                    egui::CornerRadius {
                        nw: 7,
                        sw: 7,
                        ne: 0,
                        se: 0,
                    },
                    colors.background_panel(),
                );
                let text = RecorderControl::timer_text(&state);
                let galley = ui.fonts_mut(|f| {
                    f.layout_no_wrap(text, egui::FontId::monospace(14.0), colors.text_secondary())
                });
                let total = 8.0 + 9.0 + galley.size().x;
                let left = timer.center().x - total / 2.0;
                painter.circle_filled(pos2(left + 4.0, timer.center().y), 4.0, accent(&state));
                painter.galley(
                    pos2(left + 8.0 + 9.0, timer.center().y - galley.size().y / 2.0),
                    galley,
                    colors.text_secondary(),
                );
                // Dragging the timer moves the area while waiting or paused.
                let response =
                    ui.interact(timer, egui::Id::new("recording-timer"), egui::Sense::drag());
                let movable = matches!(
                    state.status,
                    RecordingStatus::Waiting | RecordingStatus::Paused
                );
                if movable && response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
                }
                if movable && response.drag_started() {
                    self.drag_from = ui.ctx().pointer_interact_pos().map(|p| (p, state.region));
                }
                if let Some((start, region)) = self.drag_from
                    && response.dragged()
                    && let Some(now) = ui.ctx().pointer_interact_pos()
                {
                    let delta = now - start;
                    self.control.move_region(IRect::new(
                        region.x + delta.x.round() as i32,
                        region.y + delta.y.round() as i32,
                        region.width,
                        region.height,
                    ));
                }
                if response.drag_stopped() {
                    self.drag_from = None;
                }
                x = timer.right() + 1.0;
            }
            let width = if self.show_labels {
                ACTION_WIDTH
            } else {
                COMPACT_ACTION_WIDTH
            };
            let (start_text, start_icon) = start_label(state.status);
            let (pause_text, pause_icon) = pause_label(state.status);
            let restart_enabled = matches!(
                state.status,
                RecordingStatus::Recording | RecordingStatus::Paused
            );
            let buttons = [
                (start_text, start_icon, true, RecordingAction::StartStop),
                (pause_text, pause_icon, true, RecordingAction::PauseResume),
                (
                    t(c::ScreenRecordWindow_Restart),
                    icons::ROTATE_CCW,
                    restart_enabled,
                    RecordingAction::Restart,
                ),
                (
                    t(c::ScreenRecordWindow_Abort),
                    icons::X,
                    true,
                    RecordingAction::Abort,
                ),
            ];
            for (index, (label, icon, enabled, which)) in buttons.into_iter().enumerate() {
                let rect = Rect::from_min_size(pos2(x, full.top() + 1.0), vec2(width, 40.0));
                let mut look = action_look(colors).width(width);
                look.radius = if index == 3 || (index == 0 && !self.show_timer) {
                    7.0
                } else {
                    0.0
                };
                let content = if self.show_labels {
                    Content::both(icon, label)
                } else {
                    Content::icon(icon)
                };
                let response = button_at(ui, colors, &look, content, enabled, rect);
                let response = if self.show_labels {
                    response
                } else {
                    response.on_hover_text(label)
                };
                if response.clicked() {
                    action = Some(which);
                }
                x = rect.right() + 1.0;
            }
        }
        painter.rect_stroke(
            full,
            8.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        action
    }
}

/// `Button.recorder-action`: no border, square, the icon in the accent.
fn action_look(colors: &Colors) -> Look {
    let mut look = Look::button(colors)
        .height(40.0)
        .padding(8.0)
        .radius(0.0)
        .icon(15.0, Some(colors.accent_start()));
    look.border = None;
    look.hover_border = None;
    look.spacing = 7.0;
    look
}

/// The Start/Stop button's text and glyph (`UpdateUI`: `ScreenRecordForm_Start`,
/// `ScreenRecordForm_Stop`).
fn start_label(status: RecordingStatus) -> (&'static str, char) {
    match status {
        RecordingStatus::Working | RecordingStatus::Recording | RecordingStatus::Paused => {
            (t(c::ScreenRecordForm_Stop), icons::SQUARE)
        }
        _ => (t(c::ScreenRecordForm_Start), icons::CIRCLE_PLAY),
    }
}

/// The Pause/Resume button's text and glyph (`UpdateUI`: `Strings.Resume`,
/// `Strings.Pause`).
fn pause_label(status: RecordingStatus) -> (&'static str, char) {
    match status {
        RecordingStatus::Waiting | RecordingStatus::Paused => (t(c::Resume), icons::PLAY),
        _ => (t(c::Pause), icons::PAUSE),
    }
}

/// Goldenrod while waiting or paused, red while recording (`SetRecordingAccent`).
fn accent(state: &ControlState) -> Color32 {
    if state.status == RecordingStatus::Recording {
        Color32::RED
    } else {
        GOLDENROD
    }
}

/// Where ffmpeg is: the override, the installed component, or (QA only) a test binary.
pub fn ffmpeg_path(
    settings: &TaskSettings,
    profile: &std::path::Path,
    qa: bool,
) -> Option<PathBuf> {
    let installed = crate::recording_ui::installed_ffmpeg(profile, qa);
    settings
        .capture_settings
        .ffmpeg_options
        .ffmpeg_path(installed)
        .filter(|p| p.is_file())
}

/// The ffmpeg the Components window installed (`ExternalTools.Locate("ffmpeg")`); QA runs
/// may point `MODLIGHT_QA_FFMPEG` at one instead.
pub fn installed_ffmpeg(profile: &std::path::Path, qa: bool) -> Option<PathBuf> {
    if qa && let Some(path) = std::env::var_os("MODLIGHT_QA_FFMPEG").map(PathBuf::from) {
        return Some(path);
    }
    modlight_core::components::installed_path(profile, "ffmpeg")
}

impl AppState {
    /// `ScreenRecordManager.StartStopRecording`.
    pub fn start_screen_recording(
        &mut self,
        output: ScreenRecordOutput,
        method: RecordStartMethod,
        settings: TaskSettings,
    ) {
        if let Some(recording) = &self.captures.recording {
            recording.control.start_stop();
            return;
        }
        // An overlay open or being prepared (the desktop is being taken for it).
        if self.captures.overlay.is_some() || self.captures.busy {
            return;
        }
        let mut settings = settings;
        settings.capture_settings.ffmpeg_options.fix_sources();
        if output == ScreenRecordOutput::Gif {
            settings.capture_settings.ffmpeg_options.VideoCodec = FFmpegVideoCodec::gif;
        }
        if settings.capture_settings.ffmpeg_options.is_animated_image() {
            settings.capture_settings.screen_record_two_pass_encoding = true;
        }
        // `CheckFFmpeg`: offer the component, and record when it arrives.
        if ffmpeg_path(&settings, &self.env.profile, self.env.qa).is_none() {
            if !crate::components_window::require(
                self,
                "ffmpeg",
                s::Components_FeatureRecording,
                crate::components_window::AfterClose::Recording(
                    output,
                    method,
                    Box::new(settings.clone()),
                ),
            ) {
                return;
            }
            let shown = settings
                .capture_settings
                .ffmpeg_options
                .ffmpeg_path(installed_ffmpeg(&self.env.profile, self.env.qa))
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| {
                    self.env
                        .profile
                        .join(modlight_core::components::TOOLS_FOLDER)
                        .join("ffmpeg")
                        .display()
                        .to_string()
                });
            self.uploads.windows.messages.push_back(MessageBox::new(
                &format!("Modlight - {}", t(s::FFmpegIsMissing)),
                &format!("{}\n{shown}", t(s::FFmpegDoesNotExistAtTheFollowingPath)),
                MessageIcon::Warning,
            ));
            return;
        }
        if !settings
            .capture_settings
            .ffmpeg_options
            .is_source_selected()
        {
            self.uploads.windows.messages.push_back(MessageBox::new(
                &format!("Modlight - {}", t(s::FFmpeg_FFmpeg_error)),
                t(s::FFmpeg_FFmpeg_video_and_audio_source_both_can_t_be__None__),
                MessageIcon::Warning,
            ));
            return;
        }
        if settings.general_settings.toast_window_auto_hide {
            self.windows.notification = None;
        }
        let source = self.captures.source.clone();
        match method {
            RecordStartMethod::Region => {
                let kind = if settings.capture_settings.screen_record_transparent_region {
                    RegionKind::Transparent
                } else {
                    RegionKind::Default
                };
                self.captures.busy = true;
                self.rt.spawn_blocking(move || {
                    let request =
                        modlight_core::capture::region_request(kind, &settings, source.as_ref());
                    move |state: &mut AppState, _ctx: &egui::Context| {
                        state.captures.busy = false;
                        match request {
                            Ok(mut request) => {
                                // `GetRectangleRegion`: an area, no annotation.
                                request.enable_annotations = false;
                                let session = RegionSession::new(request);
                                state.captures.overlay = Some(RegionOverlay::new(
                                    session,
                                    OverlayPurpose::Recording(output),
                                    settings,
                                    state.time,
                                ));
                            }
                            Err(error) => {
                                log::warn!("The recording area could not be chosen: {error}")
                            }
                        }
                    }
                });
            }
            RecordStartMethod::ActiveWindow => {
                let Some(window) = source.active_window() else {
                    return;
                };
                let metadata = TaskMetadata {
                    window_title: Some(window.title.clone()),
                    process_name: Some(window.process_name.clone()),
                    ..TaskMetadata::default()
                };
                self.begin_recording(
                    output,
                    window.rect,
                    metadata,
                    settings,
                    false,
                    &egui::Context::default(),
                );
            }
            RecordStartMethod::CustomRegion => {
                let rect = settings.capture_settings.capture_custom_region;
                self.begin_recording(
                    output,
                    rect,
                    TaskMetadata::default(),
                    settings,
                    false,
                    &egui::Context::default(),
                );
            }
            RecordStartMethod::LastRegion => {
                let rect = self.screen_record_region();
                self.begin_recording(
                    output,
                    rect,
                    TaskMetadata::default(),
                    settings,
                    false,
                    &egui::Context::default(),
                );
            }
        }
    }

    /// The area is known: the frame, the toolbar and the session thread.
    pub fn begin_recording(
        &mut self,
        _output: ScreenRecordOutput,
        rect: IRect,
        metadata: TaskMetadata,
        settings: TaskSettings,
        activate: bool,
        _ctx: &egui::Context,
    ) {
        let bounds = self.captures.source.screen_bounds();
        let mut rect = rect.intersect(&bounds);
        if settings
            .capture_settings
            .ffmpeg_options
            .is_even_size_required()
        {
            rect = even_size(rect);
        }
        if self.captures.recording.is_some() || !rect.has_area() {
            return;
        }
        self.set_screen_record_region(rect);
        let capture = &settings.capture_settings;
        let fps = if capture.ffmpeg_options.VideoCodec == FFmpegVideoCodec::gif {
            capture.gif_fps
        } else {
            capture.screen_record_fps
        };
        let duration = if capture.screen_record_fixed_duration {
            capture.screen_record_duration
        } else {
            0.0
        };
        let control = RecorderControl::new(rect, duration);
        // The file: the screenshots folder and the name pattern (`HandleExistsFile` takes a
        // unique name here: the question would stop the recording thread).
        let two_pass = capture.screen_record_two_pass_encoding;
        let extension = if two_pass {
            "mp4"
        } else {
            capture.ffmpeg_options.extension()
        };
        let folder = self
            .uploads
            .ctx
            .screenshots_folder(&settings, Some(&metadata));
        let _ = std::fs::create_dir_all(&folder);
        let name = self
            .uploads
            .ctx
            .file_name(&settings, Some(extension), Some(&metadata));
        let path = unique(folder.join(&name));
        let output_name = self.uploads.ctx.file_name(
            &settings,
            Some(capture.ffmpeg_options.extension()),
            Some(&metadata),
        );
        let two_pass_output = unique(folder.join(output_name));
        let Some(ffmpeg_path) = ffmpeg_path(&settings, &self.env.profile, self.env.qa) else {
            return;
        };
        let frames: FrameSourceFactory = if self.env.qa {
            let source = self.captures.source.clone();
            Arc::new(move |rect: IRect, _cursor: bool| {
                let picture = source
                    .capture_rect(rect, false)
                    .map_err(|e| e.to_string())?;
                let picture = image::imageops::resize(
                    &picture,
                    (picture.width() / 2 * 2).max(2),
                    (picture.height() / 2 * 2).max(2),
                    image::imageops::FilterType::Nearest,
                );
                Ok(
                    Box::new(modlight_core::recording::session::StillFrames::new(
                        &picture,
                    )) as Box<dyn FrameSource>,
                )
            })
        } else {
            Arc::new(move |rect: IRect, cursor: bool| {
                Ok(Box::new(crate::platform::capture::ScreenFrames::new(
                    rect, cursor, fps,
                )) as Box<dyn FrameSource>)
            })
        };
        let params = SessionParams {
            ffmpeg_path,
            ffmpeg: capture.ffmpeg_options.clone(),
            fps,
            duration,
            draw_cursor: capture.screen_record_show_cursor,
            two_pass,
            auto_start: capture.screen_record_auto_start,
            start_delay: capture.screen_record_start_delay,
            path,
            two_pass_output,
            frames,
        };
        self.captures.recording = Some(Recording {
            control: control.clone(),
            settings: settings.clone(),
            metadata: metadata.clone(),
            show_timer: capture.screen_record_show_timer,
            show_labels: capture.screen_record_show_button_labels,
            ask_on_abort: capture.screen_record_ask_confirmation_on_abort,
            confirming_abort: false,
            activate,
            drag_from: None,
        });
        log::info!(
            "Starting screen recording. Video encoder: \"{}\", Audio encoder: \"{}\", FPS: {fps}",
            capture.ffmpeg_options.VideoCodec.description(),
            capture.ffmpeg_options.AudioCodec.description()
        );
        self.rt.spawn_blocking(move || {
            let outcome = run_session(params, control);
            move |state: &mut AppState, _ctx: &egui::Context| state.recording_finished(outcome)
        });
    }

    fn recording_finished(&mut self, outcome: SessionOutcome) {
        let Some(recording) = self.captures.recording.take() else {
            return;
        };
        if let Some(error) = outcome.error.filter(|_| !outcome.aborted) {
            // `FFmpegCLIManager.ShowError`: ffmpeg's output in the output box (phase 5).
            self.open_output_box(
                &error,
                modlight_core::l10n::t(modlight_core::l10n::helpers::FFmpegCLIManager_Error_title),
                true,
            );
            return;
        }
        let Some(path) = outcome.path else {
            return;
        };
        self.play_task_sound(NotificationSound::ActionCompleted, &recording.settings);
        self.run_image_task(PendingImage {
            metadata: recording.metadata,
            settings: recording.settings,
            file: Some(path),
            skip_quick_menu: true,
        });
    }

    /// The toolbar's buttons (`StartStopRecording`, `PauseResumeRecording`, ...).
    pub fn recording_action(&mut self, action: RecordingAction) {
        let Some(recording) = self.captures.recording.as_mut() else {
            return;
        };
        let control = recording.control.clone();
        match action {
            RecordingAction::StartStop => control.start_stop(),
            RecordingAction::PauseResume => control.pause_resume(),
            RecordingAction::Restart => control.restart(),
            RecordingAction::Abort => {
                if recording.ask_on_abort {
                    recording.confirming_abort = true;
                } else {
                    control.abort();
                }
            }
            RecordingAction::CancelAbort => recording.confirming_abort = false,
            RecordingAction::ConfirmAbort => {
                recording.confirming_abort = false;
                control.abort();
            }
        }
    }

    /// `StopScreenRecording`.
    pub fn recording_stop(&mut self) {
        if let Some(recording) = &self.captures.recording
            && recording.control.status() == RecordingStatus::Recording
        {
            recording.control.start_stop();
        }
    }

    /// `PauseScreenRecording`.
    pub fn recording_pause(&mut self) {
        if let Some(recording) = &self.captures.recording {
            recording.control.pause_resume();
        }
    }

    /// `AbortScreenRecording`.
    pub fn recording_abort(&mut self) {
        if let Some(recording) = &self.captures.recording {
            recording.control.abort();
        }
    }

    /// Repaint while the timer runs.
    pub fn tick_recording(&mut self, ctx: &egui::Context) {
        if self.captures.recording.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}

/// `HandleExistsFile` with a unique name.
fn unique(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    PathBuf::from(
        modlight_core::tasks::naming::unique_file_path(&path)
            .to_string_lossy()
            .into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use modlight_core::l10n::{Language, set_thread_language};

    /// The recording hotkey pressed again while the area is still being prepared (the
    /// desktop being taken for the overlay) is not a second request: the other ways to
    /// open the overlay wait for `busy` the same way.
    #[test]
    fn a_second_press_while_the_area_is_prepared_asks_nothing_more() {
        use image::RgbaImage;
        use modlight_core::capture::source::{
            CaptureError, CaptureSource, FakeSource, ScreenInfo, WindowCapture, WindowInfo,
        };
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Counting {
            inner: FakeSource,
            taken: Arc<AtomicUsize>,
        }
        impl CaptureSource for Counting {
            fn screens(&self) -> Vec<ScreenInfo> {
                self.inner.screens()
            }
            fn cursor_position(&self) -> (i32, i32) {
                self.inner.cursor_position()
            }
            fn windows(&self) -> Vec<WindowInfo> {
                self.inner.windows()
            }
            fn active_window(&self) -> Option<WindowInfo> {
                self.inner.active_window()
            }
            fn capture_rect(&self, rect: IRect, cursor: bool) -> Result<RgbaImage, CaptureError> {
                self.taken.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(200));
                self.inner.capture_rect(rect, cursor)
            }
            fn capture_window(
                &self,
                window: &WindowInfo,
                how: WindowCapture,
            ) -> Result<RgbaImage, CaptureError> {
                self.inner.capture_window(window, how)
            }
        }

        let profile = tempfile::tempdir().unwrap();
        let tools = profile.path().join(modlight_core::components::TOOLS_FOLDER);
        std::fs::create_dir_all(&tools).unwrap();
        std::fs::write(
            tools.join(modlight_core::download::tool::FFMPEG_FILE_NAME),
            b"x",
        )
        .unwrap();
        let env = crate::state::Env {
            profile: profile.path().to_path_buf(),
            qa: true,
            frozen_now: None,
            machine: "QA Mac".into(),
            system_language: "en".into(),
            system_locale: "en_US".into(),
            reduce_motion: true,
            system_dark: Some(true),
        };
        let account = modlight_core::account::AccountStore::in_memory(
            modlight_core::account::StoredAccount::default(),
            "",
        );
        let mut state = AppState::new(env, modlight_core::config::AppConfig::in_memory(), account);
        let taken = Arc::new(AtomicUsize::new(0));
        state.captures.source = Arc::new(Counting {
            inner: crate::qa_capture::qa_desktop(),
            taken: taken.clone(),
        });
        let settings = state.default_task_settings();
        for _ in 0..2 {
            state.start_screen_recording(
                ScreenRecordOutput::FFmpeg,
                RecordStartMethod::Region,
                settings.clone(),
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(1000));
        assert_eq!(taken.load(Ordering::SeqCst), 1);
    }

    /// While recording the button says upstream's `Strings.Pause` (`UpdateUI`), which
    /// in Polish is "Pauza"; `ScreenRecordWindow_Pause` ("Wstrzymaj") is only the
    /// window's text before `UpdateUI` first runs.
    #[test]
    fn the_buttons_say_what_upstream_says() {
        set_thread_language(Some(Language::Polish));
        assert_eq!(pause_label(RecordingStatus::Recording).0, "Pauza");
        assert_eq!(pause_label(RecordingStatus::Paused).0, "Ponów");
        assert_eq!(pause_label(RecordingStatus::Waiting).0, "Ponów");
        assert_eq!(start_label(RecordingStatus::Waiting).0, "Rozpocznij");
        assert_eq!(start_label(RecordingStatus::Recording).0, "Zatrzymaj");
        set_thread_language(Some(Language::English));
        assert_eq!(pause_label(RecordingStatus::Recording).0, "Pause");
        assert_eq!(start_label(RecordingStatus::Waiting).0, "Start");
        set_thread_language(None);
    }
}
