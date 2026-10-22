//! `FFmpegOptionsWindow`: the screen recording options (sources, video, audio,
//! advanced) for one task settings object, opened from the screen recorder page.
//!
//! On Windows the sources are the screen (GDI frames piped to ffmpeg, upstream's
//! `gdigrab` title; a `ddagrab` or `screen-capture-recorder` value selects it too) and
//! the cameras and microphones ffmpeg's DirectShow input lists, with upstream's texts;
//! NVENC, AMF and Quick Sync are offered with all their settings. "Get recorder devices"
//! (the screen-capture-recorder installer) is not ported: the screen does not go
//! through a DirectShow filter here.
//!
//! On a Mac the sources are the screen (ScreenCaptureKit) and the cameras and
//! microphones ffmpeg's avfoundation input lists; NVENC, AMF and Quick Sync are not
//! offered (a value synced from Windows is encoded with VideoToolbox and keeps only its
//! bitrate), and there is no "Get recorder devices" (screen-capture-recorder is a
//! DirectShow filter). Where upstream's text names DirectShow, the Mac's says
//! AVFoundation (`Mac.resx`).

use std::path::PathBuf;

use egui::{Align, Color32, Layout, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

use modlight_core::icons;
#[cfg(not(windows))]
use modlight_core::l10n::mac;
use modlight_core::l10n::{Arg, Key, capture as c, format, t};
use modlight_core::recording::args::{Devices, ScreenRecordingOptions};
use modlight_core::recording::options::{
    FFmpegAMFQuality, FFmpegAMFUsage, FFmpegAudioCodec, FFmpegNVENCPreset, FFmpegNVENCTune,
    FFmpegOptions, FFmpegPaletteGenStatsMode, FFmpegPaletteUseDither, FFmpegPreset,
    FFmpegQSVPreset, FFmpegVideoCodec, capture_device,
};

use crate::state::AppState;
use crate::task_pages::TaskTarget;
use crate::theme::{Colors, Weight};
use crate::widgets::{
    Content, Field, Look, button, checkbox_weighted, combo, layout, paint_icon, trimmed, wrapped,
};

pub const WIDTH: f32 = 940.0;
pub const HEIGHT: f32 = 720.0;
pub const MIN_WIDTH: f32 = 760.0;
pub const MIN_HEIGHT: f32 = 620.0;

/// Goldenrod, the warning borders' colour.
const GOLDENROD: Color32 = Color32::from_rgb(0xDA, 0xA5, 0x20);

/// One entry of the source lists (`FFmpegCaptureDevice`).
#[derive(Clone, Debug, PartialEq)]
pub struct CaptureDevice {
    pub value: String,
    pub title: String,
}

impl CaptureDevice {
    fn none() -> CaptureDevice {
        CaptureDevice {
            value: capture_device::NONE.into(),
            title: "None".into(),
        }
    }

    fn screen() -> CaptureDevice {
        CaptureDevice {
            value: capture_device::SCREEN.into(),
            title: capture_device::SCREEN_TITLE.into(),
        }
    }

    fn device(name: &str) -> CaptureDevice {
        CaptureDevice {
            value: name.into(),
            title: capture_device::device_title(name),
        }
    }

    /// `EqualsSource`; every screen value is the screen entry.
    fn matches(&self, value: &str) -> bool {
        if self.value == capture_device::SCREEN {
            capture_device::is_screen(value)
        } else {
            self.value.eq_ignore_ascii_case(value)
        }
    }
}

/// The window's state.
pub struct FFmpegWindow {
    pub target: TaskTarget,
    pub options: ScreenRecordingOptions,
    pub tab: usize,
    pub confirm_reset: bool,
    pub video_sources: Vec<CaptureDevice>,
    pub audio_sources: Vec<CaptureDevice>,
    pub status: String,
    pub refreshing: bool,
    generation: u64,
    /// The FFmpeg the Components window installed, if any.
    installed: Option<PathBuf>,
    /// Where an installed FFmpeg would be (for "not found at").
    expected: PathBuf,
}

impl FFmpegWindow {
    /// `ShowScreenRecordingOptions`: the options of `target`, the primary display as the
    /// area, `output.mp4`.
    pub fn new(state: &AppState, target: TaskTarget) -> FFmpegWindow {
        let settings = state.task_settings_of(target);
        let capture = &settings.capture_settings;
        let screen = state
            .captures
            .source
            .screens()
            .into_iter()
            .find(|s| s.primary);
        let (area, scale) = screen.map(|s| (s.bounds, s.scale)).unwrap_or((
            modlight_core::capture::geometry::IRect::new(0, 0, 1920, 1080),
            1.0,
        ));
        let options = ScreenRecordingOptions {
            is_recording: true,
            ffmpeg: capture.ffmpeg_options.clone(),
            fps: capture.screen_record_fps,
            duration: if capture.screen_record_fixed_duration {
                capture.screen_record_duration
            } else {
                0.0
            },
            output_path: PathBuf::from("output.mp4"),
            capture_area: area,
            frame_size: (
                (area.width as f64 * scale).round() as u32,
                (area.height as f64 * scale).round() as u32,
            ),
            draw_cursor: capture.screen_record_show_cursor,
            ..ScreenRecordingOptions::default()
        };
        FFmpegWindow {
            target,
            options,
            tab: 0,
            confirm_reset: false,
            video_sources: vec![CaptureDevice::none(), CaptureDevice::screen()],
            audio_sources: vec![CaptureDevice::none()],
            status: t(c::FFmpegOptionsWindow_Looking_for_devices).to_string(),
            refreshing: false,
            generation: 0,
            installed: crate::recording_ui::installed_ffmpeg(&state.env.profile, state.env.qa),
            expected: state
                .env
                .profile
                .join(modlight_core::components::TOOLS_FOLDER)
                .join(modlight_core::download::tool::FFMPEG_FILE_NAME),
        }
    }

    /// `FFmpegOptions.FFmpegPath`.
    fn ffmpeg_path(&self) -> PathBuf {
        let ffmpeg = &self.options.ffmpeg;
        if ffmpeg.OverrideCLIPath && !ffmpeg.CLIPath.is_empty() {
            return PathBuf::from(&ffmpeg.CLIPath);
        }
        self.installed
            .clone()
            .unwrap_or_else(|| self.expected.clone())
    }

    /// The lists and the status once the devices are known (`RefreshSourcesAsync`'s end).
    pub fn sources_found(&mut self, devices: Option<Result<Devices, String>>) {
        let mut video = vec![CaptureDevice::none(), CaptureDevice::screen()];
        let mut audio = vec![CaptureDevice::none()];
        if let Some(Ok(devices)) = &devices {
            video.extend(devices.video.iter().map(|d| CaptureDevice::device(d)));
            audio.extend(devices.audio.iter().map(|d| CaptureDevice::device(d)));
        }
        let ffmpeg = &mut self.options.ffmpeg;
        if !video.iter().any(|d| d.matches(&ffmpeg.VideoSource)) {
            ffmpeg.VideoSource = capture_device::SCREEN.into();
        }
        if !audio.iter().any(|d| d.matches(&ffmpeg.AudioSource)) {
            ffmpeg.AudioSource = capture_device::NONE.into();
        }
        self.status = match &devices {
            None => format(
                t(c::FFmpegOptionsWindow_FFmpeg_not_found),
                &[Arg::from(self.ffmpeg_path().display().to_string())],
            ),
            Some(Err(error)) => {
                log::warn!("FFmpeg devices: {error}");
                t(texts::ENUMERATION_FAILED).to_string()
            }
            Some(Ok(_)) => {
                let count = (video.len() - 2) + (audio.len() - 1);
                match count {
                    0 => t(texts::NO_DEVICES).to_string(),
                    1 => format(t(texts::ONE_DEVICE), &[Arg::from(count as i64)]),
                    _ => format(t(texts::DEVICES), &[Arg::from(count as i64)]),
                }
            }
        };
        self.video_sources = video;
        self.audio_sources = audio;
        self.refreshing = false;
    }
}

/// The texts that name the device framework: upstream's (DirectShow) on Windows, the
/// Mac's (AVFoundation, `Mac.resx`) on a Mac.
mod texts {
    use modlight_core::l10n::Key;
    #[cfg(windows)]
    use modlight_core::l10n::capture as c;
    #[cfg(not(windows))]
    use modlight_core::l10n::mac;

    #[cfg(windows)]
    pub const ENUMERATION_FAILED: Key = c::FFmpegOptionsWindow_Device_enumeration_failed;
    #[cfg(windows)]
    pub const NO_DEVICES: Key = c::FFmpegOptionsWindow_No_devices_found;
    #[cfg(windows)]
    pub const ONE_DEVICE: Key = c::FFmpegOptionsWindow_One_device_found;
    #[cfg(windows)]
    pub const DEVICES: Key = c::FFmpegOptionsWindow_Multiple_devices_found;
    #[cfg(windows)]
    pub const PATH_PLACEHOLDER: Key = c::FFmpegOptionsWindow_Path_to_ffmpeg;

    #[cfg(not(windows))]
    pub const ENUMERATION_FAILED: Key = mac::FFmpegDeviceEnumerationFailed;
    #[cfg(not(windows))]
    pub const NO_DEVICES: Key = mac::FFmpegNoDevicesFound;
    #[cfg(not(windows))]
    pub const ONE_DEVICE: Key = mac::FFmpegOneDeviceFound;
    #[cfg(not(windows))]
    pub const DEVICES: Key = mac::FFmpegMultipleDevicesFound;
    #[cfg(not(windows))]
    pub const PATH_PLACEHOLDER: Key = mac::FFmpegPathPlaceholder;
}

impl AppState {
    /// Open the window for `target` (one at a time) and look for devices.
    pub fn open_ffmpeg_options(&mut self, target: TaskTarget) {
        if self.captures.ffmpeg_window.is_some() {
            return;
        }
        self.captures.ffmpeg_window = Some(FFmpegWindow::new(self, target));
        self.refresh_ffmpeg_devices();
    }

    /// `RefreshSourcesAsync`: ffmpeg lists the devices off the UI thread. Automated runs
    /// list a fixed camera and microphone instead of running anything.
    pub fn refresh_ffmpeg_devices(&mut self) {
        let qa = self.env.qa || self.mode != crate::app::Mode::Normal;
        let Some(window) = self.captures.ffmpeg_window.as_mut() else {
            return;
        };
        window.refreshing = true;
        window.status = t(c::FFmpegOptionsWindow_Looking_for_devices).to_string();
        window.generation += 1;
        let generation = window.generation;
        let path = window.ffmpeg_path();
        if qa {
            let devices = Devices {
                video: vec!["FaceTime HD Camera".into()],
                audio: vec!["MacBook Pro Microphone".into()],
            };
            window.sources_found(Some(Ok(devices)));
            return;
        }
        self.rt.spawn_blocking(move || {
            let found = path.is_file().then(|| {
                modlight_core::recording::ffmpeg::list_devices(&path).map_err(|e| e.to_string())
            });
            move |state: &mut AppState, _ctx: &egui::Context| {
                if let Some(window) = state.captures.ffmpeg_window.as_mut()
                    && window.generation == generation
                {
                    window.sources_found(found);
                }
            }
        });
    }

    /// `BrowseForFFmpegAsync`: an open panel (never in automated runs); a pick turns the
    /// custom path on and looks for devices with it.
    pub fn browse_for_ffmpeg(&mut self) {
        if self.env.qa || self.mode != crate::app::Mode::Normal {
            return;
        }
        let Some(window) = self.captures.ffmpeg_window.as_mut() else {
            return;
        };
        // `Program Files` on Windows (`SpecialFolder.ProgramFiles`), `/Applications` on
        // a Mac, when the current path has no folder.
        let fallback = if cfg!(windows) {
            modlight_core::names::file_helpers::special_folder_path("ProgramFiles")
                .unwrap_or_default()
        } else {
            "/Applications".to_string()
        };
        let start = window
            .ffmpeg_path()
            .parent()
            .filter(|p| p.is_dir())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(fallback));
        let Some(path) = rfd::FileDialog::new()
            .set_title(t(c::FFmpegOptionsWindow_Browse_for_ffmpeg))
            .set_directory(start)
            .pick_file()
        else {
            return;
        };
        window.options.ffmpeg.OverrideCLIPath = true;
        window.options.ffmpeg.CLIPath = path.display().to_string();
        self.refresh_ffmpeg_devices();
    }

    /// The window closed: its options go back to the settings it came from.
    pub fn ffmpeg_options_closed(&mut self) {
        let Some(window) = self.captures.ffmpeg_window.take() else {
            return;
        };
        if let Ok(value) = serde_json::to_value(&window.options.ffmpeg) {
            self.set_task_member(window.target, &["CaptureSettings", "FFmpegOptions"], value);
        }
    }
}

