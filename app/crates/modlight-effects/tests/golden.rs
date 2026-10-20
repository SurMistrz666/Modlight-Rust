//! The effects against upstream: `tests/golden` holds what upstream's own effect classes
//! (compiled from `ShareX.ImageEditor/Core/ImageEffects` with SkiaSharp 3.119.4 in a Linux
//! container, see `docs/handoff/app-5-effects.md`) report and draw:
//!
//! - `manifest.json`: every effect with its metadata and parameter schema, in the
//!   browser's order. The registry must match it field by field.
//! - `cases.json`, `inputs/`, `expected/`: each effect applied with its defaults to two
//!   inputs and with changed values to one. Every output must match within the effect's
//!   threshold under a perceptual (YIQ) colour difference.
//!
//! `MODLIGHT_GOLDEN_DIR` points the test at another folder; `MODLIGHT_GOLDEN_OUT` writes
//! the port's outputs and a report there; `MODLIGHT_GOLDEN_ONLY` limits the run to a
//! comma-separated list of effect ids.

#![allow(clippy::needless_range_loop)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use modlight_editor::effects::{EffectRegistry, ParamKind, ParamValue, ParamValues};
use serde_json::Value;

fn golden_dir() -> PathBuf {
    std::env::var_os("MODLIGHT_GOLDEN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden"))
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(
        &std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
    )
    .unwrap()
}

fn registry() -> EffectRegistry {
    let mut r = EffectRegistry::with_builtin();
    for e in modlight_effects::effects() {
        r.register(e);
    }
    r
}

/// The categories ported so far (the port lands category by category).
/// While the port lands category by category, effects not registered yet are skipped;
/// once every effect is in, a missing one fails.
const REQUIRE_ALL: bool = false;

fn ported(id: &str) -> bool {
    REQUIRE_ALL || modlight_effects::find(id).is_some()
}

/// Effects not compared pixel for pixel, and why.
fn skipped(id: &str) -> Option<&'static str> {
    match id {
        "ascii_art" | "draw_text" | "text_watermark" | "matrix_digital_rain" => {
            Some("text: glyph rasteriser and font differ")
        }
        _ => None,
    }
}

/// Effects upstream seeds from the clock or `Random.Shared` (the harness may have seen
/// the same seed twice within one tick and called them deterministic).
const RANDOM_SEEDED: &[&str] = &[
    "blood_splash",
    "blueprint_drawing",
    "claymation_texture",
    "confetti",
    "fake_cursor",
    "fake_hair",
    "ink_splatter_drips",
    "slice",
    "snowfall_depth_fog",
    "snowflakes",
    "torn_edge",
];

/// Effects whose pictures are mostly anti-aliased shapes and strokes: Skia m119 fills
/// them with analytic AA whose edge stepping snaps to quarter pixels while the port
/// computes exact areas, so edge pixels differ by up to 0.2 in alpha.
const AA_SHAPES: &[&str] = &["rounded_corners", "art_deco_border", "celtic_knot_border"];

/// Effects made of many small anti-aliased shapes (dots, sparks): Skia's supersampled
/// coverage of a small circle differs from tiny-skia's by a sub-sample or two at the
/// rim, so both pictures are compared after a 3x3 box blur, which keeps every shape,
/// colour and position but not the rim's last sub-pixel.
/// Pointillism's dots that cross the canvas edge differ where Skia's edge clipper chops
/// the oval's quads at the clip (tiny differences on the first column).
const SMALL_SHAPES: &[&str] = &["color_halftone", "engraving", "pointillism"];

/// Allowed share of mismatched pixels (and the per-pixel YIQ threshold) per effect.
fn threshold(id: &str) -> (f64, f64) {
    match id {
        _ if AA_SHAPES.contains(&id) => (0.03, 0.025),
        "color_halftone" => (0.02, 0.025),
        "embroidery" => (0.05, 0.025),
        _ => (0.005, 0.025),
    }
}

