//! Snapshot tests (PORT.md section 4): every QA screen, in the default skin's dark and
//! light palettes, in English and in Polish, rendered headlessly and compared with the
//! reference pictures under `tests/snapshots/<screen>/<palette>-<language>.png`.
//!
//! `UPDATE_SNAPSHOTS=1 cargo test -p modlight-app --test snapshots` writes the pictures
//! that differ (and keeps the previous one as `.old.png`); `UPDATE_SNAPSHOTS=force`
//! rewrites them all. Nothing here opens a window or talks to a server.
//!
//! On Windows a screen is first compared, read only, with the shared picture; when it
//! matches, it passes. When it does not (the title band's caption buttons, Windows paths
//! and texts, the system's fonts), it is compared with, and with `UPDATE_SNAPSHOTS`
//! written as, `tests/snapshots/<screen>/windows-<palette>-<language>.png`. So only the
//! screens that really differ on Windows have a Windows picture, and a Windows run never
//! writes a shared one. macOS never looks at the `windows-` pictures.

use modlight_app::qa::{self, Look};
use modlight_core::l10n::Language;

/// The looks every screen is drawn in.
fn looks() -> Vec<(String, Look)> {
    let mut looks = Vec::new();
    for dark in [true, false] {
        for language in [Language::English, Language::Polish] {
            let name = format!(
                "{}-{}",
                if dark { "dark" } else { "light" },
                match language {
                    Language::English => "en",
                    Language::Polish => "pl",
                }
            );
            looks.push((
                name,
                Look {
                    dark,
                    language,
                    ..Look::default()
                },
            ));
        }
    }
    looks
}

/// The renders are deterministic on one machine; the slack is for the GPU driver's own
/// rounding of anti-aliased edges after an OS update.
const THRESHOLD: f32 = 0.6;
const MAX_FAILED_PIXELS: usize = 120;

fn options() -> egui_kittest::SnapshotOptions {
    egui_kittest::SnapshotOptions::new()
        .threshold(THRESHOLD)
        .max_failed_pixels(MAX_FAILED_PIXELS)
}

/// One screen in one look against its picture (see the module notes).
#[cfg(not(windows))]
fn check(image: &image::RgbaImage, screen: &str, look: &str) -> Result<(), String> {
    egui_kittest::try_image_snapshot_options(image, format!("{screen}/{look}"), &options())
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
fn check(image: &image::RgbaImage, screen: &str, look: &str) -> Result<(), String> {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(screen);
    if matches_shared(image, &folder.join(format!("{look}.png"))) {
        // A Windows picture kept from before the screens matched is stale now.
        if updating() {
            let _ = std::fs::remove_file(folder.join(format!("windows-{look}.png")));
        }
        return Ok(());
    }
    egui_kittest::try_image_snapshot_options(image, format!("{screen}/windows-{look}"), &options())
        .map_err(|error| error.to_string())
}

/// Whether `image` matches the shared picture at `path` as `options()` judges it
/// (`dify`, egui_kittest's comparison), without writing anything.
#[cfg(windows)]
fn matches_shared(image: &image::RgbaImage, path: &std::path::Path) -> bool {
    let Ok(shared) = image::open(path).map(|i| i.to_rgba8()) else {
        return false;
    };
    if shared.dimensions() != image.dimensions() {
        return false;
    }
    match dify::diff::get_results(shared, image.clone(), THRESHOLD, true, None, &None, &None) {
        None => true,
        Some((failed, _)) => failed as i64 <= MAX_FAILED_PIXELS as i64,
    }
}

/// Whether this run writes pictures (`UPDATE_SNAPSHOTS`, as egui_kittest reads it).
#[cfg(windows)]
fn updating() -> bool {
    matches!(
        std::env::var("UPDATE_SNAPSHOTS").as_deref(),
        Ok("true" | "1" | "yes" | "on" | "force")
    )
}

#[test]
fn every_screen_in_every_look() {
    let only = std::env::var("QA_SCREEN").ok();
    let mut failures = Vec::new();
    let mut rendered = 0;
    for screen in qa::SCREENS {
        if only.as_deref().is_some_and(|only| only != *screen) {
            continue;
        }
        for (name, look) in looks() {
            let image = match qa::render(screen, &look) {
                Ok(image) => image,
                Err(error) => {
                    failures.push(format!("{screen}/{name}: render failed: {error}"));
                    continue;
                }
            };
            rendered += 1;
            if let Err(error) = check(&image, screen, &name) {
                failures.push(format!("{screen}/{name}: {error}"));
            }
        }
    }
    assert!(rendered > 0, "no screen was rendered");
    assert!(
        failures.is_empty(),
        "{} of {rendered} snapshots differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_screen_is_listed_once() {
    let mut seen = std::collections::HashSet::new();
    for screen in qa::SCREENS {
        assert!(seen.insert(*screen), "{screen} is listed twice");
        assert!(
            screen
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "{screen} is not a plain file name"
        );
    }
}