/// What a click asked for that the window cannot do itself.
#[derive(Clone, Debug, PartialEq)]
pub enum FFmpegAsk {
    Refresh,
    Browse,
}

// ------------------------------------------------------------------ drawing

fn section_card(ui: &mut Ui, colors: &Colors, spacing: f32, add: impl FnOnce(&mut Ui)) {
    egui::Frame::NONE
        .fill(colors.background_panel())
        .stroke(Stroke::new(1.0, colors.border()))
        .corner_radius(8.0)
        .inner_margin(16.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = spacing;
            add(ui);
        });
}

fn info_card(ui: &mut Ui, colors: &Colors, border: Option<Color32>, add: impl FnOnce(&mut Ui)) {
    egui::Frame::NONE
        .fill(colors.background_toolbar())
        .stroke(Stroke::new(1.0, border.unwrap_or(colors.border())))
        .corner_radius(6.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

fn section_title(ui: &mut Ui, colors: &Colors, text: &str) {
    let galley = layout(
        ui,
        trimmed(
            text,
            16.0,
            Weight::SemiBold,
            colors.text(),
            ui.available_width(),
            1,
        ),
    );
    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter().galley(rect.min, galley, colors.text());
}

fn field_label(ui: &mut Ui, colors: &Colors, text: &str) {
    let galley = layout(
        ui,
        trimmed(
            text,
            13.0,
            Weight::Normal,
            colors.text_secondary(),
            ui.available_width(),
            1,
        ),
    );
    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter()
        .galley(rect.min, galley, colors.text_secondary());
    ui.add_space(5.0);
}

fn hint(ui: &mut Ui, colors: &Colors, text: &str, width: f32) {
    let galley = layout(
        ui,
        wrapped(text, 12.0, Weight::Normal, colors.text_secondary(), width),
    );
    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter()
        .galley(rect.min, galley, colors.text_secondary());
}

/// An icon beside wrapped hint text (the info and warning cards).
fn icon_hint(ui: &mut Ui, colors: &Colors, glyph: char, color: Color32, text: &str) {
    let width = ui.available_width();
    let galley = layout(
        ui,
        wrapped(
            text,
            12.0,
            Weight::Normal,
            colors.text_secondary(),
            width - 24.0,
        ),
    );
    let height = galley.size().y.max(16.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    paint_icon(
        ui,
        ui.painter(),
        pos2(rect.left() + 7.0, rect.center().y),
        glyph,
        14.0,
        color,
    );
    ui.painter().galley(
        pos2(rect.left() + 22.0, rect.center().y - galley.size().y / 2.0),
        galley,
        colors.text_secondary(),
    );
}

/// `NumericUpDown.number`: 150 wide.
fn number(
    ui: &mut Ui,
    colors: &Colors,
    id: &str,
    value: &mut i64,
    min: f64,
    max: f64,
    step: f64,
) -> bool {
    let mut v = *value as f64;
    let enabled = ui.is_enabled();
    if crate::controls::number_box(
        ui, colors, id, 150.0, 32.0, &mut v, min, max, step, 0, enabled,
    ) {
        *value = v.round() as i64;
        return true;
    }
    false
}

/// `ComboBox.field`: stretches, at least 220.
/// A field label over a list whose entries are upstream's texts, one per value in the
/// enum's order (the hardware encoders' `ItemsSource` arrays).
fn labelled_choice<T: Copy + PartialEq>(
    ui: &mut Ui,
    colors: &Colors,
    label: Key,
    id: &str,
    values: &[T],
    texts: &[Key],
    current: &mut T,
) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        field_label(ui, colors, t(label));
        let list: Vec<(T, String)> = values
            .iter()
            .zip(texts)
            .map(|(value, key)| (*value, t(*key).to_string()))
            .collect();
        if let Some(choice) = field_combo(ui, colors, id, current, &list) {
            *current = choice;
        }
    });
}

