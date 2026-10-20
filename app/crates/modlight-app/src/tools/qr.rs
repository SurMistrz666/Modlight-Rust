//! `QRCodeWindow` and `QRCodeViewModel`: type text and the code follows at the size of
//! the preview (or the size box); scan the screen, a region or a picture file; copy,
//! save (PNG, JPEG, BMP, SVG) and upload the code; a click on the code opens it in the
//! image viewer.

use std::sync::Arc;

use egui::{Color32, Id, Rect, Sense, TextureHandle, Ui, pos2, vec2};
use image::RgbaImage;
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, t, tools as tl};
use modlight_core::tasks::host::NotificationSound;
use modlight_tools::qr;

use super::kit::{self, Track};
use super::region::{RegionFor, ScreenPicture};
use super::{ToolWindow, WindowSpec};
use crate::state::AppState;
use crate::theme::Weight;

/// `QRCodeScanMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanMode {
    Screen,
    Region,
    ImageFile,
}

/// `QRCodeWindowOptions`.
#[derive(Clone, Debug, Default)]
pub struct QrOptions {
    pub initial_text: Option<String>,
    pub initial_image_file: Option<std::path::PathBuf>,
    pub initial_scan_mode: Option<ScanMode>,
}

pub struct QrWindow {
    pub key: u64,
    pub text: String,
    /// `QRCodeSize` (0: follow the preview).
    pub size: i64,
    available: i64,
    preview: Option<((String, i64), Arc<RgbaImage>)>,
    texture: Option<TextureHandle>,
    pub scanning: bool,
    /// `StatusText` (kept as upstream does; its window shows no status line).
    pub status: String,
    options: QrOptions,
    initialized: bool,
    /// A capture scan waiting for the window to step aside (UI seconds).
    pending_scan: Option<(ScanMode, f64)>,
}

impl QrWindow {
    pub fn new(key: u64, options: QrOptions) -> QrWindow {
        QrWindow {
            key,
            text: options.initial_text.clone().unwrap_or_default(),
            size: 0,
            available: 400,
            preview: None,
            texture: None,
            scanning: false,
            status: String::new(),
            options,
            initialized: false,
            pending_scan: None,
        }
    }

    pub fn has_valid_content(&self) -> bool {
        qr::check_content(&self.text)
    }

    pub fn effective_size(&self) -> i64 {
        qr::effective_size(self.size, self.available)
    }

    /// `RegenerateAsync` (the code is quick to make; it is made on the UI thread).
    fn regenerate(&mut self) {
        let key = (self.text.clone(), self.effective_size());
        if self.preview.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        if !self.has_valid_content() {
            self.preview = None;
            self.texture = None;
            self.status = if self.text.is_empty() || self.text.len() <= qr::MAXIMUM_CONTENT_BYTES {
                String::new()
            } else {
                t(tl::QRCodeViewModel_Content_too_large).to_string()
            };
            return;
        }
        match qr::generate(&self.text, key.1 as u32) {
            Some(image) => {
                self.preview = Some((key, Arc::new(image)));
                self.texture = None;
                self.status.clear();
            }
            None => self.status = t(tl::QRCodeViewModel_Unable_generate).to_string(),
        }
    }

    /// `ApplyScanResultsAsync`.
    pub fn apply_results(&mut self, state: &mut AppState, results: Option<Vec<String>>) {
        let results = results.unwrap_or_default();
        self.text = qr::join_results(&results);
        self.status = if results.is_empty() {
            t(tl::QRCodeViewModel_No_QR_code).to_string()
        } else {
            format(
                t(if results.len() == 1 {
                    tl::QRCodeViewModel_Decoded_one
                } else {
                    tl::QRCodeViewModel_Decoded_many
                }),
                &[Arg::from(results.len())],
            )
        };
        let settings = state.default_task_settings();
        state.play_task_sound(NotificationSound::ActionCompleted, &settings);
        self.scanning = false;
    }

