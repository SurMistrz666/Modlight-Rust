//! Snapshot tests of the editor window (PORT.md section 4): every QA scene in the
//! default skin's dark and light palettes, in English and in Polish, rendered headlessly
//! and compared with `tests/snapshots/<scene>/<palette>-<language>.png`.
//!
//! `UPDATE_SNAPSHOTS=1 cargo test -p modlight-editor --test ui_snapshots` writes the
//! pictures that differ. `EDITOR_SCENE=<name>` draws one scene. Nothing here opens a
//! window. The renders use this Mac's fonts (the font picker, Apple Color Emoji).

use modlight_core::l10n::{Language, set_thread_language};
use modlight_editor::qa;
use modlight_editor::ui::Colors;

fn render(scene: &str, dark: bool, language: Language) -> Result<image::RgbaImage, String> {
    set_thread_language(Some(language));
    let colors = Colors::default_skin(dark);
    let s = qa::scene(scene).ok_or_else(|| format!("unknown scene {scene}"))?;
    let popup = s.popup;
    struct Shot {
        scene: qa::Scene,
        colors: Colors,
        frames: u32,
    }
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(qa::SIZE.0, qa::SIZE.1))
        .with_pixels_per_point(1.0)
        .with_max_steps(16)
        .wgpu()
        .build_ui_state(
            |ui, shot: &mut Shot| {
                // The fonts go in before the first frame that paints with them.
                if shot.frames == 0 {
                    modlight_editor::ui::style::install(ui.ctx(), &shot.colors);
                } else {
                    // The whole window, without the test panel's margin.
                    let rect = ui.ctx().content_rect();
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                    child.set_clip_rect(rect);
                    shot.scene
                        .view
                        .show(&mut child, &mut shot.scene.editor, &shot.colors);
                }
                shot.frames += 1;
            },
            Shot {
                scene: s,
                colors,
                frames: 0,
            },
        );
    harness.run_steps(3);
    if let Some(id) = popup {
        egui::Popup::open_id(&harness.ctx, id);
    }
    harness.run_steps(3);
    harness.render()
}

fn options() -> egui_kittest::SnapshotOptions {
    egui_kittest::SnapshotOptions::new()
        .threshold(0.6)
        .max_failed_pixels(200)
}

#[test]
fn every_scene_in_every_look() {
    let only = std::env::var("EDITOR_SCENE").ok();
    let dump = std::env::var("EDITOR_SHOTS_DIR").ok();
    let mut failures = Vec::new();
    let mut rendered = 0;
    for scene in qa::SCENES {
        if only.as_deref().is_some_and(|o| o != *scene) {
            continue;
        }
        for dark in [true, false] {
            for language in [Language::English, Language::Polish] {
                let name = format!(
                    "{}-{}",
                    if dark { "dark" } else { "light" },
                    if language == Language::English {
                        "en"
                    } else {
                        "pl"
                    }
                );
                let image = match render(scene, dark, language) {
                    Ok(i) => i,
                    Err(e) => {
                        failures.push(format!("{scene}/{name}: {e}"));
                        continue;
                    }
                };
                rendered += 1;
                if let Some(dir) = &dump {
                    let path = std::path::Path::new(dir).join(format!("{scene}-{name}.png"));
                    let _ = image.save(path);
                }
                if let Err(error) = egui_kittest::try_image_snapshot_options(
                    &image,
                    format!("{scene}/{name}"),
                    &options(),
                ) {
                    failures.push(format!("{scene}/{name}: {error}"));
                }
            }
        }
    }
    assert!(rendered > 0, "no scene was rendered");
    assert!(
        failures.is_empty(),
        "{} of {rendered} snapshots differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