fn field_combo<T: Clone + PartialEq>(
    ui: &mut Ui,
    colors: &Colors,
    id: &str,
    current: &T,
    list: &[(T, String)],
) -> Option<T> {
    let width = ui.available_width().max(220.0);
    if ui.is_enabled() {
        combo(ui, colors, id, width, current, list)
    } else {
        let shown: Vec<(T, String)> = list.iter().filter(|(v, _)| v == current).cloned().collect();
        let _ = combo(ui, colors, id, width, current, &[]);
        let _ = shown;
        None
    }
}

/// The window. Returns asks for the app.
pub fn ui(ui: &mut Ui, window: &mut FFmpegWindow, colors: &Colors) -> Vec<FFmpegAsk> {
    let mut asks = Vec::new();
    let full = ui.max_rect();
    ui.painter()
        .rect_filled(full, 0.0, colors.background_main());
    let header_rect = header(ui, window, colors, full);
    let body = Rect::from_min_max(
        pos2(full.left() + 16.0, header_rect.bottom() + 16.0),
        full.max - vec2(16.0, 16.0),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(body)
            .layout(Layout::top_down(Align::Min)),
    );
    tab_strip(&mut child, window, colors);
    child.add_space(8.0);
    egui::ScrollArea::vertical()
        .id_salt(("ffmpeg-tab", window.tab))
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            ui.spacing_mut().item_spacing.y = 12.0;
            match window.tab {
                0 => sources_tab(ui, window, colors, &mut asks),
                1 => video_tab(ui, window, colors),
                2 => audio_tab(ui, window, colors),
                _ => advanced_tab(ui, window, colors),
            }
        });
    asks
}

