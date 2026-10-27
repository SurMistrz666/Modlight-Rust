//! Writes the menu bar glyph and its progress states as PNGs, for review:
//! `cargo run -p modlight-app --example preview_tray -- <directory>`.

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    std::fs::create_dir_all(&dir).expect("the directory");
    for size in [16, 18, 32, 36] {
        modlight_app::tray::tray_glyph(size)
            .save(dir.join(format!("tray-glyph-{size}.png")))
            .expect("saved");
    }
    for percentage in [0, 7, 42, 75, 100] {
        modlight_app::tray::template_progress_image(36, percentage)
            .save(dir.join(format!("tray-progress-{percentage}.png")))
            .expect("saved");
        modlight_app::tray::progress_image(percentage)
            .save(dir.join(format!("tray-progress-windows-{percentage}.png")))
            .expect("saved");
    }
}
