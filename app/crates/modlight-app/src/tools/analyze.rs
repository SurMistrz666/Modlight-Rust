//! `AnalyzeImageWindow` and `AnalyzeImageOptionsWindow` with their view models, and
//! `TaskHelpers.AnalyzeImage`: a picture (region, file, drop, or the capture that ran the
//! task) and a prompt sent to the provider chosen in the options, with the user's own key;
//! the answer is shown, copied when asked. Requests go out only when the user runs the
//! analysis (or the window's automatic start the options ask for), "Test connection" or
//! "Load models". The keys live in the Keychain only (QA runs hold them in memory).
//!
//! As upstream, the main menus do not list the tool (`MainMenuBuilder` hides it on
//! purpose); the hotkey, the after-capture task and the history windows open it.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use egui::{Align2, Id, Rect, Sense, TextureHandle, Ui, pos2, vec2};
use image::RgbaImage;
use modlight_core::account::CredentialStore;
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, t, tools as tl};
use modlight_core::tasks::host::NotificationSound;
use modlight_core::tasks::settings::TaskSettings;
use modlight_tools::analyze::{self, Endpoints, Picture};
use modlight_tools::settings::{AiOptions, AiProvider};

use super::jobs::{OptionsHome, tool_options};
use super::kit::{self, Track};
use super::region::RegionFor;
use super::{ToolWindow, WindowSpec};
use crate::state::{AppState, Effect};
use crate::theme::Weight;

/// The Keychain items of the three keys (service "Modlight").
const KEY_ACCOUNTS: [&str; 3] = [
    "ai-openai-api-key",
    "ai-gemini-api-key",
    "ai-openrouter-api-key",
];

/// Where requests go: the providers, or nowhere (QA runs answer from a fixture unless
/// `MODLIGHT_QA_AI_ENDPOINT` names a stub on 127.0.0.1).
#[derive(Clone, Debug, PartialEq)]
pub enum Network {
    Live(Endpoints),
    Offline(Result<String, String>),
}

impl Network {
    async fn analyze(self, options: AiOptions, picture: Picture) -> Result<String, String> {
        match self {
            Network::Offline(answer) => answer,
            Network::Live(endpoints) => {
                let client = analyze::client()?;
                analyze::analyze(&client, &options, picture, &endpoints).await
            }
        }
    }

    async fn test_connection(self, options: AiOptions) -> (bool, String) {
        match self {
            Network::Offline(_) => (false, OFFLINE.into()),
            Network::Live(endpoints) => match analyze::client() {
                Ok(client) => analyze::test_connection(&client, &options, &endpoints).await,
                Err(error) => (false, error),
            },
        }
    }

    async fn load_models(self, options: AiOptions) -> Result<Vec<String>, String> {
        match self {
            Network::Offline(_) => Err(OFFLINE.into()),
            Network::Live(endpoints) => {
                let client = analyze::client()?;
                analyze::load_models(&client, &options, &endpoints).await
            }
        }
    }
}

const OFFLINE: &str = "QA runs send no requests to AI providers.";

/// The picture in the window (`_imagePath` / `_imageData` and the preview).
#[derive(Clone)]
struct Loaded {
    file: Option<PathBuf>,
    image: Arc<RgbaImage>,
}

/// What follows a region capture.
#[derive(Clone, Copy, Debug, PartialEq)]
enum AfterRegion {
    Analyze,
    /// `InitializeAsync`: analyse only when the options start it automatically.
    Initialize,
}

pub struct AnalyzeWindow {
    pub key: u64,
    pub settings: TaskSettings,
    pub home: OptionsHome,
    pub options: AiOptions,
    loaded: Option<Loaded>,
    texture: Option<TextureHandle>,
    pub preset: Option<usize>,
    pub result: String,
    pub elapsed: String,
    pub busy: bool,
    pub error: String,
    version: u64,
    started: bool,
    pending_region: Option<(f64, AfterRegion)>,
    network: Network,
    /// The options dialog (`ShowDialog(this)`): this window waits while it is open.
    pub options_window: Option<u64>,
    /// QA renders: the name shown instead of the fixture's temporary path.
    pub qa_display: Option<String>,
}

impl AnalyzeWindow {
    pub fn new(
        key: u64,
        settings: TaskSettings,
        options: AiOptions,
        file: Option<PathBuf>,
        network: Network,
    ) -> AnalyzeWindow {
        let home = OptionsHome::of(&settings);
        let mut window = AnalyzeWindow {
            key,
            settings,
            home,
            options,
            loaded: None,
            texture: None,
            preset: None,
            result: String::new(),
            elapsed: String::new(),
            busy: false,
            error: String::new(),
            version: 0,
            started: false,
            pending_region: None,
            network,
            options_window: None,
            qa_display: None,
        };
        if let Some(file) = file {
            window.load_file(&file);
        }
        window
    }

    /// QA renders: the window as if it had opened (no automatic start).
    pub fn qa_started(&mut self) {
        self.started = true;
    }

    pub fn has_image(&self) -> bool {
        self.loaded.is_some()
    }

    pub fn has_result(&self) -> bool {
        !self.result.trim().is_empty()
    }

    pub fn can_analyze(&self) -> bool {
        self.has_image() && self.options.has_api_key() && !self.busy
    }

    /// `ImageDescription`.
    fn description(&self) -> String {
        if let Some(name) = &self.qa_display {
            return name.clone();
        }
        match self.loaded.as_ref().and_then(|l| l.file.as_ref()) {
            Some(file) => file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            None => t(tl::AnalyzeImageViewModel_Captured_region).to_string(),
        }
    }