fn header(ui: &mut Ui, window: &mut FFmpegWindow, colors: &Colors, full: Rect) -> Rect {
    let painter = ui.painter().clone();
    let inner_width = full.width() - 40.0;
    let title = layout(
        ui,
        trimmed(
            t(c::FFmpegOptionsWindow_Screen_recording),
            24.0,
            Weight::SemiBold,
            colors.text(),
            inner_width - 300.0,
            1,
        ),
    );
    let description = layout(
        ui,
        wrapped(
            t(c::FFmpegOptionsWindow_Description),
            13.0,
            Weight::Normal,
            colors.text_secondary(),
            inner_width - 300.0,
        ),
    );
    let text_h = title.size().y + 3.0 + description.size().y;
    let content_h = text_h.max(44.0);
    let rect = Rect::from_min_size(full.min, vec2(full.width(), 18.0 + content_h + 18.0 + 1.0));
    painter.rect_filled(rect, 0.0, colors.background_toolbar());
    painter.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, colors.border()),
    );
    let mid = rect.top() + 18.0 + content_h / 2.0;
    let badge = Rect::from_center_size(pos2(full.left() + 20.0 + 22.0, mid), vec2(44.0, 44.0));
    crate::widgets::gradient_rect(
        &painter,
        badge,
        10.0,
        colors.accent_start(),
        colors.accent_end(),
    );
    paint_icon(
        ui,
        &painter,
        badge.center(),
        icons::VIDEO,
        22.0,
        colors.accent_foreground(),
    );
    let text_left = badge.right() + 14.0;
    let top = mid - text_h / 2.0;
    painter.galley(pos2(text_left, top), title.clone(), colors.text());
    painter.galley(
        pos2(text_left, top + title.size().y + 3.0),
        description,
        colors.text_secondary(),
    );

    // The reset button, or the question.
    let right = full.right() - 20.0;
    let look = Look::button(colors).icon(15.0, None).spacing(8.0);
    if window.confirm_reset {
        let primary = Look::primary(colors);
        let reset_w = crate::widgets::button_width(
            ui,
            &primary,
            Content::label(t(c::FFmpegOptionsWindow_Reset)),
        );
        let reset = Rect::from_min_size(pos2(right - reset_w, mid - 20.0), vec2(reset_w, 40.0));
        let cancel_look = Look {
            string_text: Some(colors.text()),
            ..Look::button(colors)
        };
        let cancel_w = crate::widgets::button_width(
            ui,
            &cancel_look,
            Content::label(t(c::FFmpegOptionsWindow_Cancel)),
        );
        let cancel = Rect::from_min_size(
            pos2(reset.left() - 8.0 - cancel_w, mid - 20.0),
            vec2(cancel_w, 40.0),
        );
        if crate::widgets::button_at(
            ui,
            colors,
            &primary,
            Content::label(t(c::FFmpegOptionsWindow_Reset)),
            true,
            reset,
        )
        .clicked()
        {
            // `ResetOptionsAsync`: everything but the ffmpeg path.
            let (overrides, path) = (
                window.options.ffmpeg.OverrideCLIPath,
                window.options.ffmpeg.CLIPath.clone(),
            );
            window.options.ffmpeg = FFmpegOptions {
                OverrideCLIPath: overrides,
                CLIPath: path,
                ..FFmpegOptions::default()
            };
            window.confirm_reset = false;
            let video = window.video_sources.clone();
            let audio = window.audio_sources.clone();
            let ffmpeg = &mut window.options.ffmpeg;
            if !video.iter().any(|d| d.matches(&ffmpeg.VideoSource)) {
                ffmpeg.VideoSource = capture_device::SCREEN.into();
            }
            if !audio.iter().any(|d| d.matches(&ffmpeg.AudioSource)) {
                ffmpeg.AudioSource = capture_device::NONE.into();
            }
        }
        if crate::widgets::button_at(
            ui,
            colors,
            &cancel_look,
            Content::label(t(c::FFmpegOptionsWindow_Cancel)),
            true,
            cancel,
        )
        .clicked()
        {
            window.confirm_reset = false;
        }
        let question = layout(
            ui,
            trimmed(
                t(c::FFmpegOptionsWindow_Reset_all_options),
                13.0,
                Weight::SemiBold,
                colors.text(),
                260.0,
                1,
            ),
        );
        painter.galley(
            pos2(
                cancel.left() - 8.0 - question.size().x,
                mid - question.size().y / 2.0,
            ),
            question,
            colors.text(),
        );
    } else {
        let content = Content::both(icons::ROTATE_CCW, t(c::FFmpegOptionsWindow_Reset_options));
        let w = crate::widgets::button_width(ui, &look, content);
        let r = Rect::from_min_size(pos2(right - w, mid - 20.0), vec2(w, 40.0));
        if crate::widgets::button_at(ui, colors, &look, content, true, r)
            .on_hover_text(t(c::FFmpegOptionsWindow_Reset_encoding_options))
            .clicked()
        {
            window.confirm_reset = true;
        }
    }
    rect
}

fn tab_strip(ui: &mut Ui, window: &mut FFmpegWindow, colors: &Colors) {
    let tabs = [
        (icons::MONITOR, t(c::FFmpegOptionsWindow_Sources)),
        (icons::VIDEO, t(c::FFmpegOptionsWindow_Video)),
        (icons::AUDIO_LINES, t(c::FFmpegOptionsWindow_Audio)),
        (icons::TERMINAL, t(c::FFmpegOptionsWindow_Advanced)),
    ];
    let galleys: Vec<_> = tabs
        .iter()
        .map(|(_, label)| {
            layout(
                ui,
                trimmed(label, 13.0, Weight::Normal, colors.text(), 300.0, 1),
            )
        })
        .collect();
    let widths: Vec<f32> = galleys
        .iter()
        .map(|g| 11.0 + 15.0 + 6.0 + g.size().x + 11.0)
        .collect();
    let total = widths.iter().sum::<f32>() + 3.0 * (tabs.len() as f32 - 1.0) + 8.0;
    let (strip, _) = ui.allocate_exact_size(vec2(total, 8.0 + 30.0), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(strip, 6.0, colors.background_toolbar());
    let mut x = strip.left() + 4.0;
    for (index, ((glyph, _), galley)) in tabs.iter().zip(galleys).enumerate() {
        let item = Rect::from_min_size(pos2(x, strip.top() + 4.0), vec2(widths[index], 30.0));
        let response = ui.interact(item, ui.id().with(("ffmpeg-tab", index)), Sense::click());
        let selected = window.tab == index;
        let color = if selected {
            colors.accent_foreground()
        } else {
            colors.text_secondary()
        };
        if selected {
            crate::widgets::gradient_rect(
                &painter,
                item,
                4.0,
                colors.accent_start(),
                colors.accent_end(),
            );
        } else if response.hovered() {
            painter.rect_filled(item, 4.0, colors.control_background_hover());
        }
        paint_icon(
            ui,
            &painter,
            pos2(item.left() + 11.0 + 7.5, item.center().y),
            *glyph,
            15.0,
            color,
        );
        painter.galley(
            pos2(
                item.left() + 11.0 + 15.0 + 6.0,
                item.center().y - galley.size().y / 2.0,
            ),
            galley,
            color,
        );
        if response.clicked() {
            window.tab = index;
        }
        x += widths[index] + 3.0;
    }
}

fn sources_tab(ui: &mut Ui, window: &mut FFmpegWindow, colors: &Colors, asks: &mut Vec<FFmpegAsk>) {
    section_card(ui, colors, 14.0, |ui| {
        section_title(ui, colors, t(c::FFmpegOptionsWindow_FFmpeg_executable));
        let mut custom = window.options.ffmpeg.OverrideCLIPath;
        if checkbox_weighted(
            ui,
            colors,
            &mut custom,
            Some(t(c::FFmpegOptionsWindow_Use_custom_path)),
            true,
            Weight::Normal,
        )
        .changed()
        {
            window.options.ffmpeg.OverrideCLIPath = custom;
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let width = ui.available_width() - 40.0 - 8.0;
            Field {
                height: 40.0,
                width: Some(width),
                hint: t(texts::PATH_PLACEHOLDER),
                enabled: custom,
                id: Some(egui::Id::new("ffmpeg-path")),
                ..Field::new(&mut window.options.ffmpeg.CLIPath)
            }
            .show(ui, colors);
            let look = Look::button(colors)
                .width(40.0)
                .padding(0.0)
                .icon(16.0, None);
            if button(ui, colors, &look, Content::icon(icons::FOLDER_OPEN), custom)
                .on_hover_text(t(c::FFmpegOptionsWindow_Browse_for_ffmpeg))
                .clicked()
            {
                asks.push(FFmpegAsk::Browse);
            }
        });
    });
    section_card(ui, colors, 14.0, |ui| {
        // Title left, Refresh devices right (`ColumnDefinitions="*,Auto"`).
        let look = Look::button(colors).icon(15.0, None).spacing(8.0);
        let content = Content::both(icons::REFRESH_CW, t(c::FFmpegOptionsWindow_Refresh_devices));
        let w = crate::widgets::button_width(ui, &look, content);
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
        let title = layout(
            ui,
            trimmed(
                t(c::FFmpegOptionsWindow_Capture_sources),
                16.0,
                Weight::SemiBold,
                colors.text(),
                row.width() - w - 12.0,
                1,
            ),
        );
        ui.painter().galley(
            pos2(row.left(), row.center().y - title.size().y / 2.0),
            title,
            colors.text(),
        );
        let button_rect = Rect::from_min_size(pos2(row.right() - w, row.top()), vec2(w, 40.0));
        if crate::widgets::button_at(ui, colors, &look, content, !window.refreshing, button_rect)
            .clicked()
        {
            asks.push(FFmpegAsk::Refresh);
        }
        let column = (ui.available_width() - 12.0) / 2.0;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.allocate_ui_with_layout(vec2(column, 80.0), Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                field_label(ui, colors, t(c::FFmpegOptionsWindow_Video_source));
                let list: Vec<(String, String)> = window
                    .video_sources
                    .iter()
                    .map(|d| (d.value.clone(), d.title.clone()))
                    .collect();
                let current = window
                    .video_sources
                    .iter()
                    .find(|d| d.matches(&window.options.ffmpeg.VideoSource))
                    .map(|d| d.value.clone())
                    .unwrap_or_default();
                if let Some(value) = field_combo(ui, colors, "ffmpeg-video-source", &current, &list)
                {
                    window.options.ffmpeg.VideoSource = value;
                }
            });
            ui.allocate_ui_with_layout(vec2(column, 80.0), Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                field_label(ui, colors, t(c::FFmpegOptionsWindow_Audio_source));
                let list: Vec<(String, String)> = window
                    .audio_sources
                    .iter()
                    .map(|d| (d.value.clone(), d.title.clone()))
                    .collect();
                let current = window
                    .audio_sources
                    .iter()
                    .find(|d| d.matches(&window.options.ffmpeg.AudioSource))
                    .map(|d| d.value.clone())
                    .unwrap_or_default();
                if let Some(value) = field_combo(ui, colors, "ffmpeg-audio-source", &current, &list)
                {
                    window.options.ffmpeg.AudioSource = value;
                }
            });
        });
        info_card(ui, colors, None, |ui| {
            icon_hint(
                ui,
                colors,
                icons::INFO,
                colors.accent_start(),
                &window.status,
            );
        });
    });
}

