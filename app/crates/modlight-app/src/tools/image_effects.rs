//! `ShareX.ImageEffectsLib/Presentation`: `ImageEffectsWindow` with its view model, the
//! options panel (`ImageEffectOptionsPanel`) and the gradient flyout
//! (`GradientOptionsPanel`), `ImageEffectPackagerWindow` and
//! `ImageEffectsConfirmationWindow`; and the `TaskHelpers` entry points around them:
//! `OpenImageEffects` (the hotkey and the Tools menu), `OpenImageEffectsSingleton` (the
//! task settings' "Image effects configuration..."), `ApplyImageEffects` (after capture)
//! and `ImportImageEffect` (a `.sxie` package).
//!
//! The effects themselves are `modlight_effects::classic`. The presets live where
//! upstream keeps them, `ImageSettings.ImageEffectPresets` of the task settings (the
//! defaults or a hotkey's own), in upstream's JSON.
//!
//! Upstream's "Image effects page..." button opened getsharex.com, which the port does not
//! link (as the other help buttons).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Align2, Color32, Id, Rect, Sense, Stroke, StrokeKind, TextureHandle, Ui, pos2, vec2};
use image::RgbaImage;
use modlight_core::icons;
use modlight_core::l10n::{Arg, effects as fx, format, helpers as h, s, t};
use modlight_core::tasks::host::TaskAnswer;
use modlight_core::tasks::settings::{AfterCaptureTasks, FlagNames, TaskSettings};
use modlight_effects::classic::model::{
    self as m, ClassicEffect, GdiColor, Gradient, Kind, PType, Padding, Preset, PropDef, Stop,
    TextKind, Value,
};
use modlight_effects::classic::{apply as fx_apply, package, sample};
use modlight_effects::sk::C;
use serde_json::json;
use tokio::sync::oneshot;

use super::kit::{self, Track};
use super::{ToolWindow, WindowSpec};
use crate::commands::Command;
use crate::menus::Entry;
use crate::state::AppState;
use crate::task_pages::TaskTarget;
use crate::theme::{Colors, Weight};

/// `Width="1180" Height="760"`.
pub const WIDTH: f32 = 1180.0;
pub const HEIGHT: f32 = 760.0;
/// `Button.picker-button`.
const PICKER: f32 = 36.0;
/// The preview timer's interval.
const PREVIEW_DELAY: f64 = 0.1;
/// Edited presets are written back at most this often (and when the window closes).
const SAVE_EVERY: f64 = 1.0;
/// `ImageEffectPackager`'s folder inside the profile (`Program.ImageEffectsFolder`).
pub const EFFECTS_FOLDER: &str = "ImageEffects";

/// `ImageEffectsWindowMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowMode {
    Presets,
    Tool,
    Editor,
}

/// What a window opened for a task (`ShowDialog` in `ApplyImageEffects`) answers when it
/// closes.
pub struct TaskFinish {
    pub image: Arc<RgbaImage>,
    pub settings: Box<TaskSettings>,
    pub reply: oneshot::Sender<TaskAnswer>,
}

/// A flyout drawn open for the QA renders (popups do not open headlessly).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QaFlyout {
    AddEffect,
    Gradient,
}

/// `ImageEffectsWindow` and `ImageEffectsViewModel`.
pub struct ImageEffectsWindow {
    pub key: u64,
    pub mode: WindowMode,
    /// Where the presets are written back.
    pub home: TaskTarget,
    pub presets: Vec<Preset>,
    pub selected_preset: Option<usize>,
    pub selected_effect: Option<usize>,
    source: Arc<RgbaImage>,
    pub file_path: String,
    preview: Option<Arc<RgbaImage>>,
    texture: Option<TextureHandle>,
    pub busy: bool,
    pub error: String,
    pub title: String,
    version: u64,
    preview_due: Option<f64>,
    /// The options panel's `ValidationText`.
    pub validation: String,
    /// What the text editors hold while they are edited (by property).
    texts: HashMap<&'static str, String>,
    dirty: bool,
    saved_at: f64,
    finish: Option<TaskFinish>,
    pub accepted: bool,
    /// The task settings the tool's Upload runs with.
    settings: TaskSettings,
    add_open: bool,
    pub qa_flyout: Option<QaFlyout>,
}

impl ImageEffectsWindow {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        key: u64,
        mode: WindowMode,
        home: TaskTarget,
        source: Option<RgbaImage>,
        mut presets: Vec<Preset>,
        selected: i64,
        file_path: Option<String>,
        settings: TaskSettings,
        colors: &Colors,
    ) -> ImageEffectsWindow {
        let source = source.unwrap_or_else(|| sample::sample_image(sample_colors(colors)));
        if presets.is_empty() {
            presets.push(Preset::default());
        }
        let selected = (selected.max(0) as usize).min(presets.len() - 1);
        let mut window = ImageEffectsWindow {
            key,
            mode,
            home,
            presets,
            selected_preset: None,
            selected_effect: None,
            source: Arc::new(source),
            file_path: file_path.unwrap_or_default(),
            preview: None,
            texture: None,
            busy: false,
            error: String::new(),
            title: t(fx::ImageEffectsWindow_Title).to_string(),
            version: 0,
            preview_due: None,
            validation: String::new(),
            texts: HashMap::new(),
            dirty: false,
            saved_at: 0.0,
            finish: None,
            accepted: false,
            settings,
            add_open: false,
            qa_flyout: None,
        };
        window.select_preset(Some(selected), 0.0);
        window
    }

    fn is_tool(&self) -> bool {
        self.mode == WindowMode::Tool
    }

    fn is_editor(&self) -> bool {
        self.mode == WindowMode::Editor
    }

    /// `SelectedPresetIndex`.
    pub fn selected_preset_index(&self) -> usize {
        self.selected_preset.unwrap_or(0)
    }

    fn preset(&self) -> Option<&Preset> {
        self.selected_preset.and_then(|i| self.presets.get(i))
    }

    fn effect(&self) -> Option<&ClassicEffect> {
        let i = self.selected_effect?;
        self.preset()?.effects.get(i)
    }

    fn effect_mut(&mut self) -> Option<&mut ClassicEffect> {
        let (p, e) = (self.selected_preset?, self.selected_effect?);
        self.presets.get_mut(p)?.effects.get_mut(e)
    }

    /// `OnSelectedPresetChanged`: new effect items, so the options panel is built anew
    /// (`SetEffect`) even when the first effect is selected again.
    fn select_preset(&mut self, index: Option<usize>, now: f64) {
        self.selected_preset = index;
        let count = self.preset().map(|p| p.effects.len()).unwrap_or(0);
        self.texts.clear();
        self.validation.clear();
        self.set_selected_effect(if count > 0 { Some(0) } else { None });
        self.queue_preview(now);
    }

    /// `SelectedEffect` changed: the options panel is built anew.
    fn set_selected_effect(&mut self, index: Option<usize>) {
        if self.selected_effect != index {
            self.texts.clear();
            self.validation.clear();
        }
        self.selected_effect = index;
    }

    /// `QueuePreview`.
    fn queue_preview(&mut self, now: f64) {
        self.version += 1;
        self.preview_due = Some(now + PREVIEW_DELAY);
    }

    fn changed(&mut self, now: f64) {
        self.dirty = true;
        self.queue_preview(now);
    }

    // ---------------------------------------------------------------- commands

    fn new_preset(&mut self, now: f64) {
        self.presets.push(Preset::default());
        self.select_preset(Some(self.presets.len() - 1), now);
        self.dirty = true;
    }

    fn remove_preset(&mut self, now: f64) {
        let Some(index) = self.selected_preset else {
            return;
        };
        self.presets.remove(index);
        self.dirty = true;
        if self.presets.is_empty() {
            self.new_preset(now);
        } else {
            self.select_preset(Some(index.min(self.presets.len() - 1)), now);
        }
    }

    fn duplicate_preset(&mut self, now: f64) {
        let Some(copy) = self.preset().cloned() else {
            return;
        };
        self.presets.push(copy);
        self.select_preset(Some(self.presets.len() - 1), now);
        self.dirty = true;
    }

    fn move_preset(&mut self, direction: i32) {
        let Some(old) = self.selected_preset else {
            return;
        };
        let new = old as i32 + direction;
        if new < 0 || new as usize >= self.presets.len() {
            return;
        }
        let preset = self.presets.remove(old);
        self.presets.insert(new as usize, preset);
        self.selected_preset = Some(new as usize);
        self.dirty = true;
    }

    /// `AddEffect`: after the selected effect, or last.
    pub fn add_effect(&mut self, effect: ClassicEffect, now: f64) {
        let after = self.selected_effect;
        let Some(preset) = self.selected_preset.and_then(|i| self.presets.get_mut(i)) else {
            return;
        };
        let index = after.map(|i| i + 1).unwrap_or(preset.effects.len());
        preset.effects.insert(index, effect);
        self.set_selected_effect(Some(index));
        self.texts.clear();
        self.changed(now);
    }

    fn remove_effect(&mut self, now: f64) {
        let (Some(p), Some(e)) = (self.selected_preset, self.selected_effect) else {
            return;
        };
        let effects = &mut self.presets[p].effects;
        if e >= effects.len() {
            return;
        }
        effects.remove(e);
        let next = if effects.is_empty() {
            None
        } else {
            Some(e.min(effects.len() - 1))
        };
        self.texts.clear();
        self.selected_effect = None;
        self.set_selected_effect(next);
        self.changed(now);
    }

    fn duplicate_effect(&mut self, now: f64) {
        if let Some(copy) = self.effect().cloned() {
            self.add_effect(copy, now);
        }
    }

    /// `ClearEffects` (after the confirmation).
    pub fn clear_effects(&mut self, now: f64) {
        let Some(p) = self.selected_preset else {
            return;
        };
        self.presets[p].effects.clear();
        self.set_selected_effect(None);
        self.changed(now);
    }

    fn move_effect(&mut self, direction: i32, now: f64) {
        let (Some(p), Some(old)) = (self.selected_preset, self.selected_effect) else {
            return;
        };
        let effects = &mut self.presets[p].effects;
        let new = old as i32 + direction;
        if new < 0 || new as usize >= effects.len() {
            return;
        }
        let effect = effects.remove(old);
        effects.insert(new as usize, effect);
        self.selected_effect = Some(new as usize);
        self.texts.clear();
        self.changed(now);
    }

    /// `ReplaceSource`.
    pub fn replace_source(&mut self, image: RgbaImage, path: Option<String>, now: f64) {
        self.source = Arc::new(image);
        self.file_path = path.unwrap_or_default();
        self.queue_preview(now);
    }

    /// `LoadImageFile`.
    pub fn load_image_file(&mut self, path: &Path, now: f64) {
        if let Some(image) = super::files::load_image(path) {
            self.replace_source(image, Some(path.to_string_lossy().into_owned()), now);
        }
    }

    /// `ImportPreset(json)`: a package's preset joins the list, its picture paths kept
    /// only inside the image effects folder (`SanitizeImportedPreset`).
    pub fn import_preset(&mut self, text: &str, effects_folder: &Path, now: f64) {
        let parsed: Result<serde_json::Value, _> =
            serde_json::from_str(text.trim_start_matches('\u{feff}'));
        let preset = match parsed {
            Ok(value) => Preset::from_json(&value),
            Err(error) => {
                self.error = error.to_string();
                return;
            }
        };
        let Some(mut preset) = preset.filter(|p| !p.effects.is_empty()) else {
            return;
        };
        sanitize_imported_preset(&mut preset, effects_folder);
        self.presets.push(preset);
        self.select_preset(Some(self.presets.len() - 1), now);
        self.dirty = true;
    }

    // ---------------------------------------------------------------- preview

    /// `UpdatePreviewAsync`, off the UI thread.
    fn start_preview(&mut self, state: &mut AppState) {
        self.preview_due = None;
        let Some(preset) = self.preset().cloned() else {
            return;
        };
        self.version += 1;
        let version = self.version;
        self.busy = true;
        self.error.clear();
        let source = self.source.clone();
        let key = self.key;
        state.rt.spawn_blocking(move || {
            let started = std::time::Instant::now();
            let result = apply_guarded(&preset, &source);
            let ms = started.elapsed().as_millis() as u64;
            move |state: &mut AppState, _ctx: &egui::Context| {
                super::with_window::<ImageEffectsWindow, _>(state, key, |window, _| {
                    window.preview_finished(version, result, ms);
                });
            }
        });
    }

    fn preview_finished(
        &mut self,
        version: u64,
        result: Result<Option<RgbaImage>, String>,
        ms: u64,
    ) {
        if version != self.version {
            return;
        }
        self.busy = false;
        match result {
            Ok(Some(image)) => {
                self.title = format(
                    t(fx::ImageEffectsWindow_Title_with_preview),
                    &[
                        Arg::from(t(fx::ImageEffectsWindow_Title)),
                        Arg::from(image.width() as u64),
                        Arg::from(image.height() as u64),
                        Arg::from(ms),
                    ],
                );
                self.preview = Some(Arc::new(image));
                self.texture = None;
            }
            Ok(None) => {}
            Err(error) => self.error = error,
        }
    }

    /// The preview at once, on this thread (QA renders).
    pub fn preview_now(&mut self) {
        self.preview_due = None;
        self.version += 1;
        let version = self.version;
        if let Some(preset) = self.preset().cloned() {
            let result = apply_guarded(&preset, &self.source);
            self.preview_finished(version, result, 12);
        }
    }

    /// `ApplySelectedPresetAsync`, here on the UI thread (Save and Upload wait for it).
    fn apply_selected(&self) -> Option<RgbaImage> {
        let preset = self.preset()?;
        apply_guarded(preset, &self.source).ok().flatten()
    }

    // ---------------------------------------------------------------- saving

    fn save(&mut self, state: &mut AppState, now: f64) {
        if self.dirty {
            state.save_image_effect_presets(self.home, &self.presets);
            self.dirty = false;
        }
        self.saved_at = now;
    }
}

