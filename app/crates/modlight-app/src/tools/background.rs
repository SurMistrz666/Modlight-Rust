//! `BackgroundRemoverWindow` and `BackgroundRemoverViewModel`, and
//! `TaskHelpers.OpenBackgroundRemover`: a picture (browse or drop), the models of the
//! models folder (the Components window installs one), the device, "Remove Background"
//! run by `modlight_core::background_removal` off the UI thread, the before and after
//! compared with a slider, "Save" (over a PNG or WebP source) and "Save as".
//!
//! Upstream's guide button opened ShareX's documentation (getsharex.com); as the OCR
//! window's help button, the port leaves it out. Without a model the hotkey offers the
//! Components window first, as upstream does.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{Align2, Color32, Id, Rect, Sense, Stroke, StrokeKind, TextureHandle, Ui, pos2, vec2};
use image::RgbaImage;
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, s, t, tools as tl};
use modlight_core::tasks::settings::TaskSettings;
use modlight_tools::background::{self, ModelFile};
use modlight_tools::settings::{BackgroundRemovalDevice, BackgroundRemoverOptions};

use super::jobs::{OptionsHome, ToolContinuation, tool_options};
use super::kit::{self, Track};
use super::{ToolWindow, WindowSpec};
use crate::state::{AppState, Effect};
use crate::theme::Weight;

/// `DefaultNotificationDuration`.
const NOTICE_SECONDS: f64 = 2.4;

/// The notice over the window's foot (`app-toast`).
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub message: String,
    pub icon: char,
    pub until: f64,
}

pub struct BackgroundWindow {
    pub key: u64,
    pub home: OptionsHome,
    pub options: BackgroundRemoverOptions,
    pub models_folder: PathBuf,
    pub models: Vec<ModelFile>,
    pub selected: Option<usize>,
    pub image_path: Option<PathBuf>,
    source: Option<Arc<RgbaImage>>,
    result: Option<Arc<RgbaImage>>,
    source_texture: Option<TextureHandle>,
    result_texture: Option<TextureHandle>,
    /// `ComparisonSliderPosition`.
    pub slider: f32,
    pub processing: bool,
    pub notice: Option<Notice>,
    version: u64,
    /// QA renders: the path shown instead of the fixture's temporary one.
    pub qa_display: Option<String>,
}

impl BackgroundWindow {
    pub fn new(
        key: u64,
        settings: &TaskSettings,
        models_folder: PathBuf,
        models: Vec<ModelFile>,
    ) -> BackgroundWindow {
        let options: BackgroundRemoverOptions = tool_options(settings, "BackgroundRemoverOptions");
        let selected = background::select_model(&models, &options.selected_model_file_name);
        let mut window = BackgroundWindow {
            key,
            home: OptionsHome::of(settings),
            options,
            models_folder,
            models,
            selected,
            image_path: None,
            source: None,
            result: None,
            source_texture: None,
            result_texture: None,
            slider: 0.5,
            processing: false,
            notice: None,
            version: 0,
            qa_display: None,
        };
        if let Some(i) = window.selected {
            window.options.selected_model_file_name = window.models[i].file_name.clone();
        }
        window
    }

    pub fn has_image(&self) -> bool {
        self.image_path.is_some()
    }

    pub fn can_remove(&self) -> bool {
        self.has_image() && self.selected.is_some() && !self.processing
    }

    fn can_save(&self) -> bool {
        self.result.is_some() && !self.processing
    }

    fn save_options(&self, state: &mut AppState) {
        state.save_tool_options(&self.home, "BackgroundRemoverOptions", &self.options);
    }

    /// `RefreshModels`.
    pub fn refresh_models(&mut self) {
        let saved = self
            .selected
            .and_then(|i| self.models.get(i))
            .map(|m| m.file_name.clone())
            .unwrap_or_else(|| self.options.selected_model_file_name.clone());
        self.models = background::list_models(&self.models_folder);
        self.selected = background::select_model(&self.models, &saved);
        if let Some(i) = self.selected {
            self.options.selected_model_file_name = self.models[i].file_name.clone();
        }
    }