#[test]
fn metadata_matches_upstream() {
    let manifest = read_json(&golden_dir().join("manifest.json"));
    let reg = registry();
    let mut problems = Vec::new();
    let mut order = Vec::new();
    for m in manifest.as_array().unwrap() {
        let id = m["id"].as_str().unwrap();
        if !ported(id) {
            continue;
        }
        order.push(id.to_string());
        let Some(e) = reg.get(id) else {
            problems.push(format!("{id}: missing"));
            continue;
        };
        let mut check = |what: &str, ours: String, theirs: String| {
            if ours != theirs {
                problems.push(format!("{id}: {what}: ours {ours:?}, upstream {theirs:?}"));
            }
        };
        check("name", e.name().into(), m["name"].as_str().unwrap().into());
        check(
            "category",
            format!("{:?}", e.category()),
            m["category"].as_str().unwrap().into(),
        );
        check(
            "icon",
            format!("{:04x}", e.icon() as u32),
            m["icon"].as_str().unwrap().into(),
        );
        check(
            "description",
            e.description().into(),
            m["description"].as_str().unwrap().into(),
        );
        check(
            "mode",
            format!("{:?}", e.execution_mode()),
            m["mode"].as_str().unwrap().into(),
        );
        let ours = e.parameters();
        let theirs = m["parameters"].as_array().unwrap();
        check(
            "parameter count",
            ours.len().to_string(),
            theirs.len().to_string(),
        );
        for (p, t) in ours.iter().zip(theirs) {
            let key = t["key"].as_str().unwrap();
            check("key", p.key.clone(), key.into());
            check(
                &format!("{key}.label"),
                p.label.clone(),
                t["label"].as_str().unwrap().into(),
            );
            check(
                &format!("{key}.description"),
                p.description.clone().unwrap_or_default(),
                t["description"].as_str().unwrap_or("").into(),
            );
            let num = |v: &Value| v.as_f64().map(|f| format!("{f}")).unwrap_or_default();
            match (&p.kind, t["kind"].as_str().unwrap()) {
                (
                    ParamKind::Slider {
                        min,
                        max,
                        default,
                        tick,
                        snap,
                        format,
                        integer,
                    },
                    "slider",
                ) => {
                    check(&format!("{key}.min"), format!("{min}"), num(&t["min"]));
                    check(&format!("{key}.max"), format!("{max}"), num(&t["max"]));
                    check(
                        &format!("{key}.default"),
                        format!("{default}"),
                        num(&t["default"]),
                    );
                    check(&format!("{key}.tick"), format!("{tick}"), num(&t["tick"]));
                    check(
                        &format!("{key}.snap"),
                        snap.to_string(),
                        t["snap"].to_string(),
                    );
                    check(
                        &format!("{key}.format"),
                        format.clone(),
                        t["format"].as_str().unwrap().into(),
                    );
                    check(
                        &format!("{key}.integer"),
                        integer.to_string(),
                        (t["number"].as_str() == Some("int")).to_string(),
                    );
                }
                (
                    ParamKind::Numeric {
                        min,
                        max,
                        default,
                        increment,
                        format,
                        integer,
                    },
                    "numeric",
                ) => {
                    check(&format!("{key}.min"), format!("{min}"), num(&t["min"]));
                    check(&format!("{key}.max"), format!("{max}"), num(&t["max"]));
                    check(
                        &format!("{key}.default"),
                        format!("{default}"),
                        num(&t["default"]),
                    );
                    check(
                        &format!("{key}.increment"),
                        format!("{increment}"),
                        num(&t["increment"]),
                    );
                    check(
                        &format!("{key}.format"),
                        format.clone(),
                        t["format"].as_str().unwrap().into(),
                    );
                    check(
                        &format!("{key}.integer"),
                        integer.to_string(),
                        (t["number"].as_str() == Some("int")).to_string(),
                    );
                }
                (ParamKind::Checkbox { default }, "checkbox") => {
                    check(
                        &format!("{key}.default"),
                        default.to_string(),
                        t["default"].to_string(),
                    );
                }
                (
                    ParamKind::Enum {
                        options,
                        default_index,
                    },
                    "enum",
                ) => {
                    let theirs: Vec<(String, String)> = t["options"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|o| {
                            (
                                o["label"].as_str().unwrap().to_string(),
                                o["value"].as_str().unwrap().to_string(),
                            )
                        })
                        .collect();
                    check(
                        &format!("{key}.options"),
                        format!("{options:?}"),
                        format!("{theirs:?}"),
                    );
                    check(
                        &format!("{key}.default_index"),
                        default_index.to_string(),
                        t["default_index"].to_string(),
                    );
                }
                (ParamKind::Color { default }, "color") => {
                    check(
                        &format!("{key}.default"),
                        default.to_hex_argb(),
                        t["default"].as_str().unwrap().into(),
                    );
                }
                (ParamKind::Text { default }, "text") => {
                    check(
                        &format!("{key}.default"),
                        default.clone(),
                        t["default"].as_str().unwrap().into(),
                    );
                }
                (ParamKind::FilePath { default, filter }, "file") => {
                    check(
                        &format!("{key}.default"),
                        default.clone(),
                        t["default"].as_str().unwrap().into(),
                    );
                    check(
                        &format!("{key}.filter"),
                        filter.clone().unwrap_or_default(),
                        t["filter"].as_str().unwrap_or("").into(),
                    );
                }
                (k, kind) => check(&format!("{key}.kind"), format!("{k:?}"), kind.to_string()),
            }
        }
    }
    // The browser's order: category, then name.
    let ours: Vec<String> = reg
        .all()
        .iter()
        .map(|e| e.id().to_string())
        .filter(|id| order.contains(id))
        .collect();
    if ours != order {
        for (i, (a, b)) in ours.iter().zip(&order).enumerate() {
            if a != b {
                problems.push(format!("order differs at {i}: ours {a}, upstream {b}"));
                break;
            }
        }
    }
    let only = only();
    let problems: Vec<String> = problems
        .into_iter()
        .filter(|p| only.is_empty() || only.iter().any(|o| p.starts_with(&format!("{o}:"))))
        .collect();
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

fn only() -> Vec<String> {
    std::env::var("MODLIGHT_GOLDEN_ONLY")
        .map(|s| {
            s.split(',')
                .filter(|x| !x.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The values of a case, as the dialog would hand them to the effect.
fn values_for(params: &[modlight_editor::effects::ParamDef], case: &Value) -> ParamValues {
    let mut v = ParamValues::defaults(params);
    let map = case["values"].as_object().unwrap();
    for p in params {
        let Some(x) = map.get(&p.key) else { continue };
        let value = match &p.kind {
            ParamKind::Slider { .. } | ParamKind::Numeric { .. } => {
                ParamValue::Number(x.as_f64().unwrap())
            }
            ParamKind::Checkbox { .. } => ParamValue::Bool(x.as_bool().unwrap()),
            ParamKind::Enum { .. } => ParamValue::Choice(x.as_str().unwrap().to_string()),
            ParamKind::Color { .. } => ParamValue::Color(
                modlight_editor::color::Color::parse(x.as_str().unwrap()).unwrap(),
            ),
            ParamKind::Text { .. } | ParamKind::FilePath { .. } => {
                ParamValue::Text(x.as_str().unwrap_or("").to_string())
            }
        };
        v.set(&p.key, value);
    }
    v
}

/// YIQ difference of two straight colours composited over white and over black (the
/// larger of the two), as pixelmatch measures it; 0..35215.
fn yiq_delta(a: [u8; 4], b: [u8; 4]) -> f64 {
    let over = |c: [u8; 4], bg: f64| {
        let al = c[3] as f64 / 255.0;
        [
            bg + (c[0] as f64 - bg) * al,
            bg + (c[1] as f64 - bg) * al,
            bg + (c[2] as f64 - bg) * al,
        ]
    };
    let yiq = |c: [f64; 3]| {
        (
            c[0] * 0.29889531 + c[1] * 0.58662247 + c[2] * 0.11448223,
            c[0] * 0.59597799 - c[1] * 0.27417610 - c[2] * 0.32180189,
            c[0] * 0.21147017 - c[1] * 0.52261711 + c[2] * 0.31114694,
        )
    };
    let mut worst: f64 = 0.0;
    for bg in [255.0, 0.0] {
        let (y1, i1, q1) = yiq(over(a, bg));
        let (y2, i2, q2) = yiq(over(b, bg));
        let d = 0.5053 * (y1 - y2).powi(2) + 0.299 * (i1 - i2).powi(2) + 0.1957 * (q1 - q2).powi(2);
        worst = worst.max(d);
    }
    worst
}

/// A 3x3 box blur of premultiplied pixels (edges clamped), back to straight alpha.
fn box_blur(img: &image::RgbaImage) -> image::RgbaImage {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let mut out = image::RgbaImage::new(w as u32, h as u32);
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0f64; 4];
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let p = img
                        .get_pixel(
                            (x + dx).clamp(0, w - 1) as u32,
                            (y + dy).clamp(0, h - 1) as u32,
                        )
                        .0;
                    let a = p[3] as f64 / 255.0;
                    acc[0] += p[0] as f64 * a;
                    acc[1] += p[1] as f64 * a;
                    acc[2] += p[2] as f64 * a;
                    acc[3] += p[3] as f64;
                }
            }
            let a = acc[3] / 9.0;
            let un = |v: f64| {
                if a > 0.0 {
                    (v / 9.0 / (a / 255.0)).round().clamp(0.0, 255.0) as u8
                } else {
                    0
                }
            };
            out.put_pixel(
                x as u32,
                y as u32,
                image::Rgba([un(acc[0]), un(acc[1]), un(acc[2]), a.round() as u8]),
            );
        }
    }
    out
}