    /// `LoadImage(filePath)`.
    pub fn load_file(&mut self, file: &std::path::Path) -> bool {
        match modlight_tools::images::load(file) {
            Ok(image) => {
                self.loaded = Some(Loaded {
                    file: Some(file.to_path_buf()),
                    image: Arc::new(image),
                });
                self.texture = None;
                self.qa_display = None;
                self.error.clear();
                true
            }
            Err(error) => {
                self.error = error.to_string();
                false
            }
        }
    }

    /// `LoadImage(byte[])`: a captured region.
    pub fn load_picture(&mut self, image: RgbaImage) {
        self.loaded = Some(Loaded {
            file: None,
            image: Arc::new(image),
        });
        self.texture = None;
        self.qa_display = None;
        self.error.clear();
    }

    /// `AnalyzeAsync`.
    pub fn analyze(&mut self, state: &mut AppState) {
        if !self.can_analyze() {
            if self.has_image() && !self.options.has_api_key() {
                self.error = t(tl::AnalyzeImageViewModel_Configure_API_key).to_string();
            }
            return;
        }
        let Some(loaded) = self.loaded.clone() else {
            return;
        };
        self.busy = true;
        self.result = t(tl::AnalyzeImageViewModel_Thinking).to_string();
        self.elapsed.clear();
        self.error.clear();
        // The prompt is part of the options (`OnPromptChanged`).
        state.save_ai_options(&self.home, &self.options);
        self.version += 1;
        let version = self.version;
        let key = self.key;
        let options = self.options.clone();
        let picture = match loaded.file {
            Some(file) => Picture::File(file),
            None => Picture::Image(loaded.image),
        };
        let network = self.network.clone();
        let started = Instant::now();
        state.rt.spawn(async move {
            let result = network.analyze(options, picture).await;
            let ms = started.elapsed().as_millis() as i64;
            move |state: &mut AppState, _ctx: &egui::Context| {
                super::with_window::<AnalyzeWindow, _>(state, key, |window, state| {
                    if window.version != version {
                        return;
                    }
                    window.finished(state, result, ms);
                });
            }
        });
    }

    /// The answer arrived (or the request failed).
    pub fn finished(&mut self, state: &mut AppState, result: Result<String, String>, ms: i64) {
        self.busy = false;
        match result {
            Ok(text) => {
                self.result = analyze::result_text(&text);
                self.elapsed = format(t(tl::AnalyzeImageViewModel_Time_ms), &[Arg::Int(ms)]);
                if self.options.auto_copy_result && self.has_result() {
                    state.effects.push(Effect::CopyText(self.result.clone()));
                }
                if self.has_result() {
                    state.play_task_sound(NotificationSound::ActionCompleted, &self.settings);
                }
            }
            Err(error) => {
                log::warn!("Image analysis failed: {error}");
                self.result.clear();
                self.error = error;
            }
        }
    }

    /// `CaptureRegionAsync`: the window steps aside for 250 ms, then the overlay.
    fn capture_region(&mut self, ui: &Ui, now: f64, then: AfterRegion) {
        if self.busy {
            return;
        }
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        self.pending_region = Some((now + 0.25, then));
    }

    /// A region came back (or the overlay was cancelled).
    pub fn region_done(&mut self, state: &mut AppState, image: Option<RgbaImage>) {
        let then = self
            .pending_region
            .take()
            .map(|(_, then)| then)
            .unwrap_or(AfterRegion::Analyze);
        let Some(image) = image else {
            return;
        };
        self.load_picture(image);
        match then {
            AfterRegion::Analyze => self.analyze(state),
            AfterRegion::Initialize => {
                if self.options.auto_start_analyze {
                    self.analyze(state);
                }
            }
        }
    }

    /// `InitializeAsync`.
    fn initialize(&mut self, ui: &Ui, state: &mut AppState, now: f64) {
        if !self.has_image() && self.options.auto_start_region {
            self.capture_region(ui, now, AfterRegion::Initialize);
            return;
        }
        if self.has_image() && self.options.auto_start_analyze {
            self.analyze(state);
        }
    }

    fn texture(&mut self, ctx: &egui::Context) -> Option<TextureHandle> {
        if self.texture.is_none()
            && let Some(loaded) = &self.loaded
        {
            let image = &loaded.image;
            // Large pictures are shown from a copy the GPU takes.
            let max = ctx.input(|i| i.max_texture_side).max(1024) as u32;
            let preview = if image.width() > max || image.height() > max {
                let scale = max as f32 / image.width().max(image.height()) as f32;
                image::imageops::thumbnail(
                    &**image,
                    ((image.width() as f32 * scale) as u32).max(1),
                    ((image.height() as f32 * scale) as u32).max(1),
                )
            } else {
                (**image).clone()
            };
            self.texture = Some(kit::texture(
                ctx,
                &format!("analyze-image-{}", self.key),
                &preview,
                false,
            ));
        }
        self.texture.clone()
    }
}

impl ToolWindow for AnalyzeWindow {
    fn spec(&self) -> WindowSpec {
        WindowSpec::resizable(
            t(tl::AnalyzeImageWindow_ShareX_Analyze_image),
            vec2(1000.0, 650.0),
            vec2(820.0, 560.0),
        )
    }