/// `preset.ApplyEffects(source)`: an effect's failure (GDI+'s "Parameter is not valid."
/// for a bitmap too large to make) or a panic becomes the error line.
fn apply_guarded(preset: &Preset, source: &RgbaImage) -> Result<Option<RgbaImage>, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| preset.try_apply(source)))
        .map_err(|panic| {
            panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "Image effect failed.".to_string())
        })
        .and_then(|result| result)
}

/// `SanitizeImportedPreset`.
pub fn sanitize_imported_preset(preset: &mut Preset, folder: &Path) {
    let allowed = |path: &str| path.is_empty() || fx_apply::is_path_in_folder(path, folder);
    for effect in &mut preset.effects {
        let prop = match effect.kind {
            Kind::DrawImage => "ImageLocation",
            Kind::DrawBackgroundImage => "ImageFilePath",
            Kind::DrawParticles => "ImageFolder",
            _ => continue,
        };
        if !allowed(&effect.text(prop)) {
            effect.set(prop, Value::Text(String::new()));
        }
    }
}

/// The skin's colours for the sample picture (`GetThemeColor`).
fn sample_colors(colors: &Colors) -> sample::SampleColors {
    let c = |c: Color32| C::new(c.r(), c.g(), c.b(), c.a());
    sample::SampleColors {
        background_main: c(colors.background_main()),
        background_panel: c(colors.background_panel()),
        accent_start: c(colors.accent_start()),
        accent_end: c(colors.accent_end()),
        accent_foreground: c(colors.accent_foreground()),
    }
}

fn to32(c: C) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

/// A picker button: 36 x 36, a 16 px icon, a tooltip.
fn picker(ui: &mut Ui, colors: &Colors, rect: Rect, icon: char, tip: &str, enabled: bool) -> bool {
    let square = Rect::from_center_size(rect.center(), vec2(PICKER, PICKER));
    let look = crate::widgets::Look::button(colors)
        .height(PICKER)
        .width(PICKER)
        .padding(0.0)
        .icon(16.0, None);
    let mut child = kit::child(ui, square);
    crate::widgets::button_at(
        &mut child,
        colors,
        &look,
        crate::widgets::Content {
            icon: Some(icon),
            label: None,
        },
        enabled,
        square,
    )
    .on_hover_text(tip)
    .clicked()
}

/// A colour button: the swatch and its hex, the colour panel in a flyout.
fn color_button(ui: &mut Ui, colors: &Colors, rect: Rect, id: Id, value: C) -> Option<C> {
    let response = kit::button_at(ui, colors, rect, None, None, true);
    let swatch = Rect::from_min_size(
        pos2(rect.left() + 10.0, rect.center().y - 12.0),
        vec2(28.0, 24.0),
    );
    kit::paint_checkered_big(ui, colors, swatch);
    ui.painter().rect_filled(swatch, 3.0, to32(value));
    ui.painter().rect_stroke(
        swatch,
        3.0,
        Stroke::new(1.0, Color32::from_rgb(112, 112, 112)),
        StrokeKind::Inside,
    );
    let hex = if value.a == 255 {
        format!("#{:02X}{:02X}{:02X}", value.r, value.g, value.b)
    } else {
        format!(
            "#{:02X}{:02X}{:02X}{:02X}",
            value.a, value.r, value.g, value.b
        )
    };
    kit::text_at(
        ui,
        Rect::from_min_max(pos2(swatch.right() + 8.0, rect.top()), rect.max),
        Align2::LEFT_CENTER,
        &hex,
        13.0,
        Weight::Normal,
        colors.text(),
    );
    color_flyout(ui, colors, &response, id, value)
}

/// Avalonia's `ColorView` in a flyout under `response` (the editor's colour panel).
fn color_flyout(
    ui: &mut Ui,
    colors: &Colors,
    response: &egui::Response,
    id: Id,
    value: C,
) -> Option<C> {
    let editor_colors = modlight_editor::ui::style::Colors {
        dark: colors.dark,
        palette: colors.palette,
        graphite: colors.graphite,
    };
    let _ = ui;
    let mut picked = None;
    egui::Popup::menu(response)
        .id(id.with("flyout"))
        .frame(crate::menus::frame(colors))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let current = modlight_editor::color::Color::argb(value.a, value.r, value.g, value.b);
            if let Some(modlight_editor::ui::pickers::ColorEvent::Picked(c)) =
                modlight_editor::ui::pickers::color_panel(
                    ui,
                    &editor_colors,
                    id.with("panel"),
                    current,
                )
            {
                picked = Some(C::new(c.r, c.g, c.b, c.a));
            }
        });
    picked
}

/// `{0} is not a valid value for {1}.` (the converters' `FormatException`).
fn invalid_value(text: &str, type_name: &str) -> String {
    format!("{text} is not a valid value for {type_name}.")
}

/// `ConvertPropertyToString`.
fn converter_string(def: &PropDef, value: Option<&Value>) -> String {
    match value {
        Some(Value::Text(text)) => text.clone(),
        Some(Value::Font(font)) => {
            font.to_culture_string(&modlight_core::culture::Culture::current())
        }
        Some(Value::Flags(f)) => m::flags_to_string(def, *f),
        Some(Value::Enum(name)) => name.clone(),
        _ => String::new(),
    }
}

/// The add-effect menu (`OnAddEffectClick`): the categories, each with its classes.
pub fn add_effect_menu() -> Vec<Entry> {
    let mut flat = 0usize;
    m::CATALOG
        .iter()
        .map(|(category, kinds)| {
            let children = kinds
                .iter()
                .map(|kind| {
                    let entry = Entry::Item {
                        label: m::effect_name(*kind),
                        icon: None,
                        enabled: true,
                        toggle: crate::menus::Toggle::None,
                        command: Some(Command::Local(flat)),
                    };
                    flat += 1;
                    entry
                })
                .collect();
            Entry::Submenu {
                label: m::category_name(category),
                icon: None,
                enabled: true,
                children,
            }
        })
        .collect()
}

/// The class at a position of [`add_effect_menu`].
fn catalog_kind(index: usize) -> Option<Kind> {
    m::CATALOG
        .iter()
        .flat_map(|(_, kinds)| kinds.iter().copied())
        .nth(index)
}

// -------------------------------------------------------------------- the window

impl ToolWindow for ImageEffectsWindow {
    fn spec(&self) -> WindowSpec {
        WindowSpec::resizable(&self.title, vec2(WIDTH, HEIGHT), vec2(1040.0, 620.0))
    }

