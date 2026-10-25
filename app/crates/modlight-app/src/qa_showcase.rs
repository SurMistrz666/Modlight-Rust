//! The pictures of the README (`examples/showcase.rs`, `scripts/showcase/make-media.sh`):
//! QA screens drawn large and in English, and short animations of the same screens. All
//! of it comes from the QA fixtures (the "Alex Rivera" account on files.example.com): no
//! server, no window, nobody's real account or files.

use std::path::Path;
use std::time::{Duration, Instant};

use chrono::Duration as Span;
use image::RgbaImage;
use modlight_core::capture::geometry::Point;
use modlight_core::capture::region::Button;
use modlight_core::l10n::{Arg, Language, format, s, t};
use modlight_core::tasks::info::{DataType, TaskJob, TaskStatus};
use modlight_core::tasks::settings::TaskSettings;
use modlight_core::tasks::worker::WorkerTask;
use modlight_core::upload::progress::ProgressManager;
use modlight_editor::editor::Editor;
use modlight_editor::editor::input as ei;
use modlight_editor::geom::Point as EditorPoint;
use modlight_editor::model::EditorTool;

use crate::qa::{self, Look};
use crate::state::AppState;

/// The screens the README shows, and whether each one is wanted in the light palette too.
pub const STILLS: &[(&str, bool)] = &[
    ("my-space", true),
    ("my-space-details", false),
    ("region-capture-annotate", false),
    ("tab-upload-busy", false),
    ("download-youtube", false),
    ("tab-tools", false),
    ("image-effects", false),
    ("toasts", false),
    ("history", false),
    ("capture-menu", false),
    ("recording-toolbar", false),
    ("color-picker", false),
    ("ocr", false),
    ("qr-code", false),
];

/// The animations, as `out/<name>/0000.png` and on.
pub const SCENES: &[&str] = &["capture", "annotate", "editor", "upload"];

/// The animations whose last frame is a still too (in both palettes): the editor with
/// its markup finished shows more than the editor's own QA screen.
pub const SCENE_STILLS: &[&str] = &["editor"];

fn look(dark: bool, scale: f32) -> Look {
    Look {
        dark,
        language: Language::English,
        scale,
        ..Look::default()
    }
}

fn save(image: &RgbaImage, path: &Path) -> Result<(), String> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    }
    image
        .save(path)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Every still into `out` as `<screen>-dark.png` (and `<screen>-light.png`), the last
/// frames of [`SCENE_STILLS`] as `<scene>-<palette>.png`, and `out/titles.tsv`: the title
/// each one's window has in its system title bar (the editor and the tool windows; the
/// main window draws its own band).
pub fn stills(out: &Path, scale: f32) -> Result<(), String> {
    let mut titles = String::new();
    for (screen, light) in STILLS {
        for dark in [true, false] {
            if !dark && !light {
                continue;
            }
            let image = qa::render(screen, &look(dark, scale))?;
            let palette = if dark { "dark" } else { "light" };
            save(&image, &out.join(format!("{screen}-{palette}.png")))?;
        }
        let mut title = None;
        qa::render_frames(
            screen,
            &look(true, 1.0),
            1,
            |state, _| title = window_title(state),
            |_, _| Ok(()),
        )?;
        if let Some(title) = title {
            titles.push_str(&format!("{screen}\t{title}\n"));
        }
    }
    for name in SCENE_STILLS {
        let mut title = None;
        for (dark, palette) in [(true, "dark"), (false, "light")] {
            let mut last = None;
            title = run(name, &look(dark, scale), |frame, frames, image| {
                if frame + 1 == frames {
                    last = Some(image);
                }
                Ok(())
            })?;
            let last = last.ok_or_else(|| format!("{name}: no frame"))?;
            save(&last, &out.join(format!("{name}-{palette}.png")))?;
        }
        if let Some(title) = title {
            titles.push_str(&format!("{name}\t{title}\n"));
        }
    }
    std::fs::write(out.join("titles.tsv"), titles).map_err(|e| e.to_string())
}

/// The title of the window a dialog screen shows, as the system draws it.
fn window_title(state: &AppState) -> Option<String> {
    if let Some(session) = state.windows.editors.sessions.last() {
        return Some(session.editor.window_title());
    }
    state
        .tools
        .windows
        .last()
        .map(|hosted| hosted.window.spec().title)
}