    fn ui(&mut self, ui: &mut Ui, state: &mut AppState) -> bool {
        let colors = state.colors.clone();
        let now = state.time;
        if !self.started {
            self.started = true;
            self.initialize(ui, state, now);
        }
        if let Some((at, _)) = self.pending_region
            && at > 0.0
        {
            if now >= at {
                // Waiting for the overlay now: the time is spent.
                if let Some(pending) = self.pending_region.as_mut() {
                    pending.0 = -1.0;
                }
                let settings = self.settings.clone();
                state.tool_region(RegionFor::AnalyzeWindow(self.key), settings);
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
        kit::fill(ui, &colors);
        let modal = self.options_window.is_some();
        if modal {
            ui.disable();
        }
        let idle = !self.busy;

        // A file dropped on the window becomes the picture (`OnDrop`).
        let dropped: Option<PathBuf> = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .find(|p| p.is_file())
        });
        if let Some(path) = dropped
            && idle
            && !modal
        {
            self.load_file(&path);
        }

        let full = kit::inset(ui.max_rect(), 16.0);
        let cols = kit::columns(full, &[Track::Px(360.0), Track::Star(1.0)], 12.0);
        let prompt_h = 12.0 * 2.0 + kit::HEADING_HEIGHT + 9.0 + 40.0 + 9.0 + 110.0;
        let left = kit::rows(cols[0], &[Track::Star(1.0), Track::Px(prompt_h)], 12.0);

        // Left top: the picture.
        kit::panel(ui, &colors, left[0]);
        let head = Rect::from_min_size(
            left[0].min + vec2(12.0, 12.0),
            vec2(left[0].width() - 24.0, kit::HEADING_HEIGHT),
        );
        let heading = kit::heading(ui, &colors, head, t(tl::AnalyzeImageWindow_Image));
        if self.has_image() {
            let path = Rect::from_min_max(pos2(heading.right() + 8.0, head.top()), head.max);
            kit::path_text(ui, &colors, path, &self.description());
        }
        let row = Rect::from_min_size(
            pos2(left[0].left() + 12.0, head.bottom() + 8.0),
            vec2(left[0].width() - 24.0, 40.0),
        );
        let items = [
            (
                Some(icons::CROP),
                Some(t(tl::AnalyzeImageWindow_Select_region)),
            ),
            (
                Some(icons::IMAGE),
                Some(t(tl::AnalyzeImageWindow_Select_image)),
            ),
        ];
        let rects = kit::left_aligned(ui, &colors, row, &items, 8.0);
        if kit::button_at(ui, &colors, rects[0], items[0].0, items[0].1, idle).clicked() {
            self.capture_region(ui, now, AfterRegion::Analyze);
        }
        if kit::button_at(ui, &colors, rects[1], items[1].0, items[1].1, idle).clicked()
            && let Some(path) = state.tool_open_file(
                t(tl::AnalyzeImageWindow_Select_image_dialog),
                &[("Images", &super::files::IMAGE_EXTENSIONS)],
            )
            && self.load_file(&path)
        {
            self.analyze(state);
        }
        let preview = Rect::from_min_max(
            pos2(left[0].left() + 12.0, row.bottom() + 12.0),
            pos2(left[0].right() - 12.0, left[0].bottom() - 12.0),
        );
        ui.painter()
            .rect_filled(preview, 4.0, colors.control_background());
        ui.painter().rect_stroke(
            preview,
            4.0,
            egui::Stroke::new(1.0, colors.border()),
            egui::StrokeKind::Inside,
        );
        match self.texture(ui.ctx()) {
            Some(texture) => {
                kit::image_uniform(ui, &texture, preview.shrink(8.0), true);
                let click = ui.interact(
                    preview,
                    Id::new(("analyze-preview", self.key)),
                    Sense::click(),
                );
                if click.clicked()
                    && let Some(loaded) = &self.loaded
                {
                    // `ImageViewerWindowIntegration.ShowImage(data, description)`.
                    state.open_image_viewer_picture((*loaded.image).clone(), &self.description());
                }
            }
            None => kit::placeholder(
                ui,
                &colors,
                preview,
                icons::IMAGE_UP,
                t(tl::AnalyzeImageWindow_Select_or_drop_an_image),
                42.0,
            ),
        }

        // Left bottom: the prompt.
        kit::panel(ui, &colors, left[1]);
        let inner = left[1].shrink(12.0);
        let rows = kit::rows(
            inner,
            &[
                Track::Px(kit::HEADING_HEIGHT),
                Track::Px(40.0),
                Track::Star(1.0),
            ],
            9.0,
        );
        kit::heading(ui, &colors, rows[0], t(tl::AnalyzeImageWindow_Prompt));
        let presets = analyze::preset_prompts();
        let list: Vec<(usize, String)> = presets
            .iter()
            .enumerate()
            .map(|(i, p)| (i, p.to_string()))
            .collect();
        let current = self.preset.unwrap_or(usize::MAX);
        if let Some(i) = kit::combo_at(
            ui,
            &colors,
            rows[1],
            &format!("analyze-preset-{}", self.key),
            &current,
            &list,
            true,
        ) {
            // `OnSelectedPresetChanged`.
            self.preset = Some(i);
            self.options.input = presets[i].to_string();
        }
        if self.preset.is_none() {
            kit::text_at(
                ui,
                rows[1].shrink2(vec2(12.0, 0.0)),
                Align2::LEFT_CENTER,
                t(tl::AnalyzeImageWindow_Choose_a_preset_prompt),
                13.0,
                Weight::Normal,
                colors.text_secondary().gamma_multiply(0.75),
            );
        }
        kit::text_area_at(
            ui,
            &colors,
            rows[2],
            Id::new(("analyze-prompt", self.key)),
            &mut self.options.input,
            "",
            true,
            false,
            false,
        );

        // Right: the result.
        let right = cols[1];
        kit::panel(ui, &colors, right);
        let head = Rect::from_min_size(
            right.min + vec2(12.0, 12.0),
            vec2(right.width() - 24.0, 40.0),
        );
        kit::heading(ui, &colors, head, t(tl::AnalyzeImageWindow_Result));
        let items = [
            (
                Some(icons::SETTINGS),
                Some(t(tl::AnalyzeImageWindow_Options)),
            ),
            (Some(icons::COPY), Some(t(tl::AnalyzeImageWindow_Copy_all))),
        ];
        let rects = kit::right_aligned(ui, &colors, head, &items, 8.0);
        if kit::button_at(ui, &colors, rects[0], items[0].0, items[0].1, idle).clicked() {
            state.open_analyze_options(self.key, &self.home, &self.options, self.network.clone());
        }
        if kit::button_at(
            ui,
            &colors,
            rects[1],
            items[1].0,
            items[1].1,
            self.has_result(),
        )
        .clicked()
        {
            state.effects.push(Effect::CopyText(self.result.clone()));
            state.play_task_sound(NotificationSound::ActionCompleted, &self.settings);
        }
        let mut top = head.bottom() + 12.0;
        if self.busy {
            let bar = Rect::from_min_size(pos2(right.left(), top), vec2(right.width(), 4.0));
            crate::widgets::paint_progress(ui, &colors, bar, None);
            top = bar.bottom();
            ui.ctx().request_repaint();
        }
        let bottom = Rect::from_min_max(
            pos2(right.left() + 12.0, right.bottom() - 12.0 - 40.0),
            pos2(right.right() - 12.0, right.bottom() - 12.0),
        );
        let area = Rect::from_min_max(
            pos2(right.left() + 12.0, top),
            pos2(right.right() - 12.0, bottom.top() - 12.0),
        );
        result_box(
            ui,
            &colors,
            area,
            Id::new(("analyze-result", self.key)),
            &self.result,
        );
        if !self.error.trim().is_empty() {
            let galley = crate::widgets::layout(
                ui,
                crate::widgets::wrapped(&self.error, 13.0, Weight::Normal, colors.text(), 430.0),
            );
            let total = 38.0 + 10.0 + galley.size().y;
            let y = area.center().y - total / 2.0;
            crate::widgets::paint_icon(
                ui,
                ui.painter(),
                pos2(area.center().x, y + 19.0),
                icons::TRIANGLE_ALERT,
                38.0,
                colors.cancel_border(),
            );
            ui.painter().galley(
                pos2(area.center().x - galley.size().x / 2.0, y + 48.0),
                galley,
                colors.text(),
            );
        }
        kit::text_at(
            ui,
            bottom,
            Align2::LEFT_CENTER,
            &self.elapsed,
            13.0,
            Weight::Normal,
            colors.text_secondary(),
        );
        let analyze_item = [(
            Some(icons::SPARKLES),
            Some(t(tl::AnalyzeImageWindow_Analyze_image)),
        )];
        let r = kit::right_aligned(ui, &colors, bottom, &analyze_item, 8.0)[0];
        if kit::button_at(
            ui,
            &colors,
            r,
            analyze_item[0].0,
            analyze_item[0].1,
            self.can_analyze(),
        )
        .clicked()
        {
            self.analyze(state);
        }
        false
    }