/// The hint of a hardware encoder: upstream's on Windows; on a Mac the note that
/// VideoToolbox does the work and only the bitrate applies.
#[cfg(windows)]
fn hardware_hint(upstream: Key) -> &'static str {
    t(upstream)
}

#[cfg(not(windows))]
fn hardware_hint(_upstream: Key) -> &'static str {
    t(mac::FFmpegVideoToolbox)
}

/// `GetVideoCodecDescription`.
fn codec_description(codec: FFmpegVideoCodec) -> (&'static str, &'static str) {
    use FFmpegVideoCodec as V;
    match codec {
        V::libx264 => (
            t(c::FFmpegOptionsWindow_Codec_H264_software),
            t(c::FFmpegOptionsWindow_Codec_H264_software_hint),
        ),
        V::libx265 => (
            t(c::FFmpegOptionsWindow_Codec_H265_software),
            t(c::FFmpegOptionsWindow_Codec_H265_software_hint),
        ),
        V::libvpx => (
            t(c::FFmpegOptionsWindow_Codec_VP8_software),
            t(c::FFmpegOptionsWindow_Codec_VP8_software_hint),
        ),
        V::libvpx_vp9 => (
            t(c::FFmpegOptionsWindow_Codec_VP9_software),
            t(c::FFmpegOptionsWindow_Codec_VP9_software_hint),
        ),
        V::libxvid => (
            t(c::FFmpegOptionsWindow_Codec_MPEG4_Xvid),
            t(c::FFmpegOptionsWindow_Codec_MPEG4_Xvid_hint),
        ),
        V::h264_nvenc => (
            t(c::FFmpegOptionsWindow_Codec_H264_NVIDIA),
            hardware_hint(c::FFmpegOptionsWindow_Codec_H264_NVIDIA_hint),
        ),
        V::hevc_nvenc => (
            t(c::FFmpegOptionsWindow_Codec_HEVC_NVIDIA),
            hardware_hint(c::FFmpegOptionsWindow_Codec_HEVC_NVIDIA_hint),
        ),
        V::h264_amf => (
            t(c::FFmpegOptionsWindow_Codec_H264_AMD),
            hardware_hint(c::FFmpegOptionsWindow_Codec_H264_AMD_hint),
        ),
        V::hevc_amf => (
            t(c::FFmpegOptionsWindow_Codec_HEVC_AMD),
            hardware_hint(c::FFmpegOptionsWindow_Codec_HEVC_AMD_hint),
        ),
        V::h264_qsv => (
            t(c::FFmpegOptionsWindow_Codec_H264_Intel_Quick_Sync),
            hardware_hint(c::FFmpegOptionsWindow_Codec_H264_Intel_Quick_Sync_hint),
        ),
        V::hevc_qsv => (
            t(c::FFmpegOptionsWindow_Codec_HEVC_Intel_Quick_Sync),
            hardware_hint(c::FFmpegOptionsWindow_Codec_HEVC_Intel_Quick_Sync_hint),
        ),
        V::gif => (
            t(c::FFmpegOptionsWindow_Codec_animated_GIF),
            t(c::FFmpegOptionsWindow_Codec_animated_GIF_hint),
        ),
        V::libwebp => (
            t(c::FFmpegOptionsWindow_Codec_animated_WebP),
            t(c::FFmpegOptionsWindow_Codec_animated_WebP_hint),
        ),
        V::apng => (
            t(c::FFmpegOptionsWindow_Codec_animated_PNG),
            t(c::FFmpegOptionsWindow_Codec_animated_PNG_hint),
        ),
    }
}

/// `ColumnDefinitions="300,*"`, 12 apart; `add` draws column 0, then column 1.
fn two_columns(ui: &mut Ui, mut add: impl FnMut(&mut Ui, usize)) {
    let width = ui.available_width();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        ui.allocate_ui_with_layout(vec2(300.0, 0.0), Layout::top_down(Align::Min), |ui| {
            add(ui, 0)
        });
        ui.allocate_ui_with_layout(
            vec2((width - 312.0).max(200.0), 0.0),
            Layout::top_down(Align::Min),
            |ui| add(ui, 1),
        );
    });
}

