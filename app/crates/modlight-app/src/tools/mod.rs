//! Phase 5, the ShareX tools on the window's side: every tool window (one native window
//! each, as `ToolsIntegration.Show` opens them), the pinned pictures, the screen colour
//! picker's overlay, and `TaskHelpers.ExecuteJob` for the tool hotkeys. The logic of each
//! tool is in `modlight-tools`; the macOS calls are in `modlight-macos`.
//!
//! The main window calls [`viewports`] every frame (real and QA windows only) and the
//! hotkey dispatcher calls [`AppState::run_tool_job`] for every job it does not run itself.

use std::any::Any;

use egui::{Id, Rect, Ui, Vec2, ViewportBuilder, ViewportCommand, ViewportId, pos2, vec2};
use modlight_core::hotkeys::HotkeyType;
use modlight_core::tasks::settings::TaskSettings;

use crate::state::AppState;

pub mod analyze;
pub mod boxes;
pub mod color_picker;
pub mod combiner;
pub mod editor_hooks;
pub mod files;
pub mod image_viewer;
pub mod jobs;
pub mod kit;
pub mod metadata;
pub mod ocr;
pub mod pin;
pub mod qa;
pub mod qr;
pub mod region;
pub mod screen_picker;
pub mod video;

/// How a tool window opens.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowSpec {
    pub title: String,
    pub size: Vec2,
    /// `MinWidth`/`MinHeight` of a resizable window; `None` for a fixed one.
    pub min_size: Option<Vec2>,
    pub topmost: bool,
    /// `SizeToContent="Height"`: the height follows what the window last drew.
    pub fit_height: bool,
    /// `WindowState="FullScreen" WindowDecorations="None"`: no frame, covering this
    /// display area (desktop points), above the menu bar.
    pub screen: Option<Rect>,
}

impl WindowSpec {
    pub fn resizable(title: &str, size: Vec2, min: Vec2) -> WindowSpec {
        WindowSpec {
            title: title.to_string(),
            size,
            min_size: Some(min),
            topmost: false,
            fit_height: false,
            screen: None,
        }
    }

    pub fn fixed(title: &str, size: Vec2) -> WindowSpec {
        WindowSpec {
            title: title.to_string(),
            size,
            min_size: None,
            topmost: false,
            fit_height: false,
            screen: None,
        }
    }
}

/// One tool window.
pub trait ToolWindow: Any {
    fn spec(&self) -> WindowSpec;
    /// Draw one frame into `ui` (the whole window). Returns true when the window closes
    /// itself.
    fn ui(&mut self, ui: &mut Ui, state: &mut AppState) -> bool;
    /// The window is going away (closed by its button, by the system or by itself).
    fn closed(&mut self, _state: &mut AppState) {}
    /// The height the content wanted last frame (`fit_height` windows).
    fn content_height(&self) -> Option<f32> {
        None
    }
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// A window and its key.
pub struct Hosted {
    pub key: u64,
    pub window: Box<dyn ToolWindow>,
}

/// The tools' share of the app state.
#[derive(Default)]
pub struct Tools {
    pub next_key: u64,
    pub windows: Vec<Hosted>,
    /// The screen colour picker: waiting for its delay, taking the desktop, open.
    pub pending_pick: Option<screen_picker::PendingPick>,
    pub screen_pick_busy: bool,
    pub screen_picker: Option<screen_picker::ScreenPicker>,
    /// Tool windows to bring back after they stepped aside (hidden or minimised).
    pub restore: Vec<u64>,
    /// The pinned pictures (`PinToScreenManager.Windows`).
    pub pins: Vec<pin::Pin>,
    /// Input boxes that closed: their key and the text, when OK was pressed.
    pub input_answers: Vec<(u64, Option<String>)>,
    /// QA runs: the text the OCR stand-in "recognises".
    pub qa_ocr_text: Option<String>,
    /// QA runs: what the AI stand-in answers.
    pub qa_ai_text: Option<String>,
    /// What the open panels answer in QA and headless runs (tests queue paths here).
    pub qa_open: std::collections::VecDeque<std::path::PathBuf>,
}

impl Tools {
    /// Open a window; returns its key.
    pub fn open(&mut self, window: Box<dyn ToolWindow>) -> u64 {
        self.next_key += 1;
        let key = self.next_key;
        self.windows.push(Hosted { key, window });
        key
    }

    /// Open a window that needs to know its own key.
    pub fn open_keyed(&mut self, make: impl FnOnce(u64) -> Box<dyn ToolWindow>) -> u64 {
        self.next_key += 1;
        let key = self.next_key;
        self.windows.push(Hosted {
            key,
            window: make(key),
        });
        key
    }

    /// The window with `key`, as its own type.
    pub fn window_mut<T: ToolWindow>(&mut self, key: u64) -> Option<&mut T> {
        self.windows
            .iter_mut()
            .find(|h| h.key == key)
            .and_then(|h| h.window.as_any_mut().downcast_mut::<T>())
    }

    /// The first window of a type.
    pub fn first_mut<T: ToolWindow>(&mut self) -> Option<&mut T> {
        self.windows
            .iter_mut()
            .find_map(|h| h.window.as_any_mut().downcast_mut::<T>())
    }

