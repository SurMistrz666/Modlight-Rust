//! `OCRWindow` and `OCRViewModel`, and `TaskHelpers.OCRImage`: the picture's text read by
//! Vision, in the chosen language and scale, editable; copy it, open it in a search or
//! translation service (editable list), read another region or picture. Silent mode reads
//! straight to the clipboard. The text of a picture file is also written beside it.
//!
//! On Windows the text is read by Windows OCR, as upstream, and its languages are listed
//! as upstream lists them: the engine's tags with the names Windows gives them
//! (`Language.DisplayName`), sorted by name. A saved tag selects the same tag, else one
//! of the same language (a tag saved on a Mac, "pl-PL", meets Windows' "pl"), else the
//! first, and an engine with no language at all is upstream's "unavailable".
//!
//! Upstream's help button opened ShareX's documentation (getsharex.com), which the port
//! leaves out; the button is not drawn.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{Align2, Id, Rect, Sense, Ui, pos2, vec2};
use image::RgbaImage;
use modlight_core::icons;
use modlight_core::l10n::{t, tools as tl};
use modlight_core::tasks::host::NotificationSound;
use modlight_core::tasks::settings::TaskSettings;
use modlight_tools::ocr::{self, LanguageOption};
use modlight_tools::settings::{OcrOptions, OcrServiceLink};

use super::jobs::OptionsHome;
use super::kit::{self, Track};
use super::region::RegionFor;
use super::{ToolWindow, WindowSpec};
use crate::state::{AppState, Effect};
use crate::theme::Weight;

/// Vision's languages on the Mac this was written on (QA renders list these).
pub const QA_LANGUAGES: [&str; 33] = [
    "en-US", "fr-FR", "it-IT", "de-DE", "es-ES", "pt-BR", "zh-Hans", "zh-Hant", "yue-Hans",
    "yue-Hant", "ko-KR", "ja-JP", "ru-RU", "uk-UA", "th-TH", "vi-VT", "ar-SA", "ars-SA", "tr-TR",
    "id-ID", "cs-CZ", "da-DK", "nl-NL", "no-NO", "nn-NO", "nb-NO", "ms-MY", "pl-PL", "ro-RO",
    "sv-SE", "fi-FI", "hi-IN", "mr-IN",
];

/// What recognises the text: Vision in the real app, nothing (or a fixture) in QA runs.
#[derive(Clone, Debug, PartialEq)]
pub enum Recognizer {
    Vision,
    Fixed(String),
}

impl Recognizer {
    /// `OCRHelper.OCR(bitmap, language, scale, singleLine)`.
    pub fn recognize(
        &self,
        image: &RgbaImage,
        language: &str,
        scale: f64,
        single_line: bool,
    ) -> Result<String, String> {
        match self {
            Recognizer::Fixed(text) => Ok(text.clone()),
            Recognizer::Vision => {
                let scaled = ocr::scale(image, scale);
                let png = modlight_tools::images::png(&scaled);
                let lines = crate::platform::ocr::recognize(&png, language)?;
                Ok(ocr::join_lines(&lines, single_line))
            }
        }
    }
}

pub struct OcrWindow {
    pub key: u64,
    pub settings: TaskSettings,
    pub home: OptionsHome,
    pub options: OcrOptions,
    pub languages: Vec<LanguageOption>,
    pub language: Option<usize>,
    /// The picture read (upstream's window keeps it without showing it).
    pub source: Arc<RgbaImage>,
    pub result: String,
    pub error: String,
    pub busy: bool,
    pub editing_services: bool,
    pub editing: Option<usize>,
    version: u64,
    started: bool,
    recognizer: Recognizer,
    /// The picture file the text goes beside when the window closes.
    pub file: Option<PathBuf>,
    pending_region: Option<f64>,
    pub close: bool,
}

