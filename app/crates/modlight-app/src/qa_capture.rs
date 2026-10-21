//! QA fixtures for phase 3's capture screens: the Capture tab with registered hotkeys and
//! with one that failed, the capture menu, the region overlay (selection, window hover,
//! light), the recording toolbar, the after-capture, auto-capture and scrolling capture
//! windows, the quick task menu and its editor. Everything is drawn from the fake
//! desktop; nothing asks macOS for anything.

use std::sync::Arc;

use egui::{Ui, Vec2, vec2};
use image::{Rgba, RgbaImage};
use modlight_core::capture::geometry::{IRect, Point};
use modlight_core::capture::region::{Button, RegionKind, RegionRequest, RegionSession};
use modlight_core::capture::source::{CaptureSource, FakeSource, ScreenInfo, WindowInfo};
use modlight_core::hotkeys::{FakeBackend, HotkeyInfo, HotkeyManager, Keys, default_hotkeys};
use modlight_core::recording::{RecordState, RecorderControl};
use modlight_core::tasks::info::TaskMetadata;

use crate::dialogs::after_capture::AfterCapture;
use crate::region_overlay::{OverlayPurpose, RegionOverlay};
use crate::state::AppState;

/// The screens this module draws.
pub const SCREENS: &[&str] = &[
    "tab-create-problem",
    "capture-menu",
    "region-capture",
    "region-capture-hover",
    "region-capture-light",
    "region-capture-annotate",
    "recording-toolbar",
    "recording-toolbar-compact",
    "after-capture",
    "after-capture-recording",
    "auto-capture",
    "quick-task-menu",
    "quick-task-editor",
    "scrolling-capture",
];

/// A desktop the size of a QA shot: one display of 960 by 600 with three windows.
pub fn qa_desktop() -> FakeSource {
    let (w, h) = (960u32, 600u32);
    let mut desktop = RgbaImage::from_fn(w, h, |x, y| {
        let fx = x as f32 / w as f32;
        let fy = y as f32 / h as f32;
        Rgba([
            (46.0 + 90.0 * fx) as u8,
            (70.0 + 70.0 * fy) as u8,
            (150.0 + 60.0 * (1.0 - fx)) as u8,
            255,
        ])
    });
    let windows = vec![
        WindowInfo {
            id: 11,
            title: "Notes".into(),
            process_name: "Notes".into(),
            pid: 501,
            rect: IRect::new(90, 70, 420, 300),
            on_screen: true,
        },
        WindowInfo {
            id: 12,
            title: "Finder".into(),
            process_name: "Finder".into(),
            pid: 502,
            rect: IRect::new(420, 220, 440, 300),
            on_screen: true,
        },
    ];
    let fills = [Rgba([250, 248, 240, 255]), Rgba([236, 240, 246, 255])];
    for (window, fill) in windows.iter().zip(fills).rev() {
        let r = window.rect;
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                let bar = y < r.y + 26;
                let mut pixel = if bar {
                    Rgba([214, 214, 218, 255])
                } else {
                    fill
                };
                // A few lines of "text" so the magnifier has something to show.
                if !bar && (y - r.y) % 18 == 8 && x > r.x + 16 && x < r.right() - 40 {
                    pixel = Rgba([60, 60, 70, 255]);
                }
                desktop.put_pixel(x as u32, y as u32, pixel);
            }
        }
        for (dx, color) in [
            (14, Rgba([255, 95, 87, 255])),
            (32, Rgba([254, 188, 46, 255])),
            (50, Rgba([40, 200, 64, 255])),
        ] {
            for y in -5i32..=5 {
                for x in -5i32..=5 {
                    if x * x + y * y <= 25 {
                        desktop.put_pixel((r.x + dx + x) as u32, (r.y + 13 + y) as u32, color);
                    }
                }
            }
        }
    }
    let screens = vec![ScreenInfo {
        id: 1,
        bounds: IRect::new(0, 0, w as i32, h as i32),
        working_area: IRect::new(0, 25, w as i32, h as i32 - 25),
        scale: 1.0,
        primary: true,
        name: "QA Display".into(),
    }];
    FakeSource::new(desktop, 1.0, screens, windows)
}

fn session(kind: RegionKind) -> RegionSession {
    let source = qa_desktop();
    let bounds = source.screen_bounds();
    let request = RegionRequest {
        kind,
        screen_bounds: bounds,
        scale: 1.0,
        image: Some(Arc::new(source.capture_rect(bounds, false).unwrap())),
        options: Default::default(),
        enable_annotations: true,
        windows: source.windows(),
        screens: source.screens(),
        cursor: None,
        last_region: IRect::default(),
        pointer: (300, 160),
    };
    RegionSession::new(request)
}

