//! Scripted interaction through the egui view (PORT.md section 4): pointer and key
//! events go through the harness into `EditorView`, the canvas maps them to image
//! pixels and the controllers act. Each test checks the document and compares the
//! exported picture (`GetSnapshot`) with a golden PNG under `tests/snapshots/export/`.
//!
//! `UPDATE_SNAPSHOTS=1 cargo test -p modlight-editor --test interaction` rewrites the
//! goldens that differ. Nothing here opens a window.

use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use modlight_editor::editor::{Editor, EditorConfig};
use modlight_editor::geom::Point;
use modlight_editor::model::{EditorTool, Kind};
use modlight_editor::options::ImageEditorOptions;
use modlight_editor::ui::{Colors, EditorView};

struct App {
    editor: Editor,
    view: EditorView,
    colors: Colors,
    frames: u32,
}

fn harness() -> Harness<'static, App> {
    harness_on(Harness::builder())
}

fn harness_on(builder: egui_kittest::HarnessBuilder<App>) -> Harness<'static, App> {
    modlight_core::l10n::set_thread_language(Some(modlight_core::l10n::Language::English));
    modlight_editor::text::set_system_fonts_enabled(false);
    let options = ImageEditorOptions {
        show_notifications: false,
        quick_crop: false,
        ..ImageEditorOptions::default()
    };
    let mut editor = Editor::new(options, EditorConfig::default());
    editor.load_image(modlight_editor::qa::sample_image(640, 400), None);
    let app = App {
        editor,
        view: EditorView::new(),
        colors: Colors::default_skin(true),
        frames: 0,
    };
    let mut h = builder
        .with_size(egui::vec2(1200.0, 800.0))
        .with_pixels_per_point(1.0)
        .with_max_steps(8)
        .build_ui_state(
            |ui, app: &mut App| {
                if app.frames == 0 {
                    modlight_editor::ui::style::install(ui.ctx(), &app.colors);
                } else {
                    let rect = ui.ctx().content_rect();
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                    app.view.show(&mut child, &mut app.editor, &app.colors);
                }
                app.frames += 1;
            },
            app,
        );
    h.run_steps(3);
    h
}

/// The screen point of an image pixel.
fn at(h: &Harness<'_, App>, x: f32, y: f32) -> Pos2 {
    h.state()
        .view
        .canvas
        .geometry
        .expect("the canvas was laid out")
        .to_screen(Point::new(x, y))
}

fn move_to(h: &mut Harness<'_, App>, p: Pos2) {
    h.event(Event::PointerMoved(p));
    h.step();
}

fn button(h: &mut Harness<'_, App>, p: Pos2, b: PointerButton, pressed: bool, m: Modifiers) {
    h.event(Event::PointerButton {
        pos: p,
        button: b,
        pressed,
        modifiers: m,
    });
    h.step();
}

fn drag(h: &mut Harness<'_, App>, from: Pos2, to: Pos2) {
    move_to(h, from);
    button(h, from, PointerButton::Primary, true, Modifiers::NONE);
    for i in 1..=5 {
        let t = i as f32 / 5.0;
        move_to(h, from + (to - from) * t);
    }
    button(h, to, PointerButton::Primary, false, Modifiers::NONE);
    h.run_steps(2);
}

fn click(h: &mut Harness<'_, App>, p: Pos2) {
    move_to(h, p);
    button(h, p, PointerButton::Primary, true, Modifiers::NONE);
    button(h, p, PointerButton::Primary, false, Modifiers::NONE);
    h.run_steps(2);
}

fn key(h: &mut Harness<'_, App>, k: egui::Key, m: Modifiers) {
    h.key_press_modifiers(m, k);
    h.run_steps(2);
}

fn export(h: &mut Harness<'_, App>) -> image::RgbaImage {
    h.state_mut().editor.snapshot().expect("an image is open")
}

fn golden(image: &image::RgbaImage, name: &str) {
    let options = egui_kittest::SnapshotOptions::new()
        .threshold(0.6)
        .max_failed_pixels(40);
    egui_kittest::image_snapshot_options(image, format!("export/{name}"), &options);
}

