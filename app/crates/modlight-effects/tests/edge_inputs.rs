//! Every editor effect against the inputs the editor can hand it: pictures of 1x1, one
//! row, one column, odd sizes, fully transparent and fully opaque, with the parameters at
//! their defaults, at each end of their range and with every enum option. An effect runs
//! on the UI thread without a `catch_unwind`, so a panic here is a crash of the editor.
//!
//! Checked: no panic, an output of at least 1x1, and the output size the effect's
//! category promises (everything but a manipulation keeps the size).
//!
//! `MODLIGHT_EDGE_ONLY` limits the run to a comma-separated list of effect ids.

use std::cell::RefCell;
use std::sync::Mutex;

use image::{Rgba, RgbaImage};
use modlight_editor::color::Color;
use modlight_editor::effects::{
    Category, EffectContext, ImageEffect, ParamDef, ParamKind, ParamValue, ParamValues,
};

thread_local! {
    static LAST_PANIC: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// The effects outside Manipulations whose output is larger or smaller than their input
/// upstream (frames, borders, shadows, reflections); their sizes are checked against
/// upstream's by the golden tests.
const RESIZING: &[&str] = &[
    "art_deco_border",
    "border",
    "brick_wall_border",
    "candy_cane_border",
    "celtic_knot_border",
    "chain_link_border",
    "chrome_metallic_border",
    "circuit_board_border",
    "crimson_border",
    "film_strip_frame",
    "frost_ice_border",
    "glow",
    "golden_border",
    "gradient_fade_border",
    "lava_molten_border",
    "macos_window",
    "marble_border",
    "minecraft_grass",
    "mosaic_tile_border",
    "neon_glow_border",
    "ornate_scroll_border",
    "outline",
    "polaroid_frame",
    "rainbow_border",
    "reflection",
    "rope_border",
    "shadow",
    "slice",
    "stained_glass_border",
    "starfield_border",
    "watercolor_border",
    "windows11_window",
    "wooden_frame",
    "zigzag_border",
];

/// Manipulations whose largest values ask for a canvas of up to 20000 px more a side.
const HUGE: &[&str] = &["resize_canvas", "resize_image"];

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
    let mixed = |w: u32, h: u32| {
        RgbaImage::from_fn(w, h, |x, y| {
            let n = x
                .wrapping_mul(2654435761)
                .wrapping_add(y.wrapping_mul(40503))
                >> 7;
            Rgba([n as u8, (n >> 8) as u8, (n >> 16) as u8, (n >> 3) as u8])
        })
    };
    vec![
        ("1x1 opaque", opaque(1, 1)),
        ("1x1 transparent", RgbaImage::new(1, 1)),
        ("1x9", opaque(1, 9)),
        ("9x1", opaque(9, 1)),
        ("2x2 mixed", mixed(2, 2)),
        ("7x5 opaque", opaque(7, 5)),
        ("7x5 transparent", RgbaImage::new(7, 5)),
        ("33x17 mixed", mixed(33, 17)),
    ]
}

/// The value sets tried for one effect: defaults, each parameter at each end (or each
/// option), everything at its minimum and everything at its maximum.
fn value_sets(defs: &[ParamDef]) -> Vec<(String, ParamValues)> {
    let mut sets = vec![("defaults".to_string(), ParamValues::defaults(defs))];
    let ends = |d: &ParamDef| -> Vec<(String, ParamValue)> {
        match &d.kind {
            ParamKind::Slider { min, max, .. } => vec![
                ("min".into(), ParamValue::Number(*min)),
                ("max".into(), ParamValue::Number(*max)),
            ],
            ParamKind::Numeric { min, max, .. } => vec![
                ("min".into(), ParamValue::Number(*min)),
                ("max".into(), ParamValue::Number(*max)),
                ("empty".into(), ParamValue::Empty),
            ],
            ParamKind::Checkbox { default } => {
                vec![("toggled".into(), ParamValue::Bool(!*default))]
            }
            ParamKind::Enum { options, .. } => options
                .iter()
                .map(|(_, v)| (v.clone(), ParamValue::Choice(v.clone())))
                .collect(),
            ParamKind::Color { .. } => vec![
                ("transparent".into(), ParamValue::Color(Color::TRANSPARENT)),
                ("white".into(), ParamValue::Color(Color::WHITE)),
                ("black".into(), ParamValue::Color(Color::BLACK)),
            ],
            ParamKind::Text { .. } => vec![
                ("empty".into(), ParamValue::Text(String::new())),
                ("spaces".into(), ParamValue::Text("   ".into())),
                (
                    "long".into(),
                    ParamValue::Text("Zażółć gęślą jaźń 🙂 ".repeat(20)),
                ),
            ],
            ParamKind::FilePath { .. } => vec![
                ("empty".into(), ParamValue::Text(String::new())),
                (
                    "missing".into(),
                    ParamValue::Text("/nonexistent/modlight/edge.png".into()),
                ),
            ],
        }
    };
    for d in defs {
        for (what, value) in ends(d) {
            let mut v = ParamValues::defaults(defs);
            v.set(&d.key, value);
            sets.push((format!("{}={what}", d.key), v));
        }
    }
    for (label, pick_max) in [("all min", false), ("all max", true)] {
        let mut v = ParamValues::defaults(defs);
        for d in defs {
            match &d.kind {
                ParamKind::Slider { min, max, .. } | ParamKind::Numeric { min, max, .. } => {
                    v.set(
                        &d.key,
                        ParamValue::Number(if pick_max { *max } else { *min }),
                    );
                }
                _ => {}
            }
        }
        sets.push((label.into(), v));
    }
    sets
}