    fn ui(&mut self, ui: &mut Ui, state: &mut AppState) -> bool {
        let colors = state.colors.clone();
        let now = state.time;
        kit::fill(ui, &colors);
        self.handle_drops(ui, state, now);
        let typing = ui.ctx().memory(|m| m.focused().is_some());
        if !typing
            && ui.input(|i| i.key_pressed(egui::Key::Delete))
            && self.selected_effect.is_some()
        {
            self.remove_effect(now);
        }
        if ui.input(|i| i.key_pressed(egui::Key::F5)) {
            self.queue_preview(now);
        }
        if let Some(due) = self.preview_due {
            if now >= due {
                self.start_preview(state);
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs_f64(due - now));
            }
        }
        if self.busy {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(30));
        }

        let full = kit::inset(ui.max_rect(), 16.0);
        let rows = kit::rows(full, &[Track::Star(1.0), Track::Px(40.0)], 12.0);
        let cols = kit::columns(
            rows[0],
            &[Track::Px(320.0), Track::Px(360.0), Track::Star(1.0)],
            12.0,
        );
        let left = kit::rows(
            cols[0],
            &[
                Track::Px(12.0 * 2.0 + PICKER + 10.0 + 40.0 + 10.0 + 40.0),
                Track::Star(1.0),
            ],
            12.0,
        );
        self.presets_panel(ui, &colors, left[0], now);
        self.effects_panel(ui, &colors, left[1], now);
        kit::panel(ui, &colors, cols[1]);
        self.options_panel(ui, state, &colors, cols[1].shrink(12.0), now);
        self.preview_area(ui, state, &colors, cols[2], now);
        let close = self.footer(ui, state, &colors, rows[1], now);

        if self.dirty && now - self.saved_at >= SAVE_EVERY {
            self.save(state, now);
        }
        close
    }

    fn closed(&mut self, state: &mut AppState) {
        self.version += 1;
        let now = state.time;
        self.save(state, now);
        state.save_selected_image_effect_preset(self.home, self.selected_preset_index());
        if let Some(finish) = self.finish.take() {
            let presets = self.presets.clone();
            let selected = self.selected_preset_index() as i64;
            state.apply_task_preset(finish, &presets, selected);
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl ImageEffectsWindow {
    /// `OnDrop`: a picture (tool mode) becomes the source; a package is imported.
    fn handle_drops(&mut self, ui: &Ui, state: &mut AppState, now: f64) {
        let dropped: Vec<PathBuf> = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        for path in dropped {
            let text = path.to_string_lossy();
            if self.is_tool() && modlight_core::names::file_helpers::is_image_file(&text) {
                self.load_image_file(&path, now);
                break;
            }
            if !text.to_lowercase().ends_with(".sxie") {
                continue;
            }
            let folder = state.image_effects_folder();
            match package::extract_package(&path, &folder) {
                Ok(extracted) => {
                    if let Some(json) = extracted.config_json.filter(|j| !j.trim().is_empty()) {
                        self.import_preset(&json, &folder, now);
                    }
                }
                Err(error) => log::warn!("Image effect package {}: {error}", path.display()),
            }
        }
    }

    fn presets_panel(&mut self, ui: &mut Ui, colors: &Colors, rect: Rect, now: f64) {
        kit::panel(ui, colors, rect);
        let inner = rect.shrink(12.0);
        let rows = kit::rows(
            inner,
            &[Track::Px(PICKER), Track::Px(40.0), Track::Px(40.0)],
            10.0,
        );
        let has = self.selected_preset.is_some();
        let c = kit::columns(
            rows[0],
            &[
                Track::Star(1.0),
                Track::Px(PICKER),
                Track::Px(PICKER),
                Track::Px(PICKER),
                Track::Px(PICKER),
                Track::Px(PICKER),
            ],
            4.0,
        );
        kit::heading(ui, colors, c[0], t(fx::ImageEffectsWindow_Presets));
        if picker(
            ui,
            colors,
            c[1],
            icons::PLUS,
            t(fx::ImageEffectsWindow_New_preset),
            true,
        ) {
            self.new_preset(now);
        }
        if picker(
            ui,
            colors,
            c[2],
            icons::MINUS,
            t(fx::ImageEffectsWindow_Remove_preset),
            has,
        ) {
            self.remove_preset(now);
        }
        if picker(
            ui,
            colors,
            c[3],
            icons::COPY,
            t(fx::ImageEffectsWindow_Duplicate_preset),
            has,
        ) {
            self.duplicate_preset(now);
        }
        if picker(
            ui,
            colors,
            c[4],
            icons::ARROW_UP,
            t(fx::ImageEffectsWindow_Move_up),
            has,
        ) {
            self.move_preset(-1);
        }
        if picker(
            ui,
            colors,
            c[5],
            icons::ARROW_DOWN,
            t(fx::ImageEffectsWindow_Move_down),
            has,
        ) {
            self.move_preset(1);
        }
        let options: Vec<(usize, String)> = self
            .presets
            .iter()
            .enumerate()
            .map(|(i, p)| (i, preset_display_name(p)))
            .collect();
        let current = self.selected_preset.unwrap_or(usize::MAX);
        if let Some(index) = kit::combo_at(
            ui,
            colors,
            rows[1],
            &format!("fx-presets-{}", self.key),
            &current,
            &options,
            true,
        ) && Some(index) != self.selected_preset
        {
            self.select_preset(Some(index), now);
        }
        let mut name = self.preset().map(|p| p.name.clone()).unwrap_or_default();
        if kit::text_box_at(
            ui,
            colors,
            rows[2],
            Id::new(("fx-preset-name", self.key)),
            &mut name,
            t(fx::ImageEffectsWindow_Preset_name),
            has,
            false,
        )
        .changed()
            && let Some(p) = self.selected_preset
        {
            self.presets[p].name = name;
            self.dirty = true;
        }
    }

    fn effects_panel(&mut self, ui: &mut Ui, colors: &Colors, rect: Rect, now: f64) {
        kit::panel(ui, colors, rect);
        let inner = rect.shrink(12.0);
        let rows = kit::rows(
            inner,
            &[
                Track::Px(PICKER),
                Track::Px(40.0),
                Track::Star(1.0),
                Track::Px(PICKER),
            ],
            10.0,
        );
        let c = kit::columns(rows[0], &[Track::Star(1.0), Track::Px(PICKER)], 0.0);
        kit::heading(ui, colors, c[0], t(fx::ImageEffectsWindow_Effects));
        if picker(
            ui,
            colors,
            c[1],
            icons::REFRESH_CW,
            t(fx::ImageEffectsWindow_Refresh_preview),
            true,
        ) {
            self.queue_preview(now);
        }
        let has_effect = self.selected_effect.is_some();
        let mut name = self.effect().map(|e| e.name.clone()).unwrap_or_default();
        if kit::text_box_at(
            ui,
            colors,
            rows[1],
            Id::new(("fx-effect-name", self.key)),
            &mut name,
            t(fx::ImageEffectsWindow_Custom_effect_name),
            has_effect,
            false,
        )
        .changed()
            && let Some(effect) = self.effect_mut()
        {
            effect.name = name;
            self.dirty = true;
        }
        self.effect_list(ui, colors, rows[2], now);

        let has = self.selected_preset.is_some();
        let b = kit::columns(rows[3], &[Track::Star(1.0); 6], 4.0);
        let add = {
            let square = Rect::from_center_size(b[0].center(), vec2(PICKER, PICKER));
            let look = crate::widgets::Look::button(colors)
                .height(PICKER)
                .width(PICKER)
                .padding(0.0)
                .icon(16.0, None);
            let mut child = kit::child(ui, square);
            crate::widgets::button_at(
                &mut child,
                colors,
                &look,
                crate::widgets::Content {
                    icon: Some(icons::PLUS),
                    label: None,
                },
                has,
                square,
            )
            .on_hover_text(t(fx::ImageEffectsWindow_Add_effect))
        };
        if add.clicked() {
            self.add_open = !self.add_open;
        }
        if self.qa_flyout == Some(QaFlyout::AddEffect) {
            // Drawn where the popup lands: above the button (no room below), the first
            // category's classes beside it, as a pointer resting on it shows them.
            let entries = add_effect_menu();
            let height = |n: usize| n as f32 * crate::menus::ITEM_HEIGHT + 10.0;
            let at = pos2(
                add.rect.left(),
                add.rect.top() - 4.0 - height(entries.len()),
            );
            let bottom = ui.max_rect().bottom() - 8.0;
            egui::Area::new(Id::new(("fx-qa-add", self.key)))
                .order(egui::Order::Foreground)
                .fixed_pos(pos2(0.0, 0.0))
                .show(ui.ctx(), |ui| {
                    let panel = crate::menus::panel(ui, colors, at, &entries);
                    if let Some(Entry::Submenu { children, .. }) = entries.first() {
                        let top = panel.top().min(bottom - height(children.len()));
                        crate::menus::panel(ui, colors, pos2(panel.right() + 2.0, top), children);
                    }
                });
        }
        let mut open = self.add_open;
        if let Some(Command::Local(index)) =
            crate::menus::popup_open_left(colors, &add, &mut open, add_effect_menu)
            && let Some(kind) = catalog_kind(index)
        {
            self.add_effect(ClassicEffect::new(kind), now);
            open = false;
        }
        self.add_open = open;
        if picker(
            ui,
            colors,
            b[1],
            icons::MINUS,
            t(fx::ImageEffectsWindow_Remove_effect),
            has_effect,
        ) {
            self.remove_effect(now);
        }
        if picker(
            ui,
            colors,
            b[2],
            icons::COPY,
            t(fx::ImageEffectsWindow_Duplicate_effect),
            has_effect,
        ) {
            self.duplicate_effect(now);
        }
        if picker(
            ui,
            colors,
            b[3],
            icons::ARROW_UP,
            t(fx::ImageEffectsWindow_Move_up),
            has_effect,
        ) {
            self.move_effect(-1, now);
        }
        if picker(
            ui,
            colors,
            b[4],
            icons::ARROW_DOWN,
            t(fx::ImageEffectsWindow_Move_down),
            has_effect,
        ) {
            self.move_effect(1, now);
        }
        if picker(
            ui,
            colors,
            b[5],
            icons::ERASER,
            t(fx::ImageEffectsWindow_Clear_effects),
            has,
        ) {
            let key = self.key;
            ui.ctx()
                .data_mut(|d| d.insert_temp(Id::new(("fx-confirm", key)), true));
        }
    }

    /// The `ListBox.compact` of effects: a check box and the name on each line.
    fn effect_list(&mut self, ui: &mut Ui, colors: &Colors, rect: Rect, now: f64) {
        let Some(p) = self.selected_preset else {
            return;
        };
        let mut child = kit::column(ui, rect);
        child.set_clip_rect(rect);
        let mut select = None;
        let mut toggled = false;
        let selected = self.selected_effect;
        let effects = &mut self.presets[p].effects;
        egui::ScrollArea::vertical()
            .id_salt(("fx-effect-list", self.key))
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (index, effect) in effects.iter_mut().enumerate() {
                    let (r, response) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
                    let is_selected = selected == Some(index);
                    if is_selected {
                        ui.painter()
                            .rect_filled(r, 4.0, colors.accent_start().gamma_multiply(0.6));
                    } else if response.hovered() {
                        ui.painter()
                            .rect_filled(r, 4.0, colors.control_background_hover());
                    }
                    let check =
                        Rect::from_min_size(pos2(r.left() + 4.0, r.top()), vec2(24.0, r.height()));
                    let mut enabled = effect.enabled;
                    let mut cell = kit::child(ui, check);
                    if crate::widgets::checkbox(&mut cell, colors, &mut enabled, None, true)
                        .changed()
                    {
                        effect.enabled = enabled;
                        toggled = true;
                    }
                    let text_rect = Rect::from_min_max(
                        pos2(check.right() + 6.0, r.top()),
                        pos2(r.right() - 4.0, r.bottom()),
                    );
                    let galley = crate::widgets::layout(
                        ui,
                        crate::widgets::trimmed(
                            &m::display_name(effect),
                            13.0,
                            Weight::Normal,
                            if is_selected {
                                colors.accent_foreground()
                            } else {
                                colors.text()
                            },
                            text_rect.width().max(1.0),
                            1,
                        ),
                    );
                    let at = pos2(
                        text_rect.left(),
                        text_rect.center().y - galley.size().y / 2.0,
                    );
                    ui.painter().galley(at, galley, colors.text());
                    if response.clicked() {
                        select = Some(index);
                    }
                }
            });
        if toggled {
            self.changed(now);
        }
        if let Some(index) = select {
            self.set_selected_effect(Some(index));
        }
    }

    /// `ImageEffectOptionsPanel`.
    fn options_panel(
        &mut self,
        ui: &mut Ui,
        state: &mut AppState,
        colors: &Colors,
        rect: Rect,
        now: f64,
    ) {
        let title = match self.effect() {
            Some(effect) => format(
                t(fx::ImageEffectOptionsPanel_Effect_options_format),
                &[Arg::from(m::effect_name(effect.kind))],
            ),
            None => t(fx::ImageEffectOptionsPanel_Effect_options).to_string(),
        };
        let validation_h = if self.validation.is_empty() {
            0.0
        } else {
            crate::widgets::layout(
                ui,
                crate::widgets::wrapped(
                    &self.validation,
                    13.0,
                    Weight::Normal,
                    colors.cancel_border(),
                    rect.width(),
                ),
            )
            .size()
            .y
        };
        let rows = kit::rows(
            rect,
            &[
                Track::Px(kit::HEADING_HEIGHT),
                Track::Star(1.0),
                Track::Px(validation_h),
            ],
            10.0,
        );
        kit::heading(ui, colors, rows[0], &title);
        if !self.validation.is_empty() {
            kit::paragraph(
                ui,
                rows[2],
                &self.validation,
                13.0,
                Weight::Normal,
                colors.cancel_border(),
            );
        }
        let Some(kind) = self.effect().map(|e| e.kind) else {
            kit::paragraph(
                ui,
                rows[1].translate(vec2(0.0, 4.0)),
                t(fx::ImageEffectOptionsPanel_Select_an_effect),
                13.0,
                Weight::Normal,
                colors.text(),
            );
            return;
        };
        let defs = m::props(kind);
        if defs.is_empty() {
            kit::paragraph(
                ui,
                rows[1].translate(vec2(0.0, 4.0)),
                t(fx::ImageEffectOptionsPanel_No_configurable_properties),
                13.0,
                Weight::Normal,
                colors.text(),
            );
            return;
        }
        let area = rows[1];
        let mut child = kit::column(ui, area);
        child.set_clip_rect(area);
        let mut changed = false;
        egui::ScrollArea::vertical()
            .id_salt((
                "fx-options",
                self.key,
                self.selected_preset,
                self.selected_effect,
            ))
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(4.0);
                let width = (ui.available_width() - 12.0).max(100.0);
                for (index, def) in defs.iter().enumerate() {
                    if index > 0 {
                        ui.add_space(8.0);
                    }
                    let label = m::property_name(def.name);
                    let label_galley = crate::widgets::layout(
                        ui,
                        crate::widgets::wrapped(&label, 13.0, Weight::Normal, colors.text(), 140.0),
                    );
                    let editor_h: f32 = match def.ty {
                        PType::Padding => 16.0 + 3.0 + 40.0,
                        PType::Bool => 36.0,
                        _ => 40.0,
                    };
                    let h = editor_h.max(36.0).max(label_galley.size().y);
                    let (row, _) = ui.allocate_exact_size(vec2(width, h), Sense::hover());
                    let label_rect = Rect::from_min_size(row.min, vec2(140.0, h));
                    let label_at = pos2(
                        label_rect.left(),
                        label_rect.center().y - label_galley.size().y / 2.0,
                    );
                    ui.painter().galley(label_at, label_galley, colors.text());
                    let description = m::property_description(kind, def.name);
                    if !description.is_empty() {
                        let _ = ui
                            .interact(label_rect, Id::new(("fx-label", def.name)), Sense::hover())
                            .on_hover_text(description);
                    }
                    let editor = Rect::from_min_max(pos2(row.left() + 150.0, row.top()), row.max);
                    let editor = Rect::from_center_size(
                        editor.center(),
                        vec2(editor.width(), editor_h.min(h)),
                    );
                    if self.property_editor(ui, state, colors, editor, def) {
                        changed = true;
                    }
                }
                ui.add_space(4.0);
            });
        if changed {
            self.changed(now);
        }
    }

    /// One property's editor (`CreateEditor`); true when the effect changed.
    fn property_editor(
        &mut self,
        ui: &mut Ui,
        state: &mut AppState,
        colors: &Colors,
        rect: Rect,
        def: &'static PropDef,
    ) -> bool {
        let key = self.key;
        let id = Id::new(("fx-prop", key, def.name));
        let Some(value) = self.effect().and_then(|e| e.get(def.name)).cloned() else {
            return false;
        };
        let set = |window: &mut ImageEffectsWindow, value: Value| -> bool {
            let Some(effect) = window.effect_mut() else {
                return false;
            };
            let before = effect.get(def.name).cloned();
            effect.set(def.name, value);
            let changed = effect.get(def.name).cloned() != before;
            window.validation.clear();
            changed
        };
        match (def.ty, value) {
            (PType::Bool, Value::Bool(mut on)) => {
                let pill = Rect::from_min_size(rect.min, vec2(40.0, rect.height()));
                if kit::toggle_at(ui, colors, pill, id, &mut on, true) {
                    return set(self, Value::Bool(on));
                }
                false
            }
            (PType::Enum(type_name, members), Value::Enum(current)) => {
                let options: Vec<(String, String)> = members
                    .iter()
                    .map(|m| (m.to_string(), m::enum_value_name(type_name, m)))
                    .collect();
                if let Some(picked) = kit::combo_at(
                    ui,
                    colors,
                    rect,
                    &format!("fx-enum-{key}-{}", def.name),
                    &current,
                    &options,
                    true,
                ) {
                    return set(self, Value::Enum(picked));
                }
                false
            }
            (PType::Int | PType::Byte | PType::Float | PType::Double, value) => {
                let floating = matches!(def.ty, PType::Float | PType::Double);
                let mut v = match value {
                    Value::Int(i) => i as f64,
                    Value::Byte(b) => b as f64,
                    Value::Float(f) => f as f64,
                    Value::Double(d) => d,
                    _ => 0.0,
                };
                if crate::controls::number_box_trimmed_at(
                    ui,
                    colors,
                    ("fx-number", key, def.name),
                    rect,
                    &mut v,
                    -1_000_000.0,
                    1_000_000.0,
                    if floating { 0.1 } else { 1.0 },
                    if floating { 3 } else { 0 },
                    true,
                ) {
                    // `Convert.ChangeType`: a byte out of range is an error.
                    let new = match def.ty {
                        PType::Int => Value::Int(v.round() as i32),
                        PType::Byte => {
                            if !(0.0..=255.0).contains(&v.round()) {
                                self.validation =
                                    "Value was either too large or too small for an unsigned byte."
                                        .into();
                                return false;
                            }
                            Value::Byte(v.round() as u8)
                        }
                        PType::Float => Value::Float(v as f32),
                        _ => Value::Double(v),
                    };
                    return set(self, new);
                }
                false
            }
            (PType::Color, Value::Color(color)) => {
                let width = (rect.width()).min(200.0);
                let button = Rect::from_min_size(rect.min, vec2(width, rect.height()));
                if let Some(picked) = color_button(ui, colors, button, id, color.c) {
                    return set(
                        self,
                        Value::Color(GdiColor {
                            c: picked,
                            name: None,
                        }),
                    );
                }
                false
            }
            (PType::Point, Value::Point(x, y)) => self
                .pair_editor(ui, colors, rect, id, (x, y), true)
                .is_some_and(|(x, y)| set(self, Value::Point(x, y))),
            (PType::Size, Value::Size(w, h)) => self
                .pair_editor(ui, colors, rect, id, (w, h), false)
                .is_some_and(|(w, h)| set(self, Value::Size(w, h))),
            (PType::Padding, Value::Padding(padding)) => {
                let labels = [
                    t(fx::ImageEffectOptionsPanel_Left_short),
                    t(fx::ImageEffectOptionsPanel_Top_short),
                    t(fx::ImageEffectOptionsPanel_Right_short),
                    t(fx::ImageEffectOptionsPanel_Bottom_short),
                ];
                let mut values = [padding.left, padding.top, padding.right, padding.bottom];
                let cells = kit::columns(rect, &[Track::Star(1.0); 4], 6.0);
                let mut edited = false;
                for (i, cell) in cells.iter().enumerate() {
                    let parts = kit::rows(*cell, &[Track::Px(16.0), Track::Px(40.0)], 3.0);
                    kit::text_at(
                        ui,
                        parts[0],
                        Align2::CENTER_CENTER,
                        labels[i],
                        13.0,
                        Weight::Normal,
                        colors.text(),
                    );
                    if let Some(v) = kit::plain_number_at(
                        ui,
                        colors,
                        parts[1],
                        id.with(i),
                        values[i] as i64,
                        -1_000_000,
                        1_000_000,
                        true,
                    ) {
                        values[i] = v as i32;
                        edited = true;
                    }
                }
                if edited {
                    let padding = Padding::new(values[0], values[1], values[2], values[3]);
                    return set(self, Value::Padding(padding));
                }
                false
            }
            (PType::Gradient, Value::Gradient(gradient)) => {
                let label = t(fx::ImageEffectOptionsPanel_Edit_gradient);
                let width = kit::button_width(ui, colors, None, Some(label));
                let button = Rect::from_min_size(rect.min, vec2(width, rect.height()));
                let response = kit::button_at(ui, colors, button, None, Some(label), true);
                let mut edited = gradient.clone();
                let mut result = false;
                if self.qa_flyout == Some(QaFlyout::Gradient) {
                    let at = pos2(button.left(), button.bottom() + 4.0);
                    egui::Area::new(id.with("qa-gradient"))
                        .order(egui::Order::Foreground)
                        .fixed_pos(at)
                        .show(ui.ctx(), |ui| {
                            crate::menus::frame(colors).show(ui, |ui| {
                                gradient_panel(ui, colors, id, &mut edited);
                            });
                        });
                } else {
                    let shown = egui::Popup::menu(&response)
                        .id(id.with("gradient-flyout"))
                        .frame(crate::menus::frame(colors))
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            if gradient_panel(ui, colors, id, &mut edited) {
                                result = true;
                            }
                        });
                    // `flyout.Closed += gradient.Sort()`: the stops keep their places
                    // while the flyout is open (each location box stays with its stop)
                    // and are put in order once it is gone.
                    if shown.is_none() {
                        let mut sorted = gradient.clone();
                        sorted.sort();
                        if sorted != gradient {
                            return set(self, Value::Gradient(sorted));
                        }
                    }
                }
                if result && edited != gradient {
                    return set(self, Value::Gradient(edited));
                }
                false
            }
            (ty, value) => {
                let text_kind = match ty {
                    PType::Text(kind) => Some(kind),
                    _ => None,
                };
                let browse = matches!(text_kind, Some(TextKind::File | TextKind::Folder));
                let (box_rect, browse_rect) = if browse {
                    let label = t(fx::ImageEffectOptionsPanel_Browse);
                    let w = kit::button_width(ui, colors, None, Some(label));
                    let c = kit::columns(rect, &[Track::Star(1.0), Track::Px(w)], 8.0);
                    (c[0], Some(c[1]))
                } else {
                    (rect, None)
                };
                let shown = converter_string(def, Some(&value));
                let buffer = self.texts.entry(def.name).or_insert(shown);
                let mut text = buffer.clone();
                let response =
                    kit::text_box_at(ui, colors, box_rect, id, &mut text, "", true, false);
                let mut new_text = response.changed().then(|| text.clone());
                if let Some(browse_rect) = browse_rect
                    && kit::button_at(
                        ui,
                        colors,
                        browse_rect,
                        None,
                        Some(t(fx::ImageEffectOptionsPanel_Browse)),
                        true,
                    )
                    .clicked()
                {
                    let title = format(
                        t(fx::ImageEffectOptionsPanel_Select_property),
                        &[Arg::from(m::property_name(def.name))],
                    );
                    let picked = if text_kind == Some(TextKind::Folder) {
                        state.tool_pick_folder(&title, None)
                    } else {
                        state.tool_open_file(&title, &[])
                    };
                    if let Some(path) = picked {
                        new_text = Some(path.to_string_lossy().into_owned());
                    }
                }
                let Some(text) = new_text else {
                    return false;
                };
                self.texts.insert(def.name, text.clone());
                self.set_from_text(def, &text)
            }
        }
    }

    /// `SetFromText`: the property's converter reads the text; a failure is shown under
    /// the panel and leaves the value.
    fn set_from_text(&mut self, def: &PropDef, text: &str) -> bool {
        let value = match def.ty {
            PType::Text(_) => Some(Value::Text(text.to_string())),
            PType::Font => {
                match m::FontSpec::parse_in(text, &modlight_core::culture::Culture::current()) {
                    Some(font) => Some(Value::Font(font)),
                    None => {
                        self.validation = invalid_value(text, "Font");
                        None
                    }
                }
            }
            PType::Flags(type_name, _) => match m::flags_from_string(def, text) {
                Some(f) => Some(Value::Flags(f)),
                None => {
                    self.validation = invalid_value(text, type_name);
                    None
                }
            },
            _ => None,
        };
        let Some(value) = value else {
            return false;
        };
        let Some(effect) = self.effect_mut() else {
            return false;
        };
        let before = effect.get(def.name).cloned();
        effect.set(def.name, value);
        let changed = effect.get(def.name).cloned() != before;
        self.validation.clear();
        changed
    }

    /// `CreatePairEditor`: "X" box "Y" box (or "W", "H").
    fn pair_editor(
        &mut self,
        ui: &mut Ui,
        colors: &Colors,
        rect: Rect,
        id: Id,
        value: (i32, i32),
        point: bool,
    ) -> Option<(i32, i32)> {
        let (a, b) = if point {
            (
                t(fx::ImageEffectOptionsPanel_X_short),
                t(fx::ImageEffectOptionsPanel_Y_short),
            )
        } else {
            (
                t(fx::ImageEffectOptionsPanel_Width_short),
                t(fx::ImageEffectOptionsPanel_Height_short),
            )
        };
        let wa = kit::text_width(ui, a, 13.0, Weight::Normal);
        let wb = kit::text_width(ui, b, 13.0, Weight::Normal);
        let c = kit::columns(
            rect,
            &[
                Track::Px(wa),
                Track::Star(1.0),
                Track::Px(wb),
                Track::Star(1.0),
            ],
            6.0,
        );
        for (cell, label) in [(c[0], a), (c[2], b)] {
            kit::text_at(
                ui,
                cell,
                Align2::LEFT_CENTER,
                label,
                13.0,
                Weight::Normal,
                colors.text(),
            );
        }
        let mut out = None;
        if let Some(x) = kit::plain_number_at(
            ui,
            colors,
            c[1],
            id.with("a"),
            value.0 as i64,
            -1_000_000,
            1_000_000,
            true,
        ) {
            out = Some((x as i32, value.1));
        }
        if let Some(y) = kit::plain_number_at(
            ui,
            colors,
            c[3],
            id.with("b"),
            value.1 as i64,
            -1_000_000,
            1_000_000,
            true,
        ) {
            out = Some((out.map(|o| o.0).unwrap_or(value.0), y as i32));
        }
        out
    }

    /// The preview: the picture on the checkered brush, shrunk to fit, the busy bar and
    /// the error; a click opens it in the image viewer.
    fn preview_area(
        &mut self,
        ui: &mut Ui,
        state: &mut AppState,
        colors: &Colors,
        rect: Rect,
        now: f64,
    ) {
        kit::paint_checkered_big(ui, colors, rect);
        ui.painter().rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0, colors.border()),
            StrokeKind::Inside,
        );
        let inner = rect.shrink(1.0);
        if self.texture.is_none()
            && let Some(preview) = &self.preview
        {
            self.texture = Some(kit::texture(
                ui.ctx(),
                &format!("fx-preview-{}", self.key),
                preview,
                false,
            ));
        }
        if let Some(texture) = &self.texture {
            let clip = ui.clip_rect();
            ui.set_clip_rect(inner);
            kit::image_uniform(ui, texture, inner, false);
            ui.set_clip_rect(clip);
        }
        if self.busy {
            let bar = Rect::from_min_size(inner.min, vec2(inner.width(), 4.0));
            let span = inner.width() * 0.3;
            let phase = (now * 0.8).fract() as f32;
            let x = inner.left() - span + (inner.width() + span) * phase;
            let moving = Rect::from_min_max(
                pos2(x.max(inner.left()), bar.top()),
                pos2((x + span).min(inner.right()), bar.bottom()),
            );
            ui.painter().rect_filled(moving, 2.0, colors.accent_start());
        }
        if !self.error.is_empty() {
            let galley = crate::widgets::layout(
                ui,
                crate::widgets::wrapped(
                    &self.error,
                    13.0,
                    Weight::Normal,
                    colors.cancel_border(),
                    (inner.width() - 32.0).max(40.0),
                ),
            );
            let size = galley.size() + vec2(16.0, 8.0);
            let box_rect = Rect::from_min_size(
                pos2(inner.right() - 8.0 - size.x, inner.bottom() - 8.0 - size.y),
                size,
            );
            ui.painter()
                .rect_filled(box_rect, 4.0, colors.background_panel());
            ui.painter().galley(
                box_rect.min + vec2(8.0, 4.0),
                galley,
                colors.cancel_border(),
            );
        }
        let response = ui.interact(inner, Id::new(("fx-preview", self.key)), Sense::click());
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked()
            && let Some(preview) = &self.preview
        {
            let name = if self.file_path.trim().is_empty() {
                t(fx::ImageEffectsWindow_Preview_name).to_string()
            } else {
                Path::new(&self.file_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            };
            state.open_image_viewer_picture((**preview).clone(), &name);
        }
    }

    /// The footer: open, clipboard, package on the left; save, upload or continue, and
    /// close on the right. Returns true when the window closes.
    fn footer(
        &mut self,
        ui: &mut Ui,
        state: &mut AppState,
        colors: &Colors,
        rect: Rect,
        now: f64,
    ) -> bool {
        let open = t(fx::ImageEffectsWindow_Open_image);
        let clipboard = t(fx::ImageEffectsWindow_Load_from_clipboard);
        let package_label = t(fx::ImageEffectsWindow_Package_preset);
        let mut left: Vec<(Option<char>, Option<&str>)> = Vec::new();
        if self.is_tool() {
            left.push((Some(icons::FOLDER_OPEN), Some(open)));
            left.push((Some(icons::CLIPBOARD_PASTE), Some(clipboard)));
        }
        left.push((Some(icons::PACKAGE), Some(package_label)));
        let left_rects = kit::left_aligned(ui, colors, rect, &left, 8.0);
        let has = self.selected_preset.is_some();
        let mut index = 0;
        if self.is_tool() {
            if kit::button_at(ui, colors, left_rects[0], left[0].0, left[0].1, true).clicked()
                && let Some(path) = state.tool_open_file(
                    "",
                    &[(
                        t(h::ImageHelpers_Image_files),
                        &super::files::IMAGE_EXTENSIONS,
                    )],
                )
            {
                self.load_image_file(&path, now);
            }
            if kit::button_at(ui, colors, left_rects[1], left[1].0, left[1].1, true).clicked()
                && let Some(image) = state.clipboard_image()
            {
                self.replace_source(image, None, now);
            }
            index = 2;
        }
        if kit::button_at(
            ui,
            colors,
            left_rects[index],
            left[index].0,
            left[index].1,
            has,
        )
        .clicked()
        {
            self.package_preset(state);
        }

        let close_label = if self.is_editor() {
            t(fx::ImageEffectsWindow_Cancel)
        } else {
            t(fx::ImageEffectsWindow_Close)
        };
        let save = t(fx::ImageEffectsWindow_Save_image);
        let upload = t(fx::ImageEffectsWindow_Upload_image);
        let cont = t(fx::ImageEffectsWindow_Continue);
        let mut right: Vec<(Option<char>, Option<&str>)> = Vec::new();
        if self.is_tool() {
            right.push((Some(icons::SAVE), Some(save)));
            right.push((Some(icons::UPLOAD), Some(upload)));
        }
        if self.is_editor() {
            right.push((None, Some(cont)));
        }
        right.push((None, Some(close_label)));
        let rects = kit::right_aligned(ui, colors, rect, &right, 8.0);
        let mut close = false;
        let mut i = 0;
        if self.is_tool() {
            if kit::button_at(ui, colors, rects[0], right[0].0, right[0].1, true).clicked()
                && let Some(result) = self.apply_selected()
            {
                let name = Path::new(&self.file_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if let Some(path) = state.tool_save_path(
                    "",
                    &name,
                    &[
                        ("PNG", &["png"]),
                        ("JPEG", &["jpg", "jpeg", "jpe", "jfif"]),
                        ("GIF", &["gif"]),
                        ("BMP", &["bmp"]),
                        ("TIFF", &["tif", "tiff"]),
                    ],
                ) {
                    match modlight_tools::images::save(&result, &path) {
                        Ok(()) => self.file_path = path.to_string_lossy().into_owned(),
                        Err(error) => self.error = error.to_string(),
                    }
                }
            }
            if kit::button_at(ui, colors, rects[1], right[1].0, right[1].1, true).clicked()
                && let Some(result) = self.apply_selected()
            {
                state.tool_image_task(result, self.settings.clone());
            }
            i = 2;
        }
        if self.is_editor() {
            if kit::button_at(ui, colors, rects[i], right[i].0, right[i].1, true).clicked() {
                self.accepted = true;
                close = true;
            }
            i += 1;
        }
        if kit::button_at(ui, colors, rects[i], right[i].0, right[i].1, true).clicked() {
            close = true;
        }

        // The confirmation of "Clear effects", answered through the egui memory.
        let confirm_id = Id::new(("fx-confirm", self.key));
        if ui
            .ctx()
            .data_mut(|d| d.remove_temp::<bool>(confirm_id))
            .is_some()
        {
            let parent = self.key;
            state.tools.open_keyed(|key| {
                Box::new(ConfirmWindow::new(
                    key,
                    t(fx::ImageEffectsWindow_Clear_confirmation),
                    parent,
                ))
            });
        }
        close
    }

    /// `PackagePreset`: a name is needed first.
    fn package_preset(&mut self, state: &mut AppState) {
        let Some(preset) = self.preset().cloned() else {
            return;
        };
        if preset.name.trim().is_empty() {
            self.error = t(fx::ImageEffectsWindow_Enter_preset_name).to_string();
            return;
        }
        self.error.clear();
        let json = preset.to_export_string();
        let folder = state.image_effects_folder();
        state
            .tools
            .open_keyed(|key| Box::new(PackagerWindow::new(key, json, &preset.name, folder)));
    }
}