#[test]
fn draw_select_move_resize_undo_redo() {
    let mut h = harness();
    key(&mut h, egui::Key::R, Modifiers::NONE);
    assert_eq!(h.state().editor.tools.active_tool, EditorTool::Rectangle);
    let (a, b) = (at(&h, 100.0, 80.0), at(&h, 260.0, 180.0));
    drag(&mut h, a, b);
    {
        let e = &h.state().editor;
        assert_eq!(e.core.annotations().len(), 1);
        let r = e.core.annotations()[0].bounds();
        assert!(
            (r.left - 100.0).abs() < 1.0 && (r.top - 80.0).abs() < 1.0,
            "{r:?}"
        );
        assert!(
            (r.right - 260.0).abs() < 1.0 && (r.bottom - 180.0).abs() < 1.0,
            "{r:?}"
        );
    }
    // Select, then move it by dragging its border.
    key(&mut h, egui::Key::V, Modifiers::NONE);
    let (from, to) = (at(&h, 100.0, 100.0), at(&h, 140.0, 140.0));
    click(&mut h, from);
    assert!(h.state().editor.has_selection());
    drag(&mut h, from, to);
    let moved = h.state().editor.core.annotations()[0].bounds();
    assert!(
        (moved.left - 140.0).abs() < 1.0 && (moved.top - 120.0).abs() < 1.0,
        "{moved:?}"
    );
    // Resize from the bottom-right handle.
    let (from, to) = (at(&h, 300.0, 220.0), at(&h, 360.0, 260.0));
    drag(&mut h, from, to);
    let resized = h.state().editor.core.annotations()[0].bounds();
    assert!(
        (resized.right - 360.0).abs() < 1.5 && (resized.bottom - 260.0).abs() < 1.5,
        "{resized:?}"
    );
    golden(&export(&mut h), "rectangle-moved-resized");
    // Undo removes the rectangle (moves are not history steps), redo brings it back.
    key(&mut h, egui::Key::Z, Modifiers::COMMAND);
    assert!(h.state().editor.core.annotations().is_empty());
    key(&mut h, egui::Key::Y, Modifiers::COMMAND);
    assert_eq!(h.state().editor.core.annotations().len(), 1);
    // Backspace deletes the selection.
    let p = at(&h, 140.0, 140.0);
    click(&mut h, p);
    key(&mut h, egui::Key::Backspace, Modifiers::NONE);
    assert!(h.state().editor.core.annotations().is_empty());
}

#[test]
fn arrow_and_text_through_the_toolbar_and_the_text_box() {
    let mut h = harness();
    key(&mut h, egui::Key::A, Modifiers::NONE);
    assert_eq!(h.state().editor.tools.active_tool, EditorTool::Arrow);
    let (a, b) = (at(&h, 80.0, 300.0), at(&h, 300.0, 200.0));
    drag(&mut h, a, b);
    key(&mut h, egui::Key::T, Modifiers::NONE);
    let p = at(&h, 340.0, 120.0);
    click(&mut h, p);
    assert!(
        h.state().editor.text_editing.is_some(),
        "the text box opened"
    );
    h.event(Event::Text("Hello".into()));
    h.run_steps(2);
    key(&mut h, egui::Key::Enter, Modifiers::NONE);
    let e = &h.state().editor;
    assert!(e.text_editing.is_none());
    let text = e
        .core
        .annotations()
        .iter()
        .find_map(|a| match &a.kind {
            Kind::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .expect("a text annotation");
    assert_eq!(text, "Hello");
    golden(&export(&mut h), "arrow-and-text");
}

#[test]
fn crop_with_the_handles_and_the_crop_button() {
    let mut h = harness();
    key(&mut h, egui::Key::C, Modifiers::NONE);
    assert_eq!(h.state().editor.tools.active_tool, EditorTool::Crop);
    assert!(h.state().editor.input.crop.active);
    let (from, to) = (at(&h, 640.0, 400.0), at(&h, 400.0, 300.0));
    drag(&mut h, from, to);
    let rect = h.state().editor.input.crop.rect.expect("a crop rectangle");
    assert!(
        (rect.right - 400.0).abs() < 1.5 && (rect.bottom - 300.0).abs() < 1.5,
        "{rect:?}"
    );
    key(&mut h, egui::Key::Enter, Modifiers::NONE);
    assert_eq!(h.state().editor.image_size(), (400, 300));
    golden(&export(&mut h), "cropped");
}

/// Cmd+V seen by the host's key monitor while one of the editor's text fields has the
/// keyboard (here the effect browser's search box) is that field's paste: upstream's
/// `OnKeyDown` skips every shortcut while a `TextBox` is focused. It used to paste the
/// copied annotation, or the clipboard's picture, onto the canvas as well.
#[test]
fn the_hosts_paste_leaves_a_focused_text_field_alone() {
    use modlight_editor::editor::Request;
    let mut h = harness();
    h.state_mut().editor.toggle_effects_panel();
    h.run_steps(3);
    assert!(
        h.ctx.egui_wants_keyboard_input(),
        "the search box has the focus"
    );
    {
        let app = h.state_mut();
        let _ = app.editor.take_requests();
        app.view.paste_shortcut(&mut app.editor);
        assert!(
            !app.editor.take_requests().contains(&Request::Paste),
            "the paste went to the canvas too"
        );
    }
    // Without a focused field the same key pastes onto the canvas.
    h.state_mut().editor.toggle_effects_panel();
    h.run_steps(3);
    assert!(!h.ctx.egui_wants_keyboard_input());
    let app = h.state_mut();
    app.view.paste_shortcut(&mut app.editor);
    assert!(app.editor.take_requests().contains(&Request::Paste));
}

#[test]
fn cmd_wheel_zooms_one_level_and_the_context_menu_deletes() {
    let mut h = harness();
    let p = at(&h, 320.0, 200.0);
    move_to(&mut h, p);
    h.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, 1.0),
        modifiers: Modifiers::COMMAND,
        phase: egui::TouchPhase::Move,
    });
    h.run_steps(2);
    assert!(
        (h.state().editor.zoom - 1.25).abs() < 1e-6,
        "{}",
        h.state().editor.zoom
    );
    // The pixel under the pointer stays under it.
    let after = h.state().view.canvas.geometry.unwrap().to_image(p);
    assert!(
        (after.x - 320.0).abs() < 1.5 && (after.y - 200.0).abs() < 1.5,
        "{after:?}"
    );

    key(&mut h, egui::Key::E, Modifiers::NONE);
    let (a, b) = (at(&h, 100.0, 100.0), at(&h, 200.0, 180.0));
    drag(&mut h, a, b);
    assert_eq!(h.state().editor.core.annotations().len(), 1);
    let inside = at(&h, 150.0, 140.0);
    move_to(&mut h, inside);
    button(
        &mut h,
        inside,
        PointerButton::Secondary,
        true,
        Modifiers::NONE,
    );
    button(
        &mut h,
        inside,
        PointerButton::Secondary,
        false,
        Modifiers::NONE,
    );
    h.run_steps(2);
    h.get_by_label("Delete").click();
    h.run_steps(2);
    assert!(h.state().editor.core.annotations().is_empty());
}