    fn start_capture_scan(&mut self, ui: &Ui, mode: ScanMode, now: f64) {
        if self.scanning {
            return;
        }
        self.scanning = true;
        // `Hide(); await Task.Delay(250)`.
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Visible(false));
        self.pending_scan = Some((mode, now + 0.25));
    }

    fn scan_file(&mut self, state: &mut AppState, path: std::path::PathBuf) {
        self.scanning = true;
        let key = self.key;
        state.rt.spawn_blocking(move || {
            let results = super::files::load_image(&path).map(|image| qr::scan(&image, false));
            move |state: &mut AppState, _ctx: &egui::Context| {
                deliver(state, key, results);
            }
        });
    }
}

/// Scan results reach the window.
fn deliver(state: &mut AppState, key: u64, results: Option<Vec<String>>) {
    super::with_window::<QrWindow, _>(state, key, |window, state| {
        window.apply_results(state, results)
    });
}

/// A picture of the screen arrived for a scan (or the capture gave up).
pub fn scanned_picture(state: &mut AppState, key: u64, picture: ScreenPicture) {
    state.tools.restore.push(key);
    match picture {
        Some((image, _)) => {
            state.rt.spawn_blocking(move || {
                let results = qr::scan(&image, false);
                move |state: &mut AppState, _ctx: &egui::Context| {
                    deliver(state, key, Some(results));
                }
            });
        }
        None => deliver(state, key, None),
    }
}

impl ToolWindow for QrWindow {
    fn spec(&self) -> WindowSpec {
        WindowSpec::resizable(
            t(tl::QRCodeWindow_ShareX_QR_Code),
            vec2(940.0, 610.0),
            vec2(760.0, 520.0),
        )
    }

    fn ui(&mut self, ui: &mut Ui, state: &mut AppState) -> bool {
        let colors = state.colors.clone();
        let now = state.time;
        kit::fill(ui, &colors);
        let full = kit::inset(ui.max_rect(), 16.0);
        let rows = kit::rows(
            full,
            &[Track::Px(40.0), Track::Star(1.0), Track::Px(40.0)],
            12.0,
        );

        // The preview's room decides the code's size (`SetAvailableSize`).
        let cols = kit::columns(rows[1], &[Track::Px(430.0), Track::Star(1.0)], 12.0);
        let preview_host = cols[1];
        let available = ((preview_host.width() - 48.0).min(preview_host.height() - 48.0))
            .floor()
            .max(64.0) as i64;
        self.available = available;
        self.regenerate();

        if !self.initialized {
            self.initialized = true;
            if let Some(path) = self.options.initial_image_file.clone() {
                self.scan_file(state, path);
            } else if let Some(mode) = self.options.initial_scan_mode {
                self.start_capture_scan(ui, mode, now);
            }
        }
        if let Some((mode, at)) = self.pending_scan
            && now >= at
        {
            self.pending_scan = None;
            let settings = state.default_task_settings();
            match mode {
                ScanMode::Screen => state.tool_fullscreen(RegionFor::QrScan(self.key), settings),
                _ => state.tool_region(RegionFor::QrScan(self.key), settings),
            }
        } else if self.pending_scan.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }

        // The scan buttons.
        let items = [
            (
                Some(icons::SCAN_QR_CODE),
                Some(t(tl::QRCodeWindow_Scan_screen)),
            ),
            (
                Some(icons::SCAN_LINE),
                Some(t(tl::QRCodeWindow_Scan_region)),
            ),
            (Some(icons::FILE_SCAN), Some(t(tl::QRCodeWindow_Scan_image))),
        ];
        let rects = kit::left_aligned(ui, &colors, rows[0], &items, 8.0);
        let enabled = !self.scanning;
        if kit::button_at(ui, &colors, rects[0], items[0].0, items[0].1, enabled).clicked() {
            self.start_capture_scan(ui, ScanMode::Screen, now);
        }
        if kit::button_at(ui, &colors, rects[1], items[1].0, items[1].1, enabled).clicked() {
            self.start_capture_scan(ui, ScanMode::Region, now);
        }
        if kit::button_at(ui, &colors, rects[2], items[2].0, items[2].1, enabled).clicked()
            && let Some(path) = state.tool_open_file(
                t(tl::QRCodeWindow_Select_image_dialog),
                &[("Images", &super::files::IMAGE_EXTENSIONS)],
            )
        {
            self.scan_file(state, path);
        }