pub fn registered_hotkeys(failed: bool) -> HotkeyManager {
    let backend = FakeBackend {
        taken: if failed {
            vec![HotkeyInfo::new(Keys::CONTROL | Keys::SHIFT | 49)]
        } else {
            Vec::new()
        },
        ..FakeBackend::default()
    };
    let mut manager = HotkeyManager::new(Box::new(backend), default_hotkeys(), false);
    manager.register_all();
    manager
}

/// Set a screen up; the window size it is drawn at, when it is not the main window.
pub fn prepare(screen: &str, state: &mut AppState) -> Option<Vec2> {
    if screen == "tab-create" {
        state.captures.hotkeys = registered_hotkeys(false);
        return None;
    }
    let settings = state.default_task_settings();
    let size = match screen {
        "tab-create-problem" | "capture-menu" => {
            state.tab = crate::state::Tab::Create;
            state.captures.hotkeys = registered_hotkeys(screen == "tab-create-problem");
            state.captures.source = Arc::new(qa_desktop());
            return Some(crate::app::MAIN_SIZE);
        }
        "region-capture"
        | "region-capture-hover"
        | "region-capture-light"
        | "region-capture-annotate" => {
            let kind = if screen == "region-capture-light" {
                RegionKind::Light
            } else {
                RegionKind::Default
            };
            let mut session = session(kind);
            session.keyboard_enabled = true;
            match screen {
                "region-capture" => {
                    session.pointer_pressed(Button::Left, Point::new(560.0, 330.0), 1);
                    session.pointer_moved(Point::new(800.0, 470.0));
                    // Without quick crop the selection stays, with its handles.
                    session.request.options.quick_crop = false;
                    session.pointer_released(Button::Left, Point::new(800.0, 470.0));
                    session.pointer_moved(Point::new(640.0, 380.0));
                }
                "region-capture-hover" => session.pointer_moved(Point::new(250.0, 160.0)),
                "region-capture-annotate" => session.pointer_moved(Point::new(250.0, 160.0)),
                _ => {
                    session.pointer_pressed(Button::Left, Point::new(200.0, 140.0), 1);
                    session.pointer_moved(Point::new(520.0, 380.0));
                }
            }
            let mut overlay = RegionOverlay::new(session, OverlayPurpose::Capture, settings, 0.0);
            overlay.enable_workspace(state.editor_options());
            if screen == "region-capture-annotate"
                && let Some(w) = overlay.workspace.as_mut()
            {
                // An arrow and a rectangle drawn with the editor's tools, the rectangle tool
                // still in hand (its options under the toolbar).
                use modlight_editor::editor::input as ei;
                use modlight_editor::geom::Point as EP;
                let m = ei::Modifiers::NONE;
                let e = &mut w.editor;
                for (tool, from, to) in [
                    (
                        modlight_editor::model::EditorTool::Arrow,
                        (520.0, 520.0),
                        (700.0, 420.0),
                    ),
                    (
                        modlight_editor::model::EditorTool::Rectangle,
                        (640.0, 300.0),
                        (880.0, 440.0),
                    ),
                ] {
                    e.select_tool(tool);
                    ei::pointer_pressed(e, EP::new(from.0, from.1), ei::Button::Left, m, 1);
                    ei::pointer_moved(e, EP::new(to.0, to.1), false, m);
                    ei::pointer_released(e, EP::new(to.0, to.1), ei::Button::Left, m);
                }
                overlay.session.region_tool_active = false;
            }
            state.captures.overlay = Some(overlay);
            vec2(960.0, 600.0)
        }
        "recording-toolbar" | "recording-toolbar-compact" => {
            let control = RecorderControl::new(IRect::new(100, 100, 640, 360), 0.0);
            let compact = screen == "recording-toolbar-compact";
            if !compact {
                control.change_state(RecordState::AfterStart);
                control.change_state(RecordState::AfterRecordingStart);
            }
            state.captures.recording = Some(crate::recording_ui::Recording::for_qa(
                control, settings, compact,
            ));
            let recording = state.captures.recording.as_ref().unwrap();
            recording.toolbar_rect().size()
        }
        "after-capture" | "after-capture-recording" => {
            let file = (screen == "after-capture-recording").then(|| {
                std::path::PathBuf::from(crate::qa::fixture_path(
                    "/Users/alex/Pictures/Modlight/2026-10/Notes_k3P9xQ2mWz.mp4",
                ))
            });
            let metadata = if file.is_some() {
                TaskMetadata::default()
            } else {
                let source = qa_desktop();
                TaskMetadata {
                    image: Some(Arc::new(
                        source
                            .capture_rect(IRect::new(90, 70, 420, 300), false)
                            .unwrap(),
                    )),
                    window_title: Some("Notes".into()),
                    process_name: Some("Notes".into()),
                }
            };
            let window =
                AfterCapture::new(1, &settings, &metadata, file.as_deref(), &state.uploads.ctx);
            state.captures.after_capture.push_back((
                window,
                crate::capture_flow::PendingImage {
                    metadata,
                    settings,
                    file,
                    skip_quick_menu: true,
                },
            ));
            vec2(
                crate::dialogs::after_capture::WIDTH,
                crate::dialogs::after_capture::HEIGHT,
            )
        }
        "auto-capture" => {
            state
                .config
                .set("AutoCaptureRegion", serde_json::json!("90, 70, 420, 300"));
            state
                .config
                .set("AutoCaptureRepeatTime", serde_json::json!(15.0));
            let auto = &mut state.captures.auto_capture;
            auto.window_open = true;
            auto.running = true;
            auto.delay = 15.0;
            auto.next_at = 6.2;
            auto.count = 4;
            vec2(
                crate::dialogs::auto_capture::WIDTH,
                crate::dialogs::auto_capture::HEIGHT,
            )
        }
        "quick-task-menu" => {
            let menu = crate::dialogs::quick_task_menu::QuickTaskMenu::new(&state.config);
            state.captures.quick_menu = Some((
                menu,
                crate::capture_flow::PendingImage {
                    metadata: TaskMetadata::default(),
                    settings,
                    file: None,
                    skip_quick_menu: false,
                },
            ));
            vec2(320.0, 420.0)
        }
        "quick-task-editor" => {
            state.open_quick_task_menu_editor();
            vec2(
                crate::dialogs::quick_task_editor::WIDTH,
                crate::dialogs::quick_task_editor::HEIGHT,
            )
        }
        "scrolling-capture" => {
            let mut window = crate::scrolling::ScrollingWindow::new(settings);
            let page = RgbaImage::from_fn(420, 900, |x, y| {
                let line = (y % 22) == 10 && x > 24 && x < 380 - (y % 7) * 30;
                if line {
                    Rgba([70, 70, 80, 255])
                } else {
                    Rgba([248, 248, 250, 255])
                }
            });
            window.result = Some(Arc::new(page));
            window.status = Some(modlight_core::capture::scrolling::ScrollingStatus::Successful);
            window.status_text = modlight_core::l10n::t(
                modlight_core::l10n::capture::ScrollingCaptureWindow_Capture_successful,
            )
            .to_string();
            state.scrolling = Some(window);
            vec2(crate::scrolling::WIDTH, crate::scrolling::HEIGHT)
        }
        _ => return None,
    };
    Some(size)
}