    fn closed(&mut self, state: &mut AppState) {
        self.version += 1;
        state.save_ai_options(&self.home, &self.options);
        if let Some(options) = self.options_window {
            state.tools.windows.retain(|h| h.key != options);
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// The read-only result box (`IsReadOnly="True"`, 15 px, wrapping): selectable text.
fn result_box(ui: &mut Ui, colors: &crate::theme::Colors, area: Rect, id: Id, text: &str) {
    let focused = ui.memory(|m| m.has_focus(id));
    ui.painter()
        .rect_filled(area, 8.0, colors.control_background());
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
    let mut shown = text;
    egui::ScrollArea::vertical()
        .id_salt(id.with("scroll"))
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut shown)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .font(egui::FontId::new(15.0, egui::FontFamily::Proportional))
                    .text_color(colors.text())
                    .margin(egui::Margin::ZERO)
                    .desired_width(inner.width() - 6.0),
            );
        });
}

/// `AnalyzeImageOptionsWindow`: a copy of the options, saved or dropped.
pub struct AnalyzeOptionsWindow {
    pub key: u64,
    pub parent: u64,
    pub home: OptionsHome,
    pub draft: AiOptions,
    pub openai_models: Vec<String>,
    pub gemini_models: Vec<String>,
    pub open_router_models: Vec<String>,
    pub busy: bool,
    pub status: String,
    pub status_success: bool,
    network: Network,
    version: u64,
    close: bool,
}

fn models_with(list: &[&str], current: &str) -> Vec<String> {
    let mut models: Vec<String> = list.iter().map(|m| m.to_string()).collect();
    // `AddCurrentModel`.
    if !current.trim().is_empty() && !models.iter().any(|m| m == current) {
        models.push(current.to_string());
    }
    models
}