fn video_tab(ui: &mut Ui, window: &mut FFmpegWindow, colors: &Colors) {
    let ffmpeg = &mut window.options.ffmpeg;
    two_columns(ui, |ui, column| {
        if column == 0 {
            section_card(ui, colors, 12.0, |ui| {
                section_title(ui, colors, t(c::FFmpegOptionsWindow_Video_encoder));
                let mut codecs: Vec<FFmpegVideoCodec> = FFmpegVideoCodec::OFFERED.to_vec();
                if !codecs.contains(&ffmpeg.VideoCodec) {
                    codecs.push(ffmpeg.VideoCodec);
                }
                let list: Vec<(FFmpegVideoCodec, String)> = codecs
                    .iter()
                    .map(|v| {
                        (
                            *v,
                            modlight_core::l10n::enum_text(
                                "FFmpegVideoCodec",
                                v.name(),
                                Some(v.description()),
                            ),
                        )
                    })
                    .collect();
                if let Some(codec) =
                    field_combo(ui, colors, "ffmpeg-video-codec", &ffmpeg.VideoCodec, &list)
                {
                    ffmpeg.VideoCodec = codec;
                }
                let (summary, hint_text) = codec_description(ffmpeg.VideoCodec);
                info_card(ui, colors, None, |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    let width = ui.available_width();
                    let galley = layout(
                        ui,
                        wrapped(summary, 13.0, Weight::SemiBold, colors.text(), width),
                    );
                    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
                    ui.painter().galley(rect.min, galley, colors.text());
                    if !hint_text.is_empty() {
                        hint(ui, colors, hint_text, width);
                    }
                });
            });
        } else {
            section_card(ui, colors, 14.0, |ui| {
                section_title(ui, colors, t(c::FFmpegOptionsWindow_Encoder_settings));
                encoder_settings(ui, ffmpeg, colors);
            });
        }
    });
}

