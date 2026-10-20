//! QA fixtures: the editor in a known state for `--qa-shot` and the snapshot tests. A
//! made-up screenshot, annotations drawn through the real pointer path, and the
//! popups or dialogs a scene shows. Nothing here opens a window or reads the disk.

use image::{Rgba, RgbaImage};

use crate::editor::input::{self, Button, Modifiers};
use crate::editor::{Editor, EditorConfig, EmojiTarget};
use crate::geom::Point;
use crate::model::EditorTool;
use crate::options::ImageEditorOptions;
use crate::ui::EditorView;

/// Every scene the QA tools can draw.
pub const SCENES: &[&str] = &[
    "editor-select",
    "editor-rectangle",
    "editor-arrow",
    "editor-text",
    "editor-text-editing",
    "editor-balloon",
    "editor-more-tools",
    "editor-step",
    "editor-effect-tools",
    "editor-spotlight",
    "editor-crop",
    "editor-cut-out",
    "editor-background",
    "editor-effect-browser",
    "editor-effect-dialog",
    "editor-options",
    "editor-context-menu",
    "editor-file-menu",
    "editor-color-picker",
    "editor-shadow-options",
    "editor-width-picker",
    "editor-arrow-style-picker",
    "editor-zoom-picker",
    "editor-confirm",
    "editor-new-image",
    "editor-insert-image",
    "editor-emoji-picker",
    "editor-start-screen",
    "editor-toolbar-customization",
    "editor-toast",
    "editor-zoomed",
];

/// The window size the scenes are drawn at.
pub const SIZE: (f32, f32) = (1280.0, 820.0);

/// A deterministic stand-in for a screenshot: a window with a title bar, a sidebar,
/// text lines and a few coloured cards on a gradient desktop.
pub fn sample_image(width: u32, height: u32) -> RgbaImage {
    let mut img = RgbaImage::new(width, height);
    for (x, y, p) in img.enumerate_pixels_mut() {
        let t = (x as f32 / width as f32 + y as f32 / height as f32) / 2.0;
        let r = (40.0 + 120.0 * t) as u8;
        let g = (90.0 + 60.0 * (1.0 - t)) as u8;
        let b = (160.0 + 70.0 * t) as u8;
        *p = Rgba([r, g, b, 255]);
    }
    let fill = |img: &mut RgbaImage, x0: u32, y0: u32, w: u32, h: u32, c: [u8; 4]| {
        for y in y0..(y0 + h).min(img.height()) {
            for x in x0..(x0 + w).min(img.width()) {
                img.put_pixel(x, y, Rgba(c));
            }
        }
    };
    let (wx, wy, ww, wh) = (
        60,
        50,
        width.saturating_sub(120),
        height.saturating_sub(100),
    );
    fill(&mut img, wx, wy, ww, wh, [246, 247, 249, 255]);
    fill(&mut img, wx, wy, ww, 34, [226, 229, 234, 255]);
    for (i, c) in [[255, 95, 87, 255], [254, 188, 46, 255], [40, 200, 64, 255]]
        .iter()
        .enumerate()
    {
        fill(&mut img, wx + 14 + i as u32 * 20, wy + 11, 12, 12, *c);
    }
    fill(
        &mut img,
        wx,
        wy + 34,
        180,
        wh.saturating_sub(34),
        [235, 238, 242, 255],
    );
    for i in 0..8 {
        fill(
            &mut img,
            wx + 20,
            wy + 60 + i * 32,
            120 - (i % 3) * 20,
            10,
            [180, 186, 196, 255],
        );
    }
    for i in 0..10 {
        let w = ww.saturating_sub(260).saturating_sub((i % 4) * 60);
        fill(
            &mut img,
            wx + 210,
            wy + 70 + i * 26,
            w,
            9,
            [200, 205, 214, 255],
        );
    }
    let cards = [
        [88, 87, 233, 255],
        [236, 72, 153, 255],
        [16, 185, 129, 255],
        [245, 158, 11, 255],
    ];
    for (i, c) in cards.iter().enumerate() {
        fill(&mut img, wx + 210 + i as u32 * 150, wy + 360, 130, 90, *c);
    }
    img
}

fn drag(e: &mut Editor, from: (f32, f32), to: (f32, f32), release: bool) {
    let m = Modifiers::NONE;
    input::pointer_pressed(e, Point::new(from.0, from.1), Button::Left, m, 1);
    for i in 1..=6 {
        let t = i as f32 / 6.0;
        input::pointer_moved(
            e,
            Point::new(from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t),
            false,
            m,
        );
    }
    if release {
        input::pointer_released(e, Point::new(to.0, to.1), Button::Left, m);
    }
}

