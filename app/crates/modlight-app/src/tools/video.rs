//! `VideoConverterWindow` and `VideoConverterViewModel`: an input file (browse or drop),
//! the encoder and its quality or bitrate, the output folder and name, and ffmpeg (from the
//! Components window) run with progress, stop, and the folder shown when it is done.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use egui::{Align2, Id, Rect, Sense, Ui, pos2, vec2};
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, s, t, tools as tl};
use modlight_core::recording::ffmpeg::{FFmpegListener, FFmpegRun};
use modlight_core::tasks::host::MessageIcon;
use modlight_core::tasks::settings::TaskSettings;
use modlight_tools::settings::{ConverterVideoCodecs, VideoConverterOptions};
use modlight_tools::video::{self, Conversion};

use super::jobs::{OptionsHome, ToolContinuation};
use super::kit::{self, Track};
use super::{ToolWindow, WindowSpec};
use crate::state::{AppState, Effect};
use crate::theme::Weight;

/// What a running conversion shares with the window.
#[derive(Default)]
pub struct Control {
    pub stop: AtomicBool,
    pub progress: Mutex<f64>,
}

impl FFmpegListener for Control {
    fn encode_progress(&self, percentage: f32) {
        *self.progress.lock().unwrap() = (percentage as f64).clamp(0.0, 100.0);
    }
}

/// `VideoConversionResult`.
#[derive(Clone, Debug, PartialEq)]
pub struct ConversionResult {
    pub succeeded: bool,
    pub cancelled: bool,
    pub error: Option<String>,
}