pub fn is_dialog(screen: &str) -> bool {
    SCREENS.contains(&screen) && !matches!(screen, "tab-create-problem" | "capture-menu")
}

/// Draw a dialog screen; `Some(height)` when it asks for its own height.
pub fn draw(screen: &str, ui: &mut Ui, state: &mut AppState) -> Option<Option<f32>> {
    if !is_dialog(screen) {
        return None;
    }
    let colors = state.colors.clone();
    let now = state.time;
    match screen {
        "region-capture"
        | "region-capture-hover"
        | "region-capture-light"
        | "region-capture-annotate" => {
            if let Some(overlay) = state.captures.overlay.as_mut() {
                let screen_info = overlay.screens()[0].clone();
                overlay.ui(ui, &colors, &screen_info);
                overlay.toolbar(ui, &colors, &screen_info, true);
            }
        }
        "recording-toolbar" | "recording-toolbar-compact" => {
            if let Some(recording) = state.captures.recording.as_mut() {
                ui.painter()
                    .rect_filled(ui.max_rect(), 0.0, colors.background_main());
                let _ = recording.toolbar_ui(ui, &colors);
            }
        }
        "after-capture" | "after-capture-recording" => {
            if let Some((window, _)) = state.captures.after_capture.front_mut() {
                let _ = window.ui(ui, &colors);
            }
        }
        "auto-capture" => {
            let view = state.auto_capture_view();
            let _ = crate::dialogs::auto_capture::ui(
                ui,
                &colors,
                &state.captures.auto_capture,
                &view,
                now,
            );
        }
        "quick-task-menu" => {
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, colors.background_main());
            if let Some((menu, _)) = state.captures.quick_menu.as_ref() {
                let _ = menu.ui(ui, &colors);
            }
        }
        "quick-task-editor" => {
            if let Some(editor) = state.captures.quick_task_editor.as_mut() {
                let _ = editor.ui(ui, &colors);
            }
        }
        "scrolling-capture" => {
            if let Some(window) = state.scrolling.as_mut() {
                let _ = window.ui(ui, &colors);
            }
        }
        _ => {}
    }
    Some(None)
}

/// The capture menu, drawn where it opens (popups do not open headlessly).
pub fn draw_over_main(screen: &str, ctx: &egui::Context, state: &AppState) {
    if screen != "capture-menu" {
        return;
    }
    let entries = crate::capture_flow::capture_menu(state, false);
    let at = egui::pos2(18.0, 150.0);
    egui::Area::new(egui::Id::new("qa-capture-menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(at)
        .show(ctx, |ui| {
            crate::menus::panel(ui, &state.colors, at, &entries);
        });
}