struct Outcome {
    mismatch: f64,
    mean: f64,
    max: f64,
}

fn compare(ours: &image::RgbaImage, theirs: &image::RgbaImage, t: f64) -> Option<Outcome> {
    if ours.dimensions() != theirs.dimensions() {
        return None;
    }
    let limit = 35215.0 * t * t;
    let mut bad = 0usize;
    let mut sum = 0.0;
    let mut max: f64 = 0.0;
    for (a, b) in ours.pixels().zip(theirs.pixels()) {
        let d = yiq_delta(a.0, b.0);
        sum += d;
        max = max.max(d);
        if d > limit {
            bad += 1;
        }
    }
    let n = (ours.width() * ours.height()).max(1) as f64;
    Some(Outcome {
        mismatch: bad as f64 / n,
        mean: (sum / n).sqrt() / 35215f64.sqrt(),
        max: max.sqrt() / 35215f64.sqrt(),
    })
}

/// For effects that differ on every run upstream: same size, the same share of changed
/// pixels (within 25 percentage points) and channel means within 12 levels.
fn compare_stats(
    input: &image::RgbaImage,
    ours: &[image::RgbaImage],
    theirs: &image::RgbaImage,
) -> Result<String, String> {
    for o in ours {
        if o.dimensions() != theirs.dimensions() {
            return Err(format!(
                "size {:?}, upstream {:?}",
                o.dimensions(),
                theirs.dimensions()
            ));
        }
    }
    let mean = |img: &image::RgbaImage| {
        let mut m = [0f64; 4];
        for p in img.pixels() {
            let a = p.0[3] as f64 / 255.0;
            for k in 0..3 {
                m[k] += p.0[k] as f64 * a;
            }
            m[3] += p.0[3] as f64;
        }
        let n = (img.width() * img.height()).max(1) as f64;
        m.map(|v| v / n)
    };
    let changed = |img: &image::RgbaImage| {
        if img.dimensions() != input.dimensions() {
            return 1.0;
        }
        let n = img
            .pixels()
            .zip(input.pixels())
            .filter(|(a, b)| yiq_delta(a.0, b.0) > 35215.0 * 0.025 * 0.025)
            .count();
        n as f64 / (img.width() * img.height()).max(1) as f64
    };
    // Upstream's one render must look like one of ours: its changed share and channel
    // means inside the range our renders (each with its own seed) span, plus a margin.
    let (mt, ct) = (mean(theirs), changed(theirs));
    let cs: Vec<f64> = ours.iter().map(changed).collect();
    let ms: Vec<[f64; 4]> = ours.iter().map(mean).collect();
    let lo = |v: &[f64]| v.iter().cloned().fold(f64::MAX, f64::min);
    let hi = |v: &[f64]| v.iter().cloned().fold(f64::MIN, f64::max);
    let (clo, chi) = (lo(&cs), hi(&cs));
    let mut worst = 0f64;
    for k in 0..4 {
        let v: Vec<f64> = ms.iter().map(|m| m[k]).collect();
        let d = if mt[k] < lo(&v) {
            lo(&v) - mt[k]
        } else if mt[k] > hi(&v) {
            mt[k] - hi(&v)
        } else {
            0.0
        };
        worst = worst.max(d);
    }
    let cd = if ct < clo {
        clo - ct
    } else if ct > chi {
        ct - chi
    } else {
        0.0
    };
    let line = format!("changed {clo:.3}..{chi:.3} vs {ct:.3}, mean outside by {worst:.2}");
    if cd > 0.1 || worst > 8.0 {
        return Err(line);
    }
    Ok(line)
}

