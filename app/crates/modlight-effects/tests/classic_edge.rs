//! Every classic effect (`ShareX.ImageEffectsLib`) with its properties pushed to the ends
//! the options panel allows (numbers up to the panel's limits, every enum member, empty
//! and one-stop gradients, odd fonts and texts) on 1x1, one-row, one-column, transparent
//! and odd-sized pictures. A preset runs after every capture with "Add image effects",
//! so a panic or an absurd result here is a broken screenshot.

use std::cell::RefCell;
use std::sync::Mutex;

use image::{Rgba, RgbaImage};
use modlight_effects::classic::model::{
    self, ClassicEffect, FontSpec, GdiColor, Gradient, Kind, PType, Padding, Preset, Stop, Value,
};
use modlight_effects::sk::C;

thread_local! {
    static LAST_PANIC: RefCell<Option<String>> = const { RefCell::new(None) };
    /// Set on the threads whose panics are recorded instead of printed.
    static CAPTURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn inputs() -> Vec<(&'static str, RgbaImage)> {
    let opaque = |w: u32, h: u32| {
        RgbaImage::from_fn(w, h, |x, y| {
            Rgba([
                (x * 37 + y * 11) as u8,
                (x * 7 + y * 53) as u8,
                (x * 101 + y * 3) as u8,
                255,
            ])
        })
    };
    vec![
        ("1x1", opaque(1, 1)),
        ("1x9", opaque(1, 9)),
        ("9x1", opaque(9, 1)),
        ("7x5 transparent", RgbaImage::new(7, 5)),
        ("33x17", opaque(33, 17)),
    ]
}

/// The values tried for one property.
fn ends(ty: PType) -> Vec<Value> {
    match ty {
        PType::Bool => vec![Value::Bool(false), Value::Bool(true)],
        PType::Int => [-1000, -1, 0, 1, 2, 3, 1000]
            .into_iter()
            .map(Value::Int)
            .collect(),
        PType::Float => [-1000.0, -1.0, 0.0, 0.5, 1.0, 1000.0]
            .into_iter()
            .map(Value::Float)
            .collect(),
        PType::Double => [-1000.0, 0.0, 1.0, 1000.0]
            .into_iter()
            .map(Value::Double)
            .collect(),
        PType::Byte => vec![Value::Byte(0), Value::Byte(255)],
        PType::Color => vec![
            Value::Color(GdiColor::argb(0, 0, 0, 0)),
            Value::Color(GdiColor::rgb(255, 255, 255)),
        ],
        PType::Point | PType::Size => [(-1000, -1000), (-1, 1), (0, 0), (1, 1), (1000, 1000)]
            .into_iter()
            .map(|(x, y)| {
                if ty == PType::Point {
                    Value::Point(x, y)
                } else {
                    Value::Size(x, y)
                }
            })
            .collect(),
        PType::Padding => [-1000, -1, 0, 1, 1000]
            .into_iter()
            .map(|v| Value::Padding(Padding::new(v, v, v, v)))
            .collect(),
        PType::Text(_) => vec![
            Value::Text(String::new()),
            Value::Text("Zażółć gęślą jaźń\nline two %y-%mo-%d $w $h".into()),
            Value::Text("/nonexistent/modlight/edge".into()),
        ],
        PType::Enum(_, members) => members.iter().map(|m| Value::Enum(m.to_string())).collect(),
        PType::Flags(_, _) => vec![Value::Flags(0), Value::Flags(model::ANCHOR_ALL)],
        PType::Font => vec![
            Value::Font(FontSpec::new("Arial", 0.1)),
            Value::Font(FontSpec::new("No such family", 200.0)),
        ],
        PType::Gradient => vec![
            Value::Gradient(Gradient {
                mode: "Vertical".into(),
                stops: Vec::new(),
            }),
            Value::Gradient(Gradient {
                mode: "BackwardDiagonal".into(),
                stops: vec![Stop {
                    color: GdiColor::rgb(10, 20, 30),
                    location: 50.0,
                }],
            }),
            Value::Gradient(Gradient::new(
                "Horizontal",
                &[(C::rgb(255, 0, 0), 100.0), (C::new(0, 0, 255, 0), 0.0)],
            )),
        ],
    }
}

/// The effects to try for one class: defaults, each property at each end, and all
/// numbers at once at the small and the large end.
fn variants(kind: Kind) -> Vec<(String, ClassicEffect)> {
    let mut out = vec![("defaults".to_string(), ClassicEffect::new(kind))];
    for def in model::props(kind) {
        for value in ends(def.ty) {
            let mut e = ClassicEffect::new(kind);
            e.set(def.name, value.clone());
            out.push((format!("{}={value:?}", def.name), e));
        }
    }
    for (label, n) in [("all -1000", -1000), ("all 0", 0), ("all 1000", 1000)] {
        let mut e = ClassicEffect::new(kind);
        for def in model::props(kind) {
            let v = match def.ty {
                PType::Int => Value::Int(n),
                PType::Float => Value::Float(n as f32),
                PType::Double => Value::Double(n as f64),
                _ => continue,
            };
            e.set(def.name, v);
        }
        out.push((label.to_string(), e));
    }
    out
}

#[test]
fn every_classic_effect_survives_edge_values() {
    // Other tests of this file run at the same time: only the worker threads' panics
    // are recorded, the rest go to the default hook.
    let default_hook = std::sync::Arc::new(std::panic::take_hook());
    let fallback = default_hook.clone();
    std::panic::set_hook(Box::new(move |info| {
        if CAPTURE.with(|c| c.get()) {
            let text = format!("{info}");
            LAST_PANIC.with(|l| *l.borrow_mut() = Some(text.replace('\n', " ")));
        } else {
            fallback(info);
        }
    }));
    let kinds: Vec<Kind> = Kind::all().collect();
    let inputs = inputs();
    let problems = Mutex::new(Vec::<String>::new());
    let next = Mutex::new(0usize);
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(12);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                CAPTURE.with(|c| c.set(true));
                loop {
                    let i = {
                        let mut n = next.lock().unwrap();
                        *n += 1;
                        *n - 1
                    };
                    let Some(kind) = kinds.get(i) else { break };
                    let mut seen = std::collections::HashSet::new();
                    for (label, effect) in variants(*kind) {
                        let preset = Preset {
                            name: String::new(),
                            effects: vec![effect],
                        };
                        for (name, input) in &inputs {
                            LAST_PANIC.with(|l| *l.borrow_mut() = None);
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    preset.apply(input)
                                }));
                            let what = match result {
                                Err(_) => {
                                    let panic = LAST_PANIC
                                        .with(|l| l.borrow_mut().take())
                                        .unwrap_or_default();
                                    seen.insert(panic.clone()).then(|| format!("PANIC {panic}"))
                                }
                                Ok(Some(out)) if out.width() == 0 || out.height() == 0 => {
                                    Some(format!("empty {}x{}", out.width(), out.height()))
                                }
                                Ok(_) => None,
                            };
                            if let Some(what) = what {
                                problems.lock().unwrap().push(format!(
                                    "{} [{label}] on {name}: {what}",
                                    kind.type_name()
                                ));
                            }
                        }
                    }
                }
            });
        }
    });
    let _ = std::panic::take_hook();
    if let Ok(hook) = std::sync::Arc::try_unwrap(default_hook) {
        std::panic::set_hook(hook);
    }
    let mut problems = problems.into_inner().unwrap();
    problems.sort();
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