impl OcrWindow {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        key: u64,
        settings: TaskSettings,
        options: OcrOptions,
        languages: Vec<LanguageOption>,
        source: Arc<RgbaImage>,
        recognizer: Recognizer,
        file: Option<PathBuf>,
    ) -> OcrWindow {
        let language = ocr::select_language(&languages, &options.language);
        let mut options = options;
        options.scale_factor = options.scale_factor.clamp(1.0, 4.0);
        let home = OptionsHome::of_capture(&settings);
        OcrWindow {
            key,
            settings,
            home,
            options,
            languages,
            language,
            source,
            result: String::new(),
            error: String::new(),
            busy: false,
            editing_services: false,
            editing: None,
            version: 0,
            started: false,
            recognizer,
            file,
            pending_region: None,
            close: false,
        }
    }

    /// QA renders: the window as if its first recognition had run.
    pub fn qa_started(&mut self) {
        self.started = true;
    }

    pub fn can_use_result(&self) -> bool {
        !self.busy && !self.result.trim().is_empty()
    }

    /// `SaveOptions`.
    fn save(&mut self, state: &mut AppState) {
        if let Some(i) = self.language {
            self.options.language = self.languages[i].tag.clone();
        }
        let count = self.options.service_links.len() as i64;
        self.options.selected_service_link = self
            .options
            .selected_service_link
            .clamp(0, (count - 1).max(0));
        state.save_capture_options(&self.home, &["OCROptions"], &self.options);
    }

    /// `RecognizeAsync`.
    fn recognize(&mut self, state: &mut AppState) {
        let Some(index) = self.language else {
            return;
        };
        self.version += 1;
        let version = self.version;
        self.busy = true;
        self.error.clear();
        self.result.clear();
        let image = self.source.clone();
        let tag = self.languages[index].tag.clone();
        let scale = self.options.scale_factor;
        let single = self.options.single_line;
        let recognizer = self.recognizer.clone();
        let key = self.key;
        state.rt.spawn_blocking(move || {
            let result = recognizer.recognize(&image, &tag, scale, single);
            move |state: &mut AppState, _ctx: &egui::Context| {
                super::with_window::<OcrWindow, _>(state, key, |window, state| {
                    if window.version != version {
                        return;
                    }
                    window.busy = false;
                    match result {
                        Ok(text) => {
                            if window.options.auto_copy && !text.trim().is_empty() {
                                state.effects.push(Effect::CopyText(text.clone()));
                            }
                            window.result = text;
                        }
                        Err(error) => {
                            log::warn!("OCR failed: {error}");
                            window.error = error;
                        }
                    }
                });
            }
        });
    }

    fn rerun(&mut self, state: &mut AppState) {
        if self.started {
            self.save(state);
            self.recognize(state);
        }
    }

    /// `ReplaceSourceAsync`.
    pub fn replace_source(&mut self, state: &mut AppState, image: RgbaImage) {
        self.source = Arc::new(image);
        self.recognize(state);
    }
}

impl ToolWindow for OcrWindow {
    fn spec(&self) -> WindowSpec {
        WindowSpec::resizable(
            t(tl::OCRWindow_ShareX_Optical_character_recognition),
            vec2(940.0, 600.0),
            vec2(780.0, 540.0),
        )
    }