/// `ImageEffectPresetItemViewModel.DisplayName`.
fn preset_display_name(preset: &Preset) -> String {
    if preset.name.trim().is_empty() {
        t(fx::ImageEffectsWindow_Unnamed_preset).to_string()
    } else {
        preset.name.clone()
    }
}

/// `GradientOptionsPanel` (min 380 wide, 12 px padding): the direction, the stops with
/// a colour, a location and Remove each, and Add stop. Returns true when it changed.
fn gradient_panel(ui: &mut Ui, colors: &Colors, id: Id, gradient: &mut Gradient) -> bool {
    // The menu frame's own margin and the panel's 12 px padding.
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(6, 8))
        .show(ui, |ui| gradient_panel_inner(ui, colors, id, gradient))
        .inner
}

fn gradient_panel_inner(ui: &mut Ui, colors: &Colors, id: Id, gradient: &mut Gradient) -> bool {
    let width = 380.0 - 24.0;
    ui.set_min_width(380.0 - 12.0);
    let mut changed = false;
    let (rect, _) = ui.allocate_exact_size(vec2(width, 40.0), Sense::hover());
    let label = t(fx::GradientOptionsPanel_Direction);
    kit::text_at(
        ui,
        rect,
        Align2::LEFT_CENTER,
        label,
        13.0,
        Weight::Normal,
        colors.text_secondary(),
    );
    let options: Vec<(String, String)> = m::LINEAR_GRADIENT_MODE
        .iter()
        .map(|mode| {
            (
                mode.to_string(),
                m::enum_value_name("LinearGradientMode", mode),
            )
        })
        .collect();
    let combo_w = options
        .iter()
        .map(|(_, l)| kit::text_width(ui, l, 13.0, Weight::Normal))
        .fold(0.0, f32::max)
        + 50.0;
    let combo = Rect::from_min_max(pos2(rect.right() - combo_w, rect.top()), rect.max);
    if let Some(mode) = kit::combo_at(
        ui,
        colors,
        combo,
        &format!("{id:?}-direction"),
        &gradient.mode,
        &options,
        true,
    ) {
        gradient.mode = mode;
        changed = true;
    }
    ui.add_space(10.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 40.0), Sense::hover());
    kit::text_at(
        ui,
        rect,
        Align2::LEFT_CENTER,
        t(fx::GradientOptionsPanel_Color_stops),
        13.0,
        Weight::Normal,
        colors.text_secondary(),
    );
    let add = t(fx::GradientOptionsPanel_Add_stop);
    let add_w = kit::button_width(ui, colors, None, Some(add));
    let add_rect = Rect::from_min_max(pos2(rect.right() - add_w, rect.top()), rect.max);
    if kit::button_at(ui, colors, add_rect, None, Some(add), true).clicked() {
        // `OnAddStopClick`: white, 10 after the last stop.
        let location = if gradient.stops.is_empty() {
            0.0
        } else {
            (gradient
                .stops
                .iter()
                .map(|s| s.location)
                .fold(f32::MIN, f32::max)
                + 10.0)
                .min(100.0)
        };
        gradient.stops.push(Stop {
            color: GdiColor::named("White"),
            location,
        });
        changed = true;
    }
    ui.add_space(10.0);
    let mut order: Vec<usize> = (0..gradient.stops.len()).collect();
    order.sort_by(|a, b| {
        gradient.stops[*a]
            .location
            .partial_cmp(&gradient.stops[*b].location)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let remove_label = t(fx::GradientOptionsPanel_Remove);
    let remove_w = kit::button_width(ui, colors, None, Some(remove_label));
    let mut remove = None;
    egui::ScrollArea::vertical()
        .id_salt(id.with("stops"))
        .max_height(300.0)
        .show(ui, |ui| {
            for (n, index) in order.iter().enumerate() {
                if n > 0 {
                    ui.add_space(8.0);
                }
                let (row, _) = ui.allocate_exact_size(vec2(width, 40.0), Sense::hover());
                let c = kit::columns(
                    row,
                    &[Track::Px(42.0), Track::Star(1.0), Track::Px(remove_w)],
                    8.0,
                );
                let stop_id = id.with(("stop", *index));
                let stop = &mut gradient.stops[*index];
                let response = kit::button_at(ui, colors, c[0], None, None, true);
                let swatch = Rect::from_center_size(c[0].center(), vec2(28.0, 28.0));
                kit::paint_checkered_big(ui, colors, swatch);
                ui.painter().rect_filled(swatch, 3.0, to32(stop.color.c));
                if let Some(picked) = color_flyout(ui, colors, &response, stop_id, stop.color.c) {
                    stop.color = GdiColor {
                        c: picked,
                        name: None,
                    };
                    changed = true;
                }
                let mut location = stop.location as f64;
                if crate::controls::number_box_trimmed_at(
                    ui,
                    colors,
                    ("fx-stop", stop_id),
                    c[1],
                    &mut location,
                    0.0,
                    100.0,
                    1.0,
                    0,
                    true,
                ) {
                    stop.location = location as f32;
                    changed = true;
                }
                if kit::button_at(ui, colors, c[2], None, Some(remove_label), true).clicked() {
                    remove = Some(*index);
                }
            }
        });
    if let Some(index) = remove {
        gradient.stops.remove(index);
        changed = true;
    }
    // `ReorderStops` moves a row to its place but keeps it bound to its stop: the rows
    // are drawn in location order (`order`) while the stops keep their indices, which the
    // boxes are keyed by. The stops themselves are sorted when the flyout closes.
    changed
}

// -------------------------------------------------------------------- the packager

/// `ImageEffectPackagerWindow`.
pub struct PackagerWindow {
    pub key: u64,
    json: String,
    effects_folder: PathBuf,
    pub assets_folder: String,
    pub package_file: String,
    pub overwrite: bool,
    pub status: String,
    content_height: f32,
}

impl PackagerWindow {
    pub fn new(key: u64, json: String, name: &str, effects_folder: PathBuf) -> PackagerWindow {
        PackagerWindow {
            key,
            json,
            assets_folder: effects_folder.join(name).to_string_lossy().into_owned(),
            package_file: effects_folder
                .join(format!("{name}.sxie"))
                .to_string_lossy()
                .into_owned(),
            effects_folder,
            overwrite: false,
            status: String::new(),
            content_height: 300.0,
        }
    }

    /// `OnPackageClick`.
    pub fn create(&mut self, state: &mut AppState) {
        let assets = self.assets_folder.trim().to_string();
        let output = self.package_file.trim().to_string();
        let folder = self.effects_folder.to_string_lossy().into_owned();
        let inside = format!("{}/", folder.trim_end_matches('/')).to_lowercase();
        if !assets.is_empty() && !assets.to_lowercase().starts_with(&inside) {
            self.status = t(fx::ImageEffectPackagerWindow_Assets_must_be_inside).to_string();
            return;
        }
        if output.is_empty() {
            self.status = t(fx::ImageEffectPackagerWindow_Choose_package_path).to_string();
            return;
        }
        if Path::new(&output).exists() && !self.overwrite {
            self.status = t(fx::ImageEffectPackagerWindow_Already_exists).to_string();
            return;
        }
        let assets = (!assets.is_empty()).then(|| PathBuf::from(&assets));
        match package::package(Path::new(&output), &self.json, assets.as_deref()) {
            Ok(path) => {
                if path.is_file() && state.mode == crate::app::Mode::Normal && !state.env.qa {
                    crate::platform::system::reveal_in_finder(&path);
                }
                self.status = t(fx::ImageEffectPackagerWindow_Created_successfully).to_string();
            }
            Err(error) => self.status = error,
        }
    }
}

impl ToolWindow for PackagerWindow {
    fn spec(&self) -> WindowSpec {
        WindowSpec {
            fit_height: true,
            ..WindowSpec::fixed(
                t(fx::ImageEffectPackagerWindow_Title),
                vec2(680.0, self.content_height),
            )
        }
    }

    fn ui(&mut self, ui: &mut Ui, state: &mut AppState) -> bool {
        let colors = state.colors.clone();
        kit::fill(ui, &colors);
        let full = ui.max_rect();
        let inner = Rect::from_min_size(full.min + vec2(16.0, 16.0), vec2(680.0 - 32.0, 1000.0));
        let panel_h = 12.0 * 2.0 + kit::HEADING_HEIGHT + 10.0 + 40.0 + 10.0 + 40.0 + 10.0 + 30.0;
        let panel = Rect::from_min_size(inner.min, vec2(inner.width(), panel_h));
        kit::panel(ui, &colors, panel);
        let rows = kit::rows(
            panel.shrink(12.0),
            &[
                Track::Px(kit::HEADING_HEIGHT),
                Track::Px(40.0),
                Track::Px(40.0),
                Track::Px(30.0),
            ],
            10.0,
        );
        kit::heading(
            ui,
            &colors,
            rows[0],
            t(fx::ImageEffectPackagerWindow_Package_preset),
        );
        let browse = t(fx::ImageEffectPackagerWindow_Browse);
        let bw = kit::button_width(ui, &colors, None, Some(browse));
        for (n, row) in [rows[1], rows[2]].into_iter().enumerate() {
            let c = kit::columns(
                row,
                &[Track::Px(110.0), Track::Star(1.0), Track::Px(bw)],
                8.0,
            );
            let label = if n == 0 {
                t(fx::ImageEffectPackagerWindow_Assets_folder)
            } else {
                t(fx::ImageEffectPackagerWindow_Package_file)
            };
            kit::label(ui, &colors, c[0], label);
            let text = if n == 0 {
                &mut self.assets_folder
            } else {
                &mut self.package_file
            };
            let _ = kit::text_box_at(
                ui,
                &colors,
                c[1],
                Id::new(("fx-packager", self.key, n)),
                text,
                "",
                true,
                false,
            );
            if kit::button_at(ui, &colors, c[2], None, Some(browse), true).clicked() {
                if n == 0 {
                    if let Some(folder) = state.tool_pick_folder(
                        t(fx::ImageEffectPackagerWindow_Select_assets_folder),
                        None,
                    ) {
                        self.assets_folder = folder.to_string_lossy().into_owned();
                    }
                } else {
                    let name = Path::new(&self.package_file)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if let Some(file) = state.tool_save_path(
                        t(fx::ImageEffectPackagerWindow_Save_package),
                        &name,
                        &[(t(fx::ImageEffectPackagerWindow_File_type), &["sxie"])],
                    ) {
                        self.package_file = file.to_string_lossy().into_owned();
                    }
                }
            }
        }
        let c = kit::columns(rows[3], &[Track::Star(1.0), Track::Px(44.0)], 0.0);
        kit::text_at(
            ui,
            c[0],
            Align2::LEFT_CENTER,
            t(fx::ImageEffectPackagerWindow_Overwrite_existing),
            13.0,
            Weight::Normal,
            colors.text(),
        );
        let _ = kit::toggle_at(
            ui,
            &colors,
            c[1],
            Id::new(("fx-overwrite", self.key)),
            &mut self.overwrite,
            true,
        );
        let mut y = panel.bottom() + 12.0;
        let status_h = if self.status.is_empty() {
            17.0
        } else {
            kit::paragraph(
                ui,
                Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 200.0)),
                &self.status,
                13.0,
                Weight::Normal,
                colors.cancel_border(),
            )
        };
        y += status_h + 12.0;
        let footer = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 40.0));
        let open = t(fx::ImageEffectPackagerWindow_Open_effects_folder);
        let create = t(fx::ImageEffectPackagerWindow_Create_package);
        let close_label = t(fx::ImageEffectPackagerWindow_Close);
        let left = kit::left_aligned(ui, &colors, footer, &[(None, Some(open))], 8.0);
        if kit::button_at(ui, &colors, left[0], None, Some(open), true).clicked() {
            let _ = std::fs::create_dir_all(&self.effects_folder);
            if state.mode == crate::app::Mode::Normal && !state.env.qa {
                crate::platform::system::open_path(&self.effects_folder);
            }
        }
        let right = kit::right_aligned(
            ui,
            &colors,
            footer,
            &[(None, Some(create)), (None, Some(close_label))],
            8.0,
        );
        if kit::button_at(ui, &colors, right[0], None, Some(create), true).clicked() {
            self.create(state);
        }
        let close = kit::button_at(ui, &colors, right[1], None, Some(close_label), true).clicked();
        self.content_height = (footer.bottom() + 16.0 - full.top()).round();
        close
    }

    fn content_height(&self) -> Option<f32> {
        Some(self.content_height)
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// -------------------------------------------------------------------- the confirmation

/// `ImageEffectsConfirmationWindow`: the question in 16 px, Yes and No.
pub struct ConfirmWindow {
    pub key: u64,
    pub message: String,
    /// The Image effects window that asked.
    pub parent: u64,
    content_height: f32,
}

impl ConfirmWindow {
    pub fn new(key: u64, message: &str, parent: u64) -> ConfirmWindow {
        ConfirmWindow {
            key,
            message: message.to_string(),
            parent,
            content_height: 130.0,
        }
    }
}

impl ToolWindow for ConfirmWindow {
    fn spec(&self) -> WindowSpec {
        WindowSpec {
            fit_height: true,
            ..WindowSpec::fixed(
                t(fx::ImageEffectsConfirmationWindow_Title),
                vec2(460.0, self.content_height),
            )
        }
    }

    fn ui(&mut self, ui: &mut Ui, state: &mut AppState) -> bool {
        let colors = state.colors.clone();
        kit::fill(ui, &colors);
        let full = ui.max_rect();
        let width = 460.0 - 32.0;
        let text_h = kit::paragraph(
            ui,
            Rect::from_min_size(full.min + vec2(16.0, 16.0), vec2(width, 400.0)),
            &self.message,
            16.0,
            Weight::Normal,
            colors.text(),
        );
        let footer = Rect::from_min_size(
            pos2(full.left() + 16.0, full.top() + 16.0 + text_h + 16.0),
            vec2(width, 40.0),
        );
        let yes = t(fx::ImageEffectsConfirmationWindow_Yes);
        let no = t(fx::ImageEffectsConfirmationWindow_No);
        let rects = kit::right_aligned(
            ui,
            &colors,
            footer,
            &[(None, Some(yes)), (None, Some(no))],
            8.0,
        );
        let mut close = false;
        if kit::button_at(ui, &colors, rects[0], None, Some(yes), true).clicked() {
            let now = state.time;
            super::with_window::<ImageEffectsWindow, _>(state, self.parent, |window, _| {
                window.clear_effects(now);
            });
            close = true;
        }
        if kit::button_at(ui, &colors, rects[1], None, Some(no), true).clicked() {
            close = true;
        }
        self.content_height = (footer.bottom() + 16.0 - full.top()).round();
        close
    }

    fn content_height(&self) -> Option<f32> {
        Some(self.content_height)
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// -------------------------------------------------------------------- the app's side

impl AppState {
    /// `Program.ImageEffectsFolder` (`%ShareXImageEffects%`).
    pub fn image_effects_folder(&self) -> PathBuf {
        self.env.profile.join(EFFECTS_FOLDER)
    }

    /// The presets stored for `target` (`ImageSettings.ImageEffectPresets`; none stored
    /// gives upstream's default list).
    pub fn image_effect_presets(&self, target: TaskTarget) -> Vec<Preset> {
        let object = self.task_object(target);
        let value = object
            .get("ImageSettings")
            .and_then(|v| v.get("ImageEffectPresets"));
        m::presets_from_settings(value)
    }

    pub fn save_image_effect_presets(&mut self, target: TaskTarget, presets: &[Preset]) {
        self.set_task_member(
            target,
            &["ImageSettings", "ImageEffectPresets"],
            m::presets_to_settings(presets),
        );
    }

    pub fn save_selected_image_effect_preset(&mut self, target: TaskTarget, index: usize) {
        self.set_task_member(
            target,
            &["ImageSettings", "SelectedImageEffectPreset"],
            json!(index),
        );
    }

    /// `ImageSettingsReference` of a task: the defaults, or the hotkey that overrides
    /// its image settings.
    pub fn image_settings_target(&self, settings: &TaskSettings) -> TaskTarget {
        if settings.use_default_image_settings {
            return TaskTarget::Defaults;
        }
        self.captures
            .hotkeys
            .hotkeys
            .iter()
            .position(|h| {
                h.job() == settings.job
                    && h.task
                        .get("UseDefaultImageSettings")
                        .and_then(|v| v.as_bool())
                        == Some(false)
            })
            .map(TaskTarget::Hotkey)
            .unwrap_or(TaskTarget::Defaults)
    }

    /// `ClipboardHelpers.GetImage`: a picture, or a picture file, on the clipboard.
    pub fn clipboard_image(&mut self) -> Option<RgbaImage> {
        use modlight_core::tasks::upload_manager::ClipboardContent;
        match self.read_clipboard()? {
            ClipboardContent::Image(image) => Some(image),
            ClipboardContent::Files(files) => files
                .iter()
                .find(|f| modlight_core::names::file_helpers::is_image_file(&f.to_string_lossy()))
                .and_then(|f| super::files::load_image(f)),
            ClipboardContent::Text(_) => None,
        }
    }

    /// `TaskHelpers.OpenImageEffects(filePath, taskSettings)`: a picture (asked for when
    /// none is given) in the Image effects tool window.
    pub fn open_image_effects(&mut self, file: Option<PathBuf>, settings: TaskSettings) {
        let path = match file {
            Some(path) => path,
            None => match self.tool_open_file(
                "",
                &[(
                    t(h::ImageHelpers_Image_files),
                    &super::files::IMAGE_EXTENSIONS,
                )],
            ) {
                Some(path) => path,
                None => return,
            },
        };
        let Some(image) = super::files::load_image(&path) else {
            return;
        };
        let home = self.image_settings_target(&settings);
        let presets = self.image_effect_presets(home);
        let selected = settings.image_settings.selected_image_effect_preset;
        let colors = self.colors.clone();
        self.tools.open_keyed(|key| {
            Box::new(ImageEffectsWindow::new(
                key,
                WindowMode::Tool,
                home,
                Some(image),
                presets,
                selected,
                Some(path.to_string_lossy().into_owned()),
                settings,
                &colors,
            ))
        });
    }

    /// `TaskHelpers.OpenImageEffectsSingleton(taskSettings, importJson)`: the presets of
    /// `target` on the sample picture; one such window at a time.
    pub fn open_image_effects_presets(&mut self, target: TaskTarget, import_json: Option<String>) {
        let folder = self.image_effects_folder();
        let now = self.time;
        if let Some(key) = self.tools.windows.iter_mut().find_map(|h| {
            let key = h.key;
            h.window
                .as_any_mut()
                .downcast_mut::<ImageEffectsWindow>()
                .filter(|w| w.mode == WindowMode::Presets)
                .map(|w| {
                    if let Some(json) = &import_json {
                        w.import_preset(json, &folder, now);
                    }
                    key
                })
        }) {
            self.tools.restore.push(key);
            return;
        }
        let settings = self.task_settings_of(target);
        let presets = self.image_effect_presets(target);
        let selected = settings.image_settings.selected_image_effect_preset;
        let colors = self.colors.clone();
        self.tools.open_keyed(|key| {
            let mut window = ImageEffectsWindow::new(
                key,
                WindowMode::Presets,
                target,
                None,
                presets,
                selected,
                None,
                settings,
                &colors,
            );
            if let Some(json) = &import_json {
                window.import_preset(json, &folder, now);
            }
            Box::new(window)
        });
    }

    /// `TaskHelpers.ImportImageEffect(filePath)`: a `.sxie` package unpacked into the
    /// image effects folder, its preset added in the presets window, and an offer to turn
    /// "Add image effects" on.
    pub fn import_image_effect(&mut self, file: &Path) {
        let folder = self.image_effects_folder();
        let json = match package::extract_package(file, &folder) {
            Ok(extracted) => extracted.config_json,
            Err(error) => {
                self.open_error_window(&modlight_core::title(), &error, false);
                None
            }
        };
        let Some(json) = json.filter(|j| !j.is_empty()) else {
            return;
        };
        self.open_image_effects_presets(TaskTarget::Defaults, Some(json));
        if !self
            .default_task_settings()
            .after_capture_job
            .has(AfterCaptureTasks::AddImageEffects)
        {
            let mut confirm = crate::dialogs::confirm::Confirm::new(
                "",
                t(s::WouldYouLikeToEnableImageEffects),
                crate::dialogs::confirm::ConfirmAction::EnableImageEffects,
            );
            confirm.title = modlight_core::APP_NAME.to_string();
            self.windows.confirm = Some(confirm);
        }
    }

    /// The import's question was answered Yes.
    pub fn enable_image_effects(&mut self) {
        let current = self.default_task_settings().after_capture_job;
        if !current.has(AfterCaptureTasks::AddImageEffects) {
            self.toggle_after_capture_task(AfterCaptureTasks::AddImageEffects.bits());
        }
    }

    /// `ApplyImageEffects` for a task (`TaskQuestion::ImageEffects`): the Image effects
    /// window first when the task asks for it, then the chosen preset.
    pub fn image_effects_for_task(
        &mut self,
        image: Arc<RgbaImage>,
        settings: TaskSettings,
        reply: oneshot::Sender<TaskAnswer>,
    ) {
        let home = self.image_settings_target(&settings);
        let presets = self.image_effect_presets(home);
        let selected = settings.image_settings.selected_image_effect_preset;
        let finish = TaskFinish {
            image: image.clone(),
            settings: Box::new(settings.clone()),
            reply,
        };
        if settings
            .image_settings
            .show_image_effects_window_after_capture
        {
            let colors = self.colors.clone();
            let source = (*image).clone();
            self.tools.open_keyed(|key| {
                let mut window = ImageEffectsWindow::new(
                    key,
                    WindowMode::Editor,
                    home,
                    Some(source),
                    presets,
                    selected,
                    None,
                    settings,
                    &colors,
                );
                window.finish = Some(finish);
                Box::new(window)
            });
        } else {
            self.apply_task_preset(finish, &presets, selected);
        }
    }

    /// The preset upstream picks (`UseRandomImageEffect`, else the selected one) applied
    /// off the UI thread; no preset leaves the picture as it is.
    pub fn apply_task_preset(&mut self, finish: TaskFinish, presets: &[Preset], selected: i64) {
        let preset = if finish.settings.image_settings.use_random_image_effect {
            (!presets.is_empty()).then(|| {
                let index = modlight_effects::dotnet::shared_next(0, presets.len() as i32);
                presets[index as usize].clone()
            })
        } else if selected >= 0 && (selected as usize) < presets.len() {
            Some(presets[selected as usize].clone())
        } else {
            None
        };
        let TaskFinish { image, reply, .. } = finish;
        let Some(preset) = preset else {
            let _ = reply.send(TaskAnswer::Annotated(Some(image)));
            return;
        };
        self.rt.spawn_blocking(move || {
            let result = apply_guarded(&preset, &image).ok().flatten();
            let _ = reply.send(TaskAnswer::Annotated(result.map(Arc::new)));
            |_: &mut AppState, _: &egui::Context| {}
        });
    }

    /// The tray's preset radio (`BuildImageEffectPresetMenu`).
    pub fn select_image_effect_preset(&mut self, index: usize) {
        self.save_selected_image_effect_preset(TaskTarget::Defaults, index);
    }
}

/// The after-capture menu with "Add image effects" opening its presets
/// (`createChildren: BuildImageEffectPresetMenu`).
pub fn after_capture_menu_with_presets(state: &AppState, current: AfterCaptureTasks) -> Vec<Entry> {
    let label = t(h::AfterCaptureTasks_AddImageEffects);
    crate::uploads::after_capture_menu(current)
        .into_iter()
        .map(|entry| {
            if entry.label() != label {
                return entry;
            }
            Entry::submenu(
                label,
                icons::WAND_SPARKLES,
                true,
                preset_menu(state, current),
            )
        })
        .collect()
}

/// `BuildImageEffectPresetMenu`.
pub fn preset_menu(state: &AppState, current: AfterCaptureTasks) -> Vec<Entry> {
    let mut items = vec![
        Entry::check(
            t(s::MainMenuBuilder_EnableAddImageEffects),
            icons::WAND_SPARKLES,
            current.has(AfterCaptureTasks::AddImageEffects),
            Command::ToggleAfterCaptureTask(AfterCaptureTasks::AddImageEffects.bits()),
        ),
        Entry::Separator,
    ];
    let selected = state
        .default_task_settings()
        .image_settings
        .selected_image_effect_preset;
    for (index, preset) in state
        .image_effect_presets(TaskTarget::Defaults)
        .iter()
        .enumerate()
    {
        // `ImageEffectPreset.ToString()`.
        let name = if preset.name.is_empty() {
            "Name".to_string()
        } else {
            preset.name.clone()
        };
        items.push(Entry::Item {
            label: name,
            icon: Some(icons::WAND_SPARKLES),
            enabled: true,
            toggle: crate::menus::Toggle::Radio(index as i64 == selected),
            command: Some(Command::SelectImageEffectPreset(index)),
        });
    }
    if items.len() == 2 {
        items.push(Entry::header(
            t(s::MainMenuBuilder_NoImageEffectPresets),
            Some(icons::WAND_SPARKLES),
        ));
    }
    items
}

// -------------------------------------------------------------------- QA

/// The screens of this module.
pub const SCREENS: &[&str] = &[
    "image-effects",
    "image-effects-presets",
    "image-effects-editor",
    "image-effects-empty",
    "image-effects-add",
    "image-effects-gradient",
    "image-effects-packager",
    "image-effects-confirm",
];

/// A fixed gradient for the QA presets (the default one is picked at random).
fn qa_gradient() -> Gradient {
    Gradient::new(
        "Vertical",
        &[(C::rgb(0, 187, 138), 0.0), (C::rgb(0, 105, 163), 100.0)],
    )
}

fn qa_presets() -> Vec<Preset> {
    let mut default = Preset::default_preset();
    default.name = "Watermark".into();
    for effect in &mut default.effects {
        if effect.kind == Kind::DrawText {
            effect.set("Gradient", Value::Gradient(qa_gradient()));
        }
    }
    let mut shadow = ClassicEffect::new(Kind::Shadow);
    shadow.set("Offset", Value::Point(4, 4));
    let mut rounded = ClassicEffect::new(Kind::RoundedCorners);
    rounded.set("CornerRadius", Value::Int(24));
    let mut border = ClassicEffect::new(Kind::DrawBorder);
    border.enabled = false;
    let mut glow = ClassicEffect::new(Kind::Glow);
    glow.set("Gradient", Value::Gradient(qa_gradient()));
    let soft = Preset {
        name: "Soft shadow".into(),
        effects: vec![rounded, shadow, border],
    };
    let gradient = Preset {
        name: "Glow".into(),
        effects: vec![glow],
    };
    vec![default, soft, gradient]
}

/// Set the state up for one of [`SCREENS`]; the size the window wants.
pub fn qa_prepare(screen: &str, state: &mut AppState) -> Option<egui::Vec2> {
    if !SCREENS.contains(&screen) {
        return None;
    }
    let colors = state.colors.clone();
    let settings = state.default_task_settings();
    let picture = super::qa::sunset(800, 500);
    let open = |state: &mut AppState,
                mode: WindowMode,
                source: Option<RgbaImage>,
                presets: Vec<Preset>,
                selected: i64,
                setup: &dyn Fn(&mut ImageEffectsWindow)| {
        let settings = settings.clone();
        let colors = colors.clone();
        state.tools.open_keyed(|key| {
            let mut window = ImageEffectsWindow::new(
                key,
                mode,
                TaskTarget::Defaults,
                source,
                presets,
                selected,
                Some(crate::qa::fixture_path(
                    "/Users/alex/Pictures/mountain-sunset.png",
                )),
                settings,
                &colors,
            );
            setup(&mut window);
            window.preview_now();
            Box::new(window)
        })
    };
    match screen {
        "image-effects" => {
            open(
                state,
                WindowMode::Tool,
                Some(picture),
                qa_presets(),
                0,
                &|w| {
                    w.set_selected_effect(Some(1));
                },
            );
        }
        "image-effects-presets" => {
            open(state, WindowMode::Presets, None, qa_presets(), 1, &|w| {
                w.set_selected_effect(Some(1));
            });
        }
        "image-effects-editor" => {
            open(
                state,
                WindowMode::Editor,
                Some(picture),
                qa_presets(),
                1,
                &|w| {
                    w.set_selected_effect(Some(0));
                },
            );
        }
        "image-effects-empty" => {
            open(state, WindowMode::Presets, None, Vec::new(), 0, &|_| {});
        }
        "image-effects-add" => {
            open(state, WindowMode::Presets, None, qa_presets(), 1, &|w| {
                w.qa_flyout = Some(QaFlyout::AddEffect);
            });
        }
        "image-effects-gradient" => {
            open(state, WindowMode::Presets, None, qa_presets(), 2, &|w| {
                w.set_selected_effect(Some(0));
                w.qa_flyout = Some(QaFlyout::Gradient);
            });
        }
        "image-effects-packager" => {
            let folder = PathBuf::from(crate::qa::fixture_path(
                "/Users/alex/Library/Application Support/Modlight",
            ))
            .join(EFFECTS_FOLDER);
            let preset = qa_presets().remove(0);
            state.tools.open_keyed(|key| {
                Box::new(PackagerWindow::new(
                    key,
                    preset.to_export_string(),
                    &preset.name,
                    folder,
                ))
            });
            return Some(vec2(680.0, 300.0));
        }
        "image-effects-confirm" => {
            state.tools.open_keyed(|key| {
                Box::new(ConfirmWindow::new(
                    key,
                    t(fx::ImageEffectsWindow_Clear_confirmation),
                    0,
                ))
            });
            return Some(vec2(460.0, 130.0));
        }
        _ => return None,
    }
    Some(vec2(WIDTH, HEIGHT))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(presets: Vec<Preset>) -> ImageEffectsWindow {
        let colors = Colors::new(&modlight_core::theme::resolve(
            &Default::default(),
            Some(true),
            None,
        ));
        ImageEffectsWindow::new(
            1,
            WindowMode::Presets,
            TaskTarget::Defaults,
            Some(RgbaImage::new(8, 8)),
            presets,
            0,
            None,
            TaskSettings::default(),
            &colors,
        )
    }

    #[test]
    fn an_empty_list_gets_a_new_preset() {
        let w = window(Vec::new());
        assert_eq!(w.presets.len(), 1);
        assert_eq!(w.selected_preset, Some(0));
        assert_eq!(w.selected_effect, None);
    }

    #[test]
    fn effects_are_added_after_the_selection_and_moved() {
        let mut w = window(vec![Preset::default_preset()]);
        assert_eq!(w.selected_effect, Some(0));
        w.add_effect(ClassicEffect::new(Kind::Blur), 0.0);
        let kinds: Vec<Kind> = w.presets[0].effects.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, vec![Kind::Canvas, Kind::Blur, Kind::DrawText]);
        assert_eq!(w.selected_effect, Some(1));
        w.move_effect(1, 0.0);
        assert_eq!(w.presets[0].effects[2].kind, Kind::Blur);
        w.remove_effect(0.0);
        assert_eq!(w.presets[0].effects.len(), 2);
        assert_eq!(w.selected_effect, Some(1));
        w.clear_effects(0.0);
        assert!(w.presets[0].effects.is_empty());
        assert_eq!(w.selected_effect, None);
    }

    #[test]
    fn presets_are_removed_duplicated_and_moved() {
        let mut w = window(vec![Preset::default_preset()]);
        w.duplicate_preset(0.0);
        assert_eq!(w.presets.len(), 2);
        assert_eq!(w.selected_preset, Some(1));
        w.presets[1].name = "B".into();
        w.move_preset(-1);
        assert_eq!(w.presets[0].name, "B");
        assert_eq!(w.selected_preset, Some(0));
        w.remove_preset(0.0);
        w.remove_preset(0.0);
        assert_eq!(w.presets.len(), 1, "the last removal leaves a new preset");
    }

    #[test]
    fn imported_presets_lose_paths_outside_the_effects_folder() {
        let folder = std::env::temp_dir().join("modlight-fx-import-test");
        let mut w = window(Vec::new());
        let mut draw = ClassicEffect::new(Kind::DrawImage);
        draw.set("ImageLocation", Value::Text("/etc/passwd".into()));
        let mut inside = ClassicEffect::new(Kind::DrawBackgroundImage);
        // Built a part at a time: one separator throughout, the system's.
        let kept = folder
            .join("Preset")
            .join("bg.png")
            .to_string_lossy()
            .into_owned();
        inside.set("ImageFilePath", Value::Text(kept.clone()));
        let preset = Preset {
            name: "P".into(),
            effects: vec![draw, inside],
        };
        w.import_preset(&preset.to_export_string(), &folder, 0.0);
        let imported = w.presets.last().unwrap();
        assert_eq!(imported.effects[0].text("ImageLocation"), "");
        assert_eq!(imported.effects[1].text("ImageFilePath"), kept);
        assert_eq!(w.selected_preset, Some(w.presets.len() - 1));
    }

    /// `OnSelectedPresetChanged` makes new effect items, so the options panel is built
    /// anew (`SetEffect`) even when the first effect stays selected: no text box may keep
    /// the previous preset's text, which an edit would then write into this preset.
    #[test]
    fn switching_presets_rebuilds_the_text_editors() {
        let watermark = |text: &str| {
            let mut e = ClassicEffect::new(Kind::DrawText);
            e.set("Text", Value::Text(text.into()));
            Preset {
                name: text.into(),
                effects: vec![e],
            }
        };
        let mut w = window(vec![watermark("first"), watermark("second")]);
        assert_eq!(w.selected_effect, Some(0));
        // What the panel's text boxes hold while the first preset is shown.
        w.texts.insert("Text", "first".into());
        w.validation = "first is not a valid value for Font.".into();
        w.select_preset(Some(1), 0.0);
        assert_eq!(w.selected_effect, Some(0));
        assert!(w.texts.is_empty(), "{:?}", w.texts);
        assert!(w.validation.is_empty());
        // The same through removing the selected preset.
        w.texts.insert("Text", "second".into());
        w.remove_preset(0.0);
        assert!(w.texts.is_empty(), "{:?}", w.texts);
    }

    /// `GradientOptionsPanel` keeps each location box bound to its stop while the rows
    /// reorder: typing a location past another stop's must go on editing the same stop.
    #[test]
    fn a_stop_location_typed_past_another_stays_with_its_stop() {
        use egui_kittest::kittest::Queryable;
        let colors = Colors::new(&modlight_core::theme::resolve(
            &Default::default(),
            Some(true),
            None,
        ));
        let gradient = Gradient::new(
            "Vertical",
            &[
                (C::rgb(255, 0, 0), 0.0),
                (C::rgb(0, 255, 0), 20.0),
                (C::rgb(0, 0, 255), 100.0),
            ],
        );
        let mut fonts_set = false;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(vec2(420.0, 420.0))
            .build_ui_state(
                move |ui, g: &mut Gradient| {
                    // The app's fonts take effect from the next pass on.
                    if !fonts_set {
                        ui.ctx().set_fonts(crate::theme::fonts());
                        fonts_set = true;
                        return;
                    }
                    gradient_panel_inner(ui, &colors, Id::new("gradient-test"), g);
                },
                gradient,
            );
        harness.run();
        harness
            .get_by(|n| {
                n.role() == egui::accesskit::Role::TextInput && n.value().as_deref() == Some("0")
            })
            .focus();
        harness.run();
        for key in ["5", "0", "1"] {
            harness.get_by(|n| n.is_focused()).type_text(key);
            harness.run();
        }
        let location_of = |g: &Gradient, c: C| {
            g.stops
                .iter()
                .find(|s| s.color.c == c)
                .map(|s| s.location)
                .unwrap()
        };
        let g = harness.state();
        assert_eq!(location_of(g, C::rgb(0, 255, 0)), 20.0, "{:?}", g.stops);
        assert_eq!(location_of(g, C::rgb(255, 0, 0)), 100.0, "{:?}", g.stops);
    }

    #[test]
    fn the_add_menu_lists_every_class_by_category() {
        let menu = add_effect_menu();
        assert_eq!(menu.len(), 4);
        let total: usize = menu
            .iter()
            .map(|e| match e {
                Entry::Submenu { children, .. } => children.len(),
                _ => 0,
            })
            .sum();
        assert_eq!(total, 51);
        assert_eq!(catalog_kind(0), Some(Kind::DrawBackground));
        assert_eq!(catalog_kind(50), Some(Kind::WaveEdge));
    }
}