    /// `SetSourceImage`.
    pub fn set_source(&mut self, image: RgbaImage, path: PathBuf) {
        self.source = Some(Arc::new(image));
        self.result = None;
        self.source_texture = None;
        self.result_texture = None;
        self.image_path = Some(path);
        self.qa_display = None;
    }

    /// `SetResultImage`: the slider moves to show the result whole.
    pub fn set_result(&mut self, image: RgbaImage) {
        self.result = Some(Arc::new(image));
        self.result_texture = None;
        self.slider = 0.0;
    }

    /// `LoadImage(filePath)`: decoded as Skia does, without turning it by EXIF.
    pub fn load_image(&mut self, path: &std::path::Path) {
        if self.processing {
            return;
        }
        match modlight_core::image_decode::open(path) {
            Ok(image) => self.set_source(image.to_rgba8(), path.to_path_buf()),
            Err(error) => log::warn!("Failed to load image '{}': {error}", path.display()),
        }
    }

    fn show_notice(&mut self, message: String, icon: char, now: f64) {
        self.notice = Some(Notice {
            message,
            icon,
            until: now + NOTICE_SECONDS,
        });
    }

    /// `RemoveBackgroundAsync`: the source read again from its file, the model run off
    /// the UI thread.
    fn remove_background(&mut self, state: &mut AppState) {
        let (Some(path), Some(index)) = (self.image_path.clone(), self.selected) else {
            return;
        };
        let Ok(decoded) = modlight_core::image_decode::open(&path) else {
            return;
        };
        let source = decoded.to_rgba8();
        self.set_source(source.clone(), path);
        self.processing = true;
        self.version += 1;
        let version = self.version;
        let key = self.key;
        let model = self.models[index].clone();
        let device = self.options.selected_device;
        let started = std::time::Instant::now();
        state.rt.spawn_blocking(move || {
            let result = modlight_core::background_removal::remove(
                &source,
                &model.path,
                background::removal_device(device),
            );
            let total = started.elapsed().as_millis();
            move |state: &mut AppState, _ctx: &egui::Context| {
                let now = state.time;
                super::with_window::<BackgroundWindow, _>(state, key, |window, _| {
                    if window.version != version {
                        return;
                    }
                    window.processing = false;
                    match result {
                        Ok(removal) => {
                            log::debug!(
                                "Background removal (device={device:?}, execution={}, model={}, {}): total={total} ms, session={} ms, preprocess={} ms, inference={} ms, postprocess={} ms",
                                removal.execution_device,
                                model.file_name,
                                if removal.session_cached { "cached" } else { "not cached" },
                                removal.session_ms,
                                removal.preprocessing_ms,
                                removal.inference_ms,
                                removal.postprocessing_ms
                            );
                            window.set_result(removal.image);
                            window.show_notice(
                                format(
                                    t(tl::BackgroundRemoverViewModel_Background_removed_ms),
                                    &[Arg::Int(total as i64)],
                                ),
                                icons::ERASER,
                                now,
                            );
                        }
                        Err(error) => log::warn!("Background removal failed: {error:#}"),
                    }
                });
            }
        });
    }

    /// `SaveAsync` / `SaveAsAsync`.
    fn save(&mut self, state: &mut AppState, ask: bool, now: f64) {
        let Some(result) = self.result.clone() else {
            return;
        };
        let source = self.image_path.clone();
        let target = match (ask, background::save_over(source.as_deref())) {
            (false, Some(path)) => Some(path),
            _ => state.tool_save_path(
                t(tl::BackgroundRemoverWindow_Save_image_as_dialog),
                &background::suggested_name(source.as_deref()),
                &[("PNG", &["png"]), ("WebP", &["webp"])],
            ),
        };
        let Some(target) = target else {
            return;
        };
        match background::save_result(&result, &target) {
            Ok(()) => self.show_notice(
                format(
                    t(tl::BackgroundRemoverViewModel_Image_saved_file_path),
                    &[Arg::from(target.to_string_lossy().as_ref())],
                ),
                if ask { icons::SAVE_ALL } else { icons::SAVE },
                now,
            ),
            Err(error) => log::warn!("Failed to save background removal result: {error}"),
        }
    }

