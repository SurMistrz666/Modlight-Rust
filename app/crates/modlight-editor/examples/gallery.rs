//! Renders one of every annotation onto a test picture: `cargo run -p modlight-editor
//! --example gallery -- out.png`. A development aid for eyeballing the renderer.

use std::sync::Arc;

use image::{Rgba, RgbaImage};
use modlight_editor::annot_effects::{EffectCache, update_effect};
use modlight_editor::frame::{self, BackgroundState};
use modlight_editor::geom::Point;
use modlight_editor::model::*;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "gallery.png".into());
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
    let image = frame::snapshot(&BackgroundState::default(), &source, &list, None);
    image.save(&out).unwrap();
    println!("{out}");
}