fn click(e: &mut Editor, at: (f32, f32)) {
    let p = Point::new(at.0, at.1);
    input::pointer_pressed(e, p, Button::Left, Modifiers::NONE, 1);
    input::pointer_released(e, p, Button::Left, Modifiers::NONE);
}

fn type_text(e: &mut Editor, text: &str) {
    if let Some(t) = e.text_editing.as_mut() {
        t.text = text.to_string();
    }
}

/// A scene: the editor, its view, and the popup to open once the first frame has laid
/// the widgets out.
pub struct Scene {
    pub editor: Editor,
    pub view: EditorView,
    pub popup: Option<egui::Id>,
}

/// Builds a scene, or `None` for an unknown name.
pub fn scene(name: &str) -> Option<Scene> {
    let options = ImageEditorOptions {
        show_notifications: name == "editor-toast",
        ..ImageEditorOptions::default()
    };
    let mut e = Editor::new(options, EditorConfig::default());
    e.load_image(
        sample_image(960, 600),
        Some("/Users/qa/Pictures/Screenshot.png".into()),
    );
    e.zoom = 1.0;
    let mut popup = None;
    let option_popup = |key: &str| Some(egui::Id::new(("editor-option", key)).with("popup"));
    match name {
        "editor-select" | "editor-context-menu" => {
            e.select_tool(EditorTool::Rectangle);
            drag(&mut e, (300.0, 150.0), (520.0, 280.0), true);
            e.select_tool(EditorTool::Ellipse);
            drag(&mut e, (600.0, 160.0), (760.0, 260.0), true);
            e.select_tool(EditorTool::Arrow);
            drag(&mut e, (250.0, 420.0), (420.0, 330.0), true);
            e.select_tool(EditorTool::Step);
            click(&mut e, (560.0, 340.0));
            click(&mut e, (620.0, 340.0));
            e.select_tool(EditorTool::Select);
            click(&mut e, (520.0, 215.0));
            if name == "editor-context-menu" {
                input::pointer_pressed(
                    &mut e,
                    Point::new(400.0, 200.0),
                    Button::Right,
                    Modifiers::NONE,
                    1,
                );
                input::pointer_released(
                    &mut e,
                    Point::new(400.0, 200.0),
                    Button::Right,
                    Modifiers::NONE,
                );
            }
        }
        "editor-rectangle"
        | "editor-color-picker"
        | "editor-shadow-options"
        | "editor-width-picker" => {
            e.select_tool(EditorTool::Rectangle);
            drag(&mut e, (300.0, 150.0), (560.0, 320.0), true);
            popup = match name {
                "editor-color-picker" => option_popup("stroke"),
                "editor-width-picker" => option_popup("width"),
                "editor-shadow-options" => Some(egui::Id::new("editor-shadow-options")),
                _ => None,
            };
        }
        "editor-arrow" | "editor-arrow-style-picker" => {
            e.select_tool(EditorTool::Arrow);
            drag(&mut e, (260.0, 420.0), (520.0, 220.0), true);
            e.select_tool(EditorTool::Line);
            drag(&mut e, (600.0, 420.0), (780.0, 300.0), true);
            e.select_tool(EditorTool::Arrow);
            if name == "editor-arrow-style-picker" {
                popup = option_popup("arrow");
            }
        }
        "editor-text" | "editor-text-editing" => {
            e.select_tool(EditorTool::Text);
            click(&mut e, (300.0, 200.0));
            type_text(&mut e, "Modlight");
            input::commit_text_edit(&mut e);
            if name == "editor-text-editing" {
                click(&mut e, (320.0, 330.0));
                type_text(&mut e, "Typing here");
            }
        }
        "editor-balloon" => {
            e.select_tool(EditorTool::SpeechBalloon);
            drag(&mut e, (300.0, 140.0), (560.0, 260.0), true);
            type_text(&mut e, "Look at this!");
            input::commit_text_edit(&mut e);
        }
        "editor-more-tools" => {
            // Freehand, a smart eraser, an emoji and an inserted picture.
            e.select_tool(EditorTool::Freehand);
            let m = Modifiers::NONE;
            input::pointer_pressed(&mut e, Point::new(260.0, 200.0), Button::Left, m, 1);
            for i in 1..=24 {
                let t = i as f32 / 24.0;
                input::pointer_moved(
                    &mut e,
                    Point::new(260.0 + 220.0 * t, 200.0 + (t * 9.0).sin() * 40.0),
                    false,
                    m,
                );
            }
            input::pointer_released(&mut e, Point::new(480.0, 200.0), Button::Left, m);
            e.select_tool(EditorTool::SmartEraser);
            drag(&mut e, (560.0, 230.0), (760.0, 280.0), true);
            e.insert_emoji("1F389", "party popper", Some(Point::new(700.0, 420.0)));
            e.options.show_insert_image_dialog = false;
            e.insert_external_image(sample_image(240, 150), None);
            e.select_tool(EditorTool::Freehand);
        }
        "editor-step" => {
            e.select_tool(EditorTool::Step);
            for (i, x) in [300.0, 380.0, 460.0].iter().enumerate() {
                click(&mut e, (*x, 200.0 + i as f32 * 30.0));
            }
        }
        "editor-effect-tools" => {
            e.select_tool(EditorTool::Blur);
            drag(&mut e, (280.0, 120.0), (460.0, 220.0), true);
            e.select_tool(EditorTool::Pixelate);
            drag(&mut e, (500.0, 120.0), (680.0, 220.0), true);
            e.select_tool(EditorTool::Highlight);
            drag(&mut e, (280.0, 260.0), (520.0, 300.0), true);
            e.select_tool(EditorTool::Magnify);
            drag(&mut e, (560.0, 380.0), (700.0, 500.0), true);
            e.select_tool(EditorTool::Blur);
        }
        "editor-spotlight" => {
            e.select_tool(EditorTool::Spotlight);
            drag(&mut e, (420.0, 380.0), (700.0, 480.0), true);
        }
        "editor-crop" => {
            e.options.quick_crop = false;
            e.select_tool(EditorTool::Crop);
        }
        "editor-cut-out" => {
            e.select_tool(EditorTool::CutOut);
            drag(&mut e, (300.0, 100.0), (460.0, 400.0), false);
        }
        "editor-background" => {
            e.set_settings_panel_open(true);
        }
        "editor-effect-browser" => {
            e.toggle_effects_panel();
        }
        "editor-effect-dialog" => {
            e.run_effect("grayscale");
        }
        "editor-options" => {
            e.toggle_options_panel();
        }
        "editor-file-menu" => {
            e.input.file_menu_requested = true;
        }
        "editor-zoom-picker" => {
            popup = Some(egui::Id::new("editor-zoom").with("popup"));
        }
        "editor-confirm" => {
            e.is_dirty = true;
            e.request_close(false);
        }
        "editor-new-image" => e.modal_new_image(),
        "editor-insert-image" => {
            e.modal = Some(crate::editor::Modal::InsertImage {
                width: 320,
                height: 200,
            });
        }
        "editor-emoji-picker" => e.show_emoji_picker(EmojiTarget::Insert),
        "editor-start-screen" => {
            e.options.recent_image_files = vec![
                "/Users/qa/Pictures/Screenshot 2026-10-01 at 10.15.00.png".into(),
                "/Users/qa/Desktop/diagram.png".into(),
            ];
            e.show_start_screen();
        }
        "editor-toolbar-customization" => e.open_toolbar_customization(),
        "editor-toast" => {
            e.image_cropped_notification();
        }
        "editor-zoomed" => {
            e.select_tool(EditorTool::Rectangle);
            drag(&mut e, (300.0, 150.0), (420.0, 230.0), true);
            e.set_zoom(2.0);
        }
        _ => return None,
    }
    // Keep the tool the scene ended on; drop requests a fixture made (copy on change).
    let _ = e.take_requests();
    Some(Scene {
        editor: e,
        view: EditorView::new(),
        popup,
    })
}