        // The content panel.
        kit::panel(ui, &colors, cols[0]);
        let inner = cols[0].shrink(16.0);
        let parts = kit::rows(
            inner,
            &[
                Track::Px(kit::HEADING_HEIGHT),
                Track::Star(1.0),
                Track::Px(40.0),
            ],
            10.0,
        );
        kit::heading(ui, &colors, parts[0], t(tl::QRCodeWindow_Content));
        let before = self.text.clone();
        kit::text_area_at(
            ui,
            &colors,
            parts[1],
            Id::new(("qr-text", self.key)),
            &mut self.text,
            t(tl::QRCodeWindow_Enter_text_or_a_URL_to_encode),
            true,
            false,
            false,
        );
        // `MaxLength="2952"` (characters).
        if self.text.chars().count() > qr::MAXIMUM_CONTENT_BYTES {
            self.text = self.text.chars().take(qr::MAXIMUM_CONTENT_BYTES).collect();
        }
        if self.text != before {
            self.regenerate();
        }
        let label = t(tl::QRCodeWindow_QR_code_size);
        let label_w = kit::text_width(ui, label, 13.0, Weight::Normal);
        let px_w = kit::text_width(ui, t(tl::QRCodeWindow_px), 13.0, Weight::Normal);
        let size_cols = kit::columns(
            Rect::from_min_size(parts[2].min, vec2(label_w + 8.0 + 120.0 + 8.0 + px_w, 40.0)),
            &[Track::Px(label_w), Track::Px(120.0), Track::Px(px_w)],
            8.0,
        );
        kit::text_at(
            ui,
            size_cols[0],
            egui::Align2::LEFT_CENTER,
            label,
            13.0,
            Weight::Normal,
            colors.text_secondary(),
        );
        if kit::int_at(
            ui,
            &colors,
            size_cols[1],
            &format!("qr-size-{}", self.key),
            &mut self.size,
            0,
            2048,
            64,
            true,
        ) {
            self.regenerate();
        }
        kit::text_at(
            ui,
            size_cols[2],
            egui::Align2::LEFT_CENTER,
            t(tl::QRCodeWindow_px),
            13.0,
            Weight::Normal,
            colors.text_secondary(),
        );