fn run_one(
    effect: &dyn ImageEffect,
    input: &RgbaImage,
    values: &ParamValues,
) -> Result<Result<RgbaImage, String>, String> {
    LAST_PANIC.with(|l| *l.borrow_mut() = None);
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        effect.apply(input, values, &EffectContext::default())
    }))
    .map_err(|_| {
        LAST_PANIC
            .with(|l| l.borrow_mut().take())
            .unwrap_or_else(|| "panic".into())
    })
}

#[test]
fn every_effect_survives_edge_inputs() {
    let only: Vec<String> = std::env::var("MODLIGHT_EDGE_ONLY")
        .map(|s| s.split(',').map(|x| x.trim().to_string()).collect())
        .unwrap_or_default();
    let effects: Vec<_> = modlight_effects::effects()
        .into_iter()
        .filter(|e| only.is_empty() || only.iter().any(|o| o == e.id()))
        .collect();
    assert!(!effects.is_empty());
    let inputs = inputs();
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|info| {
        let text = format!("{info}");
        LAST_PANIC.with(|l| *l.borrow_mut() = Some(text.replace('\n', " ")));
    }));
    let problems = Mutex::new(Vec::<String>::new());
    let next = Mutex::new(0usize);
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(12);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = {
                        let mut n = next.lock().unwrap();
                        let i = *n;
                        *n += 1;
                        i
                    };
                    let Some(effect) = effects.get(i) else { break };
                    let defs = effect.parameters();
                    let mut seen = std::collections::HashSet::new();
                    for (label, values) in value_sets(&defs) {
                        // Their largest values make pictures of hundreds of megapixels
                        // (as upstream does); the golden tests cover their size math.
                        if HUGE.contains(&effect.id())
                            && (label.ends_with("=max") || label == "all max")
                        {
                            continue;
                        }
                        for (name, input) in &inputs {
                            let id = effect.id();
                            let report = |what: String| {
                                problems
                                    .lock()
                                    .unwrap()
                                    .push(format!("{id} [{label}] on {name}: {what}"));
                            };
                            match run_one(effect.as_ref(), input, &values) {
                                Err(panic) => {
                                    // One line per distinct panic site is enough.
                                    if seen.insert(panic.clone()) {
                                        report(format!("PANIC {panic}"));
                                    }
                                }
                                Ok(Err(_)) => {}
                                Ok(Ok(out)) => {
                                    if out.width() == 0 || out.height() == 0 {
                                        report(format!(
                                            "empty output {}x{}",
                                            out.width(),
                                            out.height()
                                        ));
                                    } else if effect.category() != Category::Manipulations
                                        && !RESIZING.contains(&id)
                                        && out.dimensions() != input.dimensions()
                                    {
                                        report(format!(
                                            "size {}x{} -> {}x{}",
                                            input.width(),
                                            input.height(),
                                            out.width(),
                                            out.height()
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            });
        }
    });
    std::panic::set_hook(default_hook);
    let mut problems = problems.into_inner().unwrap();
    problems.sort();
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

fn run_defaults(
    id: &str,
    input: &RgbaImage,
    set: &[(&str, ParamValue)],
) -> Result<RgbaImage, String> {
    let effect = modlight_effects::effects()
        .into_iter()
        .find(|e| e.id() == id)
        .unwrap();
    let mut values = ParamValues::defaults(&effect.parameters());
    for (key, value) in set {
        values.set(key, value.clone());
    }
    effect.apply(input, &values, &EffectContext::default())
}

/// Upstream's Minecraft grass makes a bitmap of `height + border - overlap` rows; when
/// the overlap is larger than the border and the picture together, `new SKBitmap` fails
/// and the editor keeps the picture as it was. The port must fail too, not hand back a
/// one-row strip that replaces the picture.
#[test]
fn minecraft_grass_fails_when_the_overlap_eats_the_picture() {
    let input = RgbaImage::from_pixel(7, 5, Rgba([200, 30, 40, 255]));
    // Default rows 2 + 4 of 16 px = 96 px of border; an overlap of 8 blocks is 128 px.
    let out = run_defaults(
        "minecraft_grass",
        &input,
        &[("overlap", ParamValue::Number(8.0))],
    );
    assert!(
        out.is_err(),
        "got a {:?} picture",
        out.map(|o| o.dimensions())
    );
    // An overlap the picture can absorb still works (5 + 96 - 96 = 5 rows).
    let out = run_defaults(
        "minecraft_grass",
        &input,
        &[("overlap", ParamValue::Number(6.0))],
    )
    .unwrap();
    assert_eq!(out.dimensions(), (7, 5));
}