    fn textures(&mut self, ctx: &egui::Context) {
        let make = |image: &RgbaImage, name: String| {
            let max = ctx.input(|i| i.max_texture_side).max(1024) as u32;
            let image = if image.width() > max || image.height() > max {
                let scale = max as f32 / image.width().max(image.height()) as f32;
                image::imageops::thumbnail(
                    image,
                    ((image.width() as f32 * scale) as u32).max(1),
                    ((image.height() as f32 * scale) as u32).max(1),
                )
            } else {
                image.clone()
            };
            kit::texture(ctx, &name, &image, false)
        };
        if self.source_texture.is_none()
            && let Some(source) = &self.source
        {
            self.source_texture = Some(make(source, format!("bg-source-{}", self.key)));
        }
        if self.result_texture.is_none()
            && let Some(result) = &self.result
        {
            self.result_texture = Some(make(result, format!("bg-result-{}", self.key)));
        }
    }

    /// `ImageComparisonSlider`: the result, the source over it left of the divider.
    fn comparison(&mut self, ui: &mut Ui, area: Rect) {
        self.textures(ui.ctx());
        let (left, right) = (self.source_texture.clone(), self.result_texture.clone());
        let sizes: Vec<egui::Vec2> = [&self.source, &self.result]
            .iter()
            .filter_map(|i| i.as_ref())
            .map(|i| vec2(i.width() as f32, i.height() as f32))
            .collect();
        if sizes.is_empty() {
            return;
        }
        let w = sizes.iter().map(|s| s.x).fold(0.0, f32::max);
        let h = sizes.iter().map(|s| s.y).fold(0.0, f32::max);
        let scale = (area.width() / w).min(area.height() / h);
        let (iw, ih) = (w * scale, h * scale);
        let bounds = Rect::from_min_max(
            pos2(
                (area.left() + (area.width() - iw) / 2.0).round(),
                (area.top() + (area.height() - ih) / 2.0).round(),
            ),
            pos2(
                (area.left() + (area.width() + iw) / 2.0).round(),
                (area.top() + (area.height() + ih) / 2.0).round(),
            ),
        );
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let painter = ui.painter_at(area);
        checker(&painter, area.min, bounds);
        match (&left, &right) {
            (Some(left), Some(right)) => {
                painter.image(right.id(), bounds, uv, Color32::WHITE);
                let response = ui.interact(area, Id::new(("bg-compare", self.key)), Sense::drag());
                if let Some(pointer) = response.interact_pointer_pos()
                    && (response.dragged()
                        || response.drag_started()
                        || response.is_pointer_button_down_on())
                {
                    self.slider = ((pointer.x - bounds.left()) / bounds.width()).clamp(0.0, 1.0);
                }
                if response.hovered() || response.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                let x = (bounds.left() + bounds.width() * self.slider).round();
                let clip =
                    Rect::from_min_max(bounds.min, pos2(x.max(bounds.left()), bounds.bottom()));
                let clipped = ui.painter_at(clip);
                checker(&clipped, area.min, bounds);
                clipped.image(left.id(), bounds, uv, Color32::WHITE);
                // The divider and its handle.
                let white = Color32::WHITE;
                painter.line_segment(
                    [pos2(x, bounds.top()), pos2(x, bounds.bottom())],
                    Stroke::new(2.0, white),
                );
                let handle = Rect::from_center_size(pos2(x, bounds.center().y), vec2(34.0, 52.0));
                painter.rect_filled(
                    handle,
                    16.0,
                    Color32::from_rgba_unmultiplied(35, 35, 35, 230),
                );
                painter.rect_stroke(handle, 16.0, Stroke::new(2.0, white), StrokeKind::Middle);
                for dx in [-5.0, 5.0] {
                    painter.line_segment(
                        [
                            pos2(x + dx, handle.top() + 17.0),
                            pos2(x + dx, handle.bottom() - 17.0),
                        ],
                        Stroke::new(2.0, white),
                    );
                }
            }
            (Some(only), None) | (None, Some(only)) => {
                painter.image(only.id(), bounds, uv, Color32::WHITE);
            }
            (None, None) => {}
        }
    }
}

