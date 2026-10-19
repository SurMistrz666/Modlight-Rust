//! Snapshot tests (PORT.md section 4): every QA screen, in the default skin's dark and
//! light palettes, in English and in Polish, rendered headlessly and compared with the
//! reference pictures under `tests/snapshots/<screen>/<palette>-<language>.png`.
//!
//! `UPDATE_SNAPSHOTS=1 cargo test -p modlight-app --test snapshots` writes the pictures
//! that differ (and keeps the previous one as `.old.png`); `UPDATE_SNAPSHOTS=force`
//! rewrites them all. Nothing here opens a window or talks to a server.

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

fn options() -> egui_kittest::SnapshotOptions {
    // The renders are deterministic on one machine; the slack is for the GPU driver's
    // own rounding of anti-aliased edges after an OS update.
    egui_kittest::SnapshotOptions::new()
        .threshold(0.6)
        .max_failed_pixels(120)
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
            if let Err(error) = egui_kittest::try_image_snapshot_options(
                &image,
                format!("{screen}/{name}"),
                &options(),
            ) {
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