/// One animation (a name from [`SCENES`]) into `out/<name>/`, in the dark palette, with
/// `title.txt` when its window has a system title bar.
pub fn scene(name: &str, out: &Path, scale: f32) -> Result<(), String> {
    let folder = out.join(name);
    let title = run(name, &look(true, scale), |frame, _, image| {
        save(&image, &folder.join(format!("{frame:04}.png")))
    })?;
    match title {
        Some(title) => std::fs::write(folder.join("title.txt"), title).map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

/// Draws the animation `name`, handing `sink` each frame with its number and the count,
/// and returns the title of its window at the end (when the system draws one).
fn run(
    name: &str,
    look: &Look,
    mut sink: impl FnMut(usize, usize, RgbaImage) -> Result<(), String>,
) -> Result<Option<String>, String> {
    let (screen, frames, step): (&str, usize, fn(&mut AppState, usize)) = match name {
        "capture" => ("region-capture-hover", 76, capture),
        "annotate" => ("region-capture-hover", 66, annotate),
        "editor" => ("image-editor-annotate", 165, editor),
        "upload" => ("tab-upload-busy", 105, upload),
        _ => {
            return Err(format!(
                "unknown scene '{name}'; known: {}",
                SCENES.join(", ")
            ));
        }
    };
    let mut title = None;
    qa::render_frames(
        screen,
        look,
        frames,
        |state, frame| {
            step(state, frame);
            if frame + 1 == frames {
                title = window_title(state);
            }
        },
        |frame, image| sink(frame, frames, image),
    )?;
    Ok(title)
}

/// `0.0` before frame `from`, `1.0` from frame `to`, eased in and out in between.
fn along(frame: usize, from: usize, to: usize) -> f32 {
    if frame <= from {
        return 0.0;
    }
    if frame >= to {
        return 1.0;
    }
    let t = (frame - from) as f32 / (to - from) as f32;
    t * t * (3.0 - 2.0 * t)
}

fn between(a: (f32, f32), b: (f32, f32), t: f32) -> (f32, f32) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// The region overlay: the pointer crosses the desktop's windows, then drags a region
/// out and lets go, the selection keeping its handles.
fn capture(state: &mut AppState, frame: usize) {
    let Some(overlay) = state.captures.overlay.as_mut() else {
        return;
    };
    let session = &mut overlay.session;
    let (start, press, end, rest) = (
        (250.0, 160.0),
        (560.0, 330.0),
        (800.0, 470.0),
        (690.0, 405.0),
    );
    let at = |p: (f32, f32)| Point::new(f64::from(p.0), f64::from(p.1));
    match frame {
        0..=21 => session.pointer_moved(at(between(start, press, along(frame, 0, 21)))),
        22 => session.pointer_pressed(Button::Left, at(press), 1),
        23..=52 => session.pointer_moved(at(between(press, end, along(frame, 22, 52)))),
        53 => {
            // Without quick crop the selection stays, with its handles.
            session.request.options.quick_crop = false;
            session.pointer_released(Button::Left, at(end));
        }
        _ => session.pointer_moved(at(between(end, rest, along(frame, 53, 70)))),
    }
}

/// The same overlay with the editor's tools on it: an arrow, then a box, drawn over the
/// desktop before the region is chosen.
fn annotate(state: &mut AppState, frame: usize) {
    if frame == 0 {
        let options = state.editor_options();
        if let Some(overlay) = state.captures.overlay.as_mut() {
            overlay.enable_workspace(options);
        }
    }
    let Some(overlay) = state.captures.overlay.as_mut() else {
        return;
    };
    if let Some(workspace) = overlay.workspace.as_mut() {
        let e = &mut workspace.editor;
        stroke(
            e,
            frame,
            EditorTool::Arrow,
            6,
            24,
            (520.0, 520.0),
            (700.0, 420.0),
        );
        stroke(
            e,
            frame,
            EditorTool::Rectangle,
            30,
            50,
            (640.0, 300.0),
            (880.0, 440.0),
        );
    }
    if frame == 52 {
        overlay.session.region_tool_active = false;
    }
}

/// A capture marked up in the editor (task mode, after a capture): a box round the text,
/// a highlighted line, an arrow with a note, and numbered steps on the cards.
fn editor(state: &mut AppState, frame: usize) {
    let Some(session) = state.windows.editors.sessions.last_mut() else {
        return;
    };
    let e = &mut session.editor;
    if frame == 0 {
        // A capture of a Retina screen: the fixture's picture in twice the pixels, so
        // that the editor's 100% shows it at the fixture's size in points, and the tools
        // twice as thick, as they look on the fixture.
        let sample = modlight_editor::qa::sample_image(960, 600);
        let picture =
            image::imageops::resize(&sample, 1920, 1200, image::imageops::FilterType::Nearest);
        e.load_image(picture, None);
        e.zoom = 1.0;
        let o = &mut e.options;
        o.thickness *= 2;
        o.text_thickness *= 2;
        o.text_font_size *= 2.0;
        o.step_thickness *= 2;
        o.step_font_size *= 2.0;
    }
    let x2 = |(x, y): (f32, f32)| (x * 2.0, y * 2.0);
    stroke(
        e,
        frame,
        EditorTool::Rectangle,
        9,
        30,
        x2((258.0, 108.0)),
        x2((862.0, 214.0)),
    );
    stroke(
        e,
        frame,
        EditorTool::Highlight,
        37,
        52,
        x2((265.0, 243.0)),
        x2((795.0, 266.0)),
    );
    stroke(
        e,
        frame,
        EditorTool::Arrow,
        59,
        76,
        x2((800.0, 330.0)),
        x2((708.0, 228.0)),
    );
    let note = "Ship it!";
    match frame {
        80 => e.select_tool(EditorTool::Text),
        84 => click(e, x2((742.0, 336.0))),
        86..=101 => {
            let typed = ((frame - 86) / 2 + 1).min(note.len());
            if let Some(text) = e.text_editing.as_mut() {
                text.text = note[..typed].to_string();
            }
        }
        108 => ei::commit_text_edit(e),
        112 => e.select_tool(EditorTool::Step),
        116 => click(e, x2((335.0, 455.0))),
        124 => click(e, x2((485.0, 455.0))),
        132 => click(e, x2((635.0, 455.0))),
        140 => {
            e.select_tool(EditorTool::Select);
            ei::clear_selection(e);
        }
        _ => {}
    }
}

/// A stroke with `tool`: the tool picked a few frames before, the button pressed at
/// `a` on frame `from` and let go at `b` on frame `to`.
fn stroke(
    e: &mut Editor,
    frame: usize,
    tool: EditorTool,
    from: usize,
    to: usize,
    a: (f32, f32),
    b: (f32, f32),
) {
    let m = ei::Modifiers::NONE;
    let at = |p: (f32, f32)| EditorPoint::new(p.0, p.1);
    if frame + 4 == from {
        e.select_tool(tool);
    } else if frame == from {
        ei::pointer_pressed(e, at(a), ei::Button::Left, m, 1);
    } else if frame > from && frame < to {
        ei::pointer_moved(e, at(between(a, b, along(frame, from, to))), false, m);
    } else if frame == to {
        ei::pointer_moved(e, at(b), false, m);
        ei::pointer_released(e, at(b), ei::Button::Left, m);
    }
}

fn click(e: &mut Editor, p: (f32, f32)) {
    let p = EditorPoint::new(p.0, p.1);
    ei::pointer_pressed(e, p, ei::Button::Left, ei::Modifiers::NONE, 1);
    ei::pointer_released(e, p, ei::Button::Left, ei::Modifiers::NONE);
}

/// A transfer of the Upload tab: its name, its length, what had gone when the scene
/// starts, and its pace in bytes a second.
type Transfer = (&'static str, i64, i64, i64);

const VIDEO: Transfer = ("holiday-video.mov", 412_000_000, 268_000_000, 67_000_000);
const NOTES: Transfer = ("design-notes.pdf", 8_400_000, 1_100_000, 1_900_000);

/// The Upload tab with its two transfers moving on at their own pace; when the video is
/// up it leaves the list for the history and its notice comes up.
fn upload(state: &mut AppState, frame: usize) {
    let settings = state.default_task_settings();
    let seconds = frame as f64 / 30.0;
    let sent = |(_, length, at_start, rate): Transfer| {
        (at_start + (rate as f64 * seconds) as i64).min(length)
    };
    let video_done = sent(VIDEO) >= VIDEO.1;
    state.uploads.tasks.tasks.clear();
    if !video_done {
        let task = transfer(VIDEO, sent(VIDEO), &settings);
        state.uploads.tasks.tasks.push(task);
    }
    let task = transfer(NOTES, sent(NOTES), &settings);
    state.uploads.tasks.tasks.push(task);
    if video_done && !state.toasts.iter().any(|toast| toast.name == VIDEO.0) {
        let mut row = crate::qa_uploads::row(state, VIDEO.0, 0);
        row.link = "https://files.example.com/alex/q9".into();
        row.when = state.env.now() - Span::seconds(1);
        let link = row.link.clone();
        state.upload_tab.insert(row);
        state.toast_upload(
            &format(t(s::MainWindow_ToastUploadedTo), &[Arg::from("Recordings")]),
            VIDEO.0,
            &link,
            false,
        );
    }
}

/// A transfer the way the Upload tab sees one: `sent` of it gone at a steady pace since
/// it began, the last sample now.
fn transfer((name, length, _, rate): Transfer, sent: i64, settings: &TaskSettings) -> WorkerTask {
    let task = WorkerTask::data_upload(DataType::File, vec![0], name, settings.clone());
    let elapsed = Duration::from_secs_f64(sent as f64 / rate as f64);
    let now = Instant::now();
    let start = now.checked_sub(elapsed).unwrap_or(now);
    let mut progress = ProgressManager::starting_at(length, 0, start);
    let steps = 16;
    let mut gone = 0;
    for step in 1..=steps {
        let total = sent * step / steps;
        let at = start + elapsed.mul_f64(step as f64 / steps as f64);
        progress.update_progress_at(total - gone, at);
        gone = total;
    }
    task.with(|s| {
        s.info.job = TaskJob::FileUpload;
        s.status = TaskStatus::Working;
        s.info.status = t(s::UploadTask_DoUploadJob_Uploading).to_string();
        s.info.progress = Some(progress);
    });
    task
}