/// The transparency checkerboard: 12 px squares of #EBEBEB and #CDCDCD from `origin`.
fn checker(painter: &egui::Painter, origin: egui::Pos2, bounds: Rect) {
    const CELL: f32 = 12.0;
    painter.rect_filled(bounds, 0.0, Color32::from_rgb(0xEB, 0xEB, 0xEB));
    let first_col = ((bounds.left() - origin.x) / CELL).floor() as i64;
    let last_col = ((bounds.right() - origin.x) / CELL).ceil() as i64;
    let first_row = ((bounds.top() - origin.y) / CELL).floor() as i64;
    let last_row = ((bounds.bottom() - origin.y) / CELL).ceil() as i64;
    let dark = Color32::from_rgb(0xCD, 0xCD, 0xCD);
    for row in first_row..last_row {
        for col in first_col..last_col {
            if (row + col).rem_euclid(2) == 1 {
                let cell = Rect::from_min_size(
                    pos2(origin.x + col as f32 * CELL, origin.y + row as f32 * CELL),
                    vec2(CELL, CELL),
                )
                .intersect(bounds);
                if cell.is_positive() {
                    painter.rect_filled(cell, 0.0, dark);
                }
            }
        }
    }
}

impl ToolWindow for BackgroundWindow {
    fn spec(&self) -> WindowSpec {
        WindowSpec::resizable(
            t(tl::BackgroundRemoverWindow_ShareX_Background_Remover),
            vec2(1080.0, 720.0),
            vec2(760.0, 520.0),
        )
    }

    fn ui(&mut self, ui: &mut Ui, state: &mut AppState) -> bool {
        let colors = state.colors.clone();
        let now = state.time;
        kit::fill(ui, &colors);
        let idle = !self.processing;
        if self.processing {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Wait);
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }

