//! Phase 5, the ShareX tools on the window's side: every tool window (one native window
//! each, as `ToolsIntegration.Show` opens them), the pinned pictures, the screen colour
//! picker's overlay, and `TaskHelpers.ExecuteJob` for the tool hotkeys. The logic of each
//! tool is in `modlight-tools`; the macOS calls are in `modlight-macos`.
//!
//! The main window calls [`viewports`] every frame (real and QA windows only) and the
//! hotkey dispatcher calls [`AppState::run_tool_job`] for every job it does not run itself.

use std::any::Any;

use egui::{Rect, Ui, Vec2, ViewportBuilder, ViewportCommand, ViewportId, pos2, vec2};
use modlight_core::hotkeys::HotkeyType;
use modlight_core::tasks::settings::TaskSettings;

use crate::state::AppState;

pub mod jobs;
pub mod kit;
pub mod qa;

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
}

impl WindowSpec {
    pub fn resizable(title: &str, size: Vec2, min: Vec2) -> WindowSpec {
        WindowSpec {
            title: title.to_string(),
            size,
            min_size: Some(min),
            topmost: false,
            fit_height: false,
        }
    }

    pub fn fixed(title: &str, size: Vec2) -> WindowSpec {
        WindowSpec {
            title: title.to_string(),
            size,
            min_size: None,
            topmost: false,
            fit_height: false,
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
}

impl Tools {
    /// Open a window; returns its key.
    pub fn open(&mut self, window: Box<dyn ToolWindow>) -> u64 {
        self.next_key += 1;
        let key = self.next_key;
        self.windows.push(Hosted { key, window });
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

/// A window centred on the screen (`WindowStartupLocation="CenterScreen"`).
fn builder(spec: &WindowSpec, size: Vec2) -> ViewportBuilder {
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
        let size = vec2(spec.size.x, height.max(80.0).round());
        let id = ViewportId::from_hash_of(("tool-window", hosted.key));
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