    fn ui(&mut self, ui: &mut Ui, state: &mut AppState) -> bool {
        let colors = state.colors.clone();
        let now = state.time;
        if !self.started {
            self.started = true;
            self.save(state);
            self.recognize(state);
        }
        if let Some(at) = self.pending_region {
            if now >= at {
                self.pending_region = None;
                let settings = self.settings.clone();
                state.tool_region(RegionFor::OcrWindow(self.key), settings);
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
        kit::fill(ui, &colors);
        let full = kit::inset(ui.max_rect(), 16.0);
        let cols = kit::columns(full, &[Track::Px(320.0), Track::Star(1.0)], 12.0);
        let idle = !self.busy;

        // Left: the source buttons.
        let items = [
            (Some(icons::CROP), Some(t(tl::OCRWindow_Select_region))),
            (Some(icons::IMAGE), Some(t(tl::OCRWindow_Select_image))),
        ];
        let row0 = Rect::from_min_size(cols[0].min, vec2(cols[0].width(), 40.0));
        let rects = kit::left_aligned(ui, &colors, row0, &items, 8.0);
        if kit::button_at(ui, &colors, rects[0], items[0].0, items[0].1, idle).clicked() {
            // `WindowState = Minimized; await Task.Delay(250)`.
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            self.pending_region = Some(now + 0.25);
        }
        if kit::button_at(ui, &colors, rects[1], items[1].0, items[1].1, idle).clicked()
            && let Some(path) = state.tool_open_file(
                t(tl::OCRWindow_Select_image_dialog),
                &[("Images", &super::files::IMAGE_EXTENSIONS)],
            )
            && let Some(image) = super::files::load_image(&path)
        {
            self.replace_source(state, image);
        }

        // Left: recognition.
        let line = 40.0;
        let toggle = 30.0;
        let rec_h = 12.0 * 2.0 + kit::HEADING_HEIGHT + 1.0 + 9.0 * 4.0 + line * 2.0 + toggle * 2.0;
        let rec = Rect::from_min_size(
            pos2(cols[0].left(), row0.bottom() + 12.0),
            vec2(cols[0].width(), rec_h),
        );
        kit::panel(ui, &colors, rec);
        let inner = rec.shrink(12.0);
        let mut y = inner.top();
        kit::heading(
            ui,
            &colors,
            Rect::from_min_size(
                pos2(inner.left(), y),
                vec2(inner.width(), kit::HEADING_HEIGHT),
            ),
            t(tl::OCRWindow_Recognition),
        );
        y += kit::HEADING_HEIGHT + 1.0 + 9.0;
        let r = kit::columns(
            Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), line)),
            &[Track::Px(90.0), Track::Star(1.0)],
            8.0,
        );
        kit::label(ui, &colors, r[0], t(tl::OCRWindow_Language));
        let options: Vec<(usize, String)> = self
            .languages
            .iter()
            .enumerate()
            .map(|(i, l)| (i, l.name.clone()))
            .collect();
        let current = self.language.unwrap_or(usize::MAX);
        if let Some(i) = kit::combo_at(
            ui,
            &colors,
            r[1],
            &format!("ocr-language-{}", self.key),
            &current,
            &options,
            !self.languages.is_empty(),
        ) && Some(i) != self.language
        {
            self.language = Some(i);
            self.rerun(state);
        }
        y += line + 9.0;
        let r = kit::columns(
            Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), line)),
            &[Track::Px(90.0), Track::Star(1.0)],
            8.0,
        );
        kit::label(ui, &colors, r[0], t(tl::OCRWindow_Scale_factor));
        let mut scale = self.options.scale_factor;
        if kit::number_at(
            ui,
            &colors,
            r[1],
            &format!("ocr-scale-{}", self.key),
            &mut scale,
            1.0,
            4.0,
            0.5,
            1,
            true,
        ) {
            self.options.scale_factor = scale;
            self.rerun(state);
        }
        y += line + 9.0;
        let r = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), toggle));
        kit::text_at(
            ui,
            r,
            Align2::LEFT_CENTER,
            t(tl::OCRWindow_Single_line),
            13.0,
            Weight::Normal,
            colors.text_secondary(),
        );
        if kit::toggle_at(
            ui,
            &colors,
            r,
            Id::new(("ocr-single", self.key)),
            &mut self.options.single_line,
            true,
        ) {
            self.rerun(state);
        }
        y += toggle + 9.0;
        let r = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), toggle));
        kit::text_at(
            ui,
            r,
            Align2::LEFT_CENTER,
            t(tl::OCRWindow_Copy_recognized_text_automatically),
            13.0,
            Weight::Normal,
            colors.text_secondary(),
        );
        if kit::toggle_at(
            ui,
            &colors,
            r,
            Id::new(("ocr-copy", self.key)),
            &mut self.options.auto_copy,
            true,
        ) {
            self.save(state);
        }

        // Left: services.
        let svc = Rect::from_min_max(
            pos2(cols[0].left(), rec.bottom() + 12.0),
            pos2(cols[0].right(), cols[0].bottom()),
        );
        kit::panel(ui, &colors, svc);
        let inner = svc.shrink(12.0);
        let mut y = inner.top();
        kit::heading(
            ui,
            &colors,
            Rect::from_min_size(
                pos2(inner.left(), y),
                vec2(inner.width(), kit::HEADING_HEIGHT),
            ),
            t(tl::OCRWindow_Services),
        );
        y += kit::HEADING_HEIGHT + 9.0;
        let edit_w = kit::button_width(ui, &colors, Some(icons::SETTINGS), None);
        let r = kit::columns(
            Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), line)),
            &[Track::Star(1.0), Track::Px(edit_w)],
            8.0,
        );
        let services: Vec<(usize, String)> = self
            .options
            .service_links
            .iter()
            .enumerate()
            .map(|(i, s)| (i, s.name.clone()))
            .collect();
        let selected = self.options.selected_service_link.max(0) as usize;
        if let Some(i) = kit::combo_at(
            ui,
            &colors,
            r[0],
            &format!("ocr-service-{}", self.key),
            &selected,
            &services,
            !services.is_empty(),
        ) {
            self.options.selected_service_link = i as i64;
            self.save(state);
        }
        if kit::button_at(ui, &colors, r[1], Some(icons::SETTINGS), None, true)
            .on_hover_text(t(tl::OCRWindow_Edit_services))
            .clicked()
        {
            // `EditServices`.
            self.editing = if self.options.service_links.is_empty() {
                None
            } else {
                Some(selected.min(self.options.service_links.len() - 1))
            };
            self.editing_services = true;
        }
        y += line + 9.0;
        let r = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), toggle));
        kit::text_at(
            ui,
            r,
            Align2::LEFT_CENTER,
            t(tl::OCRWindow_Close_after_opening),
            13.0,
            Weight::Normal,
            colors.text_secondary(),
        );
        if kit::toggle_at(
            ui,
            &colors,
            r,
            Id::new(("ocr-close", self.key)),
            &mut self.options.close_window_after_opening_service_link,
            true,
        ) {
            self.save(state);
        }
        y += toggle + 9.0;
        let r = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), line));
        if kit::button_at(
            ui,
            &colors,
            r,
            Some(icons::EXTERNAL_LINK),
            Some(t(tl::OCRWindow_Open_service)),
            self.can_use_result(),
        )
        .clicked()
            && let Some(service) = self.options.service_links.get(selected)
            && let Some(url) = ocr::service_link(&service.url, self.result.trim())
        {
            state.open_url(&url);
            if self.options.close_window_after_opening_service_link {
                self.close = true;
            }
        }

        // Right: the result.
        kit::panel(ui, &colors, cols[1]);
        let head = Rect::from_min_size(
            cols[1].min + vec2(12.0, 12.0),
            vec2(cols[1].width() - 24.0, 40.0),
        );
        kit::heading(ui, &colors, head, t(tl::OCRWindow_Result));
        let copy = kit::right_aligned(
            ui,
            &colors,
            head,
            &[(Some(icons::COPY), Some(t(tl::OCRWindow_Copy_all)))],
            8.0,
        )[0];
        if kit::button_at(
            ui,
            &colors,
            copy,
            Some(icons::COPY),
            Some(t(tl::OCRWindow_Copy_all)),
            self.can_use_result(),
        )
        .clicked()
        {
            state.effects.push(Effect::CopyText(self.result.clone()));
        }
        let mut top = head.bottom() + 12.0;
        if self.busy {
            let bar = Rect::from_min_size(
                pos2(cols[1].left(), head.bottom() + 12.0),
                vec2(cols[1].width(), 4.0),
            );
            crate::widgets::paint_progress(ui, &colors, bar, None);
            top = bar.bottom();
        }
        let area = Rect::from_min_max(
            pos2(cols[1].left() + 12.0, top),
            pos2(cols[1].right() - 12.0, cols[1].bottom() - 12.0),
        );
        let id = Id::new(("ocr-result", self.key));
        let focused = ui.memory(|m| m.has_focus(id));
        ui.painter().rect_filled(
            area,
            8.0,
            if idle {
                colors.control_background()
            } else {
                colors.control_background_disabled()
            },
        );
        ui.painter().rect_stroke(
            area,
            8.0,
            egui::Stroke::new(
                if focused { 2.0 } else { 1.0 },
                if focused {
                    colors.focus()
                } else {
                    colors.border()
                },
            ),
            egui::StrokeKind::Inside,
        );
        let inner = area.shrink2(vec2(8.0, 6.0));
        let mut child = kit::column(ui, inner);
        child.set_clip_rect(inner);
        egui::ScrollArea::vertical()
            .id_salt(id.with("scroll"))
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                ui.add_enabled(
                    idle,
                    egui::TextEdit::multiline(&mut self.result)
                        .id(id)
                        .frame(egui::Frame::NONE)
                        .font(egui::FontId::new(15.0, egui::FontFamily::Proportional))
                        .text_color(colors.text())
                        .margin(egui::Margin::ZERO)
                        .desired_width(inner.width() - 6.0),
                );
            });
        if !self.error.is_empty() {
            let galley = crate::widgets::layout(
                ui,
                crate::widgets::wrapped(
                    &self.error,
                    13.0,
                    Weight::Normal,
                    colors.text_secondary(),
                    420.0,
                ),
            );
            let total = 38.0 + 10.0 + galley.size().y;
            let top = area.center().y - total / 2.0;
            crate::widgets::paint_icon(
                ui,
                ui.painter(),
                pos2(area.center().x, top + 19.0),
                icons::TRIANGLE_ALERT,
                38.0,
                colors.cancel_border(),
            );
            ui.painter().galley(
                pos2(area.center().x - galley.size().x / 2.0, top + 48.0),
                galley,
                colors.text_secondary(),
            );
        }

        // The services editor over everything.
        if self.editing_services {
            self.services_editor(ui, state);
        }
        self.close
    }

    fn closed(&mut self, _state: &mut AppState) {
        self.version += 1;
        // `OCRImage(bmp, filePath)`: the text goes beside the picture file.
        let text = self.result.trim();
        if !text.is_empty()
            && let Some(file) = &self.file
            && let Err(error) = std::fs::write(ocr::text_file_for(file), text)
        {
            log::warn!("The recognised text could not be saved: {error}");
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl OcrWindow {
    /// The `IsEditingServices` overlay.
    fn services_editor(&mut self, ui: &mut Ui, state: &mut AppState) {
        let colors = state.colors.clone();
        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, colors.overlay_modal());
        let _ = ui.interact(full, Id::new(("ocr-modal", self.key)), Sense::click());
        let height = 18.0 * 2.0 + 40.0 + 12.0 + 40.0 + 12.0 + 40.0 * 2.0 + 10.0 + 12.0 + 40.0;
        let card = Rect::from_center_size(full.center(), vec2(620.0, height));
        kit::panel(ui, &colors, card);
        let inner = card.shrink(18.0);
        let rows = kit::rows(
            inner,
            &[
                Track::Px(40.0),
                Track::Px(40.0),
                Track::Px(90.0),
                Track::Px(40.0),
            ],
            12.0,
        );
        kit::heading(ui, &colors, rows[0], t(tl::OCRWindow_Edit_services));
        let items = [
            (Some(icons::PLUS), Some(t(tl::OCRWindow_New))),
            (Some(icons::TRASH_2), Some(t(tl::OCRWindow_Remove))),
            (Some(icons::ROTATE_CCW), Some(t(tl::OCRWindow_Reset))),
        ];
        let rects = kit::right_aligned(ui, &colors, rows[0], &items, 8.0);
        if kit::button_at(ui, &colors, rects[0], items[0].0, items[0].1, true).clicked() {
            // `NewService`.
            self.options.service_links.push(OcrServiceLink {
                name: t(tl::OCRViewModel_New_service_name).to_string(),
                url: "https://example.com/search?q={0}".into(),
            });
            let index = self.options.service_links.len() - 1;
            self.editing = Some(index);
            self.options.selected_service_link = index as i64;
            self.save(state);
        }
        if kit::button_at(
            ui,
            &colors,
            rects[1],
            items[1].0,
            items[1].1,
            self.editing.is_some(),
        )
        .clicked()
            && let Some(index) = self.editing
        {
            // `RemoveService`.
            self.options.service_links.remove(index);
            self.editing = if self.options.service_links.is_empty() {
                None
            } else {
                Some(
                    index
                        .saturating_sub(1)
                        .min(self.options.service_links.len() - 1),
                )
            };
            self.options.selected_service_link = self.editing.unwrap_or(0) as i64;
            self.save(state);
        }
        if kit::button_at(ui, &colors, rects[2], items[2].0, items[2].1, true).clicked() {
            // `ResetServices`.
            self.options.service_links = OcrOptions::default_service_links();
            self.editing = Some(0);
            self.options.selected_service_link = 0;
            self.save(state);
        }
        let services: Vec<(usize, String)> = self
            .options
            .service_links
            .iter()
            .enumerate()
            .map(|(i, s)| (i, s.name.clone()))
            .collect();
        let current = self.editing.unwrap_or(usize::MAX);
        if let Some(i) = kit::combo_at(
            ui,
            &colors,
            rows[1],
            &format!("ocr-editing-{}", self.key),
            &current,
            &services,
            true,
        ) {
            self.editing = Some(i);
        }
        if self.editing.is_none() {
            kit::text_at(
                ui,
                rows[1].shrink2(vec2(8.0, 0.0)),
                Align2::LEFT_CENTER,
                t(tl::OCRWindow_Select_a_service),
                13.0,
                Weight::Normal,
                colors.text_secondary(),
            );
        }
        let fields = kit::rows(rows[2], &[Track::Px(40.0), Track::Px(40.0)], 10.0);
        let enabled = self.editing.is_some();
        let mut changed = false;
        for (index, (label, field)) in [(t(tl::OCRWindow_Name), 0), (t(tl::OCRWindow_URL), 1)]
            .iter()
            .enumerate()
        {
            let c = kit::columns(fields[index], &[Track::Px(75.0), Track::Star(1.0)], 10.0);
            kit::label(ui, &colors, c[0], label);
            let mut empty = String::new();
            let value = match self
                .editing
                .and_then(|i| self.options.service_links.get_mut(i))
            {
                Some(service) => {
                    if *field == 0 {
                        &mut service.name
                    } else {
                        &mut service.url
                    }
                }
                None => &mut empty,
            };
            let hint = if *field == 1 {
                t(tl::OCRWindow_Use_0_where_the_recognized_text_belongs)
            } else {
                ""
            };
            if kit::text_box_at(
                ui,
                &colors,
                c[1],
                Id::new(("ocr-service-field", self.key, *field)),
                value,
                hint,
                enabled,
                false,
            )
            .changed()
            {
                changed = true;
            }
        }
        if changed {
            self.save(state);
        }
        let close = kit::right_aligned(
            ui,
            &colors,
            rows[3],
            &[(None, Some(t(tl::OCRWindow_Close)))],
            8.0,
        )[0];
        if kit::button_at(ui, &colors, close, None, Some(t(tl::OCRWindow_Close)), true).clicked() {
            self.editing_services = false;
            self.editing = None;
            self.save(state);
        }
    }
}

/// Where the language lists come from: Vision in the real app (set at its first frame),
/// the fixed list otherwise.
static VISION_LANGUAGES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// The real app reads Vision's list once.
pub fn use_vision_languages() {
    let _ = VISION_LANGUAGES.get_or_init(crate::platform::ocr::languages);
}

/// The tags the lists show, and whether they are the system's own (else the fixed list).
fn language_tags() -> (Vec<String>, bool) {
    match VISION_LANGUAGES.get() {
        Some(list) if !list.is_empty() => (list.clone(), true),
        // Windows may have no OCR language installed; the lists then say so.
        #[cfg(windows)]
        Some(_) => (Vec::new(), true),
        _ => (QA_LANGUAGES.iter().map(|s| s.to_string()).collect(), false),
    }
}

/// The tags named for the lists: the system's own on Windows by Windows, everything else
/// in the app's language.
fn named_languages((tags, system): (Vec<String>, bool)) -> Vec<LanguageOption> {
    #[cfg(windows)]
    {
        if system {
            return windows_languages(&tags);
        }
    }
    let _ = system;
    ocr::language_options(&tags)
}

/// Windows OCR's languages with the names Windows gives them (`OCRHelper.
/// AvailableLanguages`: `DisplayName`, in the language of Windows' own interface), sorted
/// by name. Read once: the names do not follow the app's language.
#[cfg(windows)]
fn windows_languages(tags: &[String]) -> Vec<LanguageOption> {
    static NAMED: std::sync::OnceLock<Vec<LanguageOption>> = std::sync::OnceLock::new();
    NAMED
        .get_or_init(|| {
            let mut options: Vec<LanguageOption> = tags
                .iter()
                .map(|tag| LanguageOption {
                    name: crate::platform::ocr::language_name(tag).unwrap_or_else(|| tag.clone()),
                    tag: tag.clone(),
                })
                .collect();
            options.sort_by_key(|o| o.name.to_lowercase());
            options
        })
        .clone()
}

/// The task settings' "Default language" (`OCROptions.Language`), with upstream's
/// "unavailable" state when the engine lists nothing.
pub fn settings_language_combo(
    ui: &mut Ui,
    colors: &crate::theme::Colors,
    b: &mut crate::settings_kit::Binder,
    path: &[&str],
) {
    let languages = named_languages(language_tags());
    if languages.is_empty() {
        ui.add_enabled_ui(false, |ui| {
            let list = vec![(
                0usize,
                t(modlight_core::l10n::s::TaskSettingsWindow_OCRLanguagesAreUnavailable)
                    .to_string(),
            )];
            let _ = crate::widgets::combo(ui, colors, "ocr-language", 250.0, &0usize, &list);
        });
        return;
    }
    let saved = b.str(path, "en");
    let current = ocr::select_language(&languages, &saved).unwrap_or(0);
    let list: Vec<(usize, String)> = languages
        .iter()
        .enumerate()
        .map(|(i, l)| (i, l.name.clone()))
        .collect();
    if let Some(i) = crate::widgets::combo(ui, colors, "ocr-language", 250.0, &current, &list) {
        b.set(path, serde_json::Value::String(languages[i].tag.clone()));
    }
}

impl AppState {
    /// Vision's languages (QA renders use the list this was written with).
    pub fn ocr_languages(&self) -> Vec<LanguageOption> {
        let tags = if self.env.qa || self.mode != crate::app::Mode::Normal {
            (QA_LANGUAGES.iter().map(|s| s.to_string()).collect(), false)
        } else {
            use_vision_languages();
            language_tags()
        };
        named_languages(tags)
    }

    fn ocr_recognizer(&self) -> Recognizer {
        if self.env.qa || self.mode != crate::app::Mode::Normal {
            Recognizer::Fixed(self.tools.qa_ocr_text.clone().unwrap_or_default())
        } else {
            Recognizer::Vision
        }
    }

    /// `OCRImage(taskSettings)`: a region first.
    pub fn ocr_region(&mut self, settings: TaskSettings) {
        self.tool_region(RegionFor::Ocr(Box::new(settings.clone())), settings);
    }

    /// `OCRImage(filePath, taskSettings)`.
    pub fn ocr_file(&mut self, file: PathBuf, settings: TaskSettings) {
        if let Some(image) = super::files::load_image(&file) {
            self.ocr_image(image, Some(file), settings);
        }
    }

    /// `OCRImage(bmp, filePath, taskSettings)`: silently to the clipboard, or the window.
    pub fn ocr_image(&mut self, image: RgbaImage, file: Option<PathBuf>, settings: TaskSettings) {
        let mut options: OcrOptions =
            modlight_tools::settings::read(Some(&settings.capture_settings.ocr_options));
        if options.silent {
            let languages = self.ocr_languages();
            let tag = ocr::select_language(&languages, &options.language)
                .map(|i| languages[i].tag.clone())
                .unwrap_or_else(|| options.language.clone());
            let recognizer = self.ocr_recognizer();
            self.rt.spawn_blocking(move || {
                let result =
                    recognizer.recognize(&image, &tag, options.scale_factor, options.single_line);
                move |state: &mut AppState, _ctx: &egui::Context| {
                    match result {
                        Ok(text) if !text.is_empty() => {
                            state.effects.push(Effect::CopyText(text.clone()));
                            if let Some(file) = &file {
                                let _ = std::fs::write(ocr::text_file_for(file), &text);
                            }
                        }
                        Ok(_) => state.effects.push(Effect::ClearClipboard),
                        Err(error) => {
                            state.uploads.windows.messages.push_back(
                                crate::dialogs::message::MessageBox::new(
                                    &format!("Modlight - {}", t(modlight_core::l10n::s::Error)),
                                    &error,
                                    modlight_core::tasks::host::MessageIcon::Error,
                                ),
                            );
                            return;
                        }
                    }
                    state.play_task_sound(NotificationSound::ActionCompleted, &settings);
                }
            });
            return;
        }
        options.refresh_default_links();
        let languages = self.ocr_languages();
        let recognizer = self.ocr_recognizer();
        self.tools.open_keyed(|key| {
            Box::new(OcrWindow::new(
                key,
                settings,
                options,
                languages,
                Arc::new(image),
                recognizer,
                file,
            ))
        });
    }

    /// A region for the OCR window or the OCR hotkey arrived.
    pub(super) fn ocr_region_done(
        &mut self,
        window: Option<u64>,
        settings: Option<TaskSettings>,
        picture: super::region::ScreenPicture,
    ) {
        match (window, settings) {
            (Some(key), _) => {
                self.tools.restore.push(key);
                if let Some((image, _)) = picture {
                    super::with_window::<OcrWindow, _>(self, key, |window, state| {
                        window.replace_source(state, (*image).clone());
                    });
                }
            }
            (None, Some(settings)) => {
                if let Some((image, _)) = picture {
                    self.ocr_image((*image).clone(), None, settings);
                }
            }
            _ => {}
        }
    }
}