#[test]
fn pixels_match_upstream() {
    let dir = golden_dir();
    let cases = read_json(&dir.join("cases.json"));
    let reg = registry();
    let only = only();
    let out = std::env::var_os("MODLIGHT_GOLDEN_OUT").map(PathBuf::from);
    if let Some(o) = &out {
        std::fs::create_dir_all(o.join("actual")).unwrap();
    }
    let mut inputs: HashMap<String, image::RgbaImage> = HashMap::new();
    let mut failures = Vec::new();
    let mut report = Vec::new();
    let mut compared = 0;
    for case in cases.as_array().unwrap() {
        let id = case["effect"].as_str().unwrap();
        let name = case["name"].as_str().unwrap();
        if !only.is_empty() && !only.iter().any(|o| o == id) {
            continue;
        }
        if case.get("error").is_some() || !ported(id) {
            continue;
        }
        let Some(effect) = reg.get(id) else {
            failures.push(format!("{name}: effect missing"));
            continue;
        };
        let input_name = case["input"].as_str().unwrap().to_string();
        let input = inputs
            .entry(input_name.clone())
            .or_insert_with(|| {
                image::open(dir.join("inputs").join(format!("{input_name}.png")))
                    .unwrap()
                    .to_rgba8()
            })
            .clone();
        let expected = image::open(dir.join("expected").join(format!("{name}.png")))
            .unwrap()
            .to_rgba8();
        let values = values_for(&effect.parameters(), case);
        let ours = match effect.apply(&input, &values, &Default::default()) {
            Ok(img) => img,
            Err(e) => {
                failures.push(format!("{name}: error {e}"));
                continue;
            }
        };
        if let Some(o) = &out {
            ours.save(o.join("actual").join(format!("{name}.png")))
                .unwrap();
        }
        if let Some(why) = skipped(id) {
            // Rendered (and saved for review) but not compared.
            report.push(format!("{name}\tskipped\t{why}"));
            continue;
        }
        compared += 1;
        if case["deterministic"] == Value::Bool(false) || RANDOM_SEEDED.contains(&id) {
            // Upstream seeds these from Random.Shared: compare what is drawn statistically.
            let mut renders = vec![ours.clone()];
            for _ in 0..39 {
                if let Ok(img) = effect.apply(&input, &values, &Default::default()) {
                    renders.push(img);
                }
            }
            match compare_stats(&input, &renders, &expected) {
                Ok(line) => report.push(format!("{name}\tstats\t{line}")),
                Err(e) => failures.push(format!("{name}: {e}")),
            }
            continue;
        }
        let (allowed, t) = threshold(id);
        let (ours_c, expected_c) = if SMALL_SHAPES.contains(&id) {
            (box_blur(&ours), box_blur(&expected))
        } else {
            (ours.clone(), expected.clone())
        };
        match compare(&ours_c, &expected_c, t) {
            None => failures.push(format!(
                "{name}: size {:?}, upstream {:?}",
                ours.dimensions(),
                expected.dimensions()
            )),
            Some(o) => {
                report.push(format!(
                    "{name}\t{:.5}\t{:.5}\t{:.4}",
                    o.mismatch, o.mean, o.max
                ));
                if o.mismatch > allowed {
                    failures.push(format!(
                        "{name}: {:.2}% pixels differ (allowed {:.2}%), mean {:.4}, max {:.3}",
                        o.mismatch * 100.0,
                        allowed * 100.0,
                        o.mean,
                        o.max
                    ));
                }
            }
        }
    }
    if let Some(o) = &out {
        std::fs::write(o.join("report.tsv"), report.join("\n")).unwrap();
    }
    assert!(compared > 0 || !only.is_empty(), "no cases compared");
    assert!(
        failures.is_empty(),
        "{} of {compared} cases differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
