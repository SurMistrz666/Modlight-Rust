//! The snapshot rule the editor's tests share (the app's tests follow the same one).
//!
//! On Windows a picture is first compared, read only, with the shared one under
//! `tests/snapshots/<name>.png`; when it matches, it passes. When it does not (Windows
//! draws text with its own fonts, upstream's "Segoe UI" among them, and its emoji with
//! Segoe UI Emoji), it is compared with, and with `UPDATE_SNAPSHOTS` written as,
//! `tests/snapshots/<folder>/windows-<file>.png`. So only the pictures that really
//! differ on Windows have a Windows copy, and a Windows run never writes a shared one.
//! macOS never looks at the `windows-` pictures.

/// One picture against its reference, `name` being `<folder>/<file>`.
#[cfg(not(windows))]
pub fn check(
    image: &image::RgbaImage,
    name: &str,
    threshold: f32,
    max_failed_pixels: usize,
) -> Result<(), String> {
    egui_kittest::try_image_snapshot_options(image, name, &options(threshold, max_failed_pixels))
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
pub fn check(
    image: &image::RgbaImage,
    name: &str,
    threshold: f32,
    max_failed_pixels: usize,
) -> Result<(), String> {
    let snapshots = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
    let (folder, file) = name.rsplit_once('/').unwrap_or(("", name));
    let shared = snapshots.join(folder).join(format!("{file}.png"));
    let windows_name = if folder.is_empty() {
        format!("windows-{file}")
    } else {
        format!("{folder}/windows-{file}")
    };
    if matches_shared(image, &shared, threshold, max_failed_pixels) {
        // A Windows picture kept from before the two matched is stale now.
        if updating() {
            let _ = std::fs::remove_file(snapshots.join(format!("{windows_name}.png")));
        }
        return Ok(());
    }
    egui_kittest::try_image_snapshot_options(
        image,
        windows_name,
        &options(threshold, max_failed_pixels),
    )
    .map_err(|error| error.to_string())
}

fn options(threshold: f32, max_failed_pixels: usize) -> egui_kittest::SnapshotOptions {
    egui_kittest::SnapshotOptions::new()
        .threshold(threshold)
        .max_failed_pixels(max_failed_pixels)
}

/// Whether `image` matches the shared picture at `path` as egui_kittest judges it
/// (`dify`), without writing anything.
#[cfg(windows)]
fn matches_shared(
    image: &image::RgbaImage,
    path: &std::path::Path,
    threshold: f32,
    max_failed_pixels: usize,
) -> bool {
    let Ok(shared) = image::open(path).map(|i| i.to_rgba8()) else {
        return false;
    };
    if shared.dimensions() != image.dimensions() {
        return false;
    }
    match dify::diff::get_results(shared, image.clone(), threshold, true, None, &None, &None) {
        None => true,
        Some((failed, _)) => failed as i64 <= max_failed_pixels as i64,
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