/// The options panel takes numbers up to a million and a package can carry any `int`:
/// a margin, size or scale that asks for a bitmap GDI+ cannot make (more than 2 GiB of
/// pixels) fails upstream with "Parameter is not valid.", which the preview shows. The
/// port must refuse the same way before it allocates, not overflow its pixel indexes or
/// reserve tens of gigabytes.
#[test]
fn bitmaps_larger_than_gdi_allows_are_refused() {
    let picture = RgbaImage::from_pixel(32, 32, Rgba([10, 20, 30, 255]));
    let one = |kind: Kind, values: &[(&str, Value)]| {
        let mut e = ClassicEffect::new(kind);
        for (name, value) in values {
            e.set(name, value.clone());
        }
        Preset {
            name: String::new(),
            effects: vec![e],
        }
    };
    let n = 1_000_000;
    let cases = [
        one(
            Kind::Canvas,
            &[("Margin", Value::Padding(Padding::new(n, n, n, n)))],
        ),
        one(
            Kind::Resize,
            &[("Width", Value::Int(n)), ("Height", Value::Int(n))],
        ),
        one(Kind::Scale, &[("WidthPercentage", Value::Float(n as f32))]),
        one(
            Kind::Skew,
            &[
                ("Horizontally", Value::Int(n)),
                ("Vertically", Value::Int(n)),
            ],
        ),
        one(Kind::Shadow, &[("Size", Value::Int(n))]),
        one(Kind::Glow, &[("Size", Value::Int(n))]),
        one(Kind::DrawBorder, &[("Size", Value::Int(n))]),
        one(Kind::DrawCheckerboard, &[("Size", Value::Int(n))]),
        one(
            Kind::Pixelate,
            &[("Size", Value::Int(n)), ("BorderSize", Value::Int(1))],
        ),
        one(
            Kind::ForceProportions,
            &[("ProportionalWidth", Value::Int(n))],
        ),
        one(Kind::Reflection, &[("Offset", Value::Int(i32::MAX - 100))]),
    ];
    for preset in &cases {
        let kind = preset.effects[0].kind;
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| preset.try_apply(&picture)));
        match result {
            Ok(Err(message)) => assert_eq!(message, "Parameter is not valid."),
            Ok(Ok(out)) => panic!(
                "{}: made {:?}",
                kind.type_name(),
                out.map(|o| o.dimensions())
            ),
            Err(_) => panic!("{}: panicked", kind.type_name()),
        }
    }
    // Auto crop pads only a picture it cropped: a uniform one stays as it is.
    let auto_crop = one(Kind::AutoCrop, &[("Padding", Value::Int(n))]);
    assert_eq!(
        auto_crop.try_apply(&picture).unwrap().unwrap().dimensions(),
        (32, 32)
    );
    let framed = RgbaImage::from_fn(32, 32, |x, y| {
        if (4..28).contains(&x) && (4..28).contains(&y) {
            Rgba([200, 0, 0, 255])
        } else {
            Rgba([10, 20, 30, 255])
        }
    });
    assert_eq!(
        auto_crop.try_apply(&framed),
        Err("Parameter is not valid.".to_string())
    );
    // Sizes GDI+ makes still work.
    let fine = one(
        Kind::Canvas,
        &[("Margin", Value::Padding(Padding::new(3, 0, 3, 0)))],
    );
    assert_eq!(
        fine.try_apply(&picture).unwrap().unwrap().dimensions(),
        (38, 32)
    );
    // ForceProportions' 1000000:1 on 32 x 32 would be 32000000 x 32: 4 GB.
    let wide = one(
        Kind::ForceProportions,
        &[("ProportionalWidth", Value::Int(1000))],
    );
    assert_eq!(
        wide.try_apply(&picture).unwrap().unwrap().dimensions(),
        (32000, 32)
    );
}
