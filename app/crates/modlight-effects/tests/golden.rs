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
const PORTED: &[&str] = &["Adjustments", "Manipulations"];

fn ported(category: &str) -> bool {
    PORTED.contains(&category)
}

/// Effects not compared pixel for pixel, and why.
fn skipped(id: &str) -> Option<&'static str> {
    let _ = id;
    None
}

/// Allowed share of mismatched pixels (and the per-pixel YIQ threshold) per effect.
fn threshold(id: &str) -> (f64, f64) {
    match id {
        // Anti-aliased edges: Skia m119 fills with analytic AA whose edge stepping snaps
        // to quarter pixels; the port computes the exact area. Edge pixels differ by up
        // to 0.2 in alpha, interiors match.
        "rounded_corners" => (0.03, 0.025),
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
        if !ported(m["category"].as_str().unwrap()) {
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

#[test]
fn pixels_match_upstream() {
    let dir = golden_dir();
    let cases = read_json(&dir.join("cases.json"));
    let reg = registry();
    let manifest = read_json(&dir.join("manifest.json"));
    let category: HashMap<String, String> = manifest
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["id"].as_str().unwrap().to_string(),
                m["category"].as_str().unwrap().to_string(),
            )
        })
        .collect();
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
        if case.get("error").is_some() || !ported(&category[id]) {
            continue;
        }
        if let Some(why) = skipped(id) {
            report.push(format!("{name}\tskipped\t{why}"));
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
        compared += 1;
        let (allowed, t) = threshold(id);
        match compare(&ours, &expected, t) {
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