fn encoder_settings(ui: &mut Ui, ffmpeg: &mut FFmpegOptions, colors: &Colors) {
    use FFmpegVideoCodec as V;
    let bitrate = |ui: &mut Ui, id: &str, value: &mut i64| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            field_label(ui, colors, t(c::FFmpegOptionsWindow_Target_bitrate));
            number(ui, colors, id, value, 100.0, 100000.0, 100.0);
        });
    };
    match ffmpeg.VideoCodec {
        V::libx264 | V::libx265 => {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                field_label(ui, colors, t(c::FFmpegOptionsWindow_Preset));
                let names = [
                    c::FFmpegOptionsWindow_Preset_ultra_fast,
                    c::FFmpegOptionsWindow_Preset_super_fast,
                    c::FFmpegOptionsWindow_Preset_very_fast,
                    c::FFmpegOptionsWindow_Preset_faster,
                    c::FFmpegOptionsWindow_Preset_fast,
                    c::FFmpegOptionsWindow_Preset_medium,
                    c::FFmpegOptionsWindow_Preset_slow,
                    c::FFmpegOptionsWindow_Preset_slower,
                    c::FFmpegOptionsWindow_Preset_very_slow,
                    c::FFmpegOptionsWindow_Preset_placebo,
                ];
                let list: Vec<(FFmpegPreset, String)> = FFmpegPreset::ALL
                    .iter()
                    .zip(names)
                    .map(|(p, k)| (*p, t(k).to_string()))
                    .collect();
                ui.scope(|ui| {
                    if let Some(preset) =
                        field_combo(ui, colors, "ffmpeg-x264-preset", &ffmpeg.x264_Preset, &list)
                    {
                        ffmpeg.x264_Preset = preset;
                    }
                })
                .response
                .on_hover_text(t(c::FFmpegOptionsWindow_Faster_presets_hint));
            });
            let index = FFmpegPreset::ALL
                .iter()
                .position(|p| *p == ffmpeg.x264_Preset)
                .unwrap_or(0);
            if index > 4 {
                info_card(ui, colors, Some(GOLDENROD), |ui| {
                    icon_hint(
                        ui,
                        colors,
                        icons::TRIANGLE_ALERT,
                        GOLDENROD,
                        t(c::FFmpegOptionsWindow_Slow_presets_warning),
                    );
                });
            }
            let mut use_bitrate = ffmpeg.x264_Use_Bitrate;
            if checkbox_weighted(
                ui,
                colors,
                &mut use_bitrate,
                Some(t(c::FFmpegOptionsWindow_Use_target_bitrate)),
                true,
                Weight::Normal,
            )
            .changed()
            {
                ffmpeg.x264_Use_Bitrate = use_bitrate;
            }
            if ffmpeg.x264_Use_Bitrate {
                bitrate(ui, "ffmpeg-x264-bitrate", &mut ffmpeg.x264_Bitrate);
            } else {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    field_label(ui, colors, t(c::FFmpegOptionsWindow_Constant_rate_factor));
                    number(
                        ui,
                        colors,
                        "ffmpeg-x264-crf",
                        &mut ffmpeg.x264_CRF,
                        0.0,
                        51.0,
                        1.0,
                    );
                    ui.add_space(6.0);
                    let width = ui.available_width();
                    hint(ui, colors, t(c::FFmpegOptionsWindow_CRF_hint), width);
                });
            }
        }
        V::libvpx | V::libvpx_vp9 => bitrate(ui, "ffmpeg-vpx-bitrate", &mut ffmpeg.VPx_Bitrate),
        V::libxvid => {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                field_label(
                    ui,
                    colors,
                    t(c::FFmpegOptionsWindow_Variable_bitrate_quality),
                );
                number(
                    ui,
                    colors,
                    "ffmpeg-xvid-q",
                    &mut ffmpeg.XviD_QScale,
                    1.0,
                    31.0,
                    1.0,
                );
                ui.add_space(8.0);
                let width = ui.available_width();
                hint(
                    ui,
                    colors,
                    t(c::FFmpegOptionsWindow_Variable_bitrate_hint),
                    width,
                );
            });
        }
        // Windows hardware encoders: upstream's panels on Windows; VideoToolbox on a Mac,
        // which takes only the bitrate.
        V::h264_nvenc | V::hevc_nvenc => {
            if cfg!(windows) {
                let presets = [
                    c::FFmpegOptionsWindow_NVENC_fastest_lowest_quality,
                    c::FFmpegOptionsWindow_NVENC_faster_lower_quality,
                    c::FFmpegOptionsWindow_NVENC_fast_low_quality,
                    c::FFmpegOptionsWindow_NVENC_medium_medium_quality,
                    c::FFmpegOptionsWindow_NVENC_slow_good_quality,
                    c::FFmpegOptionsWindow_NVENC_slower_better_quality,
                    c::FFmpegOptionsWindow_NVENC_slowest_best_quality,
                ];
                labelled_choice(
                    ui,
                    colors,
                    c::FFmpegOptionsWindow_Preset,
                    "ffmpeg-nvenc-preset",
                    FFmpegNVENCPreset::ALL,
                    &presets,
                    &mut ffmpeg.NVENC_Preset,
                );
                let tunes = [
                    c::FFmpegOptionsWindow_NVENC_high_quality,
                    c::FFmpegOptionsWindow_NVENC_low_latency,
                    c::FFmpegOptionsWindow_NVENC_ultra_low_latency,
                    c::FFmpegOptionsWindow_NVENC_lossless,
                ];
                labelled_choice(
                    ui,
                    colors,
                    c::FFmpegOptionsWindow_Tune,
                    "ffmpeg-nvenc-tune",
                    FFmpegNVENCTune::ALL,
                    &tunes,
                    &mut ffmpeg.NVENC_Tune,
                );
            }
            bitrate(ui, "ffmpeg-nvenc-bitrate", &mut ffmpeg.NVENC_Bitrate)
        }
        V::h264_amf | V::hevc_amf => {
            if cfg!(windows) {
                let usages = [
                    c::FFmpegOptionsWindow_AMF_generic_transcoding,
                    c::FFmpegOptionsWindow_AMF_ultra_low_latency_transcoding,
                    c::FFmpegOptionsWindow_AMF_low_latency_transcoding,
                    c::FFmpegOptionsWindow_AMF_webcam,
                    c::FFmpegOptionsWindow_AMF_high_quality_transcoding,
                    c::FFmpegOptionsWindow_AMF_low_latency_high_quality_transcoding,
                ];
                labelled_choice(
                    ui,
                    colors,
                    c::FFmpegOptionsWindow_Usage,
                    "ffmpeg-amf-usage",
                    FFmpegAMFUsage::ALL,
                    &usages,
                    &mut ffmpeg.AMF_Usage,
                );
                let qualities = [
                    c::FFmpegOptionsWindow_AMF_prefer_speed,
                    c::FFmpegOptionsWindow_AMF_balanced,
                    c::FFmpegOptionsWindow_AMF_prefer_quality,
                ];
                labelled_choice(
                    ui,
                    colors,
                    c::FFmpegOptionsWindow_Quality,
                    "ffmpeg-amf-quality",
                    FFmpegAMFQuality::ALL,
                    &qualities,
                    &mut ffmpeg.AMF_Quality,
                );
            }
            bitrate(ui, "ffmpeg-amf-bitrate", &mut ffmpeg.AMF_Bitrate)
        }
        V::h264_qsv | V::hevc_qsv => {
            if cfg!(windows) {
                let presets = [
                    c::FFmpegOptionsWindow_Preset_very_fast,
                    c::FFmpegOptionsWindow_Preset_faster,
                    c::FFmpegOptionsWindow_Preset_fast,
                    c::FFmpegOptionsWindow_Preset_medium,
                    c::FFmpegOptionsWindow_Preset_slow,
                    c::FFmpegOptionsWindow_Preset_slower,
                    c::FFmpegOptionsWindow_Preset_very_slow,
                ];
                labelled_choice(
                    ui,
                    colors,
                    c::FFmpegOptionsWindow_Preset,
                    "ffmpeg-qsv-preset",
                    FFmpegQSVPreset::ALL,
                    &presets,
                    &mut ffmpeg.QSV_Preset,
                );
            }
            bitrate(ui, "ffmpeg-qsv-bitrate", &mut ffmpeg.QSV_Bitrate)
        }
        V::gif => {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                field_label(ui, colors, t(c::FFmpegOptionsWindow_Palette_mode));
                let list: Vec<(FFmpegPaletteGenStatsMode, String)> = FFmpegPaletteGenStatsMode::ALL
                    .iter()
                    .map(|m| {
                        (
                            *m,
                            modlight_core::l10n::enum_text(
                                "FFmpegPaletteGenStatsMode",
                                m.name(),
                                None,
                            ),
                        )
                    })
                    .collect();
                if let Some(mode) =
                    field_combo(ui, colors, "ffmpeg-gif-stats", &ffmpeg.GIFStatsMode, &list)
                {
                    ffmpeg.GIFStatsMode = mode;
                }
            });
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                field_label(ui, colors, t(c::FFmpegOptionsWindow_Dithering_mode));
                let list: Vec<(FFmpegPaletteUseDither, String)> = FFmpegPaletteUseDither::ALL
                    .iter()
                    .map(|m| {
                        (
                            *m,
                            modlight_core::l10n::enum_text(
                                "FFmpegPaletteUseDither",
                                m.name(),
                                None,
                            ),
                        )
                    })
                    .collect();
                if let Some(dither) =
                    field_combo(ui, colors, "ffmpeg-gif-dither", &ffmpeg.GIFDither, &list)
                {
                    ffmpeg.GIFDither = dither;
                }
            });
            if ffmpeg.GIFDither == FFmpegPaletteUseDither::bayer {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    field_label(ui, colors, t(c::FFmpegOptionsWindow_Bayer_scale));
                    number(
                        ui,
                        colors,
                        "ffmpeg-gif-bayer",
                        &mut ffmpeg.GIFBayerScale,
                        0.0,
                        5.0,
                        1.0,
                    );
                    ui.add_space(6.0);
                    let width = ui.available_width();
                    hint(
                        ui,
                        colors,
                        t(c::FFmpegOptionsWindow_Bayer_scale_hint),
                        width,
                    );
                });
            }
        }
        V::libwebp | V::apng => {
            info_card(ui, colors, None, |ui| {
                icon_hint(
                    ui,
                    colors,
                    icons::CIRCLE_CHECK,
                    colors.accent_start(),
                    t(c::FFmpegOptionsWindow_No_additional_settings),
                );
            });
        }
    }
}

/// `SelectNumberOrDefault`.
fn number_combo(
    ui: &mut Ui,
    colors: &Colors,
    id: &str,
    value: &mut i64,
    items: &[i64],
    default: i64,
) {
    let current = if items.contains(value) {
        *value
    } else {
        default
    };
    let list: Vec<(i64, String)> = items.iter().map(|v| (*v, v.to_string())).collect();
    if let Some(picked) = field_combo(ui, colors, id, &current, &list) {
        *value = picked;
    }
}