        // The preview.
        kit::panel(ui, &colors, preview_host);
        let room = preview_host.shrink(16.0);
        ui.set_clip_rect(ui.clip_rect().intersect(preview_host));
        if let Some((_, image)) = self.preview.clone() {
            if self.texture.is_none() {
                self.texture = Some(kit::texture(
                    ui.ctx(),
                    &format!("qr-{}", self.key),
                    &image,
                    true,
                ));
            }
            let texture = self.texture.clone().expect("made");
            // A white border, 8 px of padding, the code fitted inside (`Stretch="Uniform"`).
            let size = texture.size_vec2();
            let scale = ((room.width() - 16.0) / size.x).min((room.height() - 16.0) / size.y);
            let drawn = Rect::from_center_size(room.center(), size * scale);
            let white = drawn.expand(8.0);
            ui.painter().rect_filled(white, 2.0, Color32::WHITE);
            ui.painter().image(
                texture.id(),
                drawn,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            let click = ui.interact(
                preview_host,
                Id::new(("qr-preview", self.key)),
                Sense::click(),
            );
            if click.clicked() {
                state.open_image_viewer_picture(
                    (*image).clone(),
                    t(tl::QRCodeWindow_Image_viewer_title),
                );
            }
        } else {
            kit::placeholder(
                ui,
                &colors,
                room,
                icons::QR_CODE,
                t(tl::QRCodeWindow_Enter_content_to_generate_a_QR_code),
                48.0,
            );
        }
        ui.set_clip_rect(ui.max_rect());

        // Copy, save, upload.
        let items = [
            (Some(icons::COPY), Some(t(tl::QRCodeWindow_Copy_image))),
            (Some(icons::SAVE), Some(t(tl::QRCodeWindow_Save))),
            (Some(icons::UPLOAD), Some(t(tl::QRCodeWindow_Upload))),
        ];
        let rects = kit::right_aligned(ui, &colors, rows[2], &items, 8.0);
        let has_code = self.preview.is_some();
        let size = self.effective_size() as u32;
        if kit::button_at(ui, &colors, rects[0], items[0].0, items[0].1, has_code).clicked()
            && self.has_valid_content()
            && let Some(image) = qr::generate(&self.text, size)
        {
            state
                .effects
                .push(crate::state::Effect::CopyImage(Arc::new(image)));
        }
        if kit::button_at(ui, &colors, rects[1], items[1].0, items[1].1, has_code).clicked()
            && self.has_valid_content()
            && let Some(path) = state.tool_save_path(
                t(tl::QRCodeWindow_Save_QR_code_dialog),
                &qr::suggested_file_name(&self.text),
                &[
                    (t(tl::QRCodeWindow_PNG_image), &["png"]),
                    (t(tl::QRCodeWindow_JPEG_image), &["jpg", "jpeg"]),
                    (t(tl::QRCodeWindow_Bitmap_image), &["bmp"]),
                    (t(tl::QRCodeWindow_SVG_image), &["svg"]),
                ],
            )
            && save_code(&self.text, size, &path)
        {
            self.status = t(tl::QRCodeViewModel_QRCode_saved).to_string();
        }
        if kit::button_at(ui, &colors, rects[2], items[2].0, items[2].1, has_code).clicked()
            && self.has_valid_content()
            && let Some(image) = qr::generate(&self.text, size)
        {
            state.tool_upload_image(image, None);
        }
        false
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// `SaveQRCodeAsync`: SVG by name, any other picture format by extension.
pub fn save_code(text: &str, size: u32, path: &std::path::Path) -> bool {
    let is_svg = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"));
    let written = if is_svg {
        qr::svg(text, size)
            .map_err(anyhow::Error::msg)
            .and_then(|svg| std::fs::write(path, svg).map_err(Into::into))
    } else {
        match qr::generate(text, size) {
            Some(image) => modlight_tools::images::save(&image, path),
            None => Ok(()),
        }
    };
    if let Err(error) = &written {
        log::warn!("Modlight: the QR code could not be saved: {error}");
    }
    written.is_ok()
}

impl AppState {
    /// `OpenQRCode(text)`: the clipboard's text when it fits in a code.
    pub fn open_qr_code(&mut self, text: Option<String>) {
        let text = text.or_else(|| self.clipboard_text().filter(|t| qr::check_content(t)));
        self.tools.open_keyed(|key| {
            Box::new(QrWindow::new(
                key,
                QrOptions {
                    initial_text: text,
                    ..QrOptions::default()
                },
            ))
        });
    }

    /// `OpenQRCodeScanFromImageFile`, `OpenQRCodeScanScreen`, `OpenQRCodeScanRegion`.
    pub fn open_qr_code_scan(&mut self, file: Option<std::path::PathBuf>, mode: Option<ScanMode>) {
        self.tools.open_keyed(|key| {
            Box::new(QrWindow::new(
                key,
                QrOptions {
                    initial_image_file: file,
                    initial_scan_mode: mode,
                    ..QrOptions::default()
                },
            ))
        });
    }
}