/// The text box over a text annotation is an egui text box at the annotation's size
/// times the zoom. egui asserts that every glyph fits its font atlas (as wide as the
/// largest texture, here 2048): a large text (sizes grow with Resize image, or come
/// from the options) edited zoomed in ended the app.
#[test]
fn editing_a_huge_text_zoomed_in_does_not_crash() {
    use modlight_editor::model::Annotation;
    let mut h = harness();
    {
        let e = &mut h.state_mut().editor;
        let mut text = Annotation::text();
        text.start = Point::new(10.0, 10.0);
        text.end = Point::new(600.0, 390.0);
        if let Some(t) = text.text_data_mut() {
            t.text = "WM".into();
            t.font_size = 900.0;
        }
        let id = text.id;
        e.core.add_annotation(text);
        e.set_zoom(4.0);
        modlight_editor::editor::input::show_text_editor(e, id);
    }
    h.run_steps(3);
    assert!(h.state().editor.text_editing.is_some());
}

/// An effect may hand back an empty picture (the effect library's crop does for an area
/// outside the image, its X box goes to 100000). The live preview showed it as the
/// picture: the canvas then made a texture of nothing. The preview keeps the picture.
#[test]
fn an_empty_effect_preview_keeps_the_picture() {
    use modlight_editor::effects::{
        Category, EffectContext, ImageEffect, ParamDef, ParamValues, register_effects,
    };
    struct Empty;
    impl ImageEffect for Empty {
        fn id(&self) -> &str {
            "qa_empty_result"
        }
        fn name(&self) -> &str {
            "Empty result"
        }
        fn category(&self) -> Category {
            Category::Filters
        }
        fn icon(&self) -> char {
            'x'
        }
        fn parameters(&self) -> Vec<ParamDef> {
            vec![ParamDef::checkbox("on", "On", true)]
        }
        fn apply(
            &self,
            _: &image::RgbaImage,
            _: &ParamValues,
            _: &EffectContext,
        ) -> Result<image::RgbaImage, String> {
            Ok(image::RgbaImage::new(0, 0))
        }
    }
    register_effects([std::sync::Arc::new(Empty) as std::sync::Arc<dyn ImageEffect>]);
    let mut h = harness_on(Harness::builder().wgpu());
    h.state_mut().editor.run_effect("qa_empty_result");
    h.run_steps(3);
    // The GPU gets the canvas's textures (an empty one is a wgpu validation error).
    h.render().expect("the frame renders");
    let shown = h.state().editor.display_image().unwrap();
    assert_eq!(shown.dimensions(), (640, 400));
    h.state_mut().editor.apply_effect_dialog();
    h.run_steps(2);
    assert_eq!(h.state().editor.image_size(), (640, 400));
}