        // A file dropped on the window becomes the picture (`OnDrop`).
        let dropped: Option<PathBuf> = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .find(|p| p.is_file())
        });
        if let Some(path) = dropped {
            self.load_image(&path);
        }

        let full = kit::inset(ui.max_rect(), 16.0);
        let rows = kit::rows(
            full,
            &[
                Track::Px(12.0 * 2.0 + 40.0 * 2.0 + 8.0),
                Track::Star(1.0),
                Track::Px(40.0),
            ],
            12.0,
        );

        // The picture and the model.
        kit::panel(ui, &colors, rows[0]);
        let inner = rows[0].shrink(12.0);
        let lines = kit::rows(inner, &[Track::Px(40.0), Track::Px(40.0)], 8.0);
        let label = |ui: &Ui, rect: Rect, text: &str| {
            kit::text_at(
                ui,
                rect,
                Align2::LEFT_CENTER,
                text,
                13.0,
                Weight::SemiBold,
                colors.text(),
            );
        };
        let image_label = t(tl::BackgroundRemoverWindow_Image);
        let browse = t(tl::BackgroundRemoverWindow_Browse);
        let c = kit::columns(
            lines[0],
            &[
                Track::Px(kit::text_width(ui, image_label, 13.0, Weight::SemiBold)),
                Track::Star(1.0),
                Track::Px(kit::button_width(ui, &colors, None, Some(browse))),
            ],
            12.0,
        );
        label(ui, c[0], image_label);
        let shown = match (&self.qa_display, &self.image_path) {
            (Some(name), _) => name.clone(),
            (None, Some(path)) => path.to_string_lossy().into_owned(),
            // `TargetNullValue='No image selected'` (not translated upstream either).
            (None, None) => "No image selected".to_string(),
        };
        kit::path_text(ui, &colors, c[1], &shown);
        if kit::button_at(ui, &colors, c[2], None, Some(browse), idle).clicked()
            && let Some(path) = state.tool_open_file(
                t(tl::BackgroundRemoverViewModel_Select_image_dialog),
                &[("Images", &super::files::IMAGE_EXTENSIONS)],
            )
        {
            self.load_image(&path);
        }

        let models_label = t(tl::BackgroundRemoverWindow_Models);
        let remove_label = t(tl::BackgroundRemoverWindow_Remove_Background);
        let c = kit::columns(
            lines[1],
            &[
                Track::Px(kit::text_width(ui, models_label, 13.0, Weight::SemiBold)),
                Track::Star(1.0),
                Track::Px(kit::button_width(ui, &colors, None, Some(remove_label))),
            ],
            12.0,
        );
        label(ui, c[0], models_label);
        let device_label = t(tl::BackgroundRemoverWindow_Device);
        let device_w = BackgroundRemovalDevice::ALL
            .iter()
            .map(|d| kit::text_width(ui, background::device_name(*d), 13.0, Weight::Normal))
            .fold(0.0, f32::max)
            + 50.0;
        let m = kit::columns(
            c[1],
            &[
                Track::Star(1.0),
                Track::Px(40.0),
                Track::Px(40.0),
                Track::Px(kit::text_width(ui, device_label, 13.0, Weight::SemiBold) + 16.0),
                Track::Px(device_w.max(88.0)),
            ],
            4.0,
        );
        let list: Vec<(usize, String)> = self
            .models
            .iter()
            .enumerate()
            .map(|(i, m)| (i, m.display_name()))
            .collect();
        let current = self.selected.unwrap_or(usize::MAX);
        if let Some(i) = kit::combo_at(
            ui,
            &colors,
            m[0],
            &format!("bg-model-{}", self.key),
            &current,
            &list,
            idle,
        ) {
            self.selected = Some(i);
            self.options.selected_model_file_name = self.models[i].file_name.clone();
            self.save_options(state);
        }
        let _ = ui
            .interact(m[0], Id::new(("bg-model-tip", self.key)), Sense::hover())
            .on_hover_text(t(tl::BackgroundRemoverWindow_Background_removal_model));
        if kit::button_at(ui, &colors, m[1], Some(icons::REFRESH_CW), None, idle)
            .on_hover_text(t(tl::BackgroundRemoverWindow_Refresh_models))
            .clicked()
        {
            self.refresh_models();
            self.save_options(state);
        }
        if kit::button_at(ui, &colors, m[2], Some(icons::FOLDER_OPEN), None, idle)
            .on_hover_text(t(tl::BackgroundRemoverWindow_Open_models_folder))
            .clicked()
        {
            let _ = std::fs::create_dir_all(&self.models_folder);
            state
                .effects
                .push(Effect::OpenFile(self.models_folder.clone()));
            self.refresh_models();
        }
        kit::text_at(
            ui,
            m[3].shrink2(vec2(8.0, 0.0)),
            Align2::LEFT_CENTER,
            device_label,
            13.0,
            Weight::SemiBold,
            colors.text(),
        );
        let devices: Vec<(BackgroundRemovalDevice, String)> = BackgroundRemovalDevice::ALL
            .iter()
            .map(|d| (*d, background::device_name(*d).to_string()))
            .collect();
        if let Some(device) = kit::combo_at(
            ui,
            &colors,
            m[4],
            &format!("bg-device-{}", self.key),
            &self.options.selected_device,
            &devices,
            idle,
        )
        .filter(|d| *d != self.options.selected_device)
        {
            self.options.selected_device = device;
            self.save_options(state);
        }
        let _ = ui
            .interact(m[4], Id::new(("bg-device-tip", self.key)), Sense::hover())
            .on_hover_text(t(tl::BackgroundRemoverWindow_Processing_device));
        if kit::button_at(
            ui,
            &colors,
            c[2],
            None,
            Some(remove_label),
            self.can_remove(),
        )
        .clicked()
        {
            self.remove_background(state);
        }

        // The comparison.
        kit::panel(ui, &colors, rows[1]);
        if self.has_image() {
            self.comparison(ui, rows[1].shrink(12.0));
        } else {
            kit::text_at(
                ui,
                rows[1],
                Align2::CENTER_CENTER,
                t(tl::BackgroundRemoverWindow_Select_an_image_to_preview_it_here),
                16.0,
                Weight::Normal,
                colors.text(),
            );
        }

        // Save, Save as.
        let items = [
            (Some(icons::SAVE), Some(t(tl::BackgroundRemoverWindow_Save))),
            (
                Some(icons::SAVE_ALL),
                Some(t(tl::BackgroundRemoverWindow_Save_as)),
            ),
        ];
        let rects = kit::right_aligned(ui, &colors, rows[2], &items, 8.0);
        let can_save = self.can_save();
        if kit::button_at(ui, &colors, rects[0], items[0].0, items[0].1, can_save).clicked() {
            self.save(state, false, now);
        }
        if kit::button_at(ui, &colors, rects[1], items[1].0, items[1].1, can_save).clicked() {
            self.save(state, true, now);
        }

        self.paint_notice(ui, &colors, now);
        false
    }

    fn closed(&mut self, state: &mut AppState) {
        self.version += 1;
        // `BackgroundRemovalService.Dispose`, off the UI thread (a run may still hold it).
        state.rt.spawn_blocking(|| {
            modlight_core::background_removal::release();
            |_: &mut AppState, _: &egui::Context| {}
        });
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl BackgroundWindow {
    /// The notice: bottom centre, 68 px above the window's foot, up to 640 wide; held
    /// while the pointer is on it, gone when clicked.
    fn paint_notice(&mut self, ui: &mut Ui, colors: &crate::theme::Colors, now: f64) {
        let Some(notice) = self.notice.clone() else {
            return;
        };
        let full = ui.max_rect();
        let text_w = 640.0 - 2.0 * 14.0 - 18.0 - 10.0;
        let galley = crate::widgets::layout(
            ui,
            crate::widgets::wrapped(&notice.message, 13.0, Weight::Normal, colors.text(), text_w),
        );
        let w = 14.0 * 2.0 + 18.0 + 10.0 + galley.size().x;
        let h = (galley.size().y.max(18.0) + 2.0 * 11.0).round();
        let rect = Rect::from_min_size(
            pos2(full.center().x - w / 2.0, full.bottom() - 68.0 - h),
            vec2(w, h),
        );
        let response = ui.interact(rect, Id::new(("bg-notice", self.key)), Sense::click());
        if response.clicked() {
            self.notice = None;
            return;
        }
        if response.hovered() {
            if let Some(n) = self.notice.as_mut() {
                n.until = n.until.max(now + 0.05);
            }
        } else if now >= notice.until {
            self.notice = None;
            return;
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
        let painter = ui.painter();
        painter.add(
            egui::Shadow {
                offset: [0, 6],
                blur: 22,
                spread: 0,
                color: Color32::from_black_alpha(0x38),
            }
            .as_shape(rect, 10.0),
        );
        painter.rect_filled(rect, 10.0, colors.background_popup());
        painter.rect_stroke(
            rect,
            10.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        crate::widgets::paint_icon(
            ui,
            painter,
            pos2(rect.left() + 14.0 + 9.0, rect.center().y),
            notice.icon,
            18.0,
            colors.text(),
        );
        painter.galley(
            pos2(
                rect.left() + 14.0 + 18.0 + 10.0,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            colors.text(),
        );
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
}

impl AppState {
    /// `<profile>/Models` (`Program.ModelsFolder`).
    pub fn models_folder(&self) -> PathBuf {
        self.env
            .profile
            .join(modlight_core::components::MODELS_FOLDER)
    }

    /// `TaskHelpers.OpenBackgroundRemover`: without any model the Components window is
    /// offered first; the tool opens either way.
    pub fn open_background_remover(&mut self, settings: TaskSettings) {
        let folder = self.models_folder();
        if !background::has_model(&folder) {
            let then = crate::components_window::AfterClose::Tool(Box::new(
                ToolContinuation::BackgroundRemover {
                    settings: Box::new(settings.clone()),
                },
            ));
            if !crate::components_window::require(
                self,
                modlight_core::components::BACKGROUND_MODEL,
                s::Components_FeatureBackgroundRemover,
                then,
            ) {
                return;
            }
        }
        self.show_background_remover(settings);
    }

    /// `ToolsIntegration.ShowBackgroundRemoverWindow`.
    pub fn show_background_remover(&mut self, settings: TaskSettings) {
        let folder = self.models_folder();
        let models = background::list_models(&folder);
        self.tools
            .open_keyed(|key| Box::new(BackgroundWindow::new(key, &settings, folder, models)));
    }
}