/// One of every annotation on a test picture, exported (`GetSnapshot`): the renderer's
/// golden picture and the `gallery` example.
pub fn gallery() -> RgbaImage {
    use std::sync::Arc;

    use crate::annot_effects::{EffectCache, update_effect};
    use crate::frame::{self, BackgroundState};
    use crate::model::*;

    let source = Arc::new(RgbaImage::from_fn(900, 600, |x, y| {
        let checker = ((x / 20) + (y / 20)) % 2 == 0;
        if checker {
            Rgba([230, 236, 245, 255])
        } else {
            Rgba([(x / 4) as u8, (y / 3) as u8, 160, 255])
        }
    }));
    let mut list = Vec::new();
    let mut rect = Annotation::rectangle();
    rect.start = Point::new(20.0, 20.0);
    rect.end = Point::new(180.0, 100.0);
    rect.shadow.enabled = true;
    if let Kind::Rectangle { corner_radius, .. } = &mut rect.kind {
        *corner_radius = 12;
    }
    list.push(rect);
    let mut dashed = Annotation::ellipse();
    dashed.start = Point::new(200.0, 20.0);
    dashed.end = Point::new(360.0, 100.0);
    dashed.set_border_style(BorderStyle::Dash);
    dashed.fill_color = "#4022C55E".into();
    list.push(dashed);
    for (i, style) in ArrowStyle::PICKER_ORDER.iter().enumerate() {
        let mut arrow = Annotation::arrow();
        arrow.start = Point::new(390.0, 30.0 + i as f32 * 22.0);
        arrow.end = Point::new(560.0, 30.0 + i as f32 * 22.0);
        set_arrow_style(&mut arrow, *style);
        list.push(arrow);
    }
    let mut curved = Annotation::arrow();
    curved.start = Point::new(600.0, 120.0);
    curved.end = Point::new(860.0, 120.0);
    set_curve_point(&mut curved, Point::new(730.0, 40.0));
    list.push(curved);
    let mut line = Annotation::line();
    line.start = Point::new(20.0, 140.0);
    line.end = Point::new(360.0, 160.0);
    line.set_border_style(BorderStyle::DashDot);
    list.push(line);
    let mut pen = Annotation::freehand();
    if let Some(points) = pen.freehand_points_mut() {
        for i in 0..60 {
            let t = i as f32 / 10.0;
            points.push(Point::new(
                20.0 + i as f32 * 5.0,
                200.0 + (t * 2.0).sin() * 20.0,
            ));
        }
    }
    list.push(pen);
    let mut text = Annotation::text();
    text.start = Point::new(380.0, 160.0);
    text.end = Point::new(700.0, 260.0);
    text.stroke_color = "#FFF23C3C".into();
    text.stroke_width = 8.0;
    if let Some(t) = text.text_data_mut() {
        t.text = "Zażółć gęślą jaźń".into();
        t.text_color = "#FFFAFAFA".into();
        t.font_size = 40.0;
        t.bold = true;
    }
    list.push(text);
    let mut balloon = Annotation::speech_balloon();
    balloon.start = Point::new(40.0, 280.0);
    balloon.end = Point::new(300.0, 380.0);
    balloon.fill_color = "#FFF23C3C".into();
    balloon.stroke_color = "#FF000000".into();
    balloon.stroke_width = 4.0;
    set_balloon_tail(&mut balloon, Point::new(80.0, 440.0));
    if let Some(t) = balloon.text_data_mut() {
        t.text = "Speech balloon with wrapping text".into();
        t.text_color = "#FFFAFAFA".into();
        t.font_size = 24.0;
    }
    list.push(balloon);
    for n in 1..=3 {
        let mut step = Annotation::number();
        step.fill_color = "#FFF23C3C".into();
        step.stroke_color = "#00000000".into();
        step.start = Point::new(360.0 + n as f32 * 60.0, 330.0);
        if let Kind::Number(d) = &mut step.kind {
            d.number = n;
            d.font_size = 30.0;
            d.step_type = if n == 2 {
                StepType::UppercaseRoman
            } else {
                StepType::Numeric
            };
        }
        if n == 3 {
            set_number_tail(&mut step, Point::new(600.0, 400.0));
        }
        list.push(step);
    }
    for (i, kind) in [
        EffectKind::Blur,
        EffectKind::Pixelate,
        EffectKind::Highlight,
        EffectKind::Magnify,
    ]
    .into_iter()
    .enumerate()
    {
        let mut effect = Annotation::effect(kind);
        effect.start = Point::new(20.0 + i as f32 * 140.0, 460.0);
        effect.end = Point::new(140.0 + i as f32 * 140.0, 580.0);
        if let Some(d) = effect.effect_data_mut() {
            d.amount = match kind {
                EffectKind::Magnify => 2.0,
                EffectKind::Pixelate => 20.0,
                _ => 10.0,
            };
            d.is_ellipse = kind == EffectKind::Magnify;
        }
        list.push(effect);
    }
    let mut spot = Annotation::spotlight();
    spot.start = Point::new(600.0, 440.0);
    spot.end = Point::new(880.0, 590.0);
    if let Kind::Spotlight(s) = &mut spot.kind {
        s.is_ellipse = true;
        s.darken_opacity = 120;
        s.canvas_size = (900.0, 600.0);
    }
    list.push(spot);
    let mut emoji = Annotation::emoji("1f600", "grinning face");
    emoji.start = Point::new(740.0, 200.0);
    emoji.end = Point::new(860.0, 320.0);
    list.push(emoji);
    let mut cache = EffectCache::default();
    for a in &mut list {
        update_effect(a, &source, 1, &mut cache);
    }
    frame::snapshot(&BackgroundState::default(), &source, &list, None)
}