/// `RunVideoConversionAsync`: ffmpeg to the end, or until stopped: the cancellation
/// calls `ffmpeg.Close` once, which is one `q`, and ffmpeg then finishes the file however
/// long that takes (a kill half way through the index would leave it unreadable).
pub fn run_conversion(
    ffmpeg: &std::path::Path,
    args: &[String],
    control: Arc<Control>,
) -> ConversionResult {
    let mut run = match FFmpegRun::start(ffmpeg, args, false, true, control.clone()) {
        Ok(run) => run,
        Err(error) => {
            return ConversionResult {
                succeeded: false,
                cancelled: false,
                error: Some(error.to_string()),
            };
        }
    };
    let mut asked = false;
    while run.is_running() {
        if !asked && control.stop.load(Ordering::SeqCst) {
            // `cancellationToken.Register(ffmpeg.Close)`: one `q`.
            run.close();
            asked = true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let cancelled = control.stop.load(Ordering::SeqCst);
    let (ok, output) = run.wait();
    ConversionResult {
        succeeded: ok,
        cancelled,
        error: (!ok && !cancelled)
            .then(|| video::last_line(&output))
            .flatten(),
    }
}

pub struct VideoWindow {
    pub key: u64,
    pub settings: TaskSettings,
    pub home: OptionsHome,
    pub options: VideoConverterOptions,
    pub conversion: Conversion,
    pub encoding: bool,
    pub progress: f64,
    pub status: String,
    control: Option<Arc<Control>>,
    ffmpeg: Option<PathBuf>,
}

impl VideoWindow {
    pub fn new(
        key: u64,
        settings: TaskSettings,
        ffmpeg: Option<PathBuf>,
        input: Option<String>,
    ) -> VideoWindow {
        let options: VideoConverterOptions =
            super::jobs::tool_options(&settings, "VideoConverterOptions");
        let home = OptionsHome::of(&settings);
        let conversion = Conversion::from_options(&options, input.as_deref());
        let status = if conversion.input.trim().is_empty() {
            t(tl::VideoConverterViewModel_Choose_video_or_animation).to_string()
        } else {
            String::new()
        };
        VideoWindow {
            key,
            settings,
            home,
            options,
            conversion,
            encoding: false,
            progress: 0.0,
            status,
            control: None,
            ffmpeg,
        }
    }

    /// `PersistSettings`, written where the options came from.
    fn persist(&mut self, state: &mut AppState) {
        self.conversion.write_options(&mut self.options);
        state.save_tool_options(&self.home, "VideoConverterOptions", &self.options);
    }

    fn load_input(&mut self, state: &mut AppState, path: &str) {
        self.conversion.load_input(path);
        self.status.clear();
        self.persist(state);
    }

    /// `StartEncodingAsync`.
    pub fn start(&mut self, state: &mut AppState) {
        if self.encoding {
            return;
        }
        if !std::path::Path::new(&self.conversion.input).is_file() {
            self.status = t(tl::VideoConverterViewModel_Select_existing_input).to_string();
            return;
        }
        let folder = self.conversion.output_folder.trim().to_string();
        if folder.is_empty() || !std::path::Path::new(&folder).is_dir() {
            self.status = t(tl::VideoConverterViewModel_Select_existing_output).to_string();
            return;
        }
        if self.conversion.output_name.trim().is_empty() {
            self.status = t(tl::VideoConverterViewModel_Enter_output_name).to_string();
            return;
        }
        self.persist(state);
        let Some(ffmpeg) = self.ffmpeg.clone() else {
            self.status = format(
                t(tl::VideoConverterViewModel_Conversion_failed_message),
                &[Arg::from(t(tl::VideoConverterWindow_FFmpeg_unavailable))],
            );
            return;
        };
        self.encoding = true;
        self.progress = 0.0;
        self.status = t(tl::VideoConverterViewModel_Converting).to_string();
        let control = Arc::new(Control::default());
        self.control = Some(control.clone());
        let args = self.conversion.arguments();
        let output = self.conversion.output_path();
        let auto_open = self.conversion.auto_open_folder;
        let key = self.key;
        state.rt.spawn_blocking(move || {
            let result = run_conversion(&ffmpeg, &args, control);
            move |state: &mut AppState, _ctx: &egui::Context| {
                if result.succeeded && !result.cancelled && auto_open {
                    // `FileHelpers.OpenFolderWithFile`.
                    state.effects.push(Effect::Reveal(PathBuf::from(&output)));
                }
                if let Some(window) = state.tools.window_mut::<VideoWindow>(key) {
                    window.finished(result, &output);
                }
            }
        });
    }

    /// The ending of `StartEncodingAsync`.
    pub fn finished(&mut self, result: ConversionResult, output: &str) {
        self.encoding = false;
        self.control = None;
        if result.succeeded && !result.cancelled {
            self.progress = 100.0;
            self.status = format(
                t(tl::VideoConverterViewModel_Conversion_complete),
                &[Arg::from(output)],
            );
        } else if result.cancelled {
            self.progress = 0.0;
            self.status = t(tl::VideoConverterViewModel_Conversion_stopped).to_string();
        } else {
            self.progress = 0.0;
            self.status = match result.error.filter(|e| !e.trim().is_empty()) {
                Some(error) => format(
                    t(tl::VideoConverterViewModel_Conversion_failed_message),
                    &[Arg::from(error)],
                ),
                None => t(tl::VideoConverterViewModel_Conversion_failed).to_string(),
            };
        }
    }

    /// `StopEncoding`.
    pub fn stop(&mut self) {
        if self.encoding {
            self.status = t(tl::VideoConverterViewModel_Stopping).to_string();
            if let Some(control) = &self.control {
                control.stop.store(true, Ordering::SeqCst);
            }
        }
    }
}

impl ToolWindow for VideoWindow {
    fn spec(&self) -> WindowSpec {
        WindowSpec::resizable(
            t(tl::VideoConverterWindow_ShareX_Video_Converter),
            vec2(850.0, 475.0),
            vec2(700.0, 475.0),
        )
    }

    fn ui(&mut self, ui: &mut Ui, state: &mut AppState) -> bool {
        let colors = state.colors.clone();
        kit::fill(ui, &colors);
        if let Some(control) = &self.control {
            self.progress = *control.progress.lock().unwrap();
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
        }
        let idle = !self.encoding;
        // A file dropped on the window becomes the input.
        let dropped: Option<PathBuf> = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .find(|p| p.is_file())
        });
        if let Some(path) = dropped
            && idle
        {
            self.load_input(state, &path.to_string_lossy());
        }

        let full = kit::inset(ui.max_rect(), 16.0);
        let footer_h = if self.encoding {
            6.0 + 8.0 + 40.0
        } else {
            40.0
        };
        let rows = kit::rows(
            full,
            &[
                Track::Px(12.0 * 2.0 + 40.0),
                Track::Star(1.0),
                Track::Px(footer_h),
            ],
            12.0,
        );

        // The input file.
        kit::panel(ui, &colors, rows[0]);
        let inner = rows[0].shrink(12.0);
        let heading = t(tl::VideoConverterWindow_Input_file);
        let heading_w = kit::text_width(ui, heading, 15.0, Weight::SemiBold);
        let browse_w =
            kit::button_width(ui, &colors, None, Some(t(tl::VideoConverterWindow_Browse)));
        let cols = kit::columns(
            inner,
            &[Track::Px(heading_w), Track::Star(1.0), Track::Px(browse_w)],
            12.0,
        );
        kit::heading(ui, &colors, cols[0], heading);
        let display = if self.conversion.input.trim().is_empty() {
            t(tl::VideoConverterViewModel_No_file_selected).to_string()
        } else {
            self.conversion.input.clone()
        };
        kit::path_text(ui, &colors, cols[1], &display);
        if kit::button_at(
            ui,
            &colors,
            cols[2],
            None,
            Some(t(tl::VideoConverterWindow_Browse)),
            idle,
        )
        .clicked()
            && let Some(path) = state.tool_open_file(
                t(tl::VideoConverterViewModel_Select_input_dialog),
                &[
                    (
                        t(tl::VideoConverterWindow_Video_animation_files),
                        &[
                            "mp4", "mkv", "webm", "avi", "mov", "m4v", "wmv", "gif", "webp", "png",
                            "apng",
                        ],
                    ),
                    ("*", &["*"]),
                ],
            )
        {
            self.load_input(state, &path.to_string_lossy());
        }

        let body = kit::columns(rows[1], &[Track::Px(360.0), Track::Star(1.0)], 12.0);
        // Encoding and quality.
        kit::panel(ui, &colors, body[0]);
        let inner = body[0].shrink(16.0);
        let mut y = inner.top();
        let line =
            |y: f32, h: f32| Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), h));
        kit::heading(
            ui,
            &colors,
            line(y, kit::HEADING_HEIGHT),
            t(tl::VideoConverterWindow_Encoding),
        );
        y += kit::HEADING_HEIGHT + 10.0;
        kit::label(
            ui,
            &colors,
            line(y, 16.0),
            t(tl::VideoConverterWindow_Video_encoder),
        );
        y += 16.0 + 5.0;
        let mut codecs: Vec<(ConverterVideoCodecs, String)> = video::OFFERED
            .iter()
            .map(|c| (*c, c.description().to_string()))
            .collect();
        if !video::OFFERED.contains(&self.conversion.codec) {
            // A Windows hardware encoder synced from another install: shown as it is.
            codecs.insert(
                0,
                (
                    self.conversion.codec,
                    self.conversion.codec.description().to_string(),
                ),
            );
        }
        let mut changed = false;
        if let Some(codec) = kit::combo_at(
            ui,
            &colors,
            line(y, 40.0),
            &format!("video-codec-{}", self.key),
            &self.conversion.codec,
            &codecs,
            idle,
        ) {
            self.conversion.set_codec(codec);
            changed = true;
        }
        y += 40.0 + 18.0;
        if self.conversion.shows_quality_controls() {
            kit::heading(
                ui,
                &colors,
                line(y, kit::HEADING_HEIGHT),
                t(tl::VideoConverterWindow_Quality),
            );
            y += kit::HEADING_HEIGHT + 10.0;
            if self.conversion.can_choose_rate_control() {
                if kit::check_normal_at(
                    ui,
                    &colors,
                    line(y, 32.0),
                    &mut self.conversion.use_bitrate,
                    t(tl::VideoConverterWindow_Use_a_target_bitrate),
                    idle,
                ) {
                    changed = true;
                }
                y += 32.0 + 10.0;
            }
            if self.conversion.shows_quality_slider() {
                let r = line(y, 16.0);
                kit::label(ui, &colors, r, t(tl::VideoConverterWindow_Constant_quality));
                kit::text_at(
                    ui,
                    r,
                    Align2::RIGHT_CENTER,
                    &format!("{:.0}", self.conversion.quality),
                    13.0,
                    Weight::SemiBold,
                    colors.text_secondary(),
                );
                y += 16.0 + 4.0;
                let slider = line(y, 24.0);
                let min = self.conversion.quality_minimum();
                let max = self.conversion.quality_maximum();
                let mut value = self.conversion.quality;
                if slider_at(
                    ui,
                    &colors,
                    slider,
                    Id::new(("video-quality", self.key)),
                    &mut value,
                    min,
                    max,
                    idle,
                ) {
                    self.conversion.quality = value.round();
                    changed = true;
                }
                y += 24.0 + 4.0;
                let r = line(y, 16.0);
                kit::text_at(
                    ui,
                    r,
                    Align2::LEFT_CENTER,
                    t(tl::VideoConverterWindow_Higher_quality),
                    12.0,
                    Weight::Normal,
                    colors.text_secondary(),
                );
                kit::text_at(
                    ui,
                    r,
                    Align2::RIGHT_CENTER,
                    t(tl::VideoConverterWindow_Smaller_file),
                    12.0,
                    Weight::Normal,
                    colors.text_secondary(),
                );
            }
            if self.conversion.shows_bitrate() {
                kit::label(
                    ui,
                    &colors,
                    line(y, 16.0),
                    t(tl::VideoConverterWindow_Target_bitrate),
                );
                y += 16.0 + 5.0;
                let kbps_w =
                    kit::text_width(ui, t(tl::VideoConverterWindow_kbps), 13.0, Weight::Normal);
                let c = kit::columns(line(y, 40.0), &[Track::Star(1.0), Track::Px(kbps_w)], 8.0);
                if kit::int_at(
                    ui,
                    &colors,
                    c[0],
                    &format!("video-bitrate-{}", self.key),
                    &mut self.conversion.bitrate,
                    100,
                    100_000,
                    100,
                    idle,
                ) {
                    changed = true;
                }
                kit::text_at(
                    ui,
                    c[1],
                    Align2::LEFT_CENTER,
                    t(tl::VideoConverterWindow_kbps),
                    13.0,
                    Weight::Normal,
                    colors.text_secondary(),
                );
            }
        }

        // The output.
        kit::panel(ui, &colors, body[1]);
        let inner = body[1].shrink(16.0);
        let mut y = inner.top();
        let line =
            |y: f32, h: f32| Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), h));
        kit::heading(
            ui,
            &colors,
            line(y, kit::HEADING_HEIGHT),
            t(tl::VideoConverterWindow_Output),
        );
        y += kit::HEADING_HEIGHT + 10.0;
        kit::label(
            ui,
            &colors,
            line(y, 16.0),
            t(tl::VideoConverterWindow_Folder),
        );
        y += 16.0 + 5.0;
        let folder_icon_w = kit::button_width(ui, &colors, Some(icons::FOLDER_OPEN), None);
        let c = kit::columns(
            line(y, 40.0),
            &[Track::Star(1.0), Track::Px(folder_icon_w)],
            8.0,
        );
        if kit::text_box_at(
            ui,
            &colors,
            c[0],
            Id::new(("video-folder", self.key)),
            &mut self.conversion.output_folder,
            t(tl::VideoConverterWindow_Choose_an_output_folder),
            idle,
            false,
        )
        .changed()
        {
            changed = true;
        }
        if kit::button_at(ui, &colors, c[1], Some(icons::FOLDER_OPEN), None, idle).clicked() {
            let start = PathBuf::from(&self.conversion.output_folder);
            if let Some(folder) = state.tool_pick_folder(
                t(tl::VideoConverterViewModel_Select_output_dialog),
                Some(&start),
            ) {
                self.conversion.output_folder = folder.to_string_lossy().into_owned();
                changed = true;
            }
        }
        y += 40.0 + 12.0;
        kit::label(
            ui,
            &colors,
            line(y, 16.0),
            t(tl::VideoConverterWindow_File_name),
        );
        y += 16.0 + 5.0;
        if kit::text_box_at(
            ui,
            &colors,
            line(y, 40.0),
            Id::new(("video-name", self.key)),
            &mut self.conversion.output_name,
            t(tl::VideoConverterWindow_video_output),
            idle,
            false,
        )
        .changed()
        {
            changed = true;
        }
        y += 40.0 + 8.0;
        kit::path_text(ui, &colors, line(y, 18.0), &self.conversion.output_path());
        y += 18.0 + 12.0;
        if kit::check_normal_at(
            ui,
            &colors,
            line(y, 32.0),
            &mut self.conversion.auto_open_folder,
            t(tl::VideoConverterWindow_Open_the_output_folder_when_finished),
            idle,
        ) {
            changed = true;
        }
        if changed {
            self.persist(state);
        }

        // Progress, status, start or stop.
        let mut footer = rows[2];
        if self.encoding {
            let bar = Rect::from_min_size(footer.min, vec2(footer.width(), 6.0));
            crate::widgets::paint_progress(ui, &colors, bar, Some((self.progress / 100.0) as f32));
            footer = Rect::from_min_max(pos2(footer.left(), bar.bottom() + 8.0), footer.max);
        }
        let item = if self.encoding {
            (
                Some(icons::SQUARE_STOP),
                Some(t(tl::VideoConverterWindow_Stop_encoding)),
            )
        } else {
            (
                Some(icons::PLAY),
                Some(t(tl::VideoConverterWindow_Start_encoding)),
            )
        };
        let button = kit::right_aligned(ui, &colors, footer, &[item], 8.0)[0];
        let status = Rect::from_min_max(footer.min, pos2(button.left() - 12.0, footer.bottom()));
        let galley = crate::widgets::layout(
            ui,
            crate::widgets::wrapped(
                &self.status,
                13.0,
                Weight::Normal,
                colors.text_secondary(),
                status.width().max(1.0),
            ),
        );
        let pos = pos2(status.left(), status.center().y - galley.size().y / 2.0);
        ui.painter().galley(pos, galley, colors.text_secondary());
        if kit::button_at(ui, &colors, button, item.0, item.1, true).clicked() {
            if self.encoding {
                self.stop();
            } else {
                self.start(state);
            }
        }
        false
    }

    fn closed(&mut self, _state: &mut AppState) {
        // `Dispose`: a running conversion is stopped.
        if let Some(control) = &self.control {
            control.stop.store(true, Ordering::SeqCst);
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Fluent's `Slider` (`TickFrequency="1" IsSnapToTickEnabled="True"`).
#[allow(clippy::too_many_arguments)]
pub fn slider_at(
    ui: &mut Ui,
    colors: &crate::theme::Colors,
    rect: Rect,
    id: Id,
    value: &mut f64,
    min: f64,
    max: f64,
    enabled: bool,
) -> bool {
    let track = Rect::from_min_max(
        pos2(rect.left() + 10.0, rect.center().y - 2.0),
        pos2(rect.right() - 10.0, rect.center().y + 2.0),
    );
    let response = ui.interact(
        rect,
        id,
        if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    let mut changed = false;
    if let Some(p) = response.interact_pointer_pos()
        && (response.dragged() || response.clicked() || response.drag_started())
    {
        let f = ((p.x - track.left()) / track.width()).clamp(0.0, 1.0) as f64;
        let v = (min + f * (max - min)).round();
        if v != *value {
            *value = v;
            changed = true;
        }
    }
    let f = if max > min {
        ((*value - min) / (max - min)).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    let dim = if enabled { 1.0 } else { 0.35 };
    let painter = ui.painter();
    painter.rect_filled(
        track,
        2.0,
        colors.control_background_hover().gamma_multiply(dim),
    );
    let filled = Rect::from_min_max(
        track.min,
        pos2(track.left() + track.width() * f, track.bottom()),
    );
    painter.rect_filled(filled, 2.0, colors.accent_start().gamma_multiply(dim));
    let knob = pos2(track.left() + track.width() * f, track.center().y);
    painter.circle_filled(knob, 10.0, colors.control_background().gamma_multiply(dim));
    painter.circle_stroke(knob, 10.0, egui::Stroke::new(1.0, colors.border()));
    painter.circle_filled(knob, 6.0, colors.accent_start().gamma_multiply(dim));
    changed
}

impl AppState {
    /// `OpenVideoConverter(filePath)`: `CheckFFmpeg`, then the window.
    pub fn open_video_converter(&mut self, file: Option<PathBuf>, settings: TaskSettings) {
        let ffmpeg = crate::recording_ui::ffmpeg_path(&settings, &self.env.profile, self.env.qa);
        if ffmpeg.is_none() {
            let then = crate::components_window::AfterClose::Tool(Box::new(
                ToolContinuation::VideoConverter {
                    file: file.clone(),
                    settings: Box::new(settings.clone()),
                },
            ));
            if !crate::components_window::require(
                self,
                "ffmpeg",
                s::Components_FeatureRecording,
                then,
            ) {
                return;
            }
            self.uploads
                .windows
                .messages
                .push_back(crate::dialogs::message::MessageBox::new(
                    &format!("Modlight - {}", t(s::FFmpegIsMissing)),
                    t(s::FFmpegDoesNotExistAtTheFollowingPath),
                    MessageIcon::Warning,
                ));
            return;
        }
        let input = file.map(|f| f.to_string_lossy().into_owned());
        self.tools
            .open_keyed(|key| Box::new(VideoWindow::new(key, settings, ffmpeg, input)));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// `StopEncoding` cancels once, which is one `q` (`ffmpeg.Close`); ffmpeg then
    /// finishes the file (writing the index, `+faststart` moving it to the front), however
    /// long that takes. A stand-in reads the `q` and takes 3 s to finish.
    #[test]
    fn a_stop_lets_ffmpeg_finish_the_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let finished = dir.path().join("finished");
        let script = dir.path().join("ffmpeg");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nhead -c 1 >/dev/null\nsleep 3\necho done > '{}'\nexit 0\n",
                finished.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let control = Arc::new(Control::default());
        control.stop.store(true, Ordering::SeqCst);
        let result = run_conversion(&script, &["-i".into(), "in.mov".into()], control);
        assert!(result.cancelled, "{result:?}");
        assert!(finished.is_file(), "ffmpeg was killed before it finished");
    }
}