impl AnalyzeOptionsWindow {
    pub fn new(
        key: u64,
        parent: u64,
        home: OptionsHome,
        options: &AiOptions,
        network: Network,
    ) -> AnalyzeOptionsWindow {
        AnalyzeOptionsWindow {
            key,
            parent,
            home,
            openai_models: models_with(&analyze::OPENAI_MODELS, &options.openai_model),
            gemini_models: models_with(&analyze::GEMINI_MODELS, &options.gemini_model),
            open_router_models: models_with(
                &analyze::OPENROUTER_MODELS,
                &options.open_router_model,
            ),
            draft: options.clone(),
            busy: false,
            status: String::new(),
            status_success: false,
            network,
            version: 0,
            close: false,
        }
    }

    fn is_openai(&self) -> bool {
        matches!(
            self.draft.provider,
            AiProvider::OpenAI | AiProvider::OpenAILegacy
        )
    }

    /// `TestConnectionAsync`.
    fn test_connection(&mut self, state: &mut AppState) {
        self.busy = true;
        self.status_success = false;
        self.status = t(tl::AnalyzeImageOptionsViewModel_Testing).to_string();
        self.version += 1;
        let (key, version) = (self.key, self.version);
        let network = self.network.clone();
        let options = self.draft.clone();
        state.rt.spawn(async move {
            let (success, message) = network.test_connection(options).await;
            move |state: &mut AppState, _ctx: &egui::Context| {
                super::with_window::<AnalyzeOptionsWindow, _>(state, key, |window, _| {
                    if window.version == version {
                        window.busy = false;
                        window.status_success = success;
                        window.status = message;
                    }
                });
            }
        });
    }

    /// `LoadModelsAsync`.
    fn load_models(&mut self, state: &mut AppState) {
        self.busy = true;
        self.status_success = false;
        self.status = t(tl::AnalyzeImageOptionsViewModel_Loading_models).to_string();
        self.version += 1;
        let (key, version) = (self.key, self.version);
        let network = self.network.clone();
        let options = self.draft.clone();
        state.rt.spawn(async move {
            let result = network.load_models(options).await;
            move |state: &mut AppState, _ctx: &egui::Context| {
                super::with_window::<AnalyzeOptionsWindow, _>(state, key, |window, _| {
                    if window.version == version {
                        window.models_loaded(result);
                    }
                });
            }
        });
    }

    pub fn models_loaded(&mut self, result: Result<Vec<String>, String>) {
        self.busy = false;
        match result {
            Ok(models) => {
                self.openai_models = models.clone();
                if models.is_empty() {
                    self.status = t(tl::AnalyzeImageOptionsViewModel_No_models_found).to_string();
                } else {
                    if !models.contains(&self.draft.openai_model) {
                        self.draft.openai_model = models[0].clone();
                    }
                    self.status_success = true;
                    self.status = format(
                        t(tl::AnalyzeImageOptionsViewModel_Models_loaded),
                        &[Arg::Int(models.len() as i64)],
                    );
                }
            }
            Err(error) => self.status = error,
        }
    }

    /// `Save`: the draft (with the window's current prompt) replaces the options.
    fn save(&mut self, state: &mut AppState) {
        let mut updated = self.draft.clone();
        let parent = self.parent;
        let input =
            super::with_window::<AnalyzeWindow, _>(state, parent, |w, _| w.options.input.clone());
        if let Some(input) = input {
            updated.input = input;
        }
        if let Err(error) = state.save_ai_keys(&updated) {
            self.status_success = false;
            self.status = error;
            return;
        }
        state.save_ai_options(&self.home, &updated);
        super::with_window::<AnalyzeWindow, _>(state, parent, |w, _| {
            w.options = updated.clone();
        });
        self.close = true;
    }

    #[allow(clippy::too_many_arguments)]
    fn key_row(
        ui: &mut Ui,
        colors: &crate::theme::Colors,
        rect: Rect,
        id: Id,
        key: &mut Option<String>,
    ) {
        let c = kit::columns(rect, &[Track::Px(120.0), Track::Star(1.0)], 10.0);
        kit::label(ui, colors, c[0], t(tl::AnalyzeImageOptionsWindow_API_key));
        let mut text = key.clone().unwrap_or_default();
        if kit::password_box_at(ui, colors, c[1], id, &mut text, true).changed() {
            *key = Some(text);
        }
    }

    fn behaviour_row(
        ui: &mut Ui,
        colors: &crate::theme::Colors,
        rect: Rect,
        id: Id,
        label: &str,
        value: &mut bool,
    ) {
        kit::text_at(
            ui,
            rect,
            Align2::LEFT_CENTER,
            label,
            13.0,
            Weight::Normal,
            colors.text(),
        );
        kit::toggle_at(ui, colors, rect, id, value, true);
    }
}

/// The heights of the options window's panels.
const OPENAI_PANEL: f32 = 12.0 * 2.0 + kit::HEADING_HEIGHT + 10.0 * 4.0 + 40.0 * 4.0;
const OTHER_PANEL: f32 = 12.0 * 2.0 + kit::HEADING_HEIGHT + 10.0 * 2.0 + 40.0 * 2.0;
const BEHAVIOUR_PANEL: f32 = 12.0 * 2.0 + kit::HEADING_HEIGHT + 8.0 * 3.0 + 30.0 * 3.0;

impl ToolWindow for AnalyzeOptionsWindow {
    fn spec(&self) -> WindowSpec {
        WindowSpec::resizable(
            t(tl::AnalyzeImageOptionsWindow_ShareX_Analyze_image_options),
            vec2(650.0, 590.0),
            vec2(580.0, 540.0),
        )
    }

