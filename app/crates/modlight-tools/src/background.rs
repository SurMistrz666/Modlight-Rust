//! The background remover around its model (`BackgroundRemoverViewModel`,
//! `BackgroundRemovalModel`, `BackgroundRemoverWindow`'s saves): the models found in the
//! models folder, their names, the device, where "Save" writes. The inference itself is
//! `modlight_core::background_removal`.

use std::path::{Path, PathBuf};

use modlight_core::background_removal::RemovalDevice;
use modlight_core::culture::Culture;
use modlight_core::l10n::{t, tools as tl};

use crate::settings::BackgroundRemovalDevice;

/// `BackgroundRemovalModel`: one `.onnx` file of the models folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelFile {
    pub path: PathBuf,
    pub file_name: String,
    pub size: u64,
}

impl ModelFile {
    /// `DisplayName`: "bria-rmbg-2.0.onnx (976.9 MB)".
    pub fn display_name(&self) -> String {
        format!("{} ({})", self.file_name, format_file_size(self.size))
    }
}

/// `FormatFileSize`: B without decimals, then KB, MB, GB with up to one.
pub fn format_file_size(size: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = size as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        return format!("{value:.0} {}", UNITS[unit]);
    }
    // "0.#": one decimal when it is not zero, in the culture's separator.
    let rounded = (value * 10.0).round() / 10.0;
    let mut text = if rounded.fract() == 0.0 {
        format!("{rounded:.0}")
    } else {
        format!("{rounded:.1}")
    };
    let decimal = Culture::current().decimal;
    if decimal != '.' {
        text = text.replace('.', &decimal.to_string());
    }
    format!("{text} {}", UNITS[unit])
}

fn is_onnx(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("onnx"))
}

/// `RefreshModels`: the folder made if missing, its `.onnx` files by name.
pub fn list_models(folder: &Path) -> Vec<ModelFile> {
    let _ = std::fs::create_dir_all(folder);
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut models: Vec<ModelFile> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_onnx(p))
        .map(|path| ModelFile {
            file_name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            size: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
            path,
        })
        .collect();
    models.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    models
}

/// The saved model (by file name, any case), else the first.
pub fn select_model(models: &[ModelFile], saved: &str) -> Option<usize> {
    models
        .iter()
        .position(|m| m.file_name.eq_ignore_ascii_case(saved))
        .or(if models.is_empty() { None } else { Some(0) })
}

/// `HasBackgroundRemovalModel`: any `.onnx` in the folder counts, ours or not; a folder
/// that cannot be read counts as having one (the window then says what it finds).
pub fn has_model(folder: &Path) -> bool {
    if !folder.is_dir() {
        return false;
    }
    match std::fs::read_dir(folder) {
        Ok(entries) => entries.flatten().any(|e| is_onnx(&e.path())),
        Err(_) => true,
    }
}

/// The device the inference runs on.
pub fn removal_device(device: BackgroundRemovalDevice) -> RemovalDevice {
    match device {
        BackgroundRemovalDevice::Auto => RemovalDevice::Auto,
        BackgroundRemovalDevice::Gpu => RemovalDevice::Gpu,
        BackgroundRemovalDevice::Cpu => RemovalDevice::Cpu,
    }
}

/// `LocalizedDisplayConverter` for the devices.
pub fn device_name(device: BackgroundRemovalDevice) -> &'static str {
    t(match device {
        BackgroundRemovalDevice::Auto => tl::LocalizedDisplayConverter_Automatic,
        BackgroundRemovalDevice::Gpu => tl::LocalizedDisplayConverter_GPU,
        BackgroundRemovalDevice::Cpu => tl::LocalizedDisplayConverter_CPU,
    })
}

/// Where "Save" writes: over a PNG or WebP source, else wherever "Save as" asks.
pub fn save_over(source: Option<&Path>) -> Option<PathBuf> {
    let source = source?;
    let extension = source
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(extension.as_str(), "png" | "webp").then(|| source.to_path_buf())
}

/// "Save as"'s suggested name: `<name>-output.png`, or `image-output.png`.
pub fn suggested_name(source: Option<&Path>) -> String {
    match source.and_then(|s| s.file_stem()) {
        Some(stem) => format!("{}-output.png", stem.to_string_lossy()),
        None => "image-output.png".into(),
    }
}

/// `SaveImageToFile`: WebP for a `.webp` name (lossless here), PNG otherwise.
pub fn save_result(image: &image::RgbaImage, path: &Path) -> anyhow::Result<()> {
    let webp = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("webp"));
    let format = if webp {
        image::ImageFormat::WebP
    } else {
        image::ImageFormat::Png
    };
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(image.clone())
        .write_to(&mut std::io::Cursor::new(&mut bytes), format)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use modlight_core::culture::set_thread_culture;

    #[test]
    fn sizes() {
        set_thread_culture(Some(Culture::english()));
        assert_eq!(format_file_size(0), "0 B");
        assert_eq!(format_file_size(1023), "1023 B");
        assert_eq!(format_file_size(1024), "1 KB");
        assert_eq!(format_file_size(1536), "1.5 KB");
        assert_eq!(format_file_size(1_024_331_469), "976.9 MB");
        assert_eq!(format_file_size(5 * 1024 * 1024 * 1024 * 1024), "5120 GB");
        set_thread_culture(Some(Culture::polish()));
        assert_eq!(format_file_size(1536), "1,5 KB");
        set_thread_culture(None);
    }

    #[test]
    fn models_in_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("Models");
        assert!(!has_model(&folder));
        assert!(list_models(&folder).is_empty());
        assert!(folder.is_dir());
        std::fs::write(folder.join("b.onnx"), [0u8; 2048]).unwrap();
        std::fs::write(folder.join("A.ONNX"), [0u8; 10]).unwrap();
        std::fs::write(folder.join("notes.txt"), "x").unwrap();
        std::fs::create_dir(folder.join("dir.onnx")).unwrap();
        assert!(has_model(&folder));
        let models = list_models(&folder);
        let names: Vec<&str> = models.iter().map(|m| m.file_name.as_str()).collect();
        assert_eq!(names, ["A.ONNX", "b.onnx"]);
        set_thread_culture(Some(Culture::english()));
        assert_eq!(models[1].display_name(), "b.onnx (2 KB)");
        set_thread_culture(None);
        assert_eq!(select_model(&models, "B.onnx"), Some(1));
        assert_eq!(select_model(&models, "gone.onnx"), Some(0));
        assert_eq!(select_model(&[], "x"), None);
    }

    #[test]
    fn saves() {
        assert_eq!(
            save_over(Some(Path::new("/a/b.PNG"))),
            Some(PathBuf::from("/a/b.PNG"))
        );
        assert_eq!(save_over(Some(Path::new("/a/b.jpg"))), None);
        assert_eq!(save_over(None), None);
        assert_eq!(
            suggested_name(Some(Path::new("/a/photo.jpg"))),
            "photo-output.png"
        );
        assert_eq!(suggested_name(None), "image-output.png");
        let dir = tempfile::tempdir().unwrap();
        let mut image = image::RgbaImage::new(3, 2);
        image.put_pixel(1, 1, image::Rgba([10, 20, 30, 40]));
        for name in ["out.png", "out.webp"] {
            let path = dir.path().join(name);
            save_result(&image, &path).unwrap();
            let back = modlight_core::image_decode::open(&path).unwrap().to_rgba8();
            assert_eq!(back.get_pixel(1, 1).0, [10, 20, 30, 40], "{name}");
        }
        assert_eq!(
            removal_device(BackgroundRemovalDevice::Gpu),
            RemovalDevice::Gpu
        );
    }
}