    /// Whether a window of a type is open.
    pub fn has<T: ToolWindow>(&mut self) -> bool {
        self.first_mut::<T>().is_some()
    }
}

/// Run `f` on the window with `key` (taken out of the list for the call, so `f` may use
/// the whole state); nothing when it is gone.
pub fn with_window<T: ToolWindow, R>(
    state: &mut AppState,
    key: u64,
    f: impl FnOnce(&mut T, &mut AppState) -> R,
) -> Option<R> {
    let index = state.tools.windows.iter().position(|h| h.key == key)?;
    let mut hosted = state.tools.windows.remove(index);
    let result = hosted
        .window
        .as_any_mut()
        .downcast_mut::<T>()
        .map(|window| f(window, state));
    let index = index.min(state.tools.windows.len());
    state.tools.windows.insert(index, hosted);
    result
}

/// A window centred on the screen (`WindowStartupLocation="CenterScreen"`).
fn builder(spec: &WindowSpec, size: Vec2) -> ViewportBuilder {
    if let Some(screen) = spec.screen {
        let mut builder = ViewportBuilder::default()
            .with_title(spec.title.clone())
            .with_decorations(false)
            .with_resizable(false)
            .with_taskbar(false)
            .with_position(screen.min)
            .with_inner_size(screen.size());
        if spec.topmost {
            builder = builder.with_always_on_top();
        }
        return builder;
    }
    let resizable = spec.min_size.is_some();
    let mut builder = ViewportBuilder::default()
        .with_title(spec.title.clone())
        .with_inner_size(size)
        .with_resizable(resizable)
        .with_minimize_button(true)
        .with_maximize_button(resizable);
    if let Some(min) = spec.min_size {
        builder = builder.with_min_inner_size(min);
    }
    if spec.topmost {
        builder = builder.with_always_on_top();
    }
    if let Some((x, y, w, h)) = modlight_macos::system::visible_frame() {
        let area = Rect::from_min_size(pos2(x, y), vec2(w, h));
        builder = builder.with_position(pos2(
            area.center().x - size.x / 2.0,
            (area.center().y - size.y / 2.0).max(area.top()),
        ));
    }
    builder
}

/// Every tool window, each its own native window; called every frame.
pub fn viewports(ctx: &egui::Context, state: &mut AppState) {
    jobs::tick(state, ctx);
    screen_picker::viewports(ctx, state);
    pin::viewports(ctx, state);
    for key in std::mem::take(&mut state.tools.restore) {
        let id = ViewportId::from_hash_of(("tool-window", key));
        ctx.send_viewport_cmd_to(id, ViewportCommand::Visible(true));
        ctx.send_viewport_cmd_to(id, ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd_to(id, ViewportCommand::Focus);
    }
    if state.tools.windows.is_empty() {
        return;
    }
    let mut windows = std::mem::take(&mut state.tools.windows);
    let mut closed = Vec::new();
    for hosted in windows.iter_mut() {
        let spec = hosted.window.spec();
        let height = if spec.fit_height {
            hosted.window.content_height().unwrap_or(spec.size.y)
        } else {
            spec.size.y
        };
        let size = match spec.screen {
            Some(screen) => screen.size(),
            None => vec2(spec.size.x, height.max(80.0).round()),
        };
        let key = hosted.key;
        let id = ViewportId::from_hash_of(("tool-window", key));
        let (close_asked, closes_itself) =
            ctx.show_viewport_immediate(id, builder(&spec, size), |ui, _| {
                let close = ui.ctx().input(|i| i.viewport().close_requested());
                if spec.fit_height {
                    let current = ui
                        .ctx()
                        .input(|i| i.viewport().inner_rect.map(|r| r.size()));
                    if current.is_some_and(|c| (c.y - size.y).abs() > 0.5) {
                        ui.ctx().send_viewport_cmd(ViewportCommand::InnerSize(size));
                    }
                }
                if spec.screen.is_some() {
                    let raised = Id::new(("tool-window-raised", key));
                    if !ui
                        .ctx()
                        .data(|d| d.get_temp::<bool>(raised).unwrap_or(false))
                        && modlight_macos::windows::raise_overlay(&spec.title)
                    {
                        ui.ctx().data_mut(|d| d.insert_temp(raised, true));
                        ui.ctx().send_viewport_cmd(ViewportCommand::Focus);
                    }
                }
                let title = ui.ctx().input(|i| i.viewport().title.clone());
                if title.as_deref() != Some(spec.title.as_str()) {
                    ui.ctx()
                        .send_viewport_cmd(ViewportCommand::Title(spec.title.clone()));
                }
                let done = hosted.window.ui(ui, state);
                (close, done)
            });
        if close_asked || closes_itself {
            closed.push(hosted.key);
        }
    }
    for key in &closed {
        if let Some(hosted) = windows.iter_mut().find(|h| h.key == *key) {
            hosted.window.closed(state);
        }
    }
    windows.retain(|h| !closed.contains(&h.key));
    // Windows opened while these were drawn come after them.
    windows.append(&mut state.tools.windows);
    state.tools.windows = windows;
}

impl AppState {
    /// `TaskHelpers.ExecuteJob` for the tools, the history windows and whatever the
    /// capture dispatcher does not run itself.
    pub fn run_tool_job(&mut self, job: HotkeyType, settings: TaskSettings) {
        jobs::run(self, job, settings, None);
    }

    /// The same, for a file (`ExecuteJob(job, filePath)`: the Finder service, the
    /// history windows' and the notices' actions).
    pub fn run_tool_job_on_file(
        &mut self,
        job: HotkeyType,
        settings: TaskSettings,
        file: std::path::PathBuf,
    ) {
        jobs::run(self, job, settings, Some(file));
    }
}