    fn ui(&mut self, ui: &mut Ui, state: &mut AppState) -> bool {
        let colors = state.colors.clone();
        kit::fill(ui, &colors);
        let key = self.key;
        let id = |name: &str| Id::new(("analyze-options", key, name.to_string()));
        let full = kit::inset(ui.max_rect(), 16.0);
        let rows = kit::rows(
            full,
            &[Track::Px(64.0), Track::Star(1.0), Track::Px(40.0)],
            12.0,
        );

        // The provider.
        kit::panel(ui, &colors, rows[0]);
        let page_item = [(
            Some(icons::KEY_ROUND),
            Some(t(tl::AnalyzeImageOptionsWindow_API_key_page)),
        )];
        let page_w = kit::button_width(ui, &colors, page_item[0].0, page_item[0].1);
        let c = kit::columns(
            rows[0].shrink(12.0),
            &[Track::Px(100.0), Track::Star(1.0), Track::Px(page_w)],
            10.0,
        );
        kit::label(ui, &colors, c[0], t(tl::AnalyzeImageOptionsWindow_Provider));
        let providers: Vec<(AiProvider, String)> = AiProvider::ALL
            .iter()
            .map(|p| (*p, analyze::provider_name(*p).to_string()))
            .collect();
        if let Some(provider) = kit::combo_at(
            ui,
            &colors,
            c[1],
            &format!("analyze-provider-{key}"),
            &self.draft.provider,
            &providers,
            true,
        ) {
            self.draft.provider = provider;
        }
        if kit::button_at(ui, &colors, c[2], page_item[0].0, page_item[0].1, true)
            .on_hover_text(t(tl::AnalyzeImageOptionsWindow_Get_API_key))
            .clicked()
        {
            state.open_url(analyze::api_key_page(self.draft.provider));
        }

        // The provider's settings and the behaviour, scrolling.
        let provider_h = if self.is_openai() {
            OPENAI_PANEL
        } else {
            OTHER_PANEL
        };
        let total = provider_h + 12.0 + BEHAVIOUR_PANEL;
        let mut scroll_ui = kit::column(ui, rows[1]);
        scroll_ui.set_clip_rect(rows[1]);
        let mut load_models = false;
        egui::ScrollArea::vertical()
            .id_salt(id("scroll"))
            .auto_shrink([false, false])
            .show(&mut scroll_ui, |ui| {
                let (area, _) =
                    ui.allocate_exact_size(vec2(rows[1].width(), total), Sense::hover());
                let panel = Rect::from_min_size(area.min, vec2(area.width(), provider_h));
                kit::panel(ui, &colors, panel);
                let inner = panel.shrink(12.0);
                let mut y = inner.top();
                let line = |y: f32| Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 40.0));
                let heading = match self.draft.provider {
                    AiProvider::OpenAI | AiProvider::OpenAILegacy => {
                        t(tl::AnalyzeImageOptionsWindow_OpenAI)
                    }
                    AiProvider::Gemini => t(tl::AnalyzeImageOptionsWindow_Google_Gemini),
                    AiProvider::OpenRouter => t(tl::AnalyzeImageOptionsWindow_OpenRouter),
                };
                kit::heading(
                    ui,
                    &colors,
                    Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), kit::HEADING_HEIGHT)),
                    heading,
                );
                y += kit::HEADING_HEIGHT + 10.0;
                match self.draft.provider {
                    AiProvider::OpenAI | AiProvider::OpenAILegacy => {
                        Self::key_row(ui, &colors, line(y), id("openai-key"), &mut self.draft.openai_api_key);
                        y += 50.0;
                        let load_item = [(
                            Some(icons::REFRESH_CW),
                            Some(t(tl::AnalyzeImageOptionsWindow_Load_models)),
                        )];
                        let load_w = kit::button_width(ui, &colors, load_item[0].0, load_item[0].1);
                        let c = kit::columns(
                            line(y),
                            &[Track::Px(120.0), Track::Star(1.0), Track::Px(load_w)],
                            10.0,
                        );
                        kit::label(ui, &colors, c[0], t(tl::AnalyzeImageOptionsWindow_Model));
                        kit::editable_combo_at(
                            ui,
                            &colors,
                            c[1],
                            id("openai-model"),
                            &mut self.draft.openai_model,
                            &self.openai_models,
                            true,
                        );
                        if kit::button_at(ui, &colors, c[2], load_item[0].0, load_item[0].1, !self.busy)
                            .clicked()
                        {
                            load_models = true;
                        }
                        y += 50.0;
                        let c = kit::columns(line(y), &[Track::Px(120.0), Track::Star(1.0)], 10.0);
                        kit::label(ui, &colors, c[0], t(tl::AnalyzeImageOptionsWindow_Custom_base_URL));
                        let mut url = self.draft.openai_custom_url.clone().unwrap_or_default();
                        if kit::text_box_at(
                            ui,
                            &colors,
                            c[1],
                            id("openai-url"),
                            &mut url,
                            t(tl::AnalyzeImageOptionsWindow_https_api_openai_com),
                            true,
                            false,
                        )
                        .changed()
                        {
                            self.draft.openai_custom_url = Some(url);
                        }
                        y += 50.0;
                        let c = kit::columns(
                            line(y),
                            &[
                                Track::Px(120.0),
                                Track::Star(1.0),
                                Track::Px(120.0),
                                Track::Star(1.0),
                            ],
                            10.0,
                        );
                        kit::label(ui, &colors, c[0], t(tl::AnalyzeImageOptionsWindow_Reasoning));
                        let efforts: Vec<(String, String)> = analyze::REASONING_EFFORTS
                            .iter()
                            .map(|e| (e.to_string(), analyze::level_name(e)))
                            .collect();
                        if let Some(effort) = kit::combo_at(
                            ui,
                            &colors,
                            c[1],
                            &format!("analyze-effort-{key}"),
                            &self.draft.openai_reasoning_effort,
                            &efforts,
                            true,
                        ) {
                            self.draft.openai_reasoning_effort = effort;
                        }
                        kit::label(ui, &colors, c[2], t(tl::AnalyzeImageOptionsWindow_Verbosity));
                        let levels: Vec<(String, String)> = analyze::VERBOSITY_LEVELS
                            .iter()
                            .map(|e| (e.to_string(), analyze::level_name(e)))
                            .collect();
                        if let Some(level) = kit::combo_at(
                            ui,
                            &colors,
                            c[3],
                            &format!("analyze-verbosity-{key}"),
                            &self.draft.openai_verbosity,
                            &levels,
                            true,
                        ) {
                            self.draft.openai_verbosity = level;
                        }
                    }
                    AiProvider::Gemini | AiProvider::OpenRouter => {
                        let gemini = self.draft.provider == AiProvider::Gemini;
                        let (key_field, model, models) = if gemini {
                            (
                                &mut self.draft.gemini_api_key,
                                &mut self.draft.gemini_model,
                                &self.gemini_models,
                            )
                        } else {
                            (
                                &mut self.draft.open_router_api_key,
                                &mut self.draft.open_router_model,
                                &self.open_router_models,
                            )
                        };
                        let name = if gemini { "gemini" } else { "openrouter" };
                        Self::key_row(ui, &colors, line(y), id(&format!("{name}-key")), key_field);
                        y += 50.0;
                        let c = kit::columns(line(y), &[Track::Px(120.0), Track::Star(1.0)], 10.0);
                        kit::label(ui, &colors, c[0], t(tl::AnalyzeImageOptionsWindow_Model));
                        kit::editable_combo_at(
                            ui,
                            &colors,
                            c[1],
                            id(&format!("{name}-model")),
                            model,
                            models,
                            true,
                        );
                    }
                }

                // Behaviour.
                let panel = Rect::from_min_size(
                    pos2(area.left(), panel.bottom() + 12.0),
                    vec2(area.width(), BEHAVIOUR_PANEL),
                );
                kit::panel(ui, &colors, panel);
                let inner = panel.shrink(12.0);
                let mut y = inner.top();
                kit::heading(
                    ui,
                    &colors,
                    Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), kit::HEADING_HEIGHT)),
                    t(tl::AnalyzeImageOptionsWindow_Behavior),
                );
                y += kit::HEADING_HEIGHT + 8.0;
                let toggles = [
                    (
                        t(tl::AnalyzeImageOptionsWindow_Capture_a_region_when_opened_without_an_image),
                        "auto-region",
                    ),
                    (
                        t(tl::AnalyzeImageOptionsWindow_Analyze_automatically_when_opened),
                        "auto-analyze",
                    ),
                    (
                        t(tl::AnalyzeImageOptionsWindow_Copy_result_automatically),
                        "auto-copy",
                    ),
                ];
                for (index, (label, name)) in toggles.iter().enumerate() {
                    let r = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 30.0));
                    let value = match index {
                        0 => &mut self.draft.auto_start_region,
                        1 => &mut self.draft.auto_start_analyze,
                        _ => &mut self.draft.auto_copy_result,
                    };
                    Self::behaviour_row(ui, &colors, r, id(name), label, value);
                    y += 38.0;
                }
            });
        if load_models {
            self.load_models(state);
        }

        // Test connection, the status, Save and Cancel.
        let test_item = [(
            Some(icons::PLUG_ZAP),
            Some(t(tl::AnalyzeImageOptionsWindow_Test_connection)),
        )];
        let test_w = kit::button_width(ui, &colors, test_item[0].0, test_item[0].1);
        let save_w = kit::button_width(
            ui,
            &colors,
            None,
            Some(t(tl::AnalyzeImageOptionsWindow_Save)),
        );
        let cancel_w = kit::button_width(
            ui,
            &colors,
            None,
            Some(t(tl::AnalyzeImageOptionsWindow_Cancel)),
        );
        let c = kit::columns(
            rows[2],
            &[
                Track::Px(test_w),
                Track::Star(1.0),
                Track::Px(save_w),
                Track::Px(cancel_w),
            ],
            8.0,
        );
        if kit::button_at(
            ui,
            &colors,
            c[0],
            test_item[0].0,
            test_item[0].1,
            !self.busy,
        )
        .clicked()
        {
            self.test_connection(state);
        }
        if !self.status.trim().is_empty() {
            let galley = crate::widgets::layout(
                ui,
                crate::widgets::trimmed(
                    &self.status,
                    13.0,
                    Weight::Normal,
                    colors.text_secondary(),
                    c[1].width().max(1.0),
                    1,
                ),
            );
            let pos = pos2(c[1].left(), c[1].center().y - galley.size().y / 2.0);
            ui.painter().galley(pos, galley, colors.text_secondary());
        }
        if kit::button_at(
            ui,
            &colors,
            c[2],
            None,
            Some(t(tl::AnalyzeImageOptionsWindow_Save)),
            !self.busy,
        )
        .clicked()
        {
            self.save(state);
        }
        if kit::button_at(
            ui,
            &colors,
            c[3],
            None,
            Some(t(tl::AnalyzeImageOptionsWindow_Cancel)),
            !self.busy,
        )
        .clicked()
        {
            self.close = true;
        }
        self.close
    }

    fn closed(&mut self, state: &mut AppState) {
        self.version += 1;
        let parent = self.parent;
        super::with_window::<AnalyzeWindow, _>(state, parent, |window, _| {
            window.options_window = None;
        });
        state.tools.restore.push(parent);
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// A key held in memory for the run (QA profiles and headless renders).
struct MemoryKey(String);

static MEMORY_KEYS: std::sync::Mutex<std::collections::BTreeMap<String, String>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

impl CredentialStore for MemoryKey {
    fn load(&self) -> Option<String> {
        MEMORY_KEYS.lock().ok()?.get(&self.0).cloned()
    }

    fn save(&self, token: &str) -> Result<(), String> {
        MEMORY_KEYS
            .lock()
            .map_err(|e| e.to_string())?
            .insert(self.0.clone(), token.to_string());
        Ok(())
    }

    fn delete(&self) -> Result<(), String> {
        MEMORY_KEYS
            .lock()
            .map_err(|e| e.to_string())?
            .remove(&self.0);
        Ok(())
    }
}

impl AppState {
    /// Where a key is kept: the Keychain; QA and headless runs hold keys in memory only
    /// (never on disk).
    fn ai_key_store(&self, account: &str) -> Box<dyn CredentialStore> {
        if self.env.qa || self.mode != crate::app::Mode::Normal {
            Box::new(MemoryKey(format!(
                "{}/{account}",
                self.env.profile.display()
            )))
        } else {
            Box::new(modlight_macos::KeychainStore::new("Modlight", account))
        }
    }

    /// The task's `AIOptions` with the keys from where they are kept.
    pub fn ai_options(&self, settings: &TaskSettings) -> AiOptions {
        let mut options: AiOptions = tool_options(settings, "AIOptions");
        options.openai_api_key = self.ai_key_store(KEY_ACCOUNTS[0]).load();
        options.gemini_api_key = self.ai_key_store(KEY_ACCOUNTS[1]).load();
        options.open_router_api_key = self.ai_key_store(KEY_ACCOUNTS[2]).load();
        options
    }

    /// The options without their keys, back where they came from.
    pub fn save_ai_options(&mut self, home: &OptionsHome, options: &AiOptions) {
        self.save_tool_options(home, "AIOptions", options);
    }

    /// The keys: kept when set, removed when cleared.
    pub fn save_ai_keys(&self, options: &AiOptions) -> Result<(), String> {
        let keys = [
            &options.openai_api_key,
            &options.gemini_api_key,
            &options.open_router_api_key,
        ];
        for (account, key) in KEY_ACCOUNTS.iter().zip(keys) {
            let store = self.ai_key_store(account);
            match key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
                Some(key) => {
                    if store.load().as_deref() != Some(key) {
                        store.save(key)?;
                    }
                }
                None => store.delete()?,
            }
        }
        Ok(())
    }

    /// Where the requests go in this run.
    fn ai_network(&self) -> Network {
        if self.env.qa || self.mode != crate::app::Mode::Normal {
            if let Ok(stub) = std::env::var("MODLIGHT_QA_AI_ENDPOINT")
                && stub.starts_with("http://127.0.0.1:")
            {
                return Network::Live(Endpoints {
                    openai: stub.clone(),
                    gemini: stub.clone(),
                    open_router: stub,
                });
            }
            Network::Offline(match &self.tools.qa_ai_text {
                Some(text) => Ok(text.clone()),
                None => Err(OFFLINE.into()),
            })
        } else {
            Network::Live(Endpoints::default())
        }
    }

    /// `TaskHelpers.AnalyzeImage(filePath, taskSettings)` (no file: the hotkey).
    pub fn open_analyze_image(&mut self, file: Option<PathBuf>, settings: TaskSettings) {
        let options = self.ai_options(&settings);
        let network = self.ai_network();
        self.tools
            .open_keyed(|key| Box::new(AnalyzeWindow::new(key, settings, options, file, network)));
    }

    /// The options dialog of the window `parent`.
    pub fn open_analyze_options(
        &mut self,
        parent: u64,
        home: &OptionsHome,
        options: &AiOptions,
        network: Network,
    ) {
        let home = home.clone();
        let options = options.clone();
        let key = self.tools.open_keyed(|key| {
            Box::new(AnalyzeOptionsWindow::new(
                key, parent, home, &options, network,
            ))
        });
        if let Some(window) = self.tools.window_mut::<AnalyzeWindow>(parent) {
            window.options_window = Some(key);
        }
    }

    /// A region for an analyze window arrived.
    pub(super) fn analyze_region_done(&mut self, key: u64, picture: super::region::ScreenPicture) {
        self.tools.restore.push(key);
        super::with_window::<AnalyzeWindow, _>(self, key, |window, state| {
            window.region_done(state, picture.map(|(image, _)| (*image).clone()));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_models_join_the_lists() {
        assert_eq!(
            models_with(&analyze::GEMINI_MODELS, "gemini-2.0-flash").len(),
            4
        );
        let list = models_with(&analyze::GEMINI_MODELS, "gemini-1.5-flash-latest");
        assert_eq!(
            list.last().map(String::as_str),
            Some("gemini-1.5-flash-latest")
        );
        assert_eq!(models_with(&analyze::GEMINI_MODELS, " ").len(), 4);
    }
}