fn audio_tab(ui: &mut Ui, window: &mut FFmpegWindow, colors: &Colors) {
    let ffmpeg = &mut window.options.ffmpeg;
    let animated = ffmpeg.is_animated_image();
    two_columns(ui, |ui, column| {
        if column == 0 {
            section_card(ui, colors, 12.0, |ui| {
                section_title(ui, colors, t(c::FFmpegOptionsWindow_Audio_encoder));
                let list: Vec<(FFmpegAudioCodec, String)> = FFmpegAudioCodec::ALL
                    .iter()
                    .map(|v| {
                        (
                            *v,
                            modlight_core::l10n::enum_text(
                                "FFmpegAudioCodec",
                                v.name(),
                                Some(v.description()),
                            ),
                        )
                    })
                    .collect();
                ui.add_enabled_ui(!animated, |ui| {
                    if let Some(codec) =
                        field_combo(ui, colors, "ffmpeg-audio-codec", &ffmpeg.AudioCodec, &list)
                    {
                        ffmpeg.AudioCodec = codec;
                    }
                });
                if animated {
                    info_card(ui, colors, Some(GOLDENROD), |ui| {
                        icon_hint(
                            ui,
                            colors,
                            icons::TRIANGLE_ALERT,
                            GOLDENROD,
                            t(c::FFmpegOptionsWindow_Animated_formats_no_audio),
                        );
                    });
                }
            });
        } else {
            section_card(ui, colors, 14.0, |ui| {
                ui.add_enabled_ui(!animated, |ui| {
                    ui.spacing_mut().item_spacing.y = 14.0;
                    section_title(ui, colors, t(c::FFmpegOptionsWindow_Encoder_settings));
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        match ffmpeg.AudioCodec {
                            FFmpegAudioCodec::libvoaacenc => {
                                field_label(ui, colors, t(c::FFmpegOptionsWindow_Bitrate));
                                let items: Vec<i64> = (2..=10).map(|x| x * 32).collect();
                                number_combo(
                                    ui,
                                    colors,
                                    "ffmpeg-aac",
                                    &mut ffmpeg.AAC_Bitrate,
                                    &items,
                                    128,
                                );
                            }
                            FFmpegAudioCodec::libopus => {
                                field_label(ui, colors, t(c::FFmpegOptionsWindow_Bitrate));
                                let items: Vec<i64> = (1..=16).map(|x| x * 32).collect();
                                number_combo(
                                    ui,
                                    colors,
                                    "ffmpeg-opus",
                                    &mut ffmpeg.Opus_Bitrate,
                                    &items,
                                    128,
                                );
                            }
                            FFmpegAudioCodec::libvorbis => {
                                field_label(ui, colors, t(c::FFmpegOptionsWindow_Quality));
                                let items: Vec<i64> = (0..=10).collect();
                                number_combo(
                                    ui,
                                    colors,
                                    "ffmpeg-vorbis",
                                    &mut ffmpeg.Vorbis_QScale,
                                    &items,
                                    3,
                                );
                            }
                            FFmpegAudioCodec::libmp3lame => {
                                field_label(ui, colors, t(c::FFmpegOptionsWindow_Quality));
                                let items: Vec<i64> = (0..=9).rev().collect();
                                number_combo(
                                    ui,
                                    colors,
                                    "ffmpeg-mp3",
                                    &mut ffmpeg.MP3_QScale,
                                    &items,
                                    4,
                                );
                                ui.add_space(6.0);
                                let width = ui.available_width();
                                hint(
                                    ui,
                                    colors,
                                    t(c::FFmpegOptionsWindow_Lower_values_higher_quality),
                                    width,
                                );
                            }
                        }
                    });
                });
            });
        }
    });
}

fn advanced_tab(ui: &mut Ui, window: &mut FFmpegWindow, colors: &Colors) {
    section_card(ui, colors, 10.0, |ui| {
        section_title(ui, colors, t(c::FFmpegOptionsWindow_Additional_arguments));
        Field {
            height: 40.0,
            hint: t(c::FFmpegOptionsWindow_Arguments_example),
            id: Some(egui::Id::new("ffmpeg-user-args")),
            ..Field::new(&mut window.options.ffmpeg.UserArgs)
        }
        .show(ui, colors);
    });
    section_card(ui, colors, 10.0, |ui| {
        ui.horizontal(|ui| {
            let label = t(c::FFmpegOptionsWindow_Use_custom_commands);
            let check_w = layout(
                ui,
                trimmed(label, 13.0, Weight::Normal, colors.text(), 400.0, 1),
            )
            .size()
            .x + 30.0;
            ui.allocate_ui_with_layout(
                vec2(ui.available_width() - check_w - 12.0, 24.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    section_title(ui, colors, t(c::FFmpegOptionsWindow_FFmpeg_command));
                },
            );
            let mut custom = window.options.ffmpeg.UseCustomCommands;
            if checkbox_weighted(ui, colors, &mut custom, Some(label), true, Weight::Normal)
                .changed()
            {
                window.options.ffmpeg.UseCustomCommands = custom;
                if custom && window.options.ffmpeg.CustomCommands.trim().is_empty() {
                    window.options.ffmpeg.CustomCommands =
                        window.options.ffmpeg_args_text(true).unwrap_or_default();
                }
            }
        });
        info_card(ui, colors, None, |ui| {
            let width = ui.available_width();
            hint(
                ui,
                colors,
                t(c::FFmpegOptionsWindow_Custom_command_placeholders),
                width,
            );
        });
        let custom = window.options.ffmpeg.UseCustomCommands;
        let mut preview = if custom {
            window.options.ffmpeg.CustomCommands.clone()
        } else {
            window.options.ffmpeg_args_text(false).unwrap_or_default()
        };
        let frame = egui::Frame::NONE
            .fill(colors.control_background())
            .stroke(Stroke::new(1.0, colors.control_border()))
            .corner_radius(8.0)
            .inner_margin(egui::Margin::symmetric(10, 8));
        frame.show(ui, |ui| {
            let output = egui::TextEdit::multiline(&mut preview)
                .id(egui::Id::new("ffmpeg-command"))
                .frame(egui::Frame::NONE)
                .font(egui::FontId::monospace(13.0))
                .text_color(colors.text())
                .desired_width(f32::INFINITY)
                .desired_rows(14)
                .interactive(custom)
                .show(ui);
            let _ = output.response.rect;
        });
        if custom && preview != window.options.ffmpeg.CustomCommands {
            window.options.ffmpeg.CustomCommands = preview;
        }
        let _ = StrokeKind::Inside;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_values_from_windows_select_the_screen() {
        let screen = CaptureDevice::screen();
        for value in ["gdigrab", "ddagrab", "GDI grab", "screen-capture-recorder"] {
            assert!(screen.matches(value), "{value}");
        }
        assert!(!screen.matches(""));
        assert!(CaptureDevice::none().matches(""));
        assert!(CaptureDevice::device("FaceTime HD Camera").matches("facetime hd camera"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_shows_upstreams_titles_and_texts() {
        assert_eq!(
            CaptureDevice::screen().title,
            "gdigrab (Graphics Device Interface)"
        );
        assert_eq!(
            CaptureDevice::device("Integrated Camera").title,
            "dshow (Integrated Camera)"
        );
        modlight_core::l10n::set_thread_language(Some(modlight_core::l10n::Language::English));
        assert!(t(texts::NO_DEVICES).contains("DirectShow"));
        assert_eq!(t(texts::PATH_PLACEHOLDER), "Path to ffmpeg.exe");
        let (_, hint) = codec_description(FFmpegVideoCodec::h264_nvenc);
        assert_eq!(hint, t(c::FFmpegOptionsWindow_Codec_H264_NVIDIA_hint));
        modlight_core::l10n::set_thread_language(None);
    }
}
